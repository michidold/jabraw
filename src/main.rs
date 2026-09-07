mod audio;
mod bluetooth;
mod gnp;
mod hid;
mod mpris;
mod tray;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ksni::TrayMethods;
use tokio::sync::mpsc;

use audio::Target;
use std::io::Write;
use hid::{Action, Msg};
use tray::{Cmd, HeadsetTray};

/// Was über Dongle und Headset bekannt ist. Beide hängen am selben
/// hidraw-Knoten und werden über die GNP-Zieladresse auseinandergehalten.
#[derive(Default, Clone, PartialEq)]
pub struct DeviceInfo {
    pub dongle_name: Option<String>,
    pub dongle_version: Option<String>,
    pub dongle_serial: Option<String>,
    pub headset_name: Option<String>,
    pub headset_version: Option<String>,
    pub headset_serial: Option<String>,
}

/// Offene GNP-Anfrage, über die Sequenznummer der Antwort zugeordnet.
enum Pending {
    Battery,
    Ident { dst: u8, sub: u8 },
}

/// Takt für Geräte-Rescan und Auffrischen der Tray-Anzeige.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Akkuabfrage alle 30 s, also jeden 15. Durchlauf.
const BATTERY_EVERY_N_TICKS: u32 = 15;
/// Ausgabegeräte alle 20 s neu einlesen; zusätzlich beim Öffnen des Menüs.
const SINKS_EVERY_N_TICKS: u32 = 10;
/// BlueZ alle 10 s befragen; der Akkustand über HFP ändert sich nur grob.
const BLUETOOTH_EVERY_N_TICKS: u32 = 5;

/// Kurzer Statusbericht als Desktop-Benachrichtigung.
async fn notify_status(conn: &zbus::Connection) {
    let device = hid::present();
    let state = audio::read_levels().await;
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

    notify(&body.join("\n")).await;
}

/// Desktop-Benachrichtigung, auch als Rückfall für den Infodialog.
async fn notify(body: &str) {
    let Ok(conn) = zbus::Connection::session().await else {
        return;
    };
    let proxy = match zbus::proxy::Builder::<zbus::Proxy>::new(&conn)
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
                "Jabraw",
                0u32,
                "audio-headset",
                "Jabraw",
                body,
                Vec::<String>::new(),
                std::collections::HashMap::<String, zbus::zvariant::Value>::new(),
                5000i32,
            ),
        )
        .await;
}

/// Kleiner Infodialog. Der Daemon bringt keine GUI mit, deshalb über das
/// Dialogwerkzeug des Desktops; ohne eines davon bleibt die Benachrichtigung.
async fn show_device_info(info: &DeviceInfo, battery: Option<u8>, charging: bool) {
    let line = |label: &str, v: &Option<String>| match v {
        Some(v) => format!("{label}: {v}\n"),
        None => String::new(),
    };
    let mut text = String::new();
    text.push_str("Headset\n");
    text.push_str(&line("  Modell", &info.headset_name));
    text.push_str(&line("  Firmware", &info.headset_version));
    text.push_str(&line("  Seriennummer", &info.headset_serial));
    if let Some(p) = battery {
        text.push_str(&format!(
            "  Akku: {p} %{}\n",
            if charging { " (lädt)" } else { "" }
        ));
    }
    // Ohne Dongle bleibt der Abschnitt leer; über Bluetooth kennt BlueZ nur
    // Name und Akkustand.
    if info.dongle_name.is_some() || info.dongle_version.is_some() {
        text.push_str("\nDongle\n");
        text.push_str(&line("  Modell", &info.dongle_name));
        text.push_str(&line("  Firmware", &info.dongle_version));
        text.push_str(&line("  Seriennummer", &info.dongle_serial));
    } else {
        text.push_str("\nÜber Bluetooth verbunden — Firmware und Seriennummer\n                       liefert nur der USB-Dongle.\n");
    }

    let attempts: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--info".into(),
                "--title=Jabraw".into(),
                "--no-wrap".into(),
                format!("--text={text}"),
            ],
        ),
        (
            "kdialog",
            vec!["--title".into(), "Jabraw".into(), "--msgbox".into(), text.clone()],
        ),
    ];
    for (bin, args) in attempts {
        match tokio::process::Command::new(bin).args(&args).status().await {
            Ok(_) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => eprintln!("{bin} nicht startbar: {e}"),
        }
    }
    notify(&text.replace('\n', " · ")).await;
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
    // BlueZ hängt am System-Bus. Fehlt er, bleibt nur der USB-Pfad.
    let system = zbus::Connection::system().await.ok();
    if system.is_none() {
        eprintln!("kein System-Bus erreichbar — Bluetooth-Geräte bleiben unsichtbar");
    }

    // Einzelinstanz-Sperre. Zwei Daemons lesen beide hidraw und schicken jeden
    // Tastendruck doppelt an MPRIS — Play unmittelbar gefolgt von Pause, also
    // sichtbar gar nichts. Der Name ist billiger und zuverlässiger als eine
    // PID-Datei, weil D-Bus ihn beim Prozessende selbst freigibt.
    match conn
        .request_name_with_flags(
            "io.github.michidold.Jabraw",
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
            eprintln!("jabraw läuft bereits — zeige Status");
            notify_status(&conn).await;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    let (hid_tx, mut hid_rx) = mpsc::unbounded_channel::<Msg>();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();

    // Schreibende Kopien der Geraete-Handles fuer GNP-Anfragen.
    let mut writers: HashMap<String, std::fs::File> = HashMap::new();
    let mut battery: Option<u8> = None;
    let mut charging = false;
    let mut info = DeviceInfo::default();
    let mut sinks: Vec<audio::Sink> = Vec::new();
    let mut bt: Option<bluetooth::BtDevice> = None;
    let mut seq: u8 = 0;
    // Ordnet Antworten den eigenen Anfragen zu; das Gerät sendet auf diesem
    // Kanal auch unaufgefordert.
    let mut pending: HashMap<u8, Pending> = HashMap::new();
    let mut ticks: u32 = 0;

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
            battery: None,
            charging: false,
            info: DeviceInfo::default(),
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
                    bt = None;
                    info = DeviceInfo::default();
                    if let Ok(mut w) = file.try_clone() {
                        // Gerätedaten ändern sich nicht; einmal beim Anstecken.
                        for (dst, sub) in [
                            (gnp::DST_DONGLE, gnp::SUB_NAME),
                            (gnp::DST_DONGLE, gnp::SUB_VERSION),
                            (gnp::DST_DONGLE, gnp::SUB_SERIAL),
                            (gnp::DST_HEADSET, gnp::SUB_NAME),
                            (gnp::DST_HEADSET, gnp::SUB_VERSION),
                            (gnp::DST_HEADSET, gnp::SUB_SERIAL),
                        ] {
                            seq = seq.wrapping_add(1);
                            let req = gnp::read_request(dst, seq, gnp::CMD_IDENT, sub);
                            if w.write_all(&req).is_ok() {
                                pending.insert(seq, Pending::Ident { dst, sub });
                            }
                        }
                        writers.insert(path.clone(), w);
                    }
                    watched.insert(path.clone());
                    names.insert(path.clone(), name);
                    hid::spawn_reader(path, file, hid_tx.clone());
                }

                // Akkustand seltener als die uebrige Anzeige abfragen; er
                // aendert sich in Prozentschritten.
                if ticks.is_multiple_of(BATTERY_EVERY_N_TICKS) {
                    seq = seq.wrapping_add(1);
                    let req = gnp::read_request(
                        gnp::DST_DONGLE,
                        seq,
                        gnp::CMD_STATUS,
                        gnp::SUB_HS_BATTERY,
                    );
                    for w in writers.values_mut() {
                        let _ = w.write_all(&req);
                    }
                    pending.insert(seq, Pending::Battery);
                }

                // Ohne Dongle gibt es keinen GNP-Kanal. Was BlueZ über ein
                // direkt gekoppeltes Headset weiß, ist dann alles, was bleibt:
                // Name und Akkustand. Die Tasten laufen in dem Betrieb über
                // AVRCP und werden vom Desktop schon an MPRIS gereicht.
                if watched.is_empty() {
                    if ticks.is_multiple_of(BLUETOOTH_EVERY_N_TICKS) {
                        bt = match &system {
                            Some(c) => bluetooth::connected_jabra(c).await,
                            None => None,
                        };
                    }
                    battery = bt.as_ref().and_then(|d| d.battery);
                    // BlueZ meldet über HFP nur den Füllstand, keinen Ladezustand.
                    charging = false;
                    info = match &bt {
                        Some(d) => DeviceInfo {
                            headset_name: Some(d.name.clone()),
                            ..DeviceInfo::default()
                        },
                        None => DeviceInfo::default(),
                    };
                }
                ticks = ticks.wrapping_add(1);
                if let Some(handle) = &tray {
                    // Der Akkustand gehört dem Headset, nicht dem Dongle —
                    // also auch den Namen des Headsets zeigen, sobald er über
                    // ident bekannt ist. Sonst der Name des USB-Geräts.
                    let device = info
                        .headset_name
                        .clone()
                        .or_else(|| names.values().next().cloned())
                        .or_else(|| bt.as_ref().map(|d| d.name.clone()));
                    let info_snapshot = info.clone();
                    if ticks.is_multiple_of(SINKS_EVERY_N_TICKS) || sinks.is_empty() {
                        sinks = audio::list_sinks().await;
                    }
                    let mut audio_state = audio::read_levels().await;
                    audio_state.sinks = sinks.clone();
                    let player = mpris::active_player(&conn).await;
                    handle.update(move |t: &mut HeadsetTray| {
                        t.device = device;
                        t.audio = audio_state;
                        t.player = player;
                        t.battery = battery;
                        t.charging = charging;
                        t.info = info_snapshot;
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
                Cmd::ShowInfo => show_device_info(&info, battery, charging).await,
                // Beim Öffnen des Menüs die Geräteliste auffrischen, damit das
                // seltene Abfrageintervall nicht zu veralteten Einträgen führt.
                Cmd::Refresh => sinks = audio::list_sinks().await,
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
                    writers.remove(&path);
                    battery = None;
                    charging = false;
                    info = DeviceInfo::default();
                    watched.remove(&path);
                    names.remove(&path);
                    state.retain(|(p, _), _| *p != path);
                }
                Msg::Report(path, data) => {
                    if data.is_empty() {
                        continue;
                    }
                    let report = data[0];
                    if report == gnp::REPORT_ID {
                        if let Some(r) = gnp::parse(&data) {
                            match pending.remove(&r.seq) {
                                Some(Pending::Battery)
                                    if r.cmd == gnp::CMD_STATUS
                                        && r.sub == gnp::SUB_HS_BATTERY =>
                                {
                                    battery = gnp::battery_percent(r.data);
                                    charging = gnp::battery_charging(r.data);
                                    if debug {
                                        println!(
                                            "  Akku: {battery:?} % lädt={charging}"
                                        );
                                    }
                                }
                                Some(Pending::Ident { dst, sub })
                                    if r.cmd == gnp::CMD_IDENT
                                        && r.sub == sub
                                        && r.src == dst =>
                                {
                                    if let Some(t) = gnp::text(r.data) {
                                        let field = match (dst, sub) {
                                            (gnp::DST_DONGLE, gnp::SUB_NAME) => &mut info.dongle_name,
                                            (gnp::DST_DONGLE, gnp::SUB_VERSION) => &mut info.dongle_version,
                                            (gnp::DST_DONGLE, gnp::SUB_SERIAL) => &mut info.dongle_serial,
                                            (_, gnp::SUB_NAME) => &mut info.headset_name,
                                            (_, gnp::SUB_VERSION) => &mut info.headset_version,
                                            (_, _) => &mut info.headset_serial,
                                        };
                                        if debug {
                                            println!("  ident dst={dst:#04x} sub={sub}: {t}");
                                        }
                                        *field = Some(t);
                                    }
                                }
                                _ => {}
                            }
                        }
                        // Unbeantwortetes sammelt sich sonst bis zum Überlauf
                        // der Sequenznummer an.
                        if pending.len() > 64 {
                            pending.clear();
                        }
                        continue;
                    }
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
