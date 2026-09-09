//! Detecting a directly paired Jabra headset through BlueZ.
//!
//! Without a dongle there is no hidraw device. BlueZ supplies name and charge
//! level on the system bus; everything beyond that comes from GNP over RFCOMM,
//! see [`crate::rfcomm`]. The media keys travel over AVRCP in this mode and the
//! desktop forwards them to MPRIS already.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// Bluetooth SIG identifier of GN Netcom. Sturdier than comparing names, which
/// users can change.
const JABRA_VENDOR_PREFIX: &str = "bluetooth:v0067";

pub struct BtDevice {
    pub name: String,
    pub battery: Option<u8>,
    /// BlueZ object path, for opening the RFCOMM connection.
    pub path: String,
}

type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn field<T: TryFrom<OwnedValue>>(
    props: &HashMap<String, OwnedValue>,
    key: &str,
) -> Option<T> {
    T::try_from(props.get(key)?.try_clone().ok()?).ok()
}

/// The first connected Jabra device, if one is paired.
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
        // The RFCOMM session asks for an authenticated link, which an unpaired
        // device cannot give.
        if field::<bool>(dev, "Paired") != Some(true) {
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
