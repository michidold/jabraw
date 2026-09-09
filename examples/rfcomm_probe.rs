//! Prototype: RFCOMM connection to the headset through BlueZ's profile API.
//!
//! SDP queries return nothing while the device is connected, so BlueZ does the
//! channel finding: a profile for the Serial Port UUID is registered and a file
//! descriptor comes back on connect.
//!
//! Sends `ident/version` and nothing else: a read command verified many times
//! over USB. No writes, nothing from the firmware groups. The only unknown is
//! the framing over RFCOMM.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const SPP_UUID: &str = "00001101-0000-1000-8000-00805f9b34fb";
const PROFILE_PATH: &str = "/de/hausnetzle/jabraw/spp";

struct Profile {
    got: Arc<Notify>,
    fd: Arc<std::sync::Mutex<Option<std::os::fd::OwnedFd>>>,
}

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    async fn new_connection(
        &self,
        device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        props: HashMap<String, OwnedValue>,
    ) {
        println!("NewConnection von {device}");
        println!("  Eigenschaften: {:?}", props.keys().collect::<Vec<_>>());
        *self.fd.lock().unwrap() = Some(fd.into());
        self.got.notify_one();
    }

    async fn request_disconnection(&self, device: OwnedObjectPath) {
        println!("RequestDisconnection von {device}");
    }

    fn release(&self) {
        println!("Release");
    }
}

/// Try the framing variants, each with nothing but `ident/version`.
fn probe(fd: std::os::fd::OwnedFd) {
    use std::io::Write;
    let mut file = std::fs::File::from(fd);
    // Address 0x04 is the headset; 0x01 does not exist without a dongle and
    // answers requests with nack (type 254).
    // A table of packets; one byte per line would not be one.
    #[rustfmt::skip]
    let variants: [(&str, Vec<u8>); 6] = [
        ("ident/name", vec![0x04, 0x00, 0x41, 0x46, 0x02, 0x00]),
        ("ident/serial", vec![0x04, 0x00, 0x42, 0x46, 0x02, 0x01]),
        ("ident/version", vec![0x04, 0x00, 0x43, 0x46, 0x02, 0x03]),
        ("status/battery", vec![0x04, 0x00, 0x44, 0x46, 0x12, 0x02]),
        ("config/busylight", vec![0x04, 0x00, 0x45, 0x46, 0x13, 0x39]),
        ("config/onHeadDetect", vec![0x04, 0x00, 0x46, 0x46, 0x13, 0x92]),
    ];
    for (label, pkt) in variants {
        print!("  {label:<18} -> ");
        if let Err(e) = file.write_all(&pkt) {
            println!("Schreiben fehlgeschlagen: {e}");
            continue;
        }
        let mut buf = [0u8; 128];
        match read_with_timeout(&file, &mut buf, 1500) {
            Some(n) if n > 0 => {
                let hex: Vec<String> = buf[..n].iter().map(|b| format!("{b:02x}")).collect();
                let txt: String = buf[6..n]
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("{:<40} {txt}", hex.join(" "));
            }
            Some(_) => println!("Verbindung geschlossen"),
            None => println!("keine Antwort"),
        }
    }
}

fn read_with_timeout(file: &std::fs::File, buf: &mut [u8], ms: i32) -> Option<usize> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let mut p = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut p, 1, ms) } <= 0 {
        return None;
    }
    let mut f = file;
    f.read(buf).ok()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let device = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/org/bluez/hci0/dev_30_50_75_DA_81_D3".to_string());

    let conn = Connection::system().await?;
    let got = Arc::new(Notify::new());
    let fd = Arc::new(std::sync::Mutex::new(None));
    conn.object_server()
        .at(
            PROFILE_PATH,
            Profile {
                got: got.clone(),
                fd: fd.clone(),
            },
        )
        .await?;

    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("Name", Value::from("jabraw"));
    opts.insert("Role", Value::from("client"));
    opts.insert("Channel", Value::from(0u16));
    opts.insert("RequireAuthentication", Value::from(true));
    opts.insert("RequireAuthorization", Value::from(false));

    let pm = zbus::Proxy::new(
        &conn,
        "org.bluez",
        "/org/bluez",
        "org.bluez.ProfileManager1",
    )
    .await?;
    pm.call::<_, _, ()>(
        "RegisterProfile",
        &(OwnedObjectPath::try_from(PROFILE_PATH)?, SPP_UUID, opts),
    )
    .await?;
    println!("Profil registriert, verbinde …");

    let dev = zbus::Proxy::new(&conn, "org.bluez", device.as_str(), "org.bluez.Device1").await?;
    match dev.call::<_, _, ()>("ConnectProfile", &(SPP_UUID,)).await {
        Ok(()) => println!("ConnectProfile angenommen"),
        Err(e) => println!("ConnectProfile: {e}"),
    }

    tokio::select! {
        _ = got.notified() => {
            let held = fd.lock().unwrap().take().expect("fd gesetzt");
            println!("Deskriptor erhalten: {}", std::os::fd::AsRawFd::as_raw_fd(&held));
            probe(held);
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(12)) => {
            println!("kein NewConnection innerhalb von 12 s");
        }
    }
    let _ = pm
        .call::<_, _, ()>(
            "UnregisterProfile",
            &(OwnedObjectPath::try_from(PROFILE_PATH)?,),
        )
        .await;
    Ok(())
}
