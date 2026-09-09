//! Raw access to the Jabra hidraw nodes.
//!
//! The detour through hidraw is necessary because the Link 380 declares the
//! multi-function button as `81 07` in its report descriptor. Bit 0 of that
//! input flag means Constant, and `hid-input.c` skips constant fields, so the
//! kernel allocates no key code for it. Over evdev the button is unreachable
//! for that reason, while the raw report states it plainly.

use std::collections::HashSet;
use std::io::Read;

use tokio::sync::mpsc;

pub const JABRA_VENDOR: u32 = 0x0b0e;

pub enum Msg {
    Report(String, Vec<u8>),
    Closed(String),
}

/// Actions the daemon derives from a bit change.
pub enum Action {
    /// MPRIS method, triggered on the rising edge.
    Mpris(&'static str),
}

/// Only the bits the kernel discards over the constant flag. Anything arriving
/// as `KEY_*` — the volume rocker above all — is handled by the desktop
/// already; repeating it here would fire every key twice.
///
/// The multi-function button alternates between Pause (bit 9) and Play
/// (bit 10), following whatever state the headset assumes. Mapping both to
/// PlayPause is sturdier than the literal translation: once that assumption
/// drifts, a literal "Pause" on an already paused player would do nothing.
pub fn action_for(report: u8, bit: u32) -> Option<Action> {
    match (report, bit) {
        (1, 9) | (1, 10) => Some(Action::Mpris("PlayPause")),
        // Report 2 bit 2 (Line) tracks playback state rather than the boom
        // arm; mapping it would make every key press toggle something else as
        // well. The arm reports nothing over hidraw at all.
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
        // Report 4 mirrors report 2 on a vendor page, offset by one bit.
        (4, 0) => "HookSwitch (vendor)",
        (4, 3) => "Line (Streamstatus, vendor)",
        (4, 4) => "PhoneMute (vendor)",
        _ => "?",
    }
}

/// The first four payload bytes of a report as a bit field.
pub fn payload_bits(data: &[u8]) -> u32 {
    let mut bits = 0u32;
    for (i, b) in data.iter().skip(1).take(4).enumerate() {
        bits |= (*b as u32) << (8 * i);
    }
    bits
}

/// Name of the device behind a hidraw node, for display in the tray.
pub fn device_name(node: &str) -> Option<String> {
    let name = std::fs::read_to_string(format!("/sys/class/hidraw/{node}/device/uevent")).ok()?;
    name.lines()
        .find_map(|l| l.strip_prefix("HID_NAME="))
        .map(clean)
}

/// The product string reaches a menu label, a dialog and a notification body,
/// and the device is free to put control characters in it.
fn clean(name: &str) -> String {
    name.trim().chars().filter(|c| !c.is_control()).collect()
}

/// Device name without opening the node, for status queries that need no read
/// access.
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
            return Some(clean(
                uevent
                    .lines()
                    .find_map(|l| l.strip_prefix("HID_NAME="))
                    .unwrap_or("Jabra"),
            ));
        }
    }
    None
}

/// hidraw nodes with the Jabra vendor that are not being watched yet.
///
/// `HID_ID` in the uevent has the form `BUS:VENDOR:PRODUCT` in hex. `warned`
/// keeps the same message from repeating on every rescan.
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
        // Writable, because the vendor channel takes requests; the udev ACL
        // grants rw anyway.
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

/// hidraw knows no async, so blocking reads run in a thread per device that
/// ends by itself when the dongle is pulled.
pub fn spawn_reader(path: String, mut file: std::fs::File, tx: mpsc::Sender<Msg>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Blocking on a full queue is the point: hidraw then drops
                    // what it cannot hand over instead of the process growing.
                    if tx
                        .blocking_send(Msg::Report(path.clone(), buf[..n].to_vec()))
                        .is_err()
                    {
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
        let _ = tx.blocking_send(Msg::Closed(path));
    });
}


#[cfg(test)]
mod tests {
    #[test]
    fn control_characters_leave_the_name() {
        assert_eq!(super::clean(" Jabra\u{7}\nLink 380 "), "JabraLink 380");
    }

    #[test]
    fn the_first_four_payload_bytes_are_the_bit_field() {
        assert_eq!(super::payload_bits(&[0x01, 0x02, 0, 0, 0x01]), 0x0100_0002);
    }
}
