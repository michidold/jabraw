//! Auswahl und Steuerung des aktiven MPRIS-Players.

use zbus::fdo::DBusProxy;

#[derive(Debug, Clone, Default)]
pub struct PlayerInfo {
    pub bus: String,
    pub identity: String,
    pub status: String,
    pub track: Option<String>,
}

async fn player_proxy<'a>(
    conn: &zbus::Connection,
    dest: &str,
) -> Option<zbus::Proxy<'a>> {
    zbus::proxy::Builder::new(conn)
        .destination(dest.to_string())
        .ok()?
        .path("/org/mpris/MediaPlayer2")
        .ok()?
        .interface("org.mpris.MediaPlayer2.Player")
        .ok()?
        .build()
        .await
        .ok()
}

async fn app_proxy<'a>(conn: &zbus::Connection, dest: &str) -> Option<zbus::Proxy<'a>> {
    zbus::proxy::Builder::new(conn)
        .destination(dest.to_string())
        .ok()?
        .path("/org/mpris/MediaPlayer2")
        .ok()?
        .interface("org.mpris.MediaPlayer2")
        .ok()?
        .build()
        .await
        .ok()
}

pub async fn list_players(conn: &zbus::Connection) -> Vec<String> {
    let Ok(dbus) = DBusProxy::new(conn).await else {
        return vec![];
    };
    match dbus.list_names().await {
        Ok(names) => names
            .into_iter()
            .filter(|n| n.as_str().starts_with("org.mpris.MediaPlayer2."))
            .map(|n| n.to_string())
            .collect(),
        Err(e) => {
            eprintln!("ListNames fehlgeschlagen: {e}");
            vec![]
        }
    }
}

async fn info(conn: &zbus::Connection, dest: &str) -> PlayerInfo {
    let mut out = PlayerInfo {
        bus: dest.to_string(),
        identity: dest.trim_start_matches("org.mpris.MediaPlayer2.").to_string(),
        ..Default::default()
    };
    // get_property statt eines rohen Properties.Get-Aufrufs: nur so wird die
    // Variante korrekt ausgepackt.
    if let Some(p) = player_proxy(conn, dest).await {
        if let Ok(s) = p.get_property::<String>("PlaybackStatus").await {
            out.status = s;
        }
        if let Ok(md) = p
            .get_property::<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>("Metadata")
            .await
        {
            let field = |k: &str| md.get(k).and_then(|v| String::try_from(v.clone()).ok());
            let title = field("xesam:title");
            let artist = md
                .get("xesam:artist")
                .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
                .and_then(|a| a.first().cloned());
            out.track = match (title, artist) {
                (Some(t), Some(a)) => Some(format!("{a} — {t}")),
                (Some(t), None) => Some(t),
                _ => None,
            };
        }
    }
    if let Some(p) = app_proxy(conn, dest).await {
        if let Ok(id) = p.get_property::<String>("Identity").await {
            out.identity = id;
        }
    }
    out
}

/// Playing > Paused > Rest; bei Gleichstand gewinnt der erste Treffer.
pub async fn active_player(conn: &zbus::Connection) -> Option<PlayerInfo> {
    let players = list_players(conn).await;
    if players.is_empty() {
        return None;
    }
    let infos = futures::future::join_all(players.iter().map(|p| info(conn, p))).await;
    let mut best: Option<(u8, PlayerInfo)> = None;
    for i in infos {
        let rank = match i.status.as_str() {
            "Playing" => 2,
            "Paused" => 1,
            _ => 0,
        };
        if best.as_ref().is_none_or(|(seen, _)| rank > *seen) {
            best = Some((rank, i));
        }
    }
    best.map(|(_, i)| i)
}

pub async fn call(conn: &zbus::Connection, dest: &str, method: &str) {
    let Some(proxy) = player_proxy(conn, dest).await else {
        return;
    };
    if let Err(e) = proxy.call::<_, _, ()>(method, &()).await {
        eprintln!("MPRIS {method} an {dest} fehlgeschlagen: {e}");
    }
}

/// Schickt `method` an den aktiven Player.
pub async fn dispatch(conn: &zbus::Connection, method: &str) {
    match active_player(conn).await {
        Some(p) => call(conn, &p.bus, method).await,
        None => eprintln!("kein MPRIS-Player aktiv"),
    }
}
