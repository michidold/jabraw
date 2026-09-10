//! Interface strings in English and German.
//!
//! No translation framework: thirty-odd strings justify neither gettext nor
//! fluent. Log and error output stays English on purpose — it addresses
//! developers and belongs in the same idiom as the README and the commit
//! messages.

pub struct Strings {
    pub no_device: &'static str,
    pub not_connected: &'static str,
    pub connected: &'static str,
    pub battery: &'static str,
    pub charging_suffix: &'static str,
    pub no_player: &'static str,
    pub play: &'static str,
    pub pause: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    pub speaker_muted: &'static str,
    pub louder: &'static str,
    pub quieter: &'static str,
    pub mic_muted: &'static str,
    pub output_device: &'static str,
    pub device_info: &'static str,
    pub settings: &'static str,
    pub quit: &'static str,
    // Dialogs
    pub headset: &'static str,
    pub dongle: &'static str,
    pub model: &'static str,
    pub firmware: &'static str,
    pub serial: &'static str,
    pub bluetooth_note: &'static str,
    pub device_col: &'static str,
    pub setting: &'static str,
    pub value: &'static str,
    pub no_settings: &'static str,
    pub edit_settings: &'static str,
    pub now: &'static str,
    pub unchanged: &'static str,
    pub nothing_changed: &'static str,
    pub only_shown: &'static str,
    pub set_to: &'static str,
    pub refused: &'static str,
    pub no_answer: &'static str,
    pub on: &'static str,
    pub off: &'static str,
    // Notification
    pub none_found: &'static str,
    pub speaker: &'static str,
    pub microphone: &'static str,
    pub muted_suffix: &'static str,
}

const EN: Strings = Strings {
    no_device: "No Jabra device",
    not_connected: "Not connected",
    connected: "connected",
    battery: "Battery",
    charging_suffix: "charging",
    no_player: "No player active",
    play: "Play",
    pause: "Pause",
    previous: "Previous track",
    next: "Next track",
    speaker_muted: "Mute speaker",
    louder: "Louder",
    quieter: "Quieter",
    mic_muted: "Mute microphone",
    output_device: "Output device",
    device_info: "Device information …",
    settings: "Device settings …",
    quit: "Quit",
    headset: "Headset",
    dongle: "Dongle",
    model: "Model",
    firmware: "Firmware",
    serial: "Serial number",
    bluetooth_note: "Connected over Bluetooth.",
    device_col: "Device",
    setting: "Setting",
    value: "Value",
    no_settings: "The device answered none of the known settings.",
    edit_settings: "Change settings …",
    now: "now",
    unchanged: "leave as it is",
    nothing_changed: "Nothing was changed",
    only_shown: "This setting can only be shown",
    set_to: "set to",
    refused: "The device refused the change",
    no_answer: "The device did not answer",
    on: "on",
    off: "off",
    none_found: "No Jabra device found",
    speaker: "Speaker",
    microphone: "Microphone",
    muted_suffix: "muted",
};

const DE: Strings = Strings {
    no_device: "Kein Jabra-Gerät",
    not_connected: "Nicht verbunden",
    connected: "verbunden",
    battery: "Akku",
    charging_suffix: "lädt",
    no_player: "Kein Player aktiv",
    play: "Wiedergabe",
    pause: "Pause",
    previous: "Vorheriger Titel",
    next: "Nächster Titel",
    speaker_muted: "Lautsprecher stumm",
    louder: "Lauter",
    quieter: "Leiser",
    mic_muted: "Mikrofon stumm",
    output_device: "Ausgabegerät",
    device_info: "Geräteinformationen …",
    settings: "Geräteeinstellungen …",
    quit: "Beenden",
    headset: "Headset",
    dongle: "Dongle",
    model: "Modell",
    firmware: "Firmware",
    serial: "Seriennummer",
    bluetooth_note: "Über Bluetooth verbunden.",
    device_col: "Gerät",
    setting: "Einstellung",
    value: "Wert",
    no_settings: "Das Gerät hat auf keine der bekannten Einstellungen geantwortet.",
    edit_settings: "Einstellungen ändern …",
    now: "jetzt",
    unchanged: "unverändert lassen",
    nothing_changed: "Nichts geändert",
    only_shown: "Diese Einstellung kann nur angezeigt werden",
    set_to: "gesetzt auf",
    refused: "Das Gerät hat die Änderung abgelehnt",
    no_answer: "Das Gerät hat nicht geantwortet",
    on: "ein",
    off: "aus",
    none_found: "Kein Jabra-Gerät gefunden",
    speaker: "Lautsprecher",
    microphone: "Mikrofon",
    muted_suffix: "stumm",
};

/// Language from the environment. English is the default, German only on an
/// explicitly German locale.
pub fn german() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .unwrap_or_default()
        .starts_with("de")
}

pub fn strings() -> &'static Strings {
    if german() {
        &DE
    } else {
        &EN
    }
}
