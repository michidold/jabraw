//! Erkennung eines direkt gekoppelten Jabra-Headsets über BlueZ.
//!
//! Ohne Dongle gibt es kein hidraw-Gerät und damit keinen GNP-Kanal. Alles,
//! was der Daemon dann noch anzeigen kann, liefert BlueZ auf dem System-Bus:
//! Name und Akkustand. Die Medientasten laufen in diesem Betrieb über AVRCP
//! und werden bereits von der Desktop-Umgebung an MPRIS weitergereicht.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// Bluetooth-SIG-Kennung von GN Netcom. Verlässlicher als ein Namensvergleich,
/// weil Nutzer ihre Geräte umbenennen können.
const JABRA_VENDOR_PREFIX: &str = "bluetooth:v0067";

pub struct BtDevice {
    pub name: String,
    pub battery: Option<u8>,
    /// BlueZ-Objektpfad, für den RFCOMM-Verbindungsaufbau.
    pub path: String,
}

type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn field<T: TryFrom<OwnedValue>>(
    props: &HashMap<String, OwnedValue>,
    key: &str,
) -> Option<T> {
    T::try_from(props.get(key)?.try_clone().ok()?).ok()
}

/// Das erste verbundene Jabra-Gerät, sofern eines gekoppelt ist.
pub async fn connected_jabra(conn: &zbus::Connection) -> Option<BtDevice> {
    let proxy = zbus::proxy::Builder::<zbus::Proxy>::new(conn)
        .destination("org.bluez")
        .ok()?
        .path("/")
        .ok()?
        .interface("org.freedesktop.DBus.ObjectManager")
        .ok()?
        .build()
        .await
        .ok()?;
    let objects: Managed = proxy.call("GetManagedObjects", &()).await.ok()?;

    for (path, ifaces) in objects {
        let Some(dev) = ifaces.get("org.bluez.Device1") else {
            continue;
        };
        if field::<bool>(dev, "Connected") != Some(true) {
            continue;
        }
        if !field::<String>(dev, "Modalias")
            .is_some_and(|m| m.starts_with(JABRA_VENDOR_PREFIX))
        {
            continue;
        }
        return Some(BtDevice {
            name: field(dev, "Alias").unwrap_or_else(|| "Jabra".to_string()),
            battery: ifaces
                .get("org.bluez.Battery1")
                .and_then(|b| field::<u8>(b, "Percentage")),
            path: path.to_string(),
        });
    }
    None
}
