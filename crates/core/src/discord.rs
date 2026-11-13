//! Discord Rich Presence por IPC (socket Unix).
//!
//! Discord expone un socket por cliente: `$XDG_RUNTIME_DIR/discord-ipc-N`
//! (también `/tmp`, TMPDIR, y las variantes de Flatpak/Snap). El protocolo son
//! frames de 8 bytes de cabecera (`op` y longitud, ambos u32 little-endian) y
//! un JSON detrás:
//!
//! - `op 0` handshake: `{"v":1,"client_id":"<app id>"}`
//! - `op 1` frame: `{"cmd":"SET_ACTIVITY","args":{"pid":N,"activity":{...}},"nonce":".."}`
//! - `op 2` frame de Discord (respuesta/evento), `op 3` close, `op 4` ping
//!
//! Hace falta una **aplicación de Discord** (gratis) y su Application ID: sin
//! eso el handshake se rechaza con `4000 Invalid Client ID`. El nombre y los
//! assets que se ven en el perfil salen de esa aplicación, así que el logo de
//! spote se pondrá allí cuando exista (campo `small_image`).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const CLOSE: u32 = 3;
/// Discord limita a ~5 actualizaciones cada 20 s; nos quedamos muy por debajo.
const MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Lo que se pinta en el perfil. Todo opcional salvo `details`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Activity {
    pub details: String,
    pub state: String,
    /// Epoch ms del inicio: Discord muestra el tiempo transcurrido solo.
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// Máximo dos botones (label, url).
    pub buttons: Vec<(String, String)>,
}

impl Activity {
    fn to_json(&self) -> Value {
        let mut activity = json!({
            "details": truncate(&self.details, 128),
            "state": truncate(&self.state, 128),
            "type": 2, // "Listening to"
            "instance": false,
        });
        let mut timestamps = serde_json::Map::new();
        if let Some(start) = self.start_ms {
            timestamps.insert("start".into(), json!(start));
        }
        if let Some(end) = self.end_ms {
            timestamps.insert("end".into(), json!(end));
        }
        if !timestamps.is_empty() {
            activity["timestamps"] = Value::Object(timestamps);
        }
        let mut assets = serde_json::Map::new();
        if let Some(url) = &self.large_image {
            // Discord acepta URLs externas aquí desde la v1 de assets.
            assets.insert("large_image".into(), json!(url));
        }
        if let Some(text) = &self.large_text {
            assets.insert("large_text".into(), json!(truncate(text, 128)));
        }
        if let Some(key) = &self.small_image {
            assets.insert("small_image".into(), json!(key));
        }
        if let Some(text) = &self.small_text {
            assets.insert("small_text".into(), json!(truncate(text, 128)));
        }
        if !assets.is_empty() {
            activity["assets"] = Value::Object(assets);
        }
        if !self.buttons.is_empty() {
            activity["buttons"] = Value::Array(
                self.buttons
                    .iter()
                    .take(2)
                    .map(|(label, url)| json!({ "label": truncate(label, 32), "url": url }))
                    .collect(),
            );
        }
        activity
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// Candidatos de socket, en orden: nativo, tmp, Flatpak y Snap.
fn socket_paths() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    for dir in [
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("TMPDIR").map(PathBuf::from),
        Some(PathBuf::from("/tmp")),
    ]
    .into_iter()
    .flatten()
    {
        bases.push(dir.clone());
        for app in [
            "app/com.discordapp.Discord",
            "app/com.discordapp.DiscordCanary",
            "snap.discord",
        ] {
            bases.push(dir.join(app));
        }
    }
    let mut paths = Vec::new();
    for base in bases {
        for index in 0..10 {
            paths.push(base.join(format!("discord-ipc-{index}")));
        }
    }
    paths
}

/// Cliente IPC conectado y autenticado.
pub struct DiscordIpc {
    stream: UnixStream,
    client_id: String,
    nonce: u64,
    connected_at: Instant,
}

impl DiscordIpc {
    /// Busca un socket vivo y hace el handshake.
    pub fn connect(client_id: &str) -> Result<Self> {
        if client_id.trim().is_empty() {
            return Err(anyhow!(
                "falta el Application ID de Discord (ver [discord] en config.toml)"
            ));
        }
        let mut last_error = None;
        for path in socket_paths() {
            if !path.exists() {
                continue;
            }
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(Duration::from_millis(300))).ok();
                    stream.set_write_timeout(Some(Duration::from_millis(300))).ok();
                    let mut ipc = Self {
                        stream,
                        client_id: client_id.trim().to_string(),
                        nonce: 1,
                        connected_at: Instant::now(),
                    };
                    ipc.send(
                        OP_HANDSHAKE,
                        &json!({ "v": 1, "client_id": ipc.client_id.clone() }),
                    )
                    .with_context(|| format!("handshake con {}", path.display()))?;
                    // Si Discord rechaza el ID contesta con un frame de error.
                    if let Some(error) = ipc.read_error() {
                        return Err(anyhow!("Discord rechazó la conexión: {error}"));
                    }
                    return Ok(ipc);
                }
                Err(err) => last_error = Some(format!("{}: {err}", path.display())),
            }
        }
        Err(anyhow!(
            "no encuentro Discord abierto{}",
            last_error
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        ))
    }

    fn send(&mut self, op: u32, payload: &Value) -> Result<()> {
        let body = serde_json::to_vec(payload)?;
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(&body);
        self.stream
            .write_all(&frame)
            .context("no pude escribir a Discord")?;
        Ok(())
    }

    /// Lee lo que haya pendiente. Devuelve el mensaje si Discord manda un error.
    fn read_error(&mut self) -> Option<String> {
        let mut header = [0u8; 8];
        if self.stream.read_exact(&mut header).is_err() {
            return None;
        }
        let op = u32::from_le_bytes(header[0..4].try_into().ok()?);
        let len = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        if len > 64 * 1024 {
            return None;
        }
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body).ok()?;
        if op == CLOSE {
            return Some("socket cerrado".into());
        }
        let value: Value = serde_json::from_slice(&body).ok()?;
        value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn set_activity(&mut self, activity: &Activity) -> Result<()> {
        self.nonce += 1;
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity.to_json() },
            "nonce": format!("{:016x}", self.nonce),
        });
        self.send(OP_FRAME, &payload)
    }

    pub fn clear(&mut self) -> Result<()> {
        self.nonce += 1;
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": Value::Null },
            "nonce": format!("{:016x}", self.nonce),
        });
        self.send(OP_FRAME, &payload)
    }

    /// Cuánto lleva viva la conexión (para reciclar si Discord se reinicia).
    pub fn age(&self) -> Duration {
        self.connected_at.elapsed()
    }

    pub fn drena(&mut self) {
        // Los eventos de Discord (READY, etc.) no nos interesan, pero hay que
        // vaciar el buffer para que el socket no se llene.
        let mut buffer = [0u8; 1024];
        self.stream.set_nonblocking(true).ok();
        while matches!(self.stream.read(&mut buffer), Ok(n) if n > 0) {}
        self.stream.set_nonblocking(false).ok();
        self.stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .ok();
    }
}

/// Trabajador con hilo propio: la UI nunca se bloquea por Discord.
pub struct DiscordWorker {
    tx: Sender<Option<Activity>>,
    /// Estado para no repetir la misma actividad en cada tick.
    last: Option<Activity>,
    last_sent: Instant,
    client_id: String,
    disabled: bool,
}

impl DiscordWorker {
    pub fn start(client_id: impl Into<String>) -> Self {
        let client_id = client_id.into();
        let (tx, rx) = std::sync::mpsc::channel::<Option<Activity>>();
        let id = client_id.clone();
        let disabled = id.trim().is_empty();
        if !disabled {
            std::thread::Builder::new()
                .name("spote-discord".into())
                .spawn(move || run(id, rx))
                .ok();
        }
        Self {
            tx,
            last: None,
            last_sent: Instant::now() - MIN_INTERVAL,
            client_id,
            disabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.disabled
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Programa una actualización. Ignora repeticiones y respeta el rate limit.
    pub fn update(&mut self, activity: Option<Activity>) {
        if self.disabled {
            return;
        }
        if self.last == activity && self.last_sent.elapsed() < Duration::from_secs(20) {
            return;
        }
        if self.last_sent.elapsed() < MIN_INTERVAL {
            return;
        }
        self.last = activity.clone();
        self.last_sent = Instant::now();
        let _ = self.tx.send(activity);
    }
}

fn run(client_id: String, rx: Receiver<Option<Activity>>) {
    let mut ipc: Option<DiscordIpc> = None;
    loop {
        let Ok(activity) = rx.recv() else { return };
        // Conexión perezosa y reciclaje si Discord se ha reiniciado.
        if ipc.as_ref().is_none_or(|current| current.age() > Duration::from_secs(600)) {
            ipc = DiscordIpc::connect(&client_id).ok();
        }
        let Some(connection) = ipc.as_mut() else { continue };
        connection.drena();
        let result = match &activity {
            Some(activity) => connection.set_activity(activity),
            None => connection.clear(),
        };
        if result.is_err() {
            // Se cayó: se descarta y se reintenta en el siguiente evento.
            ipc = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_json_de_actividad() {
        let activity = Activity {
            details: "The Suburbs".into(),
            state: "Arcade Fire".into(),
            start_ms: Some(1_700_000_000_000),
            end_ms: None,
            large_image: Some("https://i.scdn.co/image/abc".into()),
            large_text: Some("The Suburbs".into()),
            small_image: None,
            small_text: Some("spote".into()),
            buttons: vec![("Abrir en Spotify".into(), "https://open.spotify.com".into())],
        };
        let json = activity.to_json();
        assert_eq!(json["details"], "The Suburbs");
        assert_eq!(json["state"], "Arcade Fire");
        assert_eq!(json["type"], 2);
        assert_eq!(json["timestamps"]["start"], 1_700_000_000_000u64);
        assert_eq!(json["assets"]["large_image"], "https://i.scdn.co/image/abc");
        assert_eq!(json["assets"]["small_text"], "spote");
        assert_eq!(json["buttons"][0]["label"], "Abrir en Spotify");
    }

    #[test]
    fn sin_botones_ni_imagen_no_se_emiten_campos_vacios() {
        let activity = Activity {
            details: "X".into(),
            state: "Y".into(),
            ..Default::default()
        };
        let json = activity.to_json();
        assert!(json.get("assets").is_none());
        assert!(json.get("buttons").is_none());
        assert!(json.get("timestamps").is_none());
    }

    #[test]
    fn trunca_a_lo_que_acepta_discord() {
        let long = "a".repeat(400);
        assert_eq!(truncate(&long, 128).chars().count(), 128);
        assert!(truncate(&long, 128).ends_with('…'));
    }

    #[test]
    fn rutas_de_socket_incluyen_las_conocidas() {
        let paths = socket_paths();
        assert!(paths.iter().any(|p| p.ends_with("discord-ipc-0")));
        assert!(paths
            .iter()
            .any(|p| p.to_string_lossy().contains("com.discordapp.Discord")));
    }

    #[test]
    fn sin_client_id_no_conecta() {
        assert!(DiscordIpc::connect("  ").is_err());
    }
}
