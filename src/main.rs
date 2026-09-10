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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ksni::TrayMethods;
use tokio::sync::mpsc;

use audio::Target;
use hid::{Action, Msg};
use i18n::strings;
use std::io::Write;
use tray::{Change, Cmd, HeadsetTray};

/// What is known about dongle and headset. Both sit behind the same hidraw
/// node and are told apart by the GNP destination address.
#[derive(Default, Clone, PartialEq)]
pub struct DeviceInfo {
    pub dongle_name: Option<String>,
    pub dongle_version: Option<String>,
    pub dongle_serial: Option<String>,
    pub headset_name: Option<String>,
    pub headset_version: Option<String>,
    pub headset_serial: Option<String>,
}

/// An outstanding GNP request, matched to its reply by sequence number.
enum Pending {
    Battery,
    Ident { dst: u8, sub: u8 },
}

/// Work on the RFCOMM session. Reading blocks for as long as the headset takes
/// to answer, up to 900 ms per request, so the session travels to a task and
/// back rather than the loop waiting for it.
enum BtJob {
    Ident,
    Battery,
    /// `edit` says whether the table or the form follows.
    Settings {
        edit: bool,
    },
    /// Values on their way to the device, each with the sentence to report.
    Write(Vec<(u8, &'static config::Def, u8, String)>),
}

enum BtData {
    Connected,
    Ident {
        name: Option<Vec<u8>>,
        ver: Option<Vec<u8>>,
        ser: Option<Vec<u8>>,
    },
    Battery(Option<Vec<u8>>),
    Settings(Vec<(&'static str, Vec<u8>)>, bool),
    Written(Vec<(String, Option<gnp::Ack>)>),
}

struct BtResult {
    /// BlueZ path the job was started for. The headset can change while it
    /// runs, and the answer then belongs to nobody.
    device: String,
    /// Handed back unless the link went silent, which ends the session.
    session: Option<rfcomm::Session>,
    data: BtData,
}

/// Tick for device rescan and refreshing the tray.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Battery every 30 s, that is every 15th tick.
const BATTERY_EVERY_N_TICKS: u32 = 15;
/// Output devices every 20 s, and again when the menu opens.
const SINKS_EVERY_N_TICKS: u32 = 10;
/// Ask BlueZ every 10 s; the HFP charge level only moves in coarse steps.
const BLUETOOTH_EVERY_N_TICKS: u32 = 5;
/// Longest wait between two RFCOMM connect attempts, in ticks: five minutes.
const BLUETOOTH_RETRY_MAX: u32 = 150;

/// Queue depth of the two channels. Bounded on purpose: the reader thread
/// blocks on a full queue and the kernel drops what it cannot hand over, so a
/// device firing reports faster than they are handled cannot grow the process.
const HID_QUEUE: usize = 256;
const CMD_QUEUE: usize = 32;
/// One job at a time, so a single slot would do; four leaves room.
const BT_QUEUE: usize = 4;

/// A short status report as a desktop notification.
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
    body.push(format!(
        "{}: {}",
        s.speaker,
        level(state.sink_volume, state.sink_muted)
    ));
    body.push(format!(
        "{}: {}",
        s.microphone,
        level(state.source_volume, state.source_muted)
    ));
    if let Some(p) = player {
        body.push(match p.track {
            Some(t) => format!("{}: {t}", p.identity),
            None => format!("{}: {}", p.identity, p.status),
        });
    }

    notify(&body.join("\n")).await;
}

/// Escapes the markup both sinks understand.
///
/// zenity parses its text as Pango markup and the notification body allows a
/// subset of HTML, so a device that calls itself `<b>` would otherwise style
/// what it is displayed in.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Desktop notification, also the fallback for the dialogs.
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
                escape_markup(body).as_str(),
                Vec::<String>::new(),
                std::collections::HashMap::<String, zbus::zvariant::Value>::new(),
                5000i32,
            ),
        )
        .await;
}

/// Year of first publication. A year that follows the clock would state
/// something untrue in a copyright notice.
const COPYRIGHT_YEAR: &str = "2026";

/// Small information dialog. The daemon ships no GUI, so this goes through the
/// desktop's dialog tool; without one, the notification is left.
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
    // Without a dongle the section stays empty; over Bluetooth BlueZ alone
    // knows only name and charge level.
    if info.dongle_name.is_some() || info.dongle_version.is_some() {
        text.push_str(&format!("\n{}\n", s.dongle));
        text.push_str(&line(&format!("  {}", s.model), &info.dongle_name));
        text.push_str(&line(&format!("  {}", s.firmware), &info.dongle_version));
        text.push_str(&line(&format!("  {}", s.serial), &info.dongle_serial));
    } else if info.headset_version.is_none() {
        // Only when no GNP channel exists at all; over RFCOMM Bluetooth
        // supplies firmware, serial number and charging state as well.
        text.push_str(&format!("\n{}\n", s.bluetooth_note));
    }
    // Show our own version: otherwise there is no way to tell which build is
    // actually running when autostart launches an older package. Holder and
    // licence come from Cargo.toml so the two cannot drift apart, and the
    // repository address is what makes the licence followable.
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

    let markup = escape_markup(&text);
    let attempts: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--info".into(),
                "--title=Jabraw".into(),
                // Same icon as the tray, provided the package is installed.
                "--icon=jabraw".into(),
                "--no-wrap".into(),
                format!("--text={markup}"),
            ],
        ),
        (
            "kdialog",
            vec![
                "--title".into(),
                "Jabraw".into(),
                "--msgbox".into(),
                markup.clone(),
            ],
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

/// Says what became of the writes, one line each.
async fn report_writes(done: Vec<(String, Option<gnp::Ack>)>) {
    let s = strings();
    let lines: Vec<String> = done
        .into_iter()
        .map(|(told, ack)| match ack {
            Some(gnp::Ack::Ok) => told,
            Some(gnp::Ack::Nak(code)) => format!("{told}: {} ({code:#04x})", s.refused),
            None => format!("{told}: {}", s.no_answer),
        })
        .collect();
    notify(&lines.join("\n")).await;
}

/// The sentence a change is reported with.
fn told_about(def: &config::Def, raw: u8) -> String {
    format!(
        "{} {} {}",
        def.label(),
        strings().set_to,
        def.values
            .iter()
            .find(|c| c.raw == raw)
            .map_or_else(|| raw.to_string(), |c| c.label().to_string())
    )
}

/// Runs a dialog beside the main loop, one at a time.
///
/// The dialog tools return when the user closes the window, which would
/// otherwise hold up key presses and every other menu entry for as long as it
/// stands open.
fn spawn_dialog<F>(open: &Arc<AtomicBool>, fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if open.swap(true, Ordering::SeqCst) {
        return;
    }
    let open = open.clone();
    tokio::spawn(async move {
        fut.await;
        open.store(false, Ordering::SeqCst);
    });
}

fn spawn_bt(mut s: rfcomm::Session, job: BtJob, device: String, tx: mpsc::Sender<BtResult>) {
    tokio::task::spawn_blocking(move || {
        let d = gnp::DST_HEADSET;
        let (data, alive) = match job {
            BtJob::Ident => {
                let name = s.read(d, gnp::CMD_IDENT, gnp::SUB_NAME).data();
                let ver = s.read(d, gnp::CMD_IDENT, gnp::SUB_VERSION).data();
                let ser = s.read(d, gnp::CMD_IDENT, gnp::SUB_SERIAL).data();
                (BtData::Ident { name, ver, ser }, !s.is_dead())
            }
            BtJob::Battery => {
                let bat = s.read(d, gnp::CMD_STATUS, gnp::SUB_HS_BATTERY).data();
                (BtData::Battery(bat), !s.is_dead())
            }
            BtJob::Settings { edit } => {
                let rows = s.sweep(d, gnp::CMD_CONFIG, &config::subs());
                (BtData::Settings(rows, edit), !s.is_dead())
            }
            BtJob::Write(items) => {
                let done = items
                    .into_iter()
                    .map(|(dst, def, raw, told)| {
                        let mut data = def.request.to_vec();
                        data.push(raw);
                        (told, s.write(dst, gnp::CMD_CONFIG, def.sub, &data))
                    })
                    .collect();
                (BtData::Written(done), !s.is_dead())
            }
        };
        // Only a broken socket ends the session. Which subcommands a device
        // answers differs per model, and silence is one of the answers.
        let _ = tx.blocking_send(BtResult {
            device,
            session: alive.then_some(s),
            data,
        });
    });
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

/// Overview of the device settings as a table. Read-only; changing one goes
/// through [`edit_settings`], which has room for a control per setting.
async fn show_settings(settings: Vec<config::Setting>) {
    let s = strings();
    if settings.is_empty() {
        notify(s.no_settings).await;
        return;
    }

    // No --icon: zenity knows the option for the info dialog only and rejects
    // the call outright with --list.
    let mut args = vec![
        "--list".to_string(),
        "--title=Jabraw".to_string(),
        format!("--text={}", s.settings.trim_end_matches(" …")),
        "--width=520".to_string(),
        "--height=560".to_string(),
        // No empty column title: zenity 4 then returns the first row
        // immediately instead of showing the dialog.
        "--column".to_string(),
        s.device_col.to_string(),
        "--column".to_string(),
        s.setting.to_string(),
        "--column".to_string(),
        s.value.to_string(),
    ];
    for item in &settings {
        args.push(item.device.to_string());
        args.push(item.label.to_string());
        args.push(format_value(item.kind, item.values, item.raw, &item.value));
    }
    match tokio::process::Command::new("zenity")
        .args(&args)
        .status()
        .await
    {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let text: Vec<String> = settings
                .iter()
                .map(|i| {
                    format!(
                        "{} {}: {}",
                        i.device,
                        i.label,
                        format_value(i.kind, i.values, i.raw, &i.value)
                    )
                })
                .collect();
            notify(&text.join("\n")).await;
        }
        Err(e) => eprintln!("zenity failed: {e}"),
    }
}

/// One form with a drop-down per setting that can be changed.
///
/// A single dialog rather than a list to pick from and a second list to answer
/// with: every setting shows its current value in the label, every drop-down
/// starts on "leave as it is", and OK sends only what actually moved.
async fn edit_settings(settings: Vec<config::Setting>, tx: mpsc::Sender<Cmd>) {
    let s = strings();
    // A value that shares its byte with the neighbours would need the rest of
    // that byte carried over, which is not established.
    let editable: Vec<&config::Setting> = settings
        .iter()
        .filter(|i| !i.values.is_empty() && i.raw.is_some())
        .filter(|i| config::def_of(i.name).is_some_and(|d| d.mask == 0))
        .collect();
    if editable.is_empty() {
        notify(s.only_shown).await;
        return;
    }

    let mut args = vec![
        "--forms".to_string(),
        "--title=Jabraw".to_string(),
        format!("--text={}", s.edit_settings.trim_end_matches(" …")),
        "--separator=|".to_string(),
    ];
    for item in &editable {
        let shown = format_value(item.kind, item.values, item.raw, &item.value);
        args.push(format!(
            "--add-combo={} · {} ({}: {shown})",
            item.device, item.label, s.now
        ));
        let mut values = vec![s.unchanged.to_string()];
        values.extend(item.values.iter().map(|c| c.label().to_string()));
        args.push(format!("--combo-values={}", values.join("|")));
    }

    let out = match tokio::process::Command::new("zenity")
        .args(&args)
        .output()
        .await
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(_) => return,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return notify(s.only_shown).await;
        }
        Err(e) => return eprintln!("zenity failed: {e}"),
    };

    let mut changes = Vec::new();
    for (item, picked) in editable.iter().zip(out.split('|')) {
        let picked = picked.trim();
        if picked.is_empty() || picked == s.unchanged {
            continue;
        }
        let Some(choice) = item.values.iter().find(|c| c.label() == picked) else {
            continue;
        };
        if Some(choice.raw) == item.raw {
            continue;
        }
        changes.push(Change {
            name: item.name,
            dst: if item.device == "Dongle" {
                gnp::DST_DONGLE
            } else {
                gnp::DST_HEADSET
            },
            raw: choice.raw,
        });
    }
    if changes.is_empty() {
        return notify(s.nothing_changed).await;
    }
    let _ = tx.send(Cmd::SetSettings(changes)).await;
}

/// A named value wins; a switch reads as off and on; anything else keeps its
/// number rather than being dressed up as something it may not be.
fn format_value(
    kind: config::Kind,
    values: &[config::Choice],
    raw: Option<u8>,
    data: &[u8],
) -> String {
    let s = strings();
    if let Some(v) = raw {
        if let Some(c) = values.iter().find(|c| c.raw == v) {
            return c.label().to_string();
        }
    }
    match (kind, data) {
        (_, []) => "—".to_string(),
        (config::Kind::Choice, [v]) => v.to_string(),
        (_, [0]) => s.off.to_string(),
        (_, [1]) => s.on.to_string(),
        (_, [v]) => v.to_string(),
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
    // Used by the build script to generate the application menu icons, from the
    // same drawing as the tray.
    if let Some(dir) = arg_value("--write-icons") {
        std::fs::create_dir_all(&dir)?;
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
    // Monotonic runtime rather than wall clock: what matters are intervals.
    let started = std::time::Instant::now();
    let no_tray = std::env::args().any(|a| a == "--no-tray");
    // Autostart and the udev-triggered unit can both fire for one session;
    // only a start by hand should report on a daemon that already runs.
    let quiet = std::env::args().any(|a| a == "--quiet");
    let conn = zbus::Connection::session().await?;
    // BlueZ lives on the system bus. Without it only the USB path remains.
    let system = zbus::Connection::system().await.ok();
    if system.is_none() {
        eprintln!("kein System-Bus erreichbar — Bluetooth-Geräte bleiben unsichtbar");
    }

    // Single-instance lock. Two daemons would both read hidraw and send every
    // key press to MPRIS twice — play immediately followed by pause, so nothing
    // visible. The name is cheaper and more reliable than a PID file, because
    // D-Bus releases it when the process ends.
    match conn
        .request_name_with_flags(
            "io.github.michidold.Jabraw",
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await
    {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {}
        // With DoNotQueue zbus reports a taken name as an error rather than a
        // return value. Exit cleanly with 0 so autostart and systemd do not
        // treat it as a crash and restart.
        Ok(_) | Err(zbus::Error::NameTaken) => {
            if quiet {
                return Ok(());
            }
            // Invoked from the menu entry while a daemon already runs: report
            // the current state instead of ending without a word.
            eprintln!("jabraw läuft bereits — zeige Status");
            notify_status(&conn).await;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    let (hid_tx, mut hid_rx) = mpsc::channel::<Msg>(HID_QUEUE);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(CMD_QUEUE);
    let (bt_tx, mut bt_rx) = mpsc::channel::<BtResult>(BT_QUEUE);

    // Writable copies of the device handles, for GNP requests.
    let mut writers: HashMap<String, std::fs::File> = HashMap::new();
    let mut battery: Option<u8> = None;
    let mut charging = false;
    let mut info = DeviceInfo::default();
    let mut sinks: Vec<audio::Sink> = Vec::new();
    let mut bt: Option<bluetooth::BtDevice> = None;
    let mut bt_path: Option<String> = None;
    // What the RFCOMM session has supplied so far, kept across ticks.
    let mut bt_info = DeviceInfo::default();
    let mut bt_level: Option<(u8, bool)> = None;
    let mut bt_wait = BLUETOOTH_EVERY_N_TICKS;
    let mut bt_next_try: u32 = 0;
    // A job holds the session while it runs.
    let mut bt_busy = false;
    // Set once the identity was asked for, answered or not.
    let mut bt_ident_done = false;
    let mut session: Option<rfcomm::Session> = None;
    let mut seq: u8 = 0;
    // Matches replies to our own requests; the device also sends on this
    // channel unprompted.
    let mut pending: HashMap<u8, Pending> = HashMap::new();
    let mut ticks: u32 = 0;

    let dialog_open = Arc::new(AtomicBool::new(false));
    let mut watched: HashSet<String> = HashSet::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut warned: HashSet<String> = HashSet::new();
    // Previous bit state per (device, report), to detect edges.
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
                // Without an SNI host — GNOME lacking the AppIndicator
                // extension, say — the key handling stays usable.
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
                        // Device data does not change; ask once on connect.
                        for (dst, sub) in [
                            (gnp::DST_DONGLE, gnp::SUB_NAME),
                            (gnp::DST_DONGLE, gnp::SUB_VERSION),
                            (gnp::DST_DONGLE, gnp::SUB_SERIAL),
                            (gnp::DST_HEADSET, gnp::SUB_NAME),
                            (gnp::DST_HEADSET, gnp::SUB_VERSION),
                            (gnp::DST_HEADSET, gnp::SUB_SERIAL),
                        ] {
                            seq = seq.wrapping_add(1);
                            let req = gnp::read_request(dst, seq, gnp::CMD_IDENT, sub, &[]);
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

                // Battery less often than the rest of the display; it moves in
                // whole percent.
                if ticks.is_multiple_of(BATTERY_EVERY_N_TICKS) {
                    seq = seq.wrapping_add(1);
                    let req = gnp::read_request(
                        gnp::DST_DONGLE,
                        seq,
                        gnp::CMD_STATUS,
                        gnp::SUB_HS_BATTERY,
                        &[],
                    );
                    for w in writers.values_mut() {
                        let _ = w.write_all(&req);
                    }
                    pending.insert(seq, Pending::Battery);
                }

                // Without a dongle there is no GNP channel over USB. What
                // BlueZ knows about a directly paired headset is the starting
                // point; the keys travel over AVRCP in that mode and the
                // desktop passes them to MPRIS already.
                if watched.is_empty() {
                    let slow = ticks.is_multiple_of(BLUETOOTH_EVERY_N_TICKS);
                    if slow {
                        bt = match &system {
                            Some(c) => bluetooth::connected_jabra(c).await,
                            None => None,
                        };
                        let path = bt.as_ref().map(|d| d.path.clone());
                        // Another device makes everything read so far stale.
                        if path != bt_path {
                            bt_info = DeviceInfo::default();
                            bt_level = None;
                            session = None;
                            bt_ident_done = false;
                            bt_wait = BLUETOOTH_EVERY_N_TICKS;
                            bt_next_try = ticks;
                        }
                        bt_path = path;
                    }

                    // GNP over RFCOMM is more precise than the coarse HFP
                    // indicator, and adds firmware and serial number. Both the
                    // handshake and the reads run beside the loop: either can
                    // take seconds when the headset is slow to answer.
                    if bt.is_none() {
                        session = None;
                        bt_info = DeviceInfo::default();
                        bt_level = None;
                        bt_ident_done = false;
                        // The next headset starts with a clean backoff.
                        bt_wait = BLUETOOTH_EVERY_N_TICKS;
                        bt_next_try = ticks;
                    } else if !bt_busy {
                        if session.is_none() {
                            if ticks >= bt_next_try {
                                if let (Some(c), Some(p)) = (&system, &bt_path) {
                                    let (c, p) = (c.clone(), p.clone());
                                    let tx = bt_tx.clone();
                                    bt_busy = true;
                                    tokio::spawn(async move {
                                        let session = rfcomm::connect(&c, &p).await;
                                        let _ = tx
                                            .send(BtResult {
                                                device: p,
                                                session,
                                                data: BtData::Connected,
                                            })
                                            .await;
                                    });
                                }
                            }
                        } else {
                            // Identity stays put while the session stands, and
                            // the charge level moves in whole percent.
                            let job = if !bt_ident_done {
                                Some(BtJob::Ident)
                            } else if slow {
                                Some(BtJob::Battery)
                            } else {
                                None
                            };
                            if let (Some(job), Some(p)) = (job, bt_path.clone()) {
                                if let Some(s) = session.take() {
                                    bt_busy = true;
                                    spawn_bt(s, job, p, bt_tx.clone());
                                }
                            }
                        }
                    }

                    info = match &bt {
                        Some(d) => DeviceInfo {
                            headset_name: bt_info
                                .headset_name
                                .clone()
                                .or_else(|| Some(d.name.clone())),
                            ..bt_info.clone()
                        },
                        None => DeviceInfo::default(),
                    };
                    // HFP carries no charging state and rounds the level; the
                    // GNP figure wins wherever the session supplied one.
                    (battery, charging) = match bt_level {
                        Some((p, c)) => (Some(p), c),
                        None => (bt.as_ref().and_then(|d| d.battery), false),
                    };
                }
                ticks = ticks.wrapping_add(1);
                if let Some(handle) = &tray {
                    // The charge belongs to the headset, not the dongle, so
                    // show the headset's name as soon as ident has supplied it.
                    // Otherwise the name of the USB device.
                    let device = info
                        .headset_name
                        .clone()
                        .or_else(|| names.values().next().cloned())
                        .or_else(|| bt.as_ref().map(|d| d.name.clone()));
                    let info_snapshot = info.clone();
                    let has_gnp = !writers.is_empty() || session.is_some() || bt_busy;
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
                Cmd::ShowInfo => {
                    let (info, battery, charging) = (info.clone(), battery, charging);
                    spawn_dialog(&dialog_open, async move {
                        show_device_info(&info, battery, charging).await
                    });
                }
                c @ (Cmd::ShowSettings | Cmd::ShowEditor) => {
                    let edit = matches!(c, Cmd::ShowEditor);
                    // Over hidraw when a dongle is plugged in, otherwise
                    // through the existing RFCOMM session.
                    if let Some(path) = watched.iter().next().cloned() {
                        let tx = cmd_tx.clone();
                        spawn_dialog(&dialog_open, async move {
                            let rows = config_via_hidraw(path).await;
                            if edit {
                                edit_settings(rows, tx).await
                            } else {
                                show_settings(rows).await
                            }
                        });
                    } else if !bt_busy {
                        // The sweep goes through the same job as the reads;
                        // the dialog follows when the rows come back.
                        if let Some(p) = bt_path.clone() {
                            if let Some(s) = session.take() {
                                bt_busy = true;
                                spawn_bt(s, BtJob::Settings { edit }, p, bt_tx.clone());
                            }
                        }
                    }
                }
                Cmd::SetSettings(changes) => {
                    let items: Vec<(u8, &'static config::Def, u8, String)> = changes
                        .iter()
                        .filter_map(|c| {
                            let def = config::def_of(c.name)?;
                            Some((c.dst, def, c.raw, told_about(def, c.raw)))
                        })
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    if let Some(path) = watched.iter().next().cloned() {
                        tokio::spawn(async move {
                            let done = tokio::task::spawn_blocking(move || {
                                items
                                    .into_iter()
                                    .map(|(dst, def, raw, told)| {
                                        let ack = config::write(&path, dst, def, raw)
                                            .ok()
                                            .flatten();
                                        (told, ack)
                                    })
                                    .collect()
                            })
                            .await
                            .unwrap_or_default();
                            report_writes(done).await;
                        });
                    } else if !bt_busy {
                        if let Some(p) = bt_path.clone() {
                            if let Some(s) = session.take() {
                                bt_busy = true;
                                spawn_bt(s, BtJob::Write(items), p, bt_tx.clone());
                            }
                        }
                    }
                }
                // Refresh the device list when the menu opens, so the rare
                // polling interval cannot leave stale entries.
                Cmd::Refresh => sinks = audio::list_sinks().await,
                Cmd::Quit => {
                    if let Some(handle) = &tray {
                        handle.shutdown().await;
                    }
                    return Ok(());
                }
            },

            Some(res) = bt_rx.recv() => {
                bt_busy = false;
                if bt_path.as_deref() != Some(res.device.as_str()) {
                    // Answer for a headset that has since gone or changed.
                    continue;
                }
                session = res.session;
                match res.data {
                    BtData::Connected => {
                        if session.is_some() {
                            println!("GNP over RFCOMM established");
                            bt_ident_done = false;
                            bt_wait = BLUETOOTH_EVERY_N_TICKS;
                        } else {
                            // A headset without the serial profile would
                            // otherwise be asked again every round.
                            bt_wait = (bt_wait * 2).min(BLUETOOTH_RETRY_MAX);
                        }
                        bt_next_try = ticks.saturating_add(bt_wait);
                    }
                    // Asked once per session, answered or not: a headset that
                    // stays silent must not be asked again on every tick.
                    BtData::Ident { name, ver, ser } => {
                        bt_ident_done = true;
                        if let Some(v) = name.as_deref().and_then(gnp::text) {
                            bt_info.headset_name = Some(v);
                        }
                        if let Some(v) = ver.as_deref().and_then(gnp::text) {
                            bt_info.headset_version = Some(v);
                        }
                        if let Some(v) = ser.as_deref().and_then(gnp::text) {
                            bt_info.headset_serial = Some(v);
                        }
                    }
                    BtData::Battery(bat) => {
                        if let Some(d) = bat.as_deref() {
                            bt_level = gnp::battery_percent(d)
                                .map(|p| (p, gnp::battery_charging(d)));
                        }
                    }
                    BtData::Written(done) => {
                        report_writes(done).await;
                    }
                    BtData::Settings(rows, edit) => {
                        let rows = config::from_sweep(rows);
                        let tx = cmd_tx.clone();
                        spawn_dialog(&dialog_open, async move {
                            if edit {
                                edit_settings(rows, tx).await
                            } else {
                                show_settings(rows).await
                            }
                        });
                    }
                }
            }

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
                        // Unanswered requests would otherwise pile up until
                        // the sequence number wraps.
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_choice_without_names_keeps_its_number() {
        let none: &[super::config::Choice] = &[];
        assert_eq!(
            super::format_value(super::config::Kind::Choice, none, Some(0), &[0]),
            "0"
        );
        assert_ne!(
            super::format_value(super::config::Kind::Switch, none, Some(0), &[0]),
            "0"
        );
    }

    #[test]
    fn a_named_value_wins_over_both() {
        // soundMode 0 is Normal, not off and not "0".
        let d = super::config::SETTINGS
            .iter()
            .find(|d| d.name == "soundMode")
            .unwrap();
        assert_eq!(
            super::format_value(d.kind, d.values, Some(0), &[0]),
            d.values[0].label()
        );
        // A byte the table does not name falls back to the number.
        assert_eq!(super::format_value(d.kind, d.values, Some(9), &[9]), "9");
    }

    #[test]
    fn the_settings_the_model_files_cover_are_marked() {
        let by_name = |n| {
            super::config::SETTINGS
                .iter()
                .find(|d| d.name == n)
                .unwrap()
        };
        assert!(by_name("soundMode").kind == super::config::Kind::Choice);
        assert!(by_name("hsRinger").kind == super::config::Kind::Switch);
    }

    #[test]
    fn what_a_dialog_parses_as_markup_is_escaped() {
        assert_eq!(
            super::escape_markup("Jabra & <b>Evolve</b>"),
            "Jabra &amp; &lt;b&gt;Evolve&lt;/b&gt;"
        );
    }
}
