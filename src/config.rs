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
    /// Jabra's own identifier, and the key that joins this table to the model
    /// files described in `doc/settings-sources.md`.
    pub name: &'static str,
    pub kind: Kind,
    /// What the dialog shows. Where the model files name a setting, that name
    /// is followed; everything else only makes the identifier readable rather
    /// than guessing at what the setting does.
    pub label: &'static str,
    pub label_de: &'static str,
}

impl Def {
    pub fn label(&self) -> &'static str {
        if crate::i18n::german() {
            self.label_de
        } else {
            self.label
        }
    }
}

// One row per setting; rustfmt would give every field a line of its own and
// turn the table into three hundred lines.
#[rustfmt::skip]
pub const SETTINGS: &[Def] = &[
    Def { sub: 0, name: "audioType", kind: Kind::Switch,
          label: "Audio type", label_de: "Audiotyp" },
    Def { sub: 1, name: "intellitoneLevel", kind: Kind::Choice,
          label: "Hearing protection", label_de: "Gehörschutz" },
    Def { sub: 2, name: "touchAudioFeedback", kind: Kind::Switch,
          label: "Touch feedback sounds", label_de: "Töne bei Berührung" },
    Def { sub: 3, name: "ringerVolume", kind: Kind::Switch,
          label: "Ringtone volume", label_de: "Klingellautstärke" },
    Def { sub: 5, name: "phonePresence", kind: Kind::Switch,
          label: "Phone presence", label_de: "Telefon-Präsenz" },
    Def { sub: 8, name: "currentLanguage", kind: Kind::Choice,
          label: "Device language", label_de: "Gerätesprache" },
    Def { sub: 13, name: "micGain", kind: Kind::Switch,
          label: "Microphone gain", label_de: "Mikrofonverstärkung" },
    Def { sub: 14, name: "rfPower", kind: Kind::Choice,
          label: "Wireless range", label_de: "Funkreichweite" },
    Def { sub: 19, name: "hsRinger", kind: Kind::Switch,
          label: "Ringtone in headset", label_de: "Klingelton im Headset" },
    Def { sub: 21, name: "soundMode", kind: Kind::Choice,
          label: "Sound mode", label_de: "Klangmodus" },
    Def { sub: 27, name: "powersave", kind: Kind::Switch,
          label: "Power saving", label_de: "Energiesparen" },
    Def { sub: 30, name: "muteReminderInterval", kind: Kind::Choice,
          label: "Mute reminder", label_de: "Stummschalt-Erinnerung" },
    Def { sub: 33, name: "autoOpenHardphone", kind: Kind::Switch,
          label: "Open desk phone line automatically", label_de: "Tischtelefon-Leitung automatisch öffnen" },
    Def { sub: 36, name: "autoOpenSoftphone", kind: Kind::Switch,
          label: "Open softphone line automatically", label_de: "Softphone-Leitung automatisch öffnen" },
    Def { sub: 37, name: "musicMode", kind: Kind::Switch,
          label: "Music mode", label_de: "Musikmodus" },
    Def { sub: 39, name: "buttonFunction", kind: Kind::Choice,
          label: "Button function", label_de: "Tastenbelegung" },
    Def { sub: 50, name: "singleCallConf", kind: Kind::Switch,
          label: "Single call", label_de: "Einzelanruf" },
    Def { sub: 51, name: "idlePowerSave", kind: Kind::Switch,
          label: "Power saving when idle", label_de: "Energiesparen im Leerlauf" },
    Def { sub: 53, name: "hsTouchSensor", kind: Kind::Switch,
          label: "Touch sensor", label_de: "Berührungssensor" },
    Def { sub: 55, name: "ancGain", kind: Kind::Switch,
          label: "Noise cancellation strength", label_de: "Stärke der Geräuschunterdrückung" },
    Def { sub: 57, name: "busylight", kind: Kind::Switch,
          label: "Busylight", label_de: "Besetztlicht" },
    Def { sub: 58, name: "hsVoicePrompts", kind: Kind::Choice,
          label: "Audio announcements", label_de: "Sprachansagen" },
    Def { sub: 59, name: "hsMotionSensor", kind: Kind::Switch,
          label: "Motion sensor", label_de: "Bewegungssensor" },
    Def { sub: 60, name: "autoRejectBgWaiting", kind: Kind::Switch,
          label: "Reject waiting calls automatically", label_de: "Wartende Anrufe automatisch abweisen" },
    Def { sub: 61, name: "ringtoneType", kind: Kind::Switch,
          label: "Ringtone", label_de: "Klingelton" },
    Def { sub: 62, name: "ringOnSecondIncomingCall", kind: Kind::Switch,
          label: "Ring on a second call", label_de: "Klingeln bei zweitem Anruf" },
    Def { sub: 63, name: "buttonSounds", kind: Kind::Switch,
          label: "Button sounds", label_de: "Tastentöne" },
    Def { sub: 64, name: "autoPairing", kind: Kind::Switch,
          label: "Automatic pairing", label_de: "Automatisches Koppeln" },
    Def { sub: 69, name: "acceptCallOnUndock", kind: Kind::Switch,
          label: "Accept a call on undocking", label_de: "Anruf beim Entnehmen annehmen" },
    Def { sub: 74, name: "ctrlBusylight", kind: Kind::Switch,
          label: "Busylight control", label_de: "Besetztlicht-Steuerung" },
    Def { sub: 80, name: "undockOpenAudioLink", kind: Kind::Switch,
          label: "Open the audio link on undocking", label_de: "Audioverbindung beim Entnehmen öffnen" },
    Def { sub: 82, name: "ancLed", kind: Kind::Switch,
          label: "Noise cancellation indicator", label_de: "Anzeige der Geräuschunterdrückung" },
    Def { sub: 83, name: "ancMonitorLed", kind: Kind::Switch,
          label: "Monitor mode indicator", label_de: "Anzeige des Monitor-Modus" },
    Def { sub: 84, name: "audioStreaming", kind: Kind::Switch,
          label: "Audio streaming", label_de: "Audio-Streaming" },
    Def { sub: 92, name: "buttonSwapFunction", kind: Kind::Switch,
          label: "Swapped button functions", label_de: "Vertauschte Tastenbelegung" },
    Def { sub: 104, name: "sidetoneLevel", kind: Kind::Choice,
          label: "Sidetone level", label_de: "Mithörton-Pegel" },
    Def { sub: 112, name: "lowBatteryAudioNotifications", kind: Kind::Switch,
          label: "Low battery announcement", label_de: "Ansage bei schwachem Akku" },
    Def { sub: 114, name: "echoCancel", kind: Kind::Switch,
          label: "Echo cancellation", label_de: "Echounterdrückung" },
    Def { sub: 120, name: "powerNap", kind: Kind::Switch,
          label: "Power nap", label_de: "Power Nap" },
    Def { sub: 124, name: "dspSidetone", kind: Kind::Switch,
          label: "Sidetone", label_de: "Mithörton" },
    Def { sub: 125, name: "equalizer", kind: Kind::Switch,
          label: "Equalizer", label_de: "Equalizer" },
    Def { sub: 126, name: "equalizerEnable", kind: Kind::Switch,
          label: "Equalizer on", label_de: "Equalizer aktiv" },
    Def { sub: 133, name: "sidetoneMute", kind: Kind::Switch,
          label: "Sidetone while muted", label_de: "Mithörton bei Stummschaltung" },
    Def { sub: 134, name: "hallSensor", kind: Kind::Switch,
          label: "Hall sensor", label_de: "Hall-Sensor" },
    Def { sub: 135, name: "anc", kind: Kind::Switch,
          label: "Active noise cancellation", label_de: "Aktive Geräuschunterdrückung" },
    Def { sub: 138, name: "selectButtonFunction", kind: Kind::Switch,
          label: "Select button function", label_de: "Belegung der Auswahltaste" },
    Def { sub: 142, name: "autoPauseMusic", kind: Kind::Switch,
          label: "Pause music when taken off", label_de: "Musik beim Absetzen pausieren" },
    Def { sub: 143, name: "autoMuteCallAudio", kind: Kind::Switch,
          label: "Mute call audio automatically", label_de: "Gesprächston automatisch stummschalten" },
    Def { sub: 144, name: "inactivityInterval", kind: Kind::Choice,
          label: "Auto sleep after", label_de: "Ruhezustand nach" },
    Def { sub: 145, name: "autoAnswerCall", kind: Kind::Switch,
          label: "Answer calls automatically", label_de: "Anrufe automatisch annehmen" },
    Def { sub: 146, name: "onHeadDetection", kind: Kind::Switch,
          label: "On-head detection", label_de: "Trageerkennung" },
    Def { sub: 147, name: "alwaysOnVoice", kind: Kind::Switch,
          label: "Always-on voice assistant", label_de: "Sprachassistent immer aktiv" },
    Def { sub: 149, name: "automaticSpeechRecognition", kind: Kind::Switch,
          label: "Speech recognition", label_de: "Spracherkennung" },
    Def { sub: 152, name: "boomarmRotationAction", kind: Kind::Choice,
          label: "Boom arm rotation", label_de: "Mikrofonarm-Drehung" },
    Def { sub: 153, name: "streamPriority", kind: Kind::Switch,
          label: "Stream priority", label_de: "Stream-Priorität" },
    Def { sub: 188, name: "callAcceptedSound", kind: Kind::Choice,
          label: "Call accepted sound", label_de: "Ton bei angenommenem Anruf" },
];

/// Turns the replies of an RFCOMM sweep into display rows.
pub fn from_sweep(rows: Vec<(&'static str, Vec<u8>)>) -> Vec<Setting> {
    let mut out: Vec<Setting> = rows
        .into_iter()
        .map(|(name, value)| {
            let def = def_of(name);
            Setting {
                device: "Headset",
                label: def.map_or(name, |d| d.label()),
                kind: def.map_or(Kind::Switch, |d| d.kind),
                value,
            }
        })
        .collect();
    // The dialog reads in the order it shows, not in Jabra's.
    out.sort_by_key(|s| s.label);
    out
}

/// The list is short enough that a scan beats carrying a map around.
fn def_of(name: &str) -> Option<&'static Def> {
    SETTINGS.iter().find(|d| d.name == name)
}

/// Subcommand and name for a transport that has no use for the rest.
pub fn subs() -> Vec<(u8, &'static str)> {
    SETTINGS.iter().map(|d| (d.sub, d.name)).collect()
}

pub struct Setting {
    pub device: &'static str,
    pub label: &'static str,
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
            let def = def_of(name);
            found.push(Setting {
                device: label,
                label: def.map_or(name, |d| d.label()),
                kind: def.map_or(Kind::Switch, |d| d.kind),
                value: r.data.to_vec(),
            });
        }
    }
    found.sort_by_key(|s| s.label);
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
