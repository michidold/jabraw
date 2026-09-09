//! The tray menu as a StatusNotifierItem.
//!
//! SNI rather than a desktop-specific extension, so one binary serves GNOME
//! (with the AppIndicator extension), KDE and the tray-capable wlroots
//! panels.

use tokio::sync::mpsc::Sender;

use crate::audio::{AudioState, Sink};
use crate::i18n::strings;
use crate::mpris::PlayerInfo;
use crate::DeviceInfo;

/// Requests raised from the menu. The callbacks run in the tray task and must
/// not block, so the actual work is handed to the main loop.
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
    /// Known over GNP only; the HFP indicator carries no charging state.
    pub charging: bool,
    pub info: DeviceInfo,
    /// Whether a GNP channel exists, over the dongle or over Bluetooth.
    pub has_gnp: bool,
    pub tx: Sender<Cmd>,
}

impl HeadsetTray {
    fn send(&self, cmd: Cmd) {
        // Fails once the main loop has ended, and on a full queue — which
        // takes more menu clicks than a hand manages while one is pending.
        let _ = self.tx.try_send(cmd);
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

    // No `icon_name`: GNOME's AppIndicator prefers a set name over the pixmap
    // and then showed the icon theme's own rather than this one. Without a
    // name only the pixmap is left.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        crate::icon::pixmaps()
            .iter()
            .map(|(size, data)| ksni::Icon {
                width: *size as i32,
                height: *size as i32,
                data: data.clone(),
            })
            .collect()
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
                // GNP runs over the dongle and over Bluetooth alike.
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
