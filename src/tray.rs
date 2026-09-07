//! Tray-Menü als StatusNotifierItem.
//!
//! SNI statt einer Desktop-spezifischen Erweiterung, damit dasselbe Binary
//! unter GNOME (mit AppIndicator-Erweiterung), KDE und den Tray-fähigen
//! wlroots-Panels läuft.

use tokio::sync::mpsc::UnboundedSender;

use crate::audio::{AudioState, Sink};
use crate::mpris::PlayerInfo;
use crate::i18n::strings;
use crate::DeviceInfo;

/// Vom Menü ausgelöste Wünsche. Die Callbacks laufen im Tray-Task und dürfen
/// nicht blockieren, deshalb wird die eigentliche Arbeit an die Hauptschleife
/// abgegeben.
pub enum Cmd {
    PlayPause,
    Previous,
    Next,
    ToggleSinkMute,
    ToggleSourceMute,
    Volume(i32),
    ShowInfo,
    ShowSettings,
    Refresh,
    SetSink(u32),
    Quit,
}

pub struct HeadsetTray {
    pub device: Option<String>,
    pub audio: AudioState,
    pub player: Option<PlayerInfo>,
    pub battery: Option<u8>,
    /// Nur über den Dongle bekannt; Bluetooth liefert keinen Ladezustand.
    pub charging: bool,
    pub info: DeviceInfo,
    /// Nur mit Dongle: über Bluetooth gibt es keinen GNP-Kanal.
    pub has_gnp: bool,
    pub tx: UnboundedSender<Cmd>,
}

impl HeadsetTray {
    fn send(&self, cmd: Cmd) {
        // Bricht nur ab, wenn die Hauptschleife schon beendet ist.
        let _ = self.tx.send(cmd);
    }
}

fn percent(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

fn battery_text(level: u8, charging: bool) -> String {
    let s = strings();
    if charging {
        format!("{level} % ({})", s.charging_suffix)
    } else {
        format!("{level} %")
    }
}

impl ksni::Tray for HeadsetTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        match (&self.device, self.battery) {
            (Some(d), Some(p)) => format!("{d} — {}", battery_text(p, self.charging)),
            (Some(d), None) => d.clone(),
            (None, _) => strings().no_device.into(),
        }
    }

    fn icon_name(&self) -> String {
        if self.device.is_none() {
            "audio-headset-symbolic".into()
        } else if self.audio.source_muted {
            "microphone-disabled-symbolic".into()
        } else {
            "audio-headset-symbolic".into()
        }
    }

    fn status(&self) -> ksni::Status {
        match self.device {
            Some(_) => ksni::Status::Active,
            None => ksni::Status::Passive,
        }
    }

    fn menu_about_to_show(&mut self) {
        self.send(Cmd::Refresh);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let s = strings();
        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();

        items.push(
            StandardItem {
                label: match (&self.device, self.battery) {
                    (Some(d), Some(p)) => {
                        format!("{d} — {} {}", s.battery, battery_text(p, self.charging))
                    }
                    (Some(d), None) => format!("{d} — {}", s.connected),
                    (None, _) => s.not_connected.into(),
                },
                enabled: false,
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        match &self.player {
            Some(p) => {
                let line = match &p.track {
                    Some(t) => format!("{} — {t}", p.identity),
                    None => p.identity.clone(),
                };
                items.push(
                    StandardItem {
                        label: line,
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
                items.push(
                    StandardItem {
                        label: if p.status == "Playing" {
                            s.pause.into()
                        } else {
                            s.play.into()
                        },
                        activate: Box::new(|this: &mut Self| this.send(Cmd::PlayPause)),
                        ..Default::default()
                    }
                    .into(),
                );
                items.push(
                    StandardItem {
                        label: s.previous.into(),
                        activate: Box::new(|this: &mut Self| this.send(Cmd::Previous)),
                        ..Default::default()
                    }
                    .into(),
                );
                items.push(
                    StandardItem {
                        label: s.next.into(),
                        activate: Box::new(|this: &mut Self| this.send(Cmd::Next)),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            None => items.push(
                StandardItem {
                    label: s.no_player.into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
        }
        items.push(MenuItem::Separator);

        items.push(
            CheckmarkItem {
                label: format!("{} ({})", s.speaker_muted, percent(self.audio.sink_volume)),
                checked: self.audio.sink_muted,
                activate: Box::new(|this: &mut Self| this.send(Cmd::ToggleSinkMute)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: s.louder.into(),
                activate: Box::new(|this: &mut Self| this.send(Cmd::Volume(5))),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: s.quieter.into(),
                activate: Box::new(|this: &mut Self| this.send(Cmd::Volume(-5))),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            CheckmarkItem {
                label: format!("{} ({})", s.mic_muted, percent(self.audio.source_volume)),
                checked: self.audio.source_muted,
                activate: Box::new(|this: &mut Self| this.send(Cmd::ToggleSourceMute)),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        let sinks: Vec<Sink> = self.audio.sinks.clone();
        let selected = sinks.iter().position(|s| s.is_default).unwrap_or(0);
        items.push(
            SubMenu {
                label: s.output_device.into(),
                enabled: !sinks.is_empty(),
                submenu: vec![RadioGroup {
                    selected,
                    select: Box::new(move |this: &mut Self, index| {
                        if let Some(sink) = this.audio.sinks.get(index) {
                            let id = sink.id;
                            this.send(Cmd::SetSink(id));
                        }
                    }),
                    options: sinks
                        .iter()
                        .map(|s| RadioItem {
                            label: s.description.clone(),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into()],
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: s.device_info.into(),
                enabled: self.device.is_some(),
                activate: Box::new(|this: &mut Self| this.send(Cmd::ShowInfo)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: s.settings.into(),
                // GNP gibt es über den Dongle wie über Bluetooth.
                enabled: self.has_gnp,
                activate: Box::new(|this: &mut Self| this.send(Cmd::ShowSettings)),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: s.quit.into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| this.send(Cmd::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}
