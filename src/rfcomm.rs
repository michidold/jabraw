//! GNP über Bluetooth, also ohne Dongle.
//!
//! SDP-Abfragen liefern bei verbundenem Gerät keinen Kanal, deshalb übernimmt
//! BlueZ die Suche: Wir registrieren ein Profil für die Serial-Port-UUID, rufen
//! `ConnectProfile` und bekommen den Socket über `NewConnection` zurückgereicht.
//!
//! Die Rahmung ist derselbe Paketkopf wie über hidraw, nur ohne die Report-ID
//! davor — am Gerät geprüft. Das Headset ist auch hier Adresse `0x04`; die
//! Dongle-Adresse `0x01` beantwortet Anfragen erwartungsgemäß mit `nack`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::gnp;

const SPP_UUID: &str = "00001101-0000-1000-8000-00805f9b34fb";
const PROFILE_PATH: &str = "/de/hausnetzle/jabraw/spp";

type Slot = Arc<Mutex<Option<std::os::fd::OwnedFd>>>;

struct Profile {
    ready: Arc<Notify>,
    slot: Slot,
}

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    async fn new_connection(
        &self,
        _device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        _props: HashMap<String, OwnedValue>,
    ) {
        *self.slot.lock().unwrap() = Some(fd.into());
        self.ready.notify_one();
    }

    async fn request_disconnection(&self, _device: OwnedObjectPath) {}

    fn release(&self) {}
}

/// Offene RFCOMM-Sitzung zu einem Headset.
pub struct Session {
    file: std::fs::File,
    seq: u8,
    /// RFCOMM ist ein Stream: Antworten können zusammenfallen oder geteilt
    /// ankommen, deshalb wird über das Längenfeld entrahmt statt je Lesevorgang
    /// ein Paket anzunehmen wie bei hidraw.
    buf: Vec<u8>,
}

impl Session {
    /// Eine Leseanfrage und ihre Antwort. `None`, wenn das Gerät schweigt.
    pub fn read(&mut self, dst: u8, cmd: u8, sub: u8) -> Option<Vec<u8>> {
        let seq = self.next_seq();
        self.file.write_all(&gnp::read_body(dst, seq, cmd, sub)).ok()?;
        let deadline = Instant::now() + Duration::from_millis(900);
        while let Some(pkt) = self.next_packet(deadline) {
            let r = gnp::parse_body(&pkt)?;
            // Das Gerät sendet auch unaufgefordert; nur die eigene Antwort zählt.
            if r.seq == seq && r.cmd == cmd && r.sub == sub {
                return Some(r.data.to_vec());
            }
        }
        None
    }

    /// Alle Anfragen auf einmal absetzen und die Antworten einsammeln.
    ///
    /// Nacheinander abzufragen wäre bei 56 Einstellungen und je einer knappen
    /// Sekunde Wartezeit unbrauchbar langsam.
    pub fn sweep(
        &mut self,
        dst: u8,
        cmd: u8,
        subs: &[(u8, &'static str)],
    ) -> Vec<(&'static str, Vec<u8>)> {
        let mut pending = std::collections::HashMap::new();
        for (sub, name) in subs {
            let seq = self.next_seq();
            if self
                .file
                .write_all(&gnp::read_body(dst, seq, cmd, *sub))
                .is_err()
            {
                break;
            }
            pending.insert(seq, *name);
        }

        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(1500);
        while !pending.is_empty() {
            let Some(pkt) = self.next_packet(deadline) else {
                break;
            };
            let Some(r) = gnp::parse_body(&pkt) else {
                continue;
            };
            if r.cmd != cmd {
                continue;
            }
            if let Some(name) = pending.remove(&r.seq) {
                out.push((name, r.data.to_vec()));
            }
        }
        out.sort_by_key(|(n, _)| *n);
        out
    }

    fn next_seq(&mut self) -> u8 {
        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.seq = 1;
        }
        self.seq
    }

    /// Ein vollständiges Paket aus dem Strom, entrahmt über das Längenfeld.
    fn next_packet(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        loop {
            if self.buf.len() >= gnp::HEADER_LEN {
                let len = (self.buf[3] & 0x3F) as usize;
                if len >= gnp::HEADER_LEN && self.buf.len() >= len {
                    return Some(self.buf.drain(..len).collect());
                }
                if len < gnp::HEADER_LEN {
                    // Unbrauchbares Längenfeld: Strom ist aus dem Tritt.
                    self.buf.clear();
                    return None;
                }
            }
            let now = Instant::now();
            if now >= deadline || !readable(&self.file, deadline - now) {
                return None;
            }
            let mut chunk = [0u8; 256];
            match self.file.read(&mut chunk) {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
            }
        }
    }
}

fn readable(file: &std::fs::File, timeout: Duration) -> bool {
    let mut p = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, timeout.as_millis() as i32) > 0 }
}

/// Baut eine Sitzung zum angegebenen BlueZ-Gerätepfad auf.
pub async fn connect(conn: &zbus::Connection, device: &str) -> Option<Session> {
    let ready = Arc::new(Notify::new());
    let slot: Slot = Arc::new(Mutex::new(None));

    // Beim zweiten Aufruf liegt das Objekt schon; das ist kein Fehler.
    let _ = conn
        .object_server()
        .at(
            PROFILE_PATH,
            Profile {
                ready: ready.clone(),
                slot: slot.clone(),
            },
        )
        .await;

    let pm = zbus::Proxy::new(conn, "org.bluez", "/org/bluez", "org.bluez.ProfileManager1")
        .await
        .ok()?;
    let path = OwnedObjectPath::try_from(PROFILE_PATH).ok()?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("Name", Value::from("jabraw"));
    opts.insert("Role", Value::from("client"));
    opts.insert("Channel", Value::from(0u16));
    opts.insert("RequireAuthentication", Value::from(false));
    opts.insert("RequireAuthorization", Value::from(false));
    // Ein bereits registriertes Profil meldet AlreadyExists — ebenfalls in Ordnung.
    let _ = pm
        .call::<_, _, ()>("RegisterProfile", &(path, SPP_UUID, opts))
        .await;

    let dev = zbus::Proxy::new(conn, "org.bluez", device, "org.bluez.Device1")
        .await
        .ok()?;
    let _ = dev.call::<_, _, ()>("ConnectProfile", &(SPP_UUID,)).await;

    tokio::time::timeout(Duration::from_secs(8), ready.notified())
        .await
        .ok()?;
    let fd = slot.lock().unwrap().take()?;
    Some(Session {
        file: std::fs::File::from(fd),
        seq: 0x40,
        buf: Vec::new(),
    })
}
