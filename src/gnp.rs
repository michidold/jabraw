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

const DST_DEVICE: u8 = 0x01;
const SRC_PC: u8 = 0x00;
const TYPE_READ: u8 = 1;
const TYPE_RESPONSE: u8 = 3;

pub const CMD_STATUS: u8 = 18;
pub const SUB_HS_BATTERY: u8 = 2;

/// Lese-Anfrage, fertig zum Schreiben auf den hidraw-Knoten (führendes Byte ist
/// die Report-ID).
pub fn read_request(seq: u8, cmd: u8, sub: u8) -> [u8; 1 + REPORT_SIZE] {
    let mut out = [0u8; 1 + REPORT_SIZE];
    out[0] = REPORT_ID;
    out[1] = DST_DEVICE;
    out[2] = SRC_PC;
    out[3] = seq;
    out[4] = (TYPE_READ << 6) | HEADER_LEN as u8;
    out[5] = cmd;
    out[6] = sub;
    out
}

pub struct Response<'a> {
    pub seq: u8,
    pub cmd: u8,
    pub sub: u8,
    pub data: &'a [u8],
}

/// Zerlegt einen eingehenden Report 5. `report` beginnt mit der Report-ID.
pub fn parse(report: &[u8]) -> Option<Response<'_>> {
    let body = report.strip_prefix(&[REPORT_ID])?;
    if body.len() < HEADER_LEN {
        return None;
    }
    let len = (body[3] & 0x3F) as usize;
    if body[3] >> 6 != TYPE_RESPONSE || len < HEADER_LEN || len > body.len() {
        return None;
    }
    Some(Response {
        seq: body[2],
        cmd: body[4],
        sub: body[5],
        data: &body[HEADER_LEN..len],
    })
}

/// Ladestand in Prozent aus der Antwort auf [`SUB_HS_BATTERY`].
///
/// Beobachtet wurde `00 20 00 00` bei 32 %. Byte 1 trägt den Prozentwert; die
/// Bedeutung von Byte 0 ist unbekannt, Byte 2 und 3 waren stets 0 und dürften
/// nach dem Vorbild der SDK-Struktur "lädt" und "fast leer" sein.
pub fn battery_percent(data: &[u8]) -> Option<u8> {
    match data.get(1) {
        Some(&p) if p <= 100 => Some(p),
        _ => None,
    }
}

pub fn battery_charging(data: &[u8]) -> bool {
    data.get(2).is_some_and(|&b| b != 0)
}
