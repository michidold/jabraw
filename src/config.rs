//! Reading device settings through the GNP group `config`.
//!
//! Jabra's SDK knows 212 subcommands, and which of them a device answers
//! depends on the model. The daemon therefore asks a fixed list of known
//! settings and shows what comes back — a headset without ANC simply does not
//! answer `anc`, and that row is missing. This works without model detection.
//!
//! Reading only. The same subcommands could be written, which would change
//! device settings for good.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use crate::gnp;

/// Settings that mean something to a user. Other subcommands in the group are
/// operational data — clock, feature masks, password fields — and do not belong
/// in an overview.
pub const SETTINGS: &[(u8, &str)] = &[
    (0, "audioType"),
    (1, "intellitoneLevel"),
    (2, "touchAudioFeedback"),
    (3, "ringerVolume"),
    (5, "phonePresence"),
    (8, "currentLanguage"),
    (13, "micGain"),
    (14, "rfPower"),
    (19, "hsRinger"),
    (21, "soundMode"),
    (27, "powersave"),
    (30, "muteReminderInterval"),
    (33, "autoOpenHardphone"),
    (36, "autoOpenSoftphone"),
    (37, "musicMode"),
    (39, "buttonFunction"),
    (50, "singleCallConf"),
    (51, "idlePowerSave"),
    (53, "hsTouchSensor"),
    (55, "ancGain"),
    (57, "busylight"),
    (58, "hsVoicePrompts"),
    (59, "hsMotionSensor"),
    (60, "autoRejectBgWaiting"),
    (61, "ringtoneType"),
    (62, "ringOnSecondIncomingCall"),
    (63, "buttonSounds"),
    (64, "autoPairing"),
    (69, "acceptCallOnUndock"),
    (74, "ctrlBusylight"),
    (80, "undockOpenAudioLink"),
    (82, "ancLed"),
    (83, "ancMonitorLed"),
    (84, "audioStreaming"),
    (92, "buttonSwapFunction"),
    (104, "sidetoneLevel"),
    (112, "lowBatteryAudioNotifications"),
    (114, "echoCancel"),
    (120, "powerNap"),
    (124, "dspSidetone"),
    (125, "equalizer"),
    (126, "equalizerEnable"),
    (133, "sidetoneMute"),
    (134, "hallSensor"),
    (135, "anc"),
    (138, "selectButtonFunction"),
    (142, "autoPauseMusic"),
    (143, "autoMuteCallAudio"),
    (144, "inactivityInterval"),
    (145, "autoAnswerCall"),
    (146, "onHeadDetection"),
    (147, "alwaysOnVoice"),
    (149, "automaticSpeechRecognition"),
    (152, "boomarmRotationAction"),
    (153, "streamPriority"),
    (188, "callAcceptedSound"),
];

/// Turns the replies of an RFCOMM sweep into display rows.
pub fn from_sweep(rows: Vec<(&'static str, Vec<u8>)>) -> Vec<Setting> {
    rows.into_iter()
        .map(|(name, value)| Setting {
            device: "Headset",
            name,
            value,
        })
        .collect()
}

pub struct Setting {
    pub device: &'static str,
    pub name: &'static str,
    pub value: Vec<u8>,
}

/// How long to wait for replies once every request has gone out.
const COLLECT: Duration = Duration::from_millis(1200);

/// Queries both addresses. Blocking — belongs in `spawn_blocking`.
///
/// Opens its own descriptor rather than using the reader thread: hidraw hands
/// incoming reports to every open descriptor, so the query leaves the running
/// key path undisturbed.
pub fn read_all(path: &str) -> std::io::Result<Vec<Setting>> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let mut out = Vec::new();
    for (dst, label) in [(gnp::DST_DONGLE, "Dongle"), (gnp::DST_HEADSET, "Headset")] {
        out.extend(sweep(&mut file, dst, label)?);
    }
    Ok(out)
}

fn sweep(file: &mut std::fs::File, dst: u8, label: &'static str) -> std::io::Result<Vec<Setting>> {
    let mut pending = std::collections::HashMap::new();
    for (i, (sub, name)) in SETTINGS.iter().enumerate() {
        // Avoid sequence 0 so it stands apart from an empty report.
        let seq = (i as u8).wrapping_add(1).max(1);
        file.write_all(&gnp::read_request(dst, seq, gnp::CMD_CONFIG, *sub))?;
        pending.insert(seq, *name);
        std::thread::sleep(Duration::from_millis(4));
    }

    let mut found = Vec::new();
    let deadline = Instant::now() + COLLECT;
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        if !readable(file, deadline - Instant::now()) {
            break;
        }
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        let Some(r) = gnp::parse(&buf[..n]) else {
            continue;
        };
        if r.cmd != gnp::CMD_CONFIG || r.src != dst {
            continue;
        }
        if let Some(name) = pending.remove(&r.seq) {
            found.push(Setting {
                device: label,
                name,
                value: r.data.to_vec(),
            });
        }
    }
    found.sort_by_key(|s| s.name);
    Ok(found)
}

/// `poll(2)` on the descriptor, so a silent device does not block.
fn readable(file: &std::fs::File, timeout: Duration) -> bool {
    let mut fds = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    unsafe { libc::poll(&mut fds, 1, ms) > 0 }
}
