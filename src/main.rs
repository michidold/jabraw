mod audio;
mod hid;
mod mpris;
mod tray;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ksni::TrayMethods;
use tokio::sync::mpsc;

use audio::Target;
use hid::{Action, Msg};
use tray::{Cmd, HeadsetTray};

/// Takt für Geräte-Rescan und Auffrischen der Tray-Anzeige.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Kurzer Statusbericht als Desktop-Benachrichtigung.
async fn notify_status(conn: &zbus::Connection) {
    let device = hid::present();
    let state = audio::read_state().await;
    let player = mpris::active_player(conn).await;

    let mut body = vec![match &device {
        Some(d) => format!("{d}: verbunden"),
        None => "Kein Jabra-Gerät gefunden".to_string(),
    }];
    body.push(format!(
        "Lautsprecher {}{}",
        (state.sink_volume * 100.0).round() as i32,
        if state.sink_muted { "% (stumm)" } else { "%" }
    ));
    body.push(format!(
        "Mikrofon {}{}",
        (state.source_volume * 100.0).round() as i32,
        if state.source_muted { "% (stumm)" } else { "%" }
    ));
    if let Some(p) = player {
        body.push(match p.track {
            Some(t) => format!("{}: {t}", p.identity),
            None => format!("{}: {}", p.identity, p.status),
        });
    }

    let proxy = match zbus::proxy::Builder::<zbus::Proxy>::new(conn)
        .destination("org.freedesktop.Notifications")
        .and_then(|b| b.path("/org/freedesktop/Notifications"))
        .and_then(|b| b.interface("org.freedesktop.Notifications"))
    {
        Ok(b) => match b.build().await {
            Ok(p) => p,
            Err(e) => return eprintln!("Benachrichtigung nicht möglich: {e}"),
        },
        Err(e) => return eprintln!("Benachrichtigung nicht möglich: {e}"),
    };
    let _: Result<u32, _> = proxy
        .call(
            "Notify",
            &(
                "Jabra Headset",
                0u32,
                "audio-headset",
                "Jabra Headset",
                body.join("\n"),
                Vec::<String>::new(),
                std::collections::HashMap::<String, zbus::zvariant::Value>::new(),
                5000i32,
            ),
        )
        .await;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    let debug = std::env::args().any(|a| a == "--debug");
    // Monotone Laufzeit statt Uhrzeit: gemessen werden Abstände, nicht Termine.
    let started = std::time::Instant::now();
    let no_tray = std::env::args().any(|a| a == "--no-tray");
    let conn = zbus::Connection::session().await?;

    // Einzelinstanz-Sperre. Zwei Daemons lesen beide hidraw und schicken jeden
    // Tastendruck doppelt an MPRIS — Play unmittelbar gefolgt von Pause, also
    // sichtbar gar nichts. Der Name ist billiger und zuverlässiger als eine
    // PID-Datei, weil D-Bus ihn beim Prozessende selbst freigibt.
    match conn
        .request_name_with_flags(
            "io.github.michidold.JabraMediaDaemon",
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await
    {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {}
        // Bei DoNotQueue meldet zbus einen belegten Namen als Fehler, nicht als
        // Rückgabewert. Sauberes Ende mit 0, damit Autostart und systemd das
        // nicht als Absturz werten und neu starten.
        Ok(_) | Err(zbus::Error::NameTaken) => {
            // Aufruf über den Menüeintrag bei schon laufendem Daemon: statt
            // wortlos zu enden, den aktuellen Zustand melden.
            eprintln!("jabra-media-daemon läuft bereits — zeige Status");
            notify_status(&conn).await;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    let (hid_tx, mut hid_rx) = mpsc::unbounded_channel::<Msg>();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();

    let mut watched: HashSet<String> = HashSet::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut warned: HashSet<String> = HashSet::new();
    // Vorheriger Bitzustand je (Gerät, Report), um Flanken zu erkennen.
    let mut state: HashMap<(String, u8), u32> = HashMap::new();

    let tray = if no_tray {
        None
    } else {
        match (HeadsetTray {
            device: None,
            audio: audio::AudioState::default(),
            player: None,
            tx: cmd_tx.clone(),
        })
        .spawn()
        .await
        {
            Ok(h) => Some(h),
            Err(e) => {
                // Ohne SNI-Host (etwa GNOME ohne AppIndicator-Erweiterung) bleibt
                // die Tastensteuerung trotzdem nutzbar.
                eprintln!("Tray nicht verfügbar, laufe ohne Menü: {e}");
                None
            }
        }
    };

    let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                for (path, name, file) in hid::scan(&watched, &mut warned) {
                    println!("überwache {path} ({name})");
                    watched.insert(path.clone());
                    names.insert(path.clone(), name);
                    hid::spawn_reader(path, file, hid_tx.clone());
                }
                if let Some(handle) = &tray {
                    let device = names.values().next().cloned();
                    let audio_state = audio::read_state().await;
                    let player = mpris::active_player(&conn).await;
                    handle.update(move |t: &mut HeadsetTray| {
                        t.device = device;
                        t.audio = audio_state;
                        t.player = player;
                    }).await;
                }
            }

            Some(cmd) = cmd_rx.recv() => match cmd {
                Cmd::PlayPause => mpris::dispatch(&conn, "PlayPause").await,
                Cmd::Previous => mpris::dispatch(&conn, "Previous").await,
                Cmd::Next => mpris::dispatch(&conn, "Next").await,
                Cmd::ToggleSinkMute => audio::toggle_mute(Target::Sink).await,
                Cmd::ToggleSourceMute => audio::toggle_mute(Target::Source).await,
                Cmd::Volume(delta) => audio::change_volume(Target::Sink, delta).await,
                Cmd::SetSink(id) => audio::set_default_sink(id).await,
                Cmd::Quit => {
                    if let Some(handle) = &tray {
                        handle.shutdown().await;
                    }
                    return Ok(());
                }
            },

            Some(msg) = hid_rx.recv() => match msg {
                Msg::Closed(path) => {
                    println!("{path} nicht mehr verfügbar");
                    watched.remove(&path);
                    names.remove(&path);
                    state.retain(|(p, _), _| *p != path);
                }
                Msg::Report(path, data) => {
                    if data.is_empty() {
                        continue;
                    }
                    let report = data[0];
                    let bits = hid::payload_bits(&data);
                    let prev = state.entry((path.clone(), report)).or_insert(0);
                    let changed = bits ^ *prev;
                    *prev = bits;
                    if debug {
                        let hex: Vec<String> = data.iter().map(|b| format!("{b:02x}")).collect();
                        println!(
                            "[+{:.3}s] {path} Report {report}: {}",
                            started.elapsed().as_secs_f64(),
                            hex.join(" ")
                        );
                    }
                    for bit in 0..32 {
                        if changed >> bit & 1 == 0 {
                            continue;
                        }
                        let on = bits >> bit & 1 == 1;
                        let name = hid::bit_name(report, bit);
                        match hid::action_for(report, bit) {
                            Some(Action::Mpris(method)) if on => {
                                if debug {
                                    println!("  bit {bit} ({name}) -> {method}");
                                }
                                mpris::dispatch(&conn, method).await;
                            }
                            _ => {
                                if debug {
                                    println!("  bit {bit} ({name}) = {} — nicht belegt", u8::from(on));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
