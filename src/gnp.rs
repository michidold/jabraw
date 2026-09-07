//! Jabras GNP-Protokoll auf dem Vendor-Kanal (Report 5, Usage-Page `0xff00`).
//!
//! Der Kernel reicht diesen Report unverändert durch; Akkustand und übrige
//! Statuswerte sind nur darüber erreichbar, eine Battery-System-Usage-Page hat
//! das Gerät nicht.
//!
//! Paketaufbau, ermittelt aus dem Verkehr von Jabras eigenem SDK:
//!
//! ```text
//! Byte 0   Zieladresse       0x01 = Gerät
//! Byte 1   Quelladresse      0x00 = PC
//! Byte 2   Sequenznummer     wird in der Antwort gespiegelt
//! Byte 3   (Typ << 6) | Gesamtlänge in Byte
//! Byte 4   Message-Typ       18 = Status
//! Byte 5   Subcommand        2 = Akkustand des Headsets
//! Byte 6+  Nutzdaten
//! ```

/// Vendor-Report, über den die Kommandos laufen.
pub const REPORT_ID: u8 = 0x05;
/// Nutzlänge des Reports laut Descriptor (`75 08 95 3f`).
const REPORT_SIZE: usize = 63;
const HEADER_LEN: usize = 6;

/// Zieladressen. Dongle und Headset hängen am selben hidraw-Knoten und werden
/// über dieses Byte auseinandergehalten.
pub const DST_DONGLE: u8 = 0x01;
pub const DST_HEADSET: u8 = 0x04;
const SRC_PC: u8 = 0x00;
const TYPE_READ: u8 = 1;
const TYPE_RESPONSE: u8 = 3;

pub const CMD_STATUS: u8 = 18;
pub const SUB_HS_BATTERY: u8 = 2;

pub const CMD_IDENT: u8 = 2;
/// Geräteeinstellungen. Lesend unbedenklich; dieselben Subcommands sind
/// beschreibbar und verändern dann dauerhaft die Konfiguration.
pub const CMD_CONFIG: u8 = 19;
pub const SUB_NAME: u8 = 0;
pub const SUB_SERIAL: u8 = 1;
pub const SUB_VERSION: u8 = 3;

/// Der nackte Paketkopf einer Leseanfrage, ohne Transporthülle.
///
/// Über hidraw kommt die Report-ID davor, über RFCOMM nichts — beides am Gerät
/// geprüft.
pub fn read_body(dst: u8, seq: u8, cmd: u8, sub: u8) -> [u8; HEADER_LEN] {
    [
        dst,
        SRC_PC,
        seq,
        (TYPE_READ << 6) | HEADER_LEN as u8,
        cmd,
        sub,
    ]
}

/// Lese-Anfrage für den hidraw-Knoten: führendes Byte ist die Report-ID.
pub fn read_request(dst: u8, seq: u8, cmd: u8, sub: u8) -> [u8; 1 + REPORT_SIZE] {
    let mut out = [0u8; 1 + REPORT_SIZE];
    out[0] = REPORT_ID;
    out[1..1 + HEADER_LEN].copy_from_slice(&read_body(dst, seq, cmd, sub));
    out
}

pub struct Response<'a> {
    /// Absender: verrät, ob Dongle oder Headset geantwortet hat.
    pub src: u8,
    pub seq: u8,
    pub cmd: u8,
    pub sub: u8,
    pub data: &'a [u8],
}

/// Zerlegt einen eingehenden Report 5. `report` beginnt mit der Report-ID.
pub fn parse(report: &[u8]) -> Option<Response<'_>> {
    parse_body(report.strip_prefix(&[REPORT_ID])?)
}

/// Zerlegt ein Paket ohne Transporthülle, wie es über RFCOMM ankommt.
pub fn parse_body(body: &[u8]) -> Option<Response<'_>> {
    if body.len() < HEADER_LEN {
        return None;
    }
    let len = (body[3] & 0x3F) as usize;
    if body[3] >> 6 != TYPE_RESPONSE || len < HEADER_LEN || len > body.len() {
        return None;
    }
    Some(Response {
        src: body[1],
        seq: body[2],
        cmd: body[4],
        sub: body[5],
        data: &body[HEADER_LEN..len],
    })
}

/// Ladestand in Prozent aus der Antwort auf [`SUB_HS_BATTERY`].
///
/// Byte 1 trägt den Prozentwert. Am Gerät belegt, mit Hin- und Rückweg:
/// `00 20 00 00` bei 32 % ohne Kabel, `01 3a 00 00` bei 58 % am Kabel,
/// `00 3d 00 00` bei 61 % nach dem Abziehen.
///
/// Direkt am Headset ist die Antwort länger: `24 5d 10 64` bei 93 %. Byte 1
/// bleibt der Prozentwert, Byte 2 und 3 steigen beim Laden (`10 64` auf
/// `10 da`) und könnten die Zellspannung sein — unbelegt, deshalb ungenutzt.
pub fn battery_percent(data: &[u8]) -> Option<u8> {
    match data.get(1) {
        Some(&p) if p <= 100 => Some(p),
        _ => None,
    }
}

/// Byte 0 ist ein Bitfeld, Bit 0 der Ladezustand. Auf beiden Wegen geprüft:
/// über den Dongle wechselt es `0x00`/`0x01`, direkt am Headset `0x24`/`0x25`.
/// Die übrigen Bits sind unbelegt, deshalb nur Bit 0 auswerten — ein Vergleich
/// auf ungleich null meldete am Headset dauerhaft "lädt".
pub fn battery_charging(data: &[u8]) -> bool {
    data.first().is_some_and(|&b| b & 1 != 0)
}

/// Textantworten der ident-Gruppe: führendes Längenbyte, dann ASCII.
pub fn text(data: &[u8]) -> Option<String> {
    let (&len, rest) = data.split_first()?;
    let rest = rest.get(..len as usize)?;
    if rest.is_empty() || !rest.iter().all(|&b| (0x20..0x7f).contains(&b)) {
        return None;
    }
    Some(rest.iter().map(|&b| b as char).collect())
}
