//! Writes the tray icon as raw ARGB data for visual inspection.
#[path = "../src/icon.rs"]
// The example draws; the tray's cache and the PNG writer stay unused here.
#[allow(dead_code)]
mod icon;

fn main() {
    let size: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(64);
    // Into the working directory, not /tmp: a fixed name in a directory
    // everyone can write is asking to be pointed somewhere else.
    let path = std::env::args().nth(2).unwrap_or_else(|| "icon.argb".into());
    std::fs::write(&path, icon::render(size)).unwrap();
    println!("{size}x{size} nach {path} geschrieben");
}
