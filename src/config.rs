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

/// A value a setting can take, and the byte the device uses for it.
pub struct Choice {
    pub raw: u8,
    pub label: &'static str,
    pub label_de: &'static str,
}

impl Choice {
    pub fn label(&self) -> &'static str {
        if crate::i18n::german() {
            self.label_de
        } else {
            self.label
        }
    }
}

// The byte for each value comes from jabridge (github.com/Watchdog0x/jLink,
// Apache-2.0), whose table was established against hardware. Only the settings
// it addresses without a prefix byte or a bit mask are taken over, because
// those are the ones our plain read returns as a single byte. See
// doc/settings-sources.md.
#[rustfmt::skip]
const AUDIO_PROTECTION: &[Choice] = &[
    Choice { raw: 0, label: "Basic PeakStop", label_de: "Basic PeakStop" },
    Choice { raw: 1, label: "Level 1", label_de: "Stufe 1" },
    Choice { raw: 2, label: "Level 2", label_de: "Stufe 2" },
    Choice { raw: 3, label: "Level 3", label_de: "Stufe 3" },
    Choice { raw: 4, label: "Level 4", label_de: "Stufe 4" },
    Choice { raw: 5, label: "G616", label_de: "G616" },
];

#[rustfmt::skip]
const SOUND_MODE: &[Choice] = &[
    Choice { raw: 0, label: "Normal", label_de: "Normal" },
    Choice { raw: 1, label: "Bass", label_de: "Bass" },
    Choice { raw: 2, label: "Treble", label_de: "Höhen" },
];

#[rustfmt::skip]
const MUTE_REMINDER: &[Choice] = &[
    Choice { raw: 0, label: "Off", label_de: "Aus" },
    Choice { raw: 10, label: "10 seconds", label_de: "10 Sekunden" },
    Choice { raw: 20, label: "20 seconds", label_de: "20 Sekunden" },
    Choice { raw: 30, label: "30 seconds", label_de: "30 Sekunden" },
    Choice { raw: 40, label: "40 seconds", label_de: "40 Sekunden" },
    Choice { raw: 50, label: "50 seconds", label_de: "50 Sekunden" },
    Choice { raw: 60, label: "60 seconds", label_de: "60 Sekunden" },
];

#[rustfmt::skip]
const VOICE_PROMPTS: &[Choice] = &[
    Choice { raw: 0, label: "Tones", label_de: "Töne" },
    Choice { raw: 1, label: "Voice", label_de: "Sprache" },
    Choice { raw: 2, label: "Off", label_de: "Aus" },
];

// Negative levels as a signed byte: -9 dB arrives as 0xf7.
#[rustfmt::skip]
const SIDETONE_LEVEL: &[Choice] = &[
    Choice { raw: 0xf7, label: "-9 dB", label_de: "-9 dB" },
    Choice { raw: 0xfa, label: "-6 dB", label_de: "-6 dB" },
    Choice { raw: 0xfc, label: "-4 dB", label_de: "-4 dB" },
    Choice { raw: 0xfd, label: "-3 dB", label_de: "-3 dB" },
    Choice { raw: 0xfe, label: "-2 dB", label_de: "-2 dB" },
    Choice { raw: 0, label: "0 dB", label_de: "0 dB" },
    Choice { raw: 2, label: "+2 dB", label_de: "+2 dB" },
    Choice { raw: 3, label: "+3 dB", label_de: "+3 dB" },
    Choice { raw: 4, label: "+4 dB", label_de: "+4 dB" },
    Choice { raw: 6, label: "+6 dB", label_de: "+6 dB" },
];

#[rustfmt::skip]
const AUTO_SLEEP: &[Choice] = &[
    Choice { raw: 0, label: "Never", label_de: "Nie" },
    Choice { raw: 3, label: "30 minutes", label_de: "30 Minuten" },
    Choice { raw: 6, label: "1 hour", label_de: "1 Stunde" },
    Choice { raw: 12, label: "2 hours", label_de: "2 Stunden" },
    Choice { raw: 24, label: "4 hours", label_de: "4 Stunden" },
    Choice { raw: 48, label: "8 hours", label_de: "8 Stunden" },
    Choice { raw: 72, label: "12 hours", label_de: "12 Stunden" },
    Choice { raw: 96, label: "16 hours", label_de: "16 Stunden" },
];

#[rustfmt::skip]
const RINGER_VOLUME: &[Choice] = &[
    Choice { raw: 0, label: "Off", label_de: "Aus" },
    Choice { raw: 1, label: "Low", label_de: "Leise" },
    Choice { raw: 2, label: "Medium", label_de: "Mittel" },
    Choice { raw: 3, label: "High", label_de: "Laut" },
];

#[rustfmt::skip]
const RINGTONE: &[Choice] = &[
    Choice { raw: 0, label: "Tone 1", label_de: "Ton 1" },
    Choice { raw: 1, label: "Tone 2", label_de: "Ton 2" },
    Choice { raw: 2, label: "Tone 3", label_de: "Ton 3" },
    Choice { raw: 3, label: "Tone 4", label_de: "Ton 4" },
    Choice { raw: 4, label: "Tone 5", label_de: "Ton 5" },
    Choice { raw: 5, label: "Tone 6", label_de: "Ton 6" },
    Choice { raw: 6, label: "Tone 7", label_de: "Ton 7" },
    Choice { raw: 7, label: "Tone 8", label_de: "Ton 8" },
    Choice { raw: 8, label: "Tone 9", label_de: "Ton 9" },
    Choice { raw: 9, label: "Tone 10", label_de: "Ton 10" },
    Choice { raw: 128, label: "Custom", label_de: "Eigener" },
    Choice { raw: 254, label: "Random", label_de: "Zufällig" },
    Choice { raw: 255, label: "Off", label_de: "Aus" },
];

#[rustfmt::skip]
const BOOM_ARM: &[Choice] = &[
    Choice { raw: 0, label: "Disabled", label_de: "Deaktiviert" },
    Choice { raw: 1, label: "Mute", label_de: "Stummschalten" },
    Choice { raw: 4, label: "End call", label_de: "Anruf beenden" },
    Choice { raw: 8, label: "Full mute", label_de: "Komplett stumm" },
];

#[rustfmt::skip]
const BUTTON_FUNCTION: &[Choice] = &[
    Choice { raw: 0, label: "None", label_de: "Keine" },
    Choice { raw: 1, label: "Call handling", label_de: "Anrufsteuerung" },
    Choice { raw: 2, label: "Mute", label_de: "Stummschalten" },
    Choice { raw: 12, label: "Speed dial", label_de: "Kurzwahl" },
    Choice { raw: 16, label: "Push to talk", label_de: "Push-to-talk" },
    Choice { raw: 17, label: "Busylight", label_de: "Besetztlicht" },
    Choice { raw: 19, label: "Cortana", label_de: "Cortana" },
    Choice { raw: 20, label: "Music", label_de: "Musik" },
];

#[rustfmt::skip]
const CALL_ACCEPTED: &[Choice] = &[
    Choice { raw: 0, label: "Sound effects", label_de: "Klangeffekte" },
    Choice { raw: 1, label: "Voice prompts", label_de: "Sprachansagen" },
    Choice { raw: 2, label: "Off", label_de: "Aus" },
];

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
    /// Empty where the byte for each value is not established.
    pub values: &'static [Choice],
    /// Sub-item to ask for, where one subcommand addresses several.
    pub request: &'static [u8],
    /// Which byte of the reply carries the value, and which of its bits.
    pub index: usize,
    /// Zero means the whole byte.
    pub mask: u8,
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

/// Settings that mean something to a user. Other subcommands in the group are
/// operational data — clock, feature masks, password fields — and do not belong
/// in an overview.
// One row per setting; rustfmt would give every field a line of its own and
// turn the table into three hundred lines.
#[rustfmt::skip]
pub const SETTINGS: &[Def] = &[
    Def { sub: 0, name: "audioType", kind: Kind::Switch,
          label: "Audio type", label_de: "Audiotyp",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 1, name: "intellitoneLevel", kind: Kind::Choice,
          label: "Hearing protection", label_de: "Gehörschutz",
          values: AUDIO_PROTECTION, request: &[], index: 0, mask: 0x00 },
    Def { sub: 2, name: "touchAudioFeedback", kind: Kind::Switch,
          label: "Touch feedback sounds", label_de: "Töne bei Berührung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 3, name: "ringerVolume", kind: Kind::Choice,
          label: "Ringtone volume", label_de: "Klingellautstärke",
          values: RINGER_VOLUME, request: &[0], index: 1, mask: 0x00 },
    Def { sub: 5, name: "phonePresence", kind: Kind::Switch,
          label: "Phone presence", label_de: "Telefon-Präsenz",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 8, name: "currentLanguage", kind: Kind::Choice,
          label: "Device language", label_de: "Gerätesprache",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 13, name: "micGain", kind: Kind::Switch,
          label: "Microphone gain", label_de: "Mikrofonverstärkung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 14, name: "rfPower", kind: Kind::Choice,
          label: "Wireless range", label_de: "Funkreichweite",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 19, name: "hsRinger", kind: Kind::Switch,
          label: "Ringtone in headset", label_de: "Klingelton im Headset",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 21, name: "soundMode", kind: Kind::Choice,
          label: "Sound mode", label_de: "Klangmodus",
          values: SOUND_MODE, request: &[], index: 0, mask: 0x00 },
    Def { sub: 27, name: "powersave", kind: Kind::Switch,
          label: "Power saving", label_de: "Energiesparen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 30, name: "muteReminderInterval", kind: Kind::Choice,
          label: "Mute reminder", label_de: "Stummschalt-Erinnerung",
          values: MUTE_REMINDER, request: &[], index: 0, mask: 0x00 },
    Def { sub: 33, name: "autoOpenHardphone", kind: Kind::Switch,
          label: "Open desk phone line automatically", label_de: "Tischtelefon-Leitung automatisch öffnen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 36, name: "autoOpenSoftphone", kind: Kind::Switch,
          label: "Open softphone line automatically", label_de: "Softphone-Leitung automatisch öffnen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 37, name: "musicMode", kind: Kind::Switch,
          label: "Music mode", label_de: "Musikmodus",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 39, name: "buttonFunction", kind: Kind::Choice,
          label: "Button function", label_de: "Tastenbelegung",
          values: BUTTON_FUNCTION, request: &[0, 0], index: 2, mask: 0x00 },
    Def { sub: 50, name: "singleCallConf", kind: Kind::Switch,
          label: "Single call", label_de: "Einzelanruf",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 51, name: "idlePowerSave", kind: Kind::Switch,
          label: "Power saving when idle", label_de: "Energiesparen im Leerlauf",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 53, name: "hsTouchSensor", kind: Kind::Switch,
          label: "Touch sensor", label_de: "Berührungssensor",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 55, name: "ancGain", kind: Kind::Switch,
          label: "Noise cancellation strength", label_de: "Stärke der Geräuschunterdrückung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 57, name: "busylight", kind: Kind::Switch,
          label: "Busylight", label_de: "Besetztlicht",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 58, name: "hsVoicePrompts", kind: Kind::Choice,
          label: "Audio announcements", label_de: "Sprachansagen",
          values: VOICE_PROMPTS, request: &[], index: 0, mask: 0x00 },
    Def { sub: 59, name: "hsMotionSensor", kind: Kind::Switch,
          label: "Motion sensor", label_de: "Bewegungssensor",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 60, name: "autoRejectBgWaiting", kind: Kind::Switch,
          label: "Reject waiting calls automatically", label_de: "Wartende Anrufe automatisch abweisen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 61, name: "ringtoneType", kind: Kind::Choice,
          label: "Ringtone", label_de: "Klingelton",
          values: RINGTONE, request: &[0], index: 1, mask: 0x00 },
    Def { sub: 62, name: "ringOnSecondIncomingCall", kind: Kind::Switch,
          label: "Ring on a second call", label_de: "Klingeln bei zweitem Anruf",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 63, name: "buttonSounds", kind: Kind::Switch,
          label: "Button sounds", label_de: "Tastentöne",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 64, name: "autoPairing", kind: Kind::Switch,
          label: "Automatic pairing", label_de: "Automatisches Koppeln",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 69, name: "acceptCallOnUndock", kind: Kind::Switch,
          label: "Accept a call on undocking", label_de: "Anruf beim Entnehmen annehmen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 74, name: "ctrlBusylight", kind: Kind::Switch,
          label: "Busylight control", label_de: "Besetztlicht-Steuerung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 80, name: "undockOpenAudioLink", kind: Kind::Switch,
          label: "Open the audio link on undocking", label_de: "Audioverbindung beim Entnehmen öffnen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 82, name: "ancLed", kind: Kind::Switch,
          label: "Noise cancellation indicator", label_de: "Anzeige der Geräuschunterdrückung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 83, name: "ancMonitorLed", kind: Kind::Switch,
          label: "Monitor mode indicator", label_de: "Anzeige des Monitor-Modus",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 84, name: "audioStreaming", kind: Kind::Switch,
          label: "Audio streaming", label_de: "Audio-Streaming",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 92, name: "buttonSwapFunction", kind: Kind::Switch,
          label: "Swapped button functions", label_de: "Vertauschte Tastenbelegung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 104, name: "sidetoneLevel", kind: Kind::Choice,
          label: "Sidetone level", label_de: "Mithörton-Pegel",
          values: SIDETONE_LEVEL, request: &[], index: 0, mask: 0x00 },
    Def { sub: 112, name: "lowBatteryAudioNotifications", kind: Kind::Switch,
          label: "Low battery announcement", label_de: "Ansage bei schwachem Akku",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 114, name: "echoCancel", kind: Kind::Switch,
          label: "Echo cancellation", label_de: "Echounterdrückung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 120, name: "powerNap", kind: Kind::Switch,
          label: "Power nap", label_de: "Power Nap",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 124, name: "dspSidetone", kind: Kind::Choice,
          label: "Sidetone", label_de: "Mithörton",
          values: SIDETONE_LEVEL, request: &[], index: 1, mask: 0x00 },
    Def { sub: 125, name: "equalizer", kind: Kind::Switch,
          label: "Equalizer", label_de: "Equalizer",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 126, name: "equalizerEnable", kind: Kind::Switch,
          label: "Equalizer on", label_de: "Equalizer aktiv",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 133, name: "sidetoneMute", kind: Kind::Switch,
          label: "Sidetone while muted", label_de: "Mithörton bei Stummschaltung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 134, name: "hallSensor", kind: Kind::Switch,
          label: "Hall sensor", label_de: "Hall-Sensor",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 135, name: "anc", kind: Kind::Switch,
          label: "Active noise cancellation", label_de: "Aktive Geräuschunterdrückung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 138, name: "selectButtonFunction", kind: Kind::Switch,
          label: "Select button function", label_de: "Belegung der Auswahltaste",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 142, name: "autoPauseMusic", kind: Kind::Switch,
          label: "Pause music when taken off", label_de: "Musik beim Absetzen pausieren",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 143, name: "autoMuteCallAudio", kind: Kind::Switch,
          label: "Mute call audio automatically", label_de: "Gesprächston automatisch stummschalten",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 144, name: "inactivityInterval", kind: Kind::Choice,
          label: "Auto sleep after", label_de: "Ruhezustand nach",
          values: AUTO_SLEEP, request: &[], index: 0, mask: 0x00 },
    Def { sub: 145, name: "autoAnswerCall", kind: Kind::Switch,
          label: "Answer calls automatically", label_de: "Anrufe automatisch annehmen",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 146, name: "onHeadDetection", kind: Kind::Switch,
          label: "On-head detection", label_de: "Trageerkennung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 147, name: "alwaysOnVoice", kind: Kind::Switch,
          label: "Always-on voice assistant", label_de: "Sprachassistent immer aktiv",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 149, name: "automaticSpeechRecognition", kind: Kind::Switch,
          label: "Speech recognition", label_de: "Spracherkennung",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 152, name: "boomarmRotationAction", kind: Kind::Choice,
          label: "Boom arm rotation", label_de: "Mikrofonarm-Drehung",
          values: BOOM_ARM, request: &[], index: 0, mask: 0x0d },
    Def { sub: 153, name: "streamPriority", kind: Kind::Switch,
          label: "Stream priority", label_de: "Stream-Priorität",
          values: &[], request: &[], index: 0, mask: 0x00 },
    Def { sub: 188, name: "callAcceptedSound", kind: Kind::Choice,
          label: "Call accepted sound", label_de: "Ton bei angenommenem Anruf",
          values: CALL_ACCEPTED, request: &[], index: 0, mask: 0x00 },
];

/// Turns the replies of an RFCOMM sweep into display rows.
pub fn from_sweep(rows: Vec<(&'static str, Vec<u8>)>) -> Vec<Setting> {
    let mut out: Vec<Setting> = rows
        .into_iter()
        .map(|(name, value)| {
            let def = def_of(name);
            Setting {
                device: "Headset",
                name,
                label: def.map_or(name, |d| d.label()),
                kind: def.map_or(Kind::Switch, |d| d.kind),
                values: def.map_or(&[][..], |d| d.values),
                raw: raw_of(def, &value),
                value,
            }
        })
        .collect();
    // The dialog reads in the order it shows, not in Jabra's.
    out.sort_by_key(|s| s.label);
    out
}

/// The list is short enough that a scan beats carrying a map around.
pub fn def_of(name: &str) -> Option<&'static Def> {
    SETTINGS.iter().find(|d| d.name == name)
}

fn raw_of(def: Option<&Def>, data: &[u8]) -> Option<u8> {
    let def = def?;
    let byte = *data.get(def.index)?;
    Some(if def.mask == 0 { byte } else { byte & def.mask })
}

/// Subcommand, name and the sub-item to ask for.
pub fn subs() -> Vec<(u8, &'static str, &'static [u8])> {
    SETTINGS
        .iter()
        .map(|d| (d.sub, d.name, d.request))
        .collect()
}

pub struct Setting {
    pub device: &'static str,
    /// Jabra's identifier, carried through the dialog to name the setting a
    /// write is meant for.
    pub name: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub values: &'static [Choice],
    /// The byte the value lives in, once index and mask have been applied.
    pub raw: Option<u8>,
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
        file.write_all(&gnp::read_request(
            dst,
            seq,
            gnp::CMD_CONFIG,
            def.sub,
            def.request,
        ))?;
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
                name,
                label: def.map_or(name, |d| d.label()),
                kind: def.map_or(Kind::Switch, |d| d.kind),
                values: def.map_or(&[][..], |d| d.values),
                raw: raw_of(def, r.data),
                value: r.data.to_vec(),
            });
        }
    }
    found.sort_by_key(|s| s.label);
    Ok(found)
}

/// Changes one setting and waits for the device to answer for it.
///
/// Only where the value has a byte to itself: a setting that shares one with
/// its neighbours would need the rest of that byte carried over, and that is
/// not established here. Blocking — belongs in `spawn_blocking`.
pub fn write(path: &str, dst: u8, def: &Def, raw: u8) -> std::io::Result<Option<gnp::Ack>> {
    if def.mask != 0 {
        return Ok(None);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let seq = 0x21;
    let mut payload = def.request.to_vec();
    payload.push(raw);
    file.write_all(&gnp::write_request(
        dst,
        seq,
        gnp::CMD_CONFIG,
        def.sub,
        &payload,
    ))?;

    let deadline = Instant::now() + Duration::from_millis(2000);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        if !readable(&file, deadline - Instant::now()) {
            break;
        }
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if let Some(ack) = gnp::write_ack(&buf[..n], dst, seq) {
            return Ok(Some(ack));
        }
    }
    Ok(None)
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
