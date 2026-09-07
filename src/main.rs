mod audio;
mod bluetooth;
mod config;
mod gnp;
mod hid;
mod i18n;
mod icon;
mod mpris;
mod rfcomm;
mod tray;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ksni::TrayMethods;
use tokio::sync::mpsc;

use audio::Target;
use i18n::strings;
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

    let s = strings();
    let level = |v: f32, muted: bool| {
        let p = (v * 100.0).round() as i32;
        if muted {
            format!("{p} % ({})", s.muted_suffix)
        } else {
            format!("{p} %")
        }
    };
    let mut body = vec![match &device {
        Some(d) => format!("{d}: {}", s.connected),
        None => s.none_found.to_string(),
    }];
    body.push(format!("{}: {}", s.speaker, level(state.sink_volume, state.sink_muted)));
    body.push(format!("{}: {}", s.microphone, level(state.source_volume, state.source_muted)));
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

/// Jahr der ersten Veröffentlichung; ein mitlaufendes Jahr wäre für einen
/// Urheberrechtsvermerk falsch.
const COPYRIGHT_YEAR: &str = "2026";

/// Kleiner Infodialog. Der Daemon bringt keine GUI mit, deshalb über das
/// Dialogwerkzeug des Desktops; ohne eines davon bleibt die Benachrichtigung.
async fn show_device_info(info: &DeviceInfo, battery: Option<u8>, charging: bool) {
    let line = |label: &str, v: &Option<String>| match v {
        Some(v) => format!("{label}: {v}\n"),
        None => String::new(),
    };
    let s = strings();
    let mut text = String::new();
    text.push_str(&format!("{}\n", s.headset));
    text.push_str(&line(&format!("  {}", s.model), &info.headset_name));
    text.push_str(&line(&format!("  {}", s.firmware), &info.headset_version));
    text.push_str(&line(&format!("  {}", s.serial), &info.headset_serial));
    if let Some(p) = battery {
        text.push_str(&format!(
            "  {}: {p} %{}\n",
            s.battery,
            if charging {
                format!(" ({})", s.charging_suffix)
            } else {
                String::new()
            }
        ));
    }
    // Ohne Dongle bleibt der Abschnitt leer; über Bluetooth kennt BlueZ nur
    // Name und Akkustand.
    if info.dongle_name.is_some() || info.dongle_version.is_some() {
        text.push_str(&format!("\n{}\n", s.dongle));
        text.push_str(&line(&format!("  {}", s.model), &info.dongle_name));
        text.push_str(&line(&format!("  {}", s.firmware), &info.dongle_version));
        text.push_str(&line(&format!("  {}", s.serial), &info.dongle_serial));
    } else if info.headset_version.is_none() {
        // Nur wenn gar kein GNP-Kanal steht; mit RFCOMM liefert auch Bluetooth
        // Firmware, Seriennummer und Ladezustand.
        text.push_str(&format!("\n{}\n", s.bluetooth_note));
    }
    // Eigene Version mit anzeigen: sonst ist von außen nicht erkennbar, welcher
    // Stand tatsächlich läuft, wenn Autostart ein älteres Paket startet.
    // Urheber und Lizenz kommen aus Cargo.toml, damit beides nicht auseinander
    // läuft; die Repository-Adresse macht die Lizenzangabe nachschlagbar.
    let author = env!("CARGO_PKG_AUTHORS")
        .split('<')
        .next()
        .unwrap_or("")
        .trim();
    text.push_str(&format!(
        "\nJabraw {}\n© {COPYRIGHT_YEAR} {author} · {}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_LICENSE"),
        env!("CARGO_PKG_REPOSITORY"),
    ));

    let attempts: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--info".into(),
                "--title=Jabraw".into(),
                // Dasselbe Symbol wie im Tray, sofern das Paket installiert ist.
                "--icon=jabraw".into(),
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

async fn config_via_hidraw(path: String) -> Vec<config::Setting> {
    match tokio::task::spawn_blocking(move || config::read_all(&path)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            eprintln!("reading settings failed: {e}");
            Vec::new()
        }
        Err(e) => {
            eprintln!("settings task failed: {e}");
            Vec::new()
        }
    }
}

/// Übersicht der Geräteeinstellungen als Tabelle.
async fn show_settings(settings: Vec<config::Setting>) {
    let s = strings();
    if settings.is_empty() {
        notify(s.no_settings).await;
        return;
    }

    // Kein --icon: zenity kennt die Option nur beim Info-Dialog und lehnt den
    // Aufruf mit --list andernfalls ab.
    let mut args = vec![
        "--list".to_string(),
        "--title=Jabraw".to_string(),
        format!("--text={}", s.settings.trim_end_matches(" …")),
        "--width=520".to_string(),
        "--height=560".to_string(),
        // Kein leerer Spaltenname: zenity 4 kehrt dann sofort mit der ersten
        // Zeile zurueck, statt den Dialog anzuzeigen.
        "--column".to_string(),
        s.device_col.to_string(),
        "--column".to_string(),
        s.setting.to_string(),
        "--column".to_string(),
        s.value.to_string(),
    ];
    for item in &settings {
        args.push(item.device.to_string());
        args.push(item.name.to_string());
        args.push(format_value(&item.value));
    }
    match tokio::process::Command::new("zenity").args(&args).status().await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let text: Vec<String> = settings
                .iter()
                .map(|i| format!("{} {}: {}", i.device, i.name, format_value(&i.value)))
                .collect();
            notify(&text.join("\n")).await;
        }
        Err(e) => eprintln!("zenity failed: {e}"),
    }
}

/// Einzelbytes 0 und 1 sind durchweg Schalter; alles andere bleibt roh, weil
/// die Bedeutung je Einstellung anders und undokumentiert ist.
fn format_value(data: &[u8]) -> String {
    let s = strings();
    match data {
        [] => "—".to_string(),
        [0] => s.off.to_string(),
        [1] => s.on.to_string(),
        [v] => v.to_string(),
        _ => data
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    // Vom Bauskript genutzt, um die Symbole für die Anwendungsübersicht zu
    // erzeugen; dieselbe Zeichnung wie im Tray.
    if let Some(dir) = arg_value("--write-icons") {
        for size in [16u32, 24, 32, 48, 64, 128, 256] {
            let path = format!("{dir}/{size}x{size}.png");
            std::fs::write(&path, icon::png(size))?;
            println!("{path}");
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("jabraw {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
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
    let mut bt_path: Option<String> = None;
    let mut session: Option<rfcomm::Session> = None;
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
            has_gnp: false,
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
                        bt_path = bt.as_ref().map(|d| d.path.clone());
                    }
                    battery = bt.as_ref().and_then(|d| d.battery);
                    // HFP kennt keinen Ladezustand; über RFCOMM kommt er weiter
                    // unten aus der GNP-Antwort.
                    charging = false;
                    info = match &bt {
                        Some(d) => DeviceInfo {
                            headset_name: Some(d.name.clone()),
                            ..DeviceInfo::default()
                        },
                        None => DeviceInfo::default(),
                    };

                    // GNP über RFCOMM liefert genauere Werte als der grobe
                    // HFP-Indikator, dazu Firmware und Seriennummer.
                    if bt.is_none() {
                        session = None;
                    } else if session.is_none() {
                        if let (Some(c), Some(p)) = (&system, &bt_path) {
                            session = rfcomm::connect(c, p).await;
                            if session.is_some() {
                                println!("GNP over RFCOMM established");
                            }
                        }
                    }
                    if let Some(mut s) = session.take() {
                        let out = tokio::task::spawn_blocking(move || {
                            let d = gnp::DST_HEADSET;
                            let name = s.read(d, gnp::CMD_IDENT, gnp::SUB_NAME);
                            let ver = s.read(d, gnp::CMD_IDENT, gnp::SUB_VERSION);
                            let ser = s.read(d, gnp::CMD_IDENT, gnp::SUB_SERIAL);
                            let bat = s.read(d, gnp::CMD_STATUS, gnp::SUB_HS_BATTERY);
                            (s, name, ver, ser, bat)
                        })
                        .await;
                        if let Ok((s, name, ver, ser, bat)) = out {
                            let any = name.is_some() || bat.is_some();
                            if let Some(v) = name.as_deref().and_then(gnp::text) {
                                info.headset_name = Some(v);
                            }
                            if let Some(v) = ver.as_deref().and_then(gnp::text) {
                                info.headset_version = Some(v);
                            }
                            if let Some(v) = ser.as_deref().and_then(gnp::text) {
                                info.headset_serial = Some(v);
                            }
                            if let Some(d) = bat.as_deref() {
                                if let Some(p) = gnp::battery_percent(d) {
                                    battery = Some(p);
                                }
                                charging = gnp::battery_charging(d);
                            }
                            // Bleibt alles stumm, ist die Verbindung tot.
                            session = any.then_some(s);
                        }
                    }
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
                    let has_gnp = !writers.is_empty() || session.is_some();
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
                        t.has_gnp = has_gnp;
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
                Cmd::ShowSettings => {
                    // Über hidraw, wenn ein Dongle steckt; sonst über die
                    // bestehende RFCOMM-Sitzung.
                    if let Some(path) = watched.iter().next().cloned() {
                        show_settings(config_via_hidraw(path).await).await;
                    } else if let Some(mut s) = session.take() {
                        let out = tokio::task::spawn_blocking(move || {
                            let rows = s.sweep(
                                gnp::DST_HEADSET,
                                gnp::CMD_CONFIG,
                                config::SETTINGS,
                            );
                            (s, rows)
                        })
                        .await;
                        if let Ok((s, rows)) = out {
                            session = Some(s);
                            show_settings(config::from_sweep(rows)).await;
                        }
                    }
                }
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
