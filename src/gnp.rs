//! Jabra's GNP protocol on the vendor report (report 5, usage page `0xff00`).
//!
//! The kernel passes this report through untouched. Battery level and the other
//! status values are reachable only here; the device exposes no battery system
//! usage page.
//!
//! Packet layout, derived from the traffic Jabra's own SDK produces:
//!
//! ```text
//! byte 0   destination      0x01 = device
//! byte 1   source           0x00 = PC
//! byte 2   sequence number  mirrored in the reply
//! byte 3   (type << 6) | total length in bytes
//! byte 4   message type     18 = status
//! byte 5   subcommand       2 = headset battery
//! byte 6+  payload
//! ```

/// Vendor report the commands travel on.
pub const REPORT_ID: u8 = 0x05;
/// Payload length of the report per the descriptor (`75 08 95 3f`).
const REPORT_SIZE: usize = 63;
pub const HEADER_LEN: usize = 6;

/// Destination addresses. Dongle and headset sit behind the same hidraw node
/// and are told apart by this byte.
pub const DST_DONGLE: u8 = 0x01;
pub const DST_HEADSET: u8 = 0x04;
const SRC_PC: u8 = 0x00;
const TYPE_READ: u8 = 1;
const TYPE_RESPONSE: u8 = 3;

pub const CMD_STATUS: u8 = 18;
pub const SUB_HS_BATTERY: u8 = 2;

pub const CMD_IDENT: u8 = 2;
/// Device settings. Harmless to read; the same subcommands are writable and
/// then change the configuration for good.
pub const CMD_CONFIG: u8 = 19;
pub const SUB_NAME: u8 = 0;
pub const SUB_SERIAL: u8 = 1;
pub const SUB_VERSION: u8 = 3;

/// The bare packet header of a read request, without transport wrapping.
///
/// Over hidraw the report id goes in front, over RFCOMM nothing does. Both
/// checked against the hardware.
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

/// Read request for the hidraw node: the leading byte is the report id.
pub fn read_request(dst: u8, seq: u8, cmd: u8, sub: u8) -> [u8; 1 + REPORT_SIZE] {
    let mut out = [0u8; 1 + REPORT_SIZE];
    out[0] = REPORT_ID;
    out[1..1 + HEADER_LEN].copy_from_slice(&read_body(dst, seq, cmd, sub));
    out
}

pub struct Response<'a> {
    /// Sender: says whether the dongle or the headset answered.
    pub src: u8,
    pub seq: u8,
    pub cmd: u8,
    pub sub: u8,
    pub data: &'a [u8],
}

/// Splits an incoming report 5. `report` starts with the report id.
pub fn parse(report: &[u8]) -> Option<Response<'_>> {
    parse_body(report.strip_prefix(&[REPORT_ID])?)
}

/// Splits a packet without transport wrapping, as it arrives over RFCOMM.
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

/// Charge level in percent from a [`SUB_HS_BATTERY`] reply.
///
/// Byte 1 carries the percentage. Established against the hardware in both
/// directions: `00 20 00 00` at 32% off the cable, `01 3a 00 00` at 58% on it,
/// `00 3d 00 00` at 61% after unplugging.
///
/// Queried straight at the headset the reply is longer: `24 5d 10 64` at 93%.
/// Byte 1 stays the percentage; bytes 2 and 3 rise while charging (`10 64` to
/// `10 da`) and could be cell voltage, which is unestablished and so unused.
pub fn battery_percent(data: &[u8]) -> Option<u8> {
    match data.get(1) {
        Some(&p) if p <= 100 => Some(p),
        _ => None,
    }
}

/// Byte 0 is a bit field and bit 0 the charging state. Checked on both
/// transports: through the dongle it toggles `0x00`/`0x01`, straight at the
/// headset `0x24`/`0x25`. The remaining bits are unestablished, so only bit 0
/// is read — testing for non-zero claimed charging permanently on the headset.
pub fn battery_charging(data: &[u8]) -> bool {
    data.first().is_some_and(|&b| b & 1 != 0)
}

/// Text replies of the ident group: a leading length byte, then ASCII.
pub fn text(data: &[u8]) -> Option<String> {
    let (&len, rest) = data.split_first()?;
    let rest = rest.get(..len as usize)?;
    if rest.is_empty() || !rest.iter().all(|&b| (0x20..0x7f).contains(&b)) {
        return None;
    }
    Some(rest.iter().map(|&b| b as char).collect())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_not_a_response() {
        let req = read_body(DST_HEADSET, 7, CMD_IDENT, SUB_NAME);
        assert!(parse_body(&req).is_none());
    }

    #[test]
    fn a_length_past_the_packet_is_rejected() {
        let body = [0, DST_HEADSET, 7, (TYPE_RESPONSE << 6) | 20, CMD_IDENT, SUB_NAME];
        assert!(parse_body(&body).is_none());
    }

    #[test]
    fn the_payload_is_cut_at_the_length_field() {
        let mut body = vec![0, DST_HEADSET, 7, (TYPE_RESPONSE << 6) | 8, CMD_IDENT, SUB_NAME];
        body.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        let r = parse_body(&body).unwrap();
        assert_eq!((r.src, r.seq, r.cmd, r.sub), (DST_HEADSET, 7, CMD_IDENT, SUB_NAME));
        assert_eq!(r.data, &[0xaa, 0xbb]);
    }

    #[test]
    fn over_hidraw_the_report_id_has_to_be_there() {
        let body = [0, DST_HEADSET, 7, (TYPE_RESPONSE << 6) | 6, CMD_IDENT, SUB_NAME];
        assert!(parse(&body).is_none());
        let mut report = vec![REPORT_ID];
        report.extend_from_slice(&body);
        assert!(parse(&report).is_some());
    }

    #[test]
    fn text_stops_at_the_length_byte() {
        assert_eq!(text(&[3, b'a', b'b', b'c', b'd']).as_deref(), Some("abc"));
    }

    #[test]
    fn text_rejects_what_it_cannot_show() {
        // Length past the end, a control character, and nothing at all.
        assert!(text(&[9, b'a']).is_none());
        assert!(text(&[2, b'a', 0x07]).is_none());
        assert!(text(&[0]).is_none());
        assert!(text(&[]).is_none());
    }

    #[test]
    fn the_charge_level_is_the_second_byte() {
        assert_eq!(battery_percent(&[0x00, 32, 0, 0]), Some(32));
        // Above 100 is not a percentage, so it is no answer either.
        assert_eq!(battery_percent(&[0x00, 101]), None);
        assert_eq!(battery_percent(&[0x00]), None);
    }

    #[test]
    fn charging_is_bit_zero_of_the_first() {
        assert!(battery_charging(&[0x25]));
        assert!(!battery_charging(&[0x24]));
        assert!(!battery_charging(&[]));
    }
}
