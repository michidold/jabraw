//! GNP over Bluetooth, that is without a dongle.
//!
//! SDP queries return no channel while the device is connected, so BlueZ does
//! the finding: a profile for the Serial Port UUID is registered,
//! `ConnectProfile` is called, and the socket arrives through `NewConnection`.
//!
//! The framing is the same packet header as over hidraw with no report id in
//! front, checked against the hardware. The headset is address `0x04` here too;
//! the dongle address `0x01` answers with `nack`, as it should.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::gnp;

const SPP_UUID: &str = "00001101-0000-1000-8000-00805f9b34fb";
const PROFILE_PATH: &str = "/de/hausnetzle/jabraw/spp";

/// The socket BlueZ hands over, together with the device it belongs to.
type Slot = Arc<Mutex<Option<(String, std::os::fd::OwnedFd)>>>;

struct Profile {
    ready: Arc<Notify>,
    slot: Slot,
}

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    async fn new_connection(
        &self,
        device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        _props: HashMap<String, OwnedValue>,
    ) {
        *self.slot.lock().unwrap() = Some((device.to_string(), fd.into()));
        self.ready.notify_one();
    }

    async fn request_disconnection(&self, _device: OwnedObjectPath) {}

    fn release(&self) {}
}

/// What came back from a read.
///
/// Silence and a broken socket have to be told apart: most headsets answer
/// only a part of the subcommands, and taking the missing ones for a dead link
/// would drop a session that still reports its battery.
pub enum Reply {
    Data(Vec<u8>),
    /// Nothing arrived in time. The device may not know the subcommand.
    Silent,
    /// The socket is gone — the write failed, or the stream ended.
    Dead,
}

impl Reply {
    pub fn data(self) -> Option<Vec<u8>> {
        match self {
            Reply::Data(d) => Some(d),
            _ => None,
        }
    }
}

/// An open RFCOMM session to a headset.
pub struct Session {
    file: std::fs::File,
    seq: u8,
    /// RFCOMM is a stream: replies coalesce or split across reads, so packets
    /// are cut on the length field rather than assuming one packet per read as
    /// hidraw allows.
    buf: Vec<u8>,
    /// Set once the socket has failed; the session is over then.
    dead: bool,
}

impl Session {
    /// One read request and its reply.
    pub fn read(&mut self, dst: u8, cmd: u8, sub: u8) -> Reply {
        let seq = self.next_seq();
        if self
            .file
            .write_all(&gnp::read_body(dst, seq, cmd, sub, &[]))
            .is_err()
        {
            self.dead = true;
            return Reply::Dead;
        }
        let deadline = Instant::now() + Duration::from_millis(900);
        while let Some(pkt) = self.next_packet(deadline) {
            // The device also sends unprompted; only our own reply counts, and
            // a packet that makes no sense is not a reason to stop reading.
            if let Some(r) = gnp::parse_body(&pkt) {
                if r.seq == seq && r.cmd == cmd && r.sub == sub {
                    return Reply::Data(r.data.to_vec());
                }
            }
        }
        if self.dead {
            Reply::Dead
        } else {
            Reply::Silent
        }
    }

    /// Whether the socket has failed. A sweep reports it this way, since it
    /// collects whatever answers arrive rather than waiting for one.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Sends every request first and collects the replies afterwards.
    ///
    /// Asking one after another would be unusably slow at 56 settings and
    /// close to a second of timeout each.
    pub fn sweep(
        &mut self,
        dst: u8,
        cmd: u8,
        subs: &[(u8, &'static str, &'static [u8])],
    ) -> Vec<(&'static str, Vec<u8>)> {
        let mut pending = std::collections::HashMap::new();
        for (sub, name, data) in subs {
            let seq = self.next_seq();
            if self
                .file
                .write_all(&gnp::read_body(dst, seq, cmd, *sub, data))
                .is_err()
            {
                self.dead = true;
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

    /// A complete packet from the stream, cut on the length field.
    fn next_packet(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        loop {
            if self.buf.len() >= gnp::HEADER_LEN {
                let len = (self.buf[3] & 0x3F) as usize;
                if len >= gnp::HEADER_LEN && self.buf.len() >= len {
                    return Some(self.buf.drain(..len).collect());
                }
                if len < gnp::HEADER_LEN {
                    // Unusable length field: the stream is out of step.
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
                Ok(0) | Err(_) => {
                    self.dead = true;
                    return None;
                }
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

/// Notify and slot of the registered profile object.
///
/// Registered once for the process: zbus keeps the interface first put at a
/// path and answers a second `at()` with `false`, so a per-attempt profile
/// would leave BlueZ handing the socket to the object nobody waits on — every
/// reconnect after the first would then sit out its timeout.
static PROFILE: OnceLock<(Arc<Notify>, Slot)> = OnceLock::new();

async fn profile(conn: &zbus::Connection) -> Option<&'static (Arc<Notify>, Slot)> {
    if let Some(p) = PROFILE.get() {
        return Some(p);
    }
    let ready = Arc::new(Notify::new());
    let slot: Slot = Arc::new(Mutex::new(None));
    let profile = Profile {
        ready: ready.clone(),
        slot: slot.clone(),
    };
    match conn.object_server().at(PROFILE_PATH, profile).await {
        Ok(true) => Some(PROFILE.get_or_init(|| (ready, slot))),
        Ok(false) => {
            eprintln!("{PROFILE_PATH} ist belegt — kein RFCOMM");
            None
        }
        Err(e) => {
            eprintln!("Profil nicht registrierbar: {e}");
            None
        }
    }
}

/// How long the whole handshake may take. zbus puts no deadline on a method
/// call, so an unanswered `ConnectProfile` would wait for as long as BlueZ
/// takes to give up — the budget below covers every step, not just the wait
/// for the socket.
const CONNECT_BUDGET: Duration = Duration::from_secs(12);

/// Opens a session to the given BlueZ device path.
pub async fn connect(conn: &zbus::Connection, device: &str) -> Option<Session> {
    tokio::time::timeout(CONNECT_BUDGET, handshake(conn, device))
        .await
        .ok()
        .flatten()
}

async fn handshake(conn: &zbus::Connection, device: &str) -> Option<Session> {
    let (ready, slot) = profile(conn).await?;
    // A socket left over from an earlier attempt would be handed out as this
    // one, along with any notification nobody collected.
    slot.lock().unwrap().take();

    let pm = zbus::Proxy::new(conn, "org.bluez", "/org/bluez", "org.bluez.ProfileManager1")
        .await
        .ok()?;
    let path = OwnedObjectPath::try_from(PROFILE_PATH).ok()?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("Name", Value::from("jabraw"));
    opts.insert("Role", Value::from("client"));
    opts.insert("Channel", Value::from(0u16));
    // The headset is paired, so an authenticated link costs nothing and keeps
    // the connection off BT_SECURITY_LOW.
    opts.insert("RequireAuthentication", Value::from(true));
    opts.insert("RequireAuthorization", Value::from(false));
    // An already registered profile answers AlreadyExists, equally fine.
    let _ = pm
        .call::<_, _, ()>("RegisterProfile", &(path, SPP_UUID, opts))
        .await;

    let dev = zbus::Proxy::new(conn, "org.bluez", device, "org.bluez.Device1")
        .await
        .ok()?;
    // A profile BlueZ still counts as connected from an earlier session makes
    // ConnectProfile answer Ok and hand over nothing, and the wait below then
    // sits out its budget for a socket that never arrives. Tearing it down
    // first is what makes BlueZ produce a new one; with nothing connected the
    // call fails and that is fine.
    let _ = dev
        .call::<_, _, ()>("DisconnectProfile", &(SPP_UUID,))
        .await;
    let _ = dev.call::<_, _, ()>("ConnectProfile", &(SPP_UUID,)).await;

    let deadline = Instant::now() + Duration::from_secs(8);
    let fd = loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        tokio::time::timeout(left, ready.notified()).await.ok()?;
        // The profile stays registered, so a socket for another device can
        // turn up here; only the one asked for is this session.
        match slot.lock().unwrap().take() {
            Some((path, fd)) if path == device => break fd,
            _ => continue,
        }
    };
    Some(Session {
        file: std::fs::File::from(fd),
        seq: 0x40,
        buf: Vec::new(),
        dead: false,
    })
}
