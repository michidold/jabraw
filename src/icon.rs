//! Das Tray-Symbol: schwarzes Headset auf gelbem Quadrat mit runden Ecken.
//!
//! Programmatisch gezeichnet statt als Bilddatei mitgeliefert. Das erspart
//! einen PNG-Decoder als Abhängigkeit, und der Host kann jede Kantenlänge
//! anfragen — Panels reichen von 16 bis 64 Pixel.

/// Gelb des Hintergrunds und Schwarz des Headsets.
const YELLOW: (u8, u8, u8) = (0xFF, 0xC1, 0x07);
const BLACK: (u8, u8, u8) = (0x14, 0x14, 0x14);
/// Kantenglättung durch Überabtastung; 4×4 reicht bei diesen Größen.
const SS: u32 = 4;

/// Die Größen, die Panels üblicherweise anfragen. Der Host sucht sich die
/// passende aus.
const SIZES: [u32; 3] = [22, 32, 48];

/// Gerenderte Symbole, einmal je Prozesslauf. Der Tray fragt die Eigenschaft
/// bei jeder Aktualisierung ab; neu zu rechnen wäre reine Verschwendung.
pub fn pixmaps() -> &'static [(u32, Vec<u8>)] {
    static CACHE: std::sync::OnceLock<Vec<(u32, Vec<u8>)>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| SIZES.iter().map(|&s| (s, render(s))).collect())
}

/// ARGB32 in Network Byte Order, wie die StatusNotifierItem-Spezifikation es
/// verlangt.
pub fn render(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let n = size as f32;
    for y in 0..size {
        for x in 0..size {
            let (mut bg, mut fg) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n;
                    if plate(px, py) {
                        bg += 1.0;
                    }
                    if headset(px, py) {
                        fg += 1.0;
                    }
                }
            }
            let samples = (SS * SS) as f32;
            let (bg, fg) = (bg / samples, fg / samples);
            // Headset liegt auf der Platte; ausserhalb der Platte durchsichtig.
            let alpha = bg;
            let mix = |c1: u8, c2: u8| (c1 as f32 * (1.0 - fg) + c2 as f32 * fg) as u8;
            out.push((alpha * 255.0) as u8);
            out.push(mix(YELLOW.0, BLACK.0));
            out.push(mix(YELLOW.1, BLACK.1));
            out.push(mix(YELLOW.2, BLACK.2));
        }
    }
    out
}

fn plate(x: f32, y: f32) -> bool {
    rounded_rect(x, y, 0.02, 0.02, 0.98, 0.98, 0.26)
}

fn headset(x: f32, y: f32) -> bool {
    // Bügel als oberer Halbring, Ohrmuscheln als abgerundete Rechtecke,
    // Mikrofonarm als Kapsel — sonst wäre es ein Kopfhörer, kein Headset.
    let band = ring(x, y, 0.5, 0.60, 0.245, 0.33) && y <= 0.60;
    let left = rounded_rect(x, y, 0.155, 0.545, 0.315, 0.80, 0.075);
    let right = rounded_rect(x, y, 0.685, 0.545, 0.845, 0.80, 0.075);
    let boom = capsule(x, y, 0.30, 0.735, 0.50, 0.845, 0.032);
    band || left || right || boom
}

fn rounded_rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    (x - cx).hypot(y - cy) <= r
}

fn ring(x: f32, y: f32, cx: f32, cy: f32, inner: f32, outer: f32) -> bool {
    let d = (x - cx).hypot(y - cy);
    d >= inner && d <= outer
}

fn capsule(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32) -> bool {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((x - ax) * dx + (y - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    (x - (ax + t * dx)).hypot(y - (ay + t * dy)) <= r
}

/// Das Symbol als PNG, für den Eintrag in der Anwendungsübersicht.
///
/// Eigener Encoder statt einer Bibliothek: PNG braucht einen zlib-Strom, und
/// der darf laut Format unkomprimierte Blöcke enthalten. Für ein paar Kilobyte
/// Symbol lohnt keine Abhängigkeit.
pub fn png(size: u32) -> Vec<u8> {
    let argb = render(size);
    let mut raw = Vec::with_capacity((size * (size * 4 + 1)) as usize);
    for y in 0..size {
        raw.push(0); // Filtertyp "None"
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            let (a, r, g, b) = (argb[i], argb[i + 1], argb[i + 2], argb[i + 3]);
            raw.extend_from_slice(&[r, g, b, a]);
        }
    }

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = Vec::with_capacity(4 + data.len());
    crc.extend_from_slice(tag);
    crc.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc).to_be_bytes());
}

/// zlib-Strom aus gespeicherten, also unkomprimierten Deflate-Blöcken.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, part) in data.chunks(0xFFFF).enumerate() {
        let last = (i + 1) * 0xFFFF >= data.len();
        out.push(u8::from(last));
        out.extend_from_slice(&(part.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(part.len() as u16)).to_le_bytes());
        out.extend_from_slice(part);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
