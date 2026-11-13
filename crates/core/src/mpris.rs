//! Acceso a MPRIS: leer el estado del cliente y mandarle órdenes.
//!
//! zbus 6 es async; aquí se envuelve en un runtime current-thread por llamada.
//! Son operaciones puntuales (unas pocas por segundo como mucho), así que crear
//! el runtime cada vez es irrelevante y evita compartir uno entre hilos.

use anyhow::{anyhow, Context, Result};

pub const PLAYER_NAME: &str = "org.mpris.MediaPlayer2.spotify";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

fn block_on<F, T>(future: F) -> Result<T>
where
    F: std::future::Future<Output = zbus::Result<T>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("runtime de zbus")?;
    Ok(runtime.block_on(future)?)
}

/// Llama a un método del reproductor. `body` es el tuple de argumentos MPRIS.
pub fn call<B>(method: &str, body: &B) -> Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    block_on(async {
        let connection = zbus::Connection::session().await?;
        let proxy = zbus::Proxy::new(&connection, PLAYER_NAME, PLAYER_PATH, PLAYER_IFACE).await?;
        proxy.call_method(method, body).await?;
        Ok(())
    })
}

/// `SetPosition` necesita el trackid como object path: `/com/spotify/track/<id>`.
pub fn set_position(spotify_id: &str, position_ms: u64) -> Result<()> {
    let path = format!("/com/spotify/track/{spotify_id}");
    let object = zbus::ObjectPath::try_from(path.as_str())
        .map_err(|err| anyhow!("trackid inválido: {err}"))?;
    call("SetPosition", &(object, (position_ms as i64) * 1000))
}

/// Abre (y normalmente reproduce) un URI de Spotify.
pub fn open_uri(uri: &str) -> Result<()> {
    call("OpenUri", &(uri,))
}

/// PID del proceso cliente. Es el proceso principal, el que tiene los buffers.
pub fn client_pid() -> Result<u32> {
    let pid = block_on(async {
        let connection = zbus::Connection::session().await?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        let reply = proxy
            .call_method("GetConnectionUnixProcessID", &(PLAYER_NAME,))
            .await?;
        Ok(reply.body().deserialize::<u32>().unwrap_or(0))
    })?;
    if pid == 0 {
        return Err(anyhow!("no pude resolver el PID del cliente de Spotify"));
    }
    Ok(pid)
}
