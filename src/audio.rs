//! Volume, mute and output device.
//!
//! Driven through the CLI of whichever audio pipeline is running: `wpctl` under
//! PipeWire/WirePlumber, `pactl` under PulseAudio. That avoids binding to
//! libpulse and works on both stacks.

use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

/// Ceiling for the helpers. `wpctl` answers in milliseconds and `pw-dump` in
/// tens of them; one that hangs would otherwise stop the whole loop, since the
/// tick waits for it.
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sink {
    pub id: u32,
    pub description: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioState {
    pub sink_muted: bool,
    pub sink_volume: f32,
    pub source_muted: bool,
    pub source_volume: f32,
    pub sinks: Vec<Sink>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    Sink,
    Source,
}

impl Target {
    fn wpctl(self) -> &'static str {
        match self {
            Target::Sink => "@DEFAULT_AUDIO_SINK@",
            Target::Source => "@DEFAULT_AUDIO_SOURCE@",
        }
    }
    fn pactl_id(self) -> &'static str {
        match self {
            Target::Sink => "@DEFAULT_SINK@",
            Target::Source => "@DEFAULT_SOURCE@",
        }
    }
    fn pactl_mute(self) -> &'static str {
        match self {
            Target::Sink => "set-sink-mute",
            Target::Source => "set-source-mute",
        }
    }
    fn pactl_volume(self) -> &'static str {
        match self {
            Target::Sink => "set-sink-volume",
            Target::Source => "set-source-volume",
        }
    }
}

async fn output(bin: &str, args: &[&str]) -> Option<String> {
    let run = tokio::process::Command::new(bin)
        .args(args)
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, run).await.ok()?.ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn run(bin: &str, args: &[&str]) -> bool {
    let status = tokio::process::Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    matches!(
        tokio::time::timeout(TIMEOUT, status).await,
        Ok(Ok(s)) if s.success()
    )
}

/// `wpctl get-volume` answers "Volume: 0.65" or "Volume: 0.65 [MUTED]".
fn parse_wpctl_volume(s: &str) -> (f32, bool) {
    let muted = s.contains("[MUTED]");
    let vol = s
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0);
    (vol, muted)
}

async fn read_target(target: Target) -> (f32, bool) {
    if let Some(s) = output("wpctl", &["get-volume", target.wpctl()]).await {
        return parse_wpctl_volume(&s);
    }
    // PulseAudio: volume and mute come from separate queries.
    let muted = output(
        "pactl",
        &[
            match target {
                Target::Sink => "get-sink-mute",
                Target::Source => "get-source-mute",
            },
            target.pactl_id(),
        ],
    )
    .await
    .map(|s| s.contains("yes"))
    .unwrap_or(false);
    let vol = output(
        "pactl",
        &[
            match target {
                Target::Sink => "get-sink-volume",
                Target::Source => "get-source-volume",
            },
            target.pactl_id(),
        ],
    )
    .await
    .and_then(|s| {
        s.split('%')
            .next()
            .and_then(|p| p.rsplit(' ').next().map(str::to_string))
    })
    .and_then(|p| p.trim().parse::<f32>().ok())
    .map(|p| p / 100.0)
    .unwrap_or(0.0);
    (vol, muted)
}

/// Output devices from `pw-dump`. The default sink is named in the metadata
/// object under `default.audio.sink` and matched by node name.
pub async fn list_sinks() -> Vec<Sink> {
    let Some(json) = output("pw-dump", &[]).await else {
        return vec![];
    };
    let Ok(root) = serde_json::from_str::<Value>(&json) else {
        return vec![];
    };
    let Some(objects) = root.as_array() else {
        return vec![];
    };

    let default_name = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Metadata")
        .filter_map(|o| o["metadata"].as_array())
        .flatten()
        .find(|m| m["key"] == "default.audio.sink")
        .and_then(|m| m["value"]["name"].as_str())
        .map(str::to_string);

    let mut sinks = Vec::new();
    for o in objects {
        let props = &o["info"]["props"];
        if props["media.class"] != "Audio/Sink" {
            continue;
        }
        let Some(id) = o["id"].as_u64() else { continue };
        let name = props["node.name"].as_str().unwrap_or_default();
        let description = props["node.description"]
            .as_str()
            .or_else(|| props["node.nick"].as_str())
            .unwrap_or(name)
            .to_string();
        sinks.push(Sink {
            id: id as u32,
            description,
            is_default: Some(name) == default_name.as_deref(),
        });
    }
    sinks.sort_by(|a, b| a.description.cmp(&b.description));
    sinks
}

/// Volume and mute. Cheap enough for the display tick: two `wpctl` calls at
/// roughly 3 ms each.
///
/// The device list is deliberately not part of this. It comes from `pw-dump`,
/// which serialises the entire PipeWire graph (some 500 KB, 45 ms) and put
/// measurable load on a two-second tick, while output devices barely change and
/// the menu is usually closed.
pub async fn read_levels() -> AudioState {
    let (sink_volume, sink_muted) = read_target(Target::Sink).await;
    let (source_volume, source_muted) = read_target(Target::Source).await;
    AudioState {
        sink_muted,
        sink_volume,
        source_muted,
        source_volume,
        sinks: Vec::new(),
    }
}

pub async fn toggle_mute(target: Target) {
    if run("wpctl", &["set-mute", target.wpctl(), "toggle"]).await {
        return;
    }
    if run("pactl", &[target.pactl_mute(), target.pactl_id(), "toggle"]).await {
        return;
    }
    eprintln!("Stummschaltung fehlgeschlagen: weder wpctl noch pactl verfügbar");
}

/// `delta` in percentage points, positive or negative.
pub async fn change_volume(target: Target, delta: i32) {
    let step = format!("{}%{}", delta.abs(), if delta < 0 { "-" } else { "+" });
    if run("wpctl", &["set-volume", "-l", "1.5", target.wpctl(), &step]).await {
        return;
    }
    let pa_step = format!("{}{}%", if delta < 0 { "-" } else { "+" }, delta.abs());
    if run(
        "pactl",
        &[target.pactl_volume(), target.pactl_id(), &pa_step],
    )
    .await
    {
        return;
    }
    eprintln!("Lautstärke fehlgeschlagen: weder wpctl noch pactl verfügbar");
}

pub async fn set_default_sink(id: u32) {
    let id = id.to_string();
    if run("wpctl", &["set-default", &id]).await {
        return;
    }
    if run("pactl", &["set-default-sink", &id]).await {
        return;
    }
    eprintln!("Ausgabegerät setzen fehlgeschlagen");
}
