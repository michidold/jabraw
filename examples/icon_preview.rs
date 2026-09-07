//! Writes the tray icon as raw ARGB data for visual inspection.
#[path = "../src/icon.rs"]
mod icon;

fn main() {
    let size: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(64);
    std::fs::write("/tmp/icon.argb", icon::render(size)).unwrap();
    println!("{size}x{size} geschrieben");
}
