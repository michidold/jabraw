//! Rohzugriff auf die Jabra-hidraw-Nodes.
//!
//! Der Umweg über hidraw ist nötig, weil die Multifunktionstaste im
//! Report-Descriptor des Link 380 als `81 07` deklariert ist. Bit 0 dieses
//! Input-Flags bedeutet Constant, und `hid-input.c` überspringt konstante
//! Felder — der Kernel legt dafür also keinen Tastencode an. Über evdev ist die
//! Taste damit unerreichbar, im Rohreport steht sie klar drin.

use std::collections::HashSet;
use std::io::Read;

use tokio::sync::mpsc;

pub const JABRA_VENDOR: u32 = 0x0b0e;

pub enum Msg {
    Report(String, Vec<u8>),
    Closed(String),
}

/// Aktionen, die der Daemon aus einem Bitwechsel ableitet.
pub enum Action {
    /// MPRIS-Methode, ausgelöst auf der steigenden Flanke.
    Mpris(&'static str),
}

/// Nur die Bits, die der Kernel wegen des Constant-Flags verwirft. Alles, was
/// als `KEY_*` ankommt — insbesondere die Lautstärke —, verarbeitet der Desktop
/// bereits; würde es hier nochmal auftauchen, löste jede Taste doppelt aus.
///
/// Die Multifunktionstaste schickt abwechselnd Pause (Bit 9) und Play (Bit 10),
/// je nachdem was das Headset für den Zustand hält. Beide auf PlayPause
/// abzubilden ist robuster als die wörtliche Übersetzung: läuft die Annahme des
/// Headsets aus dem Tritt, bliebe ein wörtliches "Pause" auf einem bereits
/// pausierten Player wirkungslos.
pub fn action_for(report: u8, bit: u32) -> Option<Action> {
    match (report, bit) {
        (1, 9) | (1, 10) => Some(Action::Mpris("PlayPause")),
        // Report 2 Bit 2 (Line) wechselt zusammen mit dem Wiedergabezustand,
        // nicht mit dem Mikroarm — belegt man es, schaltet jeder Tastendruck
        // zusätzlich etwas anderes. Der Arm meldet sich auf hidraw gar nicht.
        _ => None,
    }
}

pub fn bit_name(report: u8, bit: u32) -> &'static str {
    match (report, bit) {
        (1, 0) => "VolumeDown",
        (1, 1) => "VolumeUp",
        (1, 2) => "Mute",
        (1, 3) => "PlayPause",
        (1, 4) => "Stop",
        (1, 5) => "Next",
        (1, 6) => "Previous",
        (1, 7) => "FastForward",
        (1, 8) => "Rewind",
        (1, 9) => "Pause",
        (1, 10) => "Play",
        (2, 0) => "HookSwitch",
        (2, 2) => "Line (Streamstatus)",
        (2, 3) => "PhoneMute",
        (2, 4) => "Flash",
        (2, 5) => "Redial",
        (2, 6) => "SpeedDial",
        (2, 11) => "ProgrammableButton",
        // Report 4 spiegelt Report 2 auf einer Vendor-Page, ein Bit versetzt.
        (4, 0) => "HookSwitch (vendor)",
        (4, 3) => "Line (Streamstatus, vendor)",
        (4, 4) => "PhoneMute (vendor)",
        _ => "?",
    }
}

/// Die ersten vier Nutzbytes eines Reports als Bitfeld.
pub fn payload_bits(data: &[u8]) -> u32 {
    let mut bits = 0u32;
    for (i, b) in data.iter().skip(1).take(4).enumerate() {
        bits |= (*b as u32) << (8 * i);
    }
    bits
}

/// Name des Geräts hinter einem hidraw-Node, für die Anzeige im Tray.
pub fn device_name(node: &str) -> Option<String> {
    let name = std::fs::read_to_string(format!("/sys/class/hidraw/{node}/device/uevent")).ok()?;
    name.lines()
        .find_map(|l| l.strip_prefix("HID_NAME="))
        .map(|s| s.trim().to_string())
}

/// Gerätename, ohne den Node zu öffnen — für Statusabfragen, die keinen
/// Lesezugriff brauchen.
pub fn present() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/hidraw").ok()?;
    for entry in entries.flatten() {
        let Ok(uevent) = std::fs::read_to_string(entry.path().join("device/uevent")) else {
            continue;
        };
        let vendor = uevent
            .lines()
            .find_map(|l| l.strip_prefix("HID_ID="))
            .and_then(|id| id.split(':').nth(1))
            .and_then(|v| u32::from_str_radix(v.trim(), 16).ok());
        if vendor == Some(JABRA_VENDOR) {
            return Some(
                uevent
                    .lines()
                    .find_map(|l| l.strip_prefix("HID_NAME="))
                    .unwrap_or("Jabra")
                    .trim()
                    .to_string(),
            );
        }
    }
    None
}

/// hidraw-Nodes mit Jabra-Vendor, die noch nicht überwacht werden.
///
/// `HID_ID` im uevent hat die Form `BUS:VENDOR:PRODUCT` in Hex. `warned`
/// verhindert, dass sich dieselbe Fehlermeldung im Rescan-Takt wiederholt.
pub fn scan(
    watched: &HashSet<String>,
    warned: &mut HashSet<String>,
) -> Vec<(String, String, std::fs::File)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/hidraw") {
        Ok(e) => e,
        Err(e) => {
            eprintln!("kann /sys/class/hidraw nicht lesen: {e}");
            return out;
        }
    };
    for entry in entries.flatten() {
        let node = entry.file_name();
        let node = node.to_string_lossy().to_string();
        let dev_path = format!("/dev/{node}");
        if watched.contains(&dev_path) {
            continue;
        }
        let Ok(uevent) = std::fs::read_to_string(entry.path().join("device/uevent")) else {
            continue;
        };
        let vendor = uevent
            .lines()
            .find_map(|l| l.strip_prefix("HID_ID="))
            .and_then(|id| id.split(':').nth(1))
            .and_then(|v| u32::from_str_radix(v.trim(), 16).ok());
        if vendor != Some(JABRA_VENDOR) {
            continue;
        }
        // Schreibend, weil der Vendor-Kanal Anfragen entgegennimmt; die
        // udev-ACL vergibt ohnehin rw.
        match std::fs::OpenOptions::new().read(true).write(true).open(&dev_path) {
            Ok(f) => {
                warned.remove(&dev_path);
                let name = device_name(&node).unwrap_or_else(|| "Jabra".into());
                out.push((dev_path, name, f));
            }
            Err(e) => {
                if warned.insert(dev_path.clone()) {
                    eprintln!("überspringe {dev_path}: {e}");
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        eprintln!("  -> udev-Regel fehlt oder greift nicht, siehe README");
                    }
                }
            }
        }
    }
    out
}

/// hidraw kennt kein async; blockierende Reads laufen deshalb je Gerät in einem
/// eigenen Thread, der beim Abziehen des Dongles von selbst endet.
pub fn spawn_reader(path: String, mut file: std::fs::File, tx: mpsc::UnboundedSender<Msg>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Msg::Report(path.clone(), buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        break;
                    }
                }
            }
        }
        let _ = tx.send(Msg::Closed(path));
    });
}
