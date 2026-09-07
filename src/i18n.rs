//! Oberflächentexte in Deutsch und Englisch.
//!
//! Kein Übersetzungsrahmenwerk: rund dreißig Zeichenketten rechtfertigen weder
//! gettext noch fluent. Log- und Fehlerausgaben bleiben bewusst englisch, sie
//! richten sich an Entwickler und stehen so im selben Idiom wie README und
//! Commit-Nachrichten.

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
    // Dialog
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
    pub on: &'static str,
    pub off: &'static str,
    // Benachrichtigung
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
    bluetooth_note: "Connected over Bluetooth. Charging state needs the USB dongle.",
    device_col: "Device",
    setting: "Setting",
    value: "Value",
    no_settings: "The device answered none of the known settings.",
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
    bluetooth_note: "Über Bluetooth verbunden. Der Ladezustand braucht den USB-Dongle.",
    device_col: "Gerät",
    setting: "Einstellung",
    value: "Wert",
    no_settings: "Das Gerät hat auf keine der bekannten Einstellungen geantwortet.",
    on: "ein",
    off: "aus",
    none_found: "Kein Jabra-Gerät gefunden",
    speaker: "Lautsprecher",
    microphone: "Mikrofon",
    muted_suffix: "stumm",
};

/// Sprache aus der Umgebung. Englisch ist die Vorgabe, Deutsch nur bei
/// ausdrücklich deutscher Locale.
pub fn strings() -> &'static Strings {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .unwrap_or_default();
    if locale.starts_with("de") {
        &DE
    } else {
        &EN
    }
}
