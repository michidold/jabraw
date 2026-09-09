//! Reading device settings through the GNP group `config`.
//!
//! Jabra's SDK knows 212 subcommands, and which of them a device answers
//! depends on the model. The daemon therefore asks a fixed list of known
//! settings and shows what comes back — a headset without ANC simply does not
//! answer `anc`, and that row is missing. This works without model detection.
//!
//! Reading only. The same subcommands could be written, which would change
//! device settings for good.
//!
//! What the values mean is not in the protocol. `doc/settings-sources.md` says
//! where Jabra describes it, what that description answers and what it leaves
//! open.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use crate::gnp;

/// Settings that mean something to a user. Other subcommands in the group are
/// operational data — clock, feature masks, password fields — and do not belong
/// in an overview.
/// Whether a value may be read as a switch.
///
/// Jabra's device model files say which settings are On/Off and which are a
/// choice from a longer list — see `doc/settings-sources.md`. Where they say
/// choice, 0 and 1 are two of several values, and calling them off and on puts
/// a wrong word on the screen: `soundMode` 0 is bass, not off.
///
/// Everything the model files do not cover stays `Switch`, which is the
/// reading that fits two thirds of the values this device answers, and which
/// holds wherever the files do confirm it.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Switch,
    Choice,
}

pub struct Def {
    pub sub: u8,
    pub name: &'static str,
    pub kind: Kind,
}

// One row per setting; rustfmt would give every field a line of its own and
// turn the table into three hundred lines.
#[rustfmt::skip]
pub const SETTINGS: &[Def] = &[
    Def { sub: 0, name: "audioType", kind: Kind::Switch },
    Def { sub: 1, name: "intellitoneLevel", kind: Kind::Choice },
    Def { sub: 2, name: "touchAudioFeedback", kind: Kind::Switch },
    Def { sub: 3, name: "ringerVolume", kind: Kind::Switch },
    Def { sub: 5, name: "phonePresence", kind: Kind::Switch },
    Def { sub: 8, name: "currentLanguage", kind: Kind::Choice },
    Def { sub: 13, name: "micGain", kind: Kind::Switch },
    Def { sub: 14, name: "rfPower", kind: Kind::Choice },
    Def { sub: 19, name: "hsRinger", kind: Kind::Switch },
    Def { sub: 21, name: "soundMode", kind: Kind::Choice },
    Def { sub: 27, name: "powersave", kind: Kind::Switch },
    Def { sub: 30, name: "muteReminderInterval", kind: Kind::Choice },
    Def { sub: 33, name: "autoOpenHardphone", kind: Kind::Switch },
    Def { sub: 36, name: "autoOpenSoftphone", kind: Kind::Switch },
    Def { sub: 37, name: "musicMode", kind: Kind::Switch },
    Def { sub: 39, name: "buttonFunction", kind: Kind::Choice },
    Def { sub: 50, name: "singleCallConf", kind: Kind::Switch },
    Def { sub: 51, name: "idlePowerSave", kind: Kind::Switch },
    Def { sub: 53, name: "hsTouchSensor", kind: Kind::Switch },
    Def { sub: 55, name: "ancGain", kind: Kind::Switch },
    Def { sub: 57, name: "busylight", kind: Kind::Switch },
    Def { sub: 58, name: "hsVoicePrompts", kind: Kind::Choice },
    Def { sub: 59, name: "hsMotionSensor", kind: Kind::Switch },
    Def { sub: 60, name: "autoRejectBgWaiting", kind: Kind::Switch },
    Def { sub: 61, name: "ringtoneType", kind: Kind::Switch },
    Def { sub: 62, name: "ringOnSecondIncomingCall", kind: Kind::Switch },
    Def { sub: 63, name: "buttonSounds", kind: Kind::Switch },
    Def { sub: 64, name: "autoPairing", kind: Kind::Switch },
    Def { sub: 69, name: "acceptCallOnUndock", kind: Kind::Switch },
    Def { sub: 74, name: "ctrlBusylight", kind: Kind::Switch },
    Def { sub: 80, name: "undockOpenAudioLink", kind: Kind::Switch },
    Def { sub: 82, name: "ancLed", kind: Kind::Switch },
    Def { sub: 83, name: "ancMonitorLed", kind: Kind::Switch },
    Def { sub: 84, name: "audioStreaming", kind: Kind::Switch },
    Def { sub: 92, name: "buttonSwapFunction", kind: Kind::Switch },
    Def { sub: 104, name: "sidetoneLevel", kind: Kind::Choice },
    Def { sub: 112, name: "lowBatteryAudioNotifications", kind: Kind::Switch },
    Def { sub: 114, name: "echoCancel", kind: Kind::Switch },
    Def { sub: 120, name: "powerNap", kind: Kind::Switch },
    Def { sub: 124, name: "dspSidetone", kind: Kind::Switch },
    Def { sub: 125, name: "equalizer", kind: Kind::Switch },
    Def { sub: 126, name: "equalizerEnable", kind: Kind::Switch },
    Def { sub: 133, name: "sidetoneMute", kind: Kind::Switch },
    Def { sub: 134, name: "hallSensor", kind: Kind::Switch },
    Def { sub: 135, name: "anc", kind: Kind::Switch },
    Def { sub: 138, name: "selectButtonFunction", kind: Kind::Switch },
    Def { sub: 142, name: "autoPauseMusic", kind: Kind::Switch },
    Def { sub: 143, name: "autoMuteCallAudio", kind: Kind::Switch },
    Def { sub: 144, name: "inactivityInterval", kind: Kind::Choice },
    Def { sub: 145, name: "autoAnswerCall", kind: Kind::Switch },
    Def { sub: 146, name: "onHeadDetection", kind: Kind::Switch },
    Def { sub: 147, name: "alwaysOnVoice", kind: Kind::Switch },
    Def { sub: 149, name: "automaticSpeechRecognition", kind: Kind::Switch },
    Def { sub: 152, name: "boomarmRotationAction", kind: Kind::Choice },
    Def { sub: 153, name: "streamPriority", kind: Kind::Switch },
    Def { sub: 188, name: "callAcceptedSound", kind: Kind::Choice },
];

/// Turns the replies of an RFCOMM sweep into display rows.
pub fn from_sweep(rows: Vec<(&'static str, Vec<u8>)>) -> Vec<Setting> {
    rows.into_iter()
        .map(|(name, value)| Setting {
            device: "Headset",
            name,
            kind: kind_of(name),
            value,
        })
        .collect()
}

/// The list is short enough that a scan beats carrying a map around.
fn kind_of(name: &str) -> Kind {
    SETTINGS
        .iter()
        .find(|d| d.name == name)
        .map_or(Kind::Switch, |d| d.kind)
}

/// Subcommand and name for a transport that has no use for the rest.
pub fn subs() -> Vec<(u8, &'static str)> {
    SETTINGS.iter().map(|d| (d.sub, d.name)).collect()
}

pub struct Setting {
    pub device: &'static str,
    pub name: &'static str,
    pub kind: Kind,
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
    for (i, def) in SETTINGS.iter().enumerate() {
        // Avoid sequence 0 so it stands apart from an empty report.
        let seq = (i as u8).wrapping_add(1).max(1);
        file.write_all(&gnp::read_request(dst, seq, gnp::CMD_CONFIG, def.sub))?;
        pending.insert(seq, def.name);
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
                kind: kind_of(name),
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
