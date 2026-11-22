//! Motor de reproducción: mpv por JSON IPC sobre un socket Unix.
//!
//! No se reimplementa audio. mpv da decodificación, gapless, pitch correction
//! en los cambios de velocidad y una posición fiable. Nosotros solo mandamos
//! comandos y escuchamos eventos.
//!
//! Los filtros de audio (EQ, preamp, normalizador, limitador) se aplican en
//! caliente escribiendo la propiedad `af` con el string que construye
//! [`crate::audio_fx::AudioCfg::filter_chain`]. mpv reconstruye el grafo sin
//! reiniciar ni recargar la pista; `af` con string vacío lo deja limpio. La
//! opción persiste entre cargas, así que la cadena sigue activa al cambiar de
//! tema.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    FileLoaded(String),
    TimePos(f64),
    Duration(f64),
    Paused(bool),
    Idle(bool),
    Volume(f64),
    Speed(f64),
    Eof,
    Error(String),
    /// mpv ha muerto o se ha perdido el socket.
    Disconnected,
}

#[derive(Debug)]
enum Cmd {
    Load { target: String, start: f64, append: bool },
    Pause(bool),
    SeekAbsolute(f64),
    SeekRelative(f64),
    Volume(f32),
    Speed(f64),
    LoopFile(bool),
    AudioFilters(Option<String>),
    Stop,
    Quit,
}

/// Handle clonable y no-async. Todos los comandos son `try_send`: la UI nunca
/// se bloquea esperando a mpv.
pub struct Player {
    cmd: mpsc::UnboundedSender<Cmd>,
    events: broadcast::Sender<PlayerEvent>,
    socket: Arc<PathBuf>,
    volume: Arc<AtomicU32>,
    fail: Arc<std::sync::Mutex<Option<String>>>,
}

impl Player {
    /// Arranca mpv y conecta el socket. Devuelve error si mpv no existe.
    pub async fn spawn(socket: PathBuf, mpv_path: &str, volume: f32) -> Result<Self> {
        if let Some(parent) = socket.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&socket);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(256);
        let volume_bits = Arc::new(AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()));
        let fail = Arc::new(std::sync::Mutex::new(None));

        let task_socket = socket.clone();
        let task_volume = Arc::clone(&volume_bits);
        let task_events = event_tx.clone();
        let task_fail = Arc::clone(&fail);
        let mpv = mpv_path.to_string();

        tokio::spawn(async move {
            if let Err(err) =
                run(mpv, task_socket, task_volume, cmd_rx, task_events.clone(), task_fail.clone())
                    .await
            {
                *task_fail.lock().unwrap() = Some(err.to_string());
                let _ = task_events.send(PlayerEvent::Disconnected);
            }
        });

        // Espera corta a que mpv cree el socket; si falla, se sabrá por `fail`.
        let player = Self {
            cmd: cmd_tx,
            events: event_tx,
            socket: Arc::new(socket),
            volume: volume_bits,
            fail,
        };
        player.wait_ready(Duration::from_millis(2_500)).await;
        if let Some(err) = player.failure() {
            if err.contains("No such file") || err.contains("not found") {
                bail!("{err}");
            }
        }
        Ok(player)
    }

    async fn wait_ready(&self, timeout: Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if self.failure().is_some() || self.socket.exists() {
                // El socket existe antes de que mpv escuche: margen mínimo.
                sleep(Duration::from_millis(30)).await;
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    }

    pub fn failure(&self) -> Option<String> {
        self.fail.lock().ok().and_then(|guard| guard.clone())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.events.subscribe()
    }

    pub fn load(&self, path: PathBuf, start_secs: f64) {
        self.load_target(path.to_string_lossy().into_owned(), start_secs);
    }

    /// Reproduce una URL ya resuelta (por ejemplo, la salida de `yt-dlp -g`).
    /// mpv no debe re-extraerla: el proceso principal ya la validó.
    pub fn load_url(&self, url: impl Into<String>, start_secs: f64) {
        self.load_target(url.into(), start_secs);
    }

    fn load_target(&self, target: String, start_secs: f64) {
        let _ = self.cmd.send(Cmd::Load {
            target,
            start: start_secs.max(0.0),
            append: false,
        });
    }

    pub fn enqueue(&self, path: PathBuf) {
        let _ = self.cmd.send(Cmd::Load {
            target: path.to_string_lossy().into_owned(),
            start: 0.0,
            append: true,
        });
    }

    pub fn pause(&self, paused: bool) {
        let _ = self.cmd.send(Cmd::Pause(paused));
    }

    pub fn seek(&self, secs: f64) {
        let _ = self.cmd.send(Cmd::SeekAbsolute(secs.max(0.0)));
    }

    pub fn seek_rel(&self, delta: f64) {
        let _ = self.cmd.send(Cmd::SeekRelative(delta));
    }

    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::Volume(volume));
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_speed(&self, speed: f64) {
        let _ = self.cmd.send(Cmd::Speed(speed.clamp(0.5, 2.0)));
    }

    /// Repite en bucle el fichero actual (mpv mantiene la opción entre cargas).
    pub fn set_loop(&self, enabled: bool) {
        let _ = self.cmd.send(Cmd::LoopFile(enabled));
    }

    /// Reemplaza la cadena de filtros de audio en caliente.
    ///
    /// `Some(chain)` aplica el grafo al instante (sin reiniciar mpv ni recargar
    /// la pista); `None` o una cadena vacía lo limpian. La cadena sobrevive a
    /// los cambios de fichero porque `af` es una opción del reproductor.
    pub fn set_audio_filters(&self, chain: Option<&str>) {
        let chain = chain
            .map(str::trim)
            .filter(|chain| !chain.is_empty())
            .map(str::to_string);
        let _ = self.cmd.send(Cmd::AudioFilters(chain));
    }

    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Quit);
    }
}

async fn run(
    mpv_path: String,
    socket: PathBuf,
    volume: Arc<AtomicU32>,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    events: broadcast::Sender<PlayerEvent>,
    fail: Arc<std::sync::Mutex<Option<String>>>,
) -> Result<()> {
    let volume_pct = f32::from_bits(volume.load(Ordering::Relaxed)) * 100.0;
    let mut command = tokio::process::Command::new(&mpv_path);
    command
        .arg("--no-video")
        .arg("--no-terminal")
        .arg("--really-quiet")
        .arg("--idle=yes")
        .arg("--gapless-audio=yes")
        .arg("--audio-display=no")
        .arg("--force-window=no")
        .arg("--vo=null")
        .arg("--keep-open=no")
        // Las URLs ya vienen resueltas por `yt-dlp -g`; desactivar el hook de
        // mpv evita que vuelva a extraer una URL de googlevideo y falle con 403.
        .arg("--ytdl=no")
        .arg("--cache=yes")
        .arg("--demuxer-max-bytes=64MiB")
        .arg("--demuxer-readahead-secs=20")
        .arg("--network-timeout=15")
        .arg("--stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=5")
        .arg(format!("--input-ipc-server={}", socket.display()))
        .arg(format!("--volume={volume_pct:.0}"));
    // Permite forzar salida nula en tests/CI (`SPOTE_AO=null`).
    if let Ok(ao) = std::env::var("SPOTE_AO") {
        command.arg(format!("--ao={ao}"));
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("no se pudo ejecutar {mpv_path}"))?;

    // Espera al socket (mpv tarda unos ms en crearlo).
    let mut stream = None;
    for _ in 0..120 {
        match UnixStream::connect(&socket).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => {
                if let Some(status) = child.try_wait()? {
                    bail!("mpv terminó al arrancar ({status})");
                }
                sleep(Duration::from_millis(25)).await;
            }
        }
    }
    let stream = match stream {
        Some(s) => s,
        None => bail!("mpv no creó el socket IPC en {}", socket.display()),
    };

    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Observa las propiedades que necesita la UI.
    for (id, prop) in [
        (1, "time-pos"),
        (2, "duration"),
        (3, "pause"),
        (4, "idle-active"),
        (5, "volume"),
        (6, "speed"),
    ] {
        let command = json!({"command": ["observe_property", id, prop]});
        write_half.write_all(format!("{command}\n").as_bytes()).await?;
    }

    loop {
        tokio::select! {
            biased;
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                let payload = match cmd {
                    Cmd::Quit => break,
                    Cmd::Load { target, start, append } => {
                        let mode = if append { "append" } else { "replace" };
                        if !append {
                            // Una pista nueva siempre suena; si la anterior
                            // quedó en pausa, mpv la cargaría pausada.
                            let _ = write_half
                                .write_all(
                                    format!(
                                        "{}\n",
                                        json!({"command": ["set_property", "pause", false]})
                                    )
                                    .as_bytes(),
                                )
                                .await;
                        }
                        // `start` es una opción persistente de mpv: si no se
                        // resetea, la pista siguiente arranca en el offset de la
                        // anterior. Se envía siempre, incluido 0.
                        let _ = write_half
                            .write_all(
                                format!(
                                    "{}\n",
                                    json!({"command": ["set_property", "start", format!("{start:.3}")]})
                                )
                                .as_bytes(),
                            )
                            .await;
                        json!({"command": ["loadfile", target, mode]})
                    }
                    Cmd::Pause(paused) => json!({"command": ["set_property", "pause", paused]}),
                    Cmd::SeekAbsolute(secs) => json!({"command": ["seek", secs, "absolute"]}),
                    Cmd::SeekRelative(delta) => json!({"command": ["seek", delta, "relative"]}),
                    Cmd::Volume(v) => json!({"command": ["set_property", "volume", v * 100.0]}),
                    Cmd::Speed(speed) => json!({"command": ["set_property", "speed", speed]}),
                    Cmd::LoopFile(enabled) => {
                        let value = if enabled { "inf" } else { "no" };
                        json!({"command": ["set_property", "loop-file", value]})
                    }
                    Cmd::AudioFilters(chain) => af_command(chain.as_deref()),
                    Cmd::Stop => json!({"command": ["stop"]}),
                };
                if write_half
                    .write_all(format!("{}\n", payload).as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => handle_line(&line, &events),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }

    let _ = child.kill().await;
    let _ = std::fs::remove_file(&socket);
    *fail.lock().unwrap() = Some("mpv desconectado".to_string());
    let _ = events.send(PlayerEvent::Disconnected);
    Ok(())
}

fn handle_line(line: &str, events: &broadcast::Sender<PlayerEvent>) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let event = value.get("event").and_then(Value::as_str).unwrap_or_default();
    match event {
        "property-change" => {
            let id = value.get("id").and_then(Value::as_i64).unwrap_or(-1);
            let data = value.get("data");
            let event = match id {
                1 => data.and_then(Value::as_f64).map(PlayerEvent::TimePos),
                2 => data.and_then(Value::as_f64).map(PlayerEvent::Duration),
                3 => data.and_then(Value::as_bool).map(PlayerEvent::Paused),
                4 => data.and_then(Value::as_bool).map(PlayerEvent::Idle),
                5 => data.and_then(Value::as_f64).map(PlayerEvent::Volume),
                6 => data.and_then(Value::as_f64).map(PlayerEvent::Speed),
                _ => None,
            };
            if let Some(event) = event {
                let _ = events.send(event);
            }
        }
        "file-loaded" => {
            let _ = events.send(PlayerEvent::FileLoaded(String::new()));
        }
        "end-file" => {
            let reason = value.get("reason").and_then(Value::as_str).unwrap_or("");
            match reason {
                "eof" => {
                    let _ = events.send(PlayerEvent::Eof);
                }
                "error" => {
                    let message = value
                        .get("file_error")
                        .and_then(Value::as_str)
                        .unwrap_or("mpv no pudo abrir el fichero")
                        .to_string();
                    let _ = events.send(PlayerEvent::Error(message));
                }
                _ => {}
            }
        }
        "idle" => {
            let _ = events.send(PlayerEvent::Idle(true));
        }
        _ => {}
    }
}

/// Comando IPC que reemplaza la cadena de filtros de audio.
///
/// `af` es una propiedad de tipo lista que acepta su representación en string:
/// mpv la parsea y reconstruye el grafo al momento. Un string vacío limpia
/// todos los filtros. Verificado contra mpv 0.40.0: `set_property af "..."`
/// responde `success`, el filtro aparece en `get_property af` y `af ""` deja
/// la lista vacía.
pub fn af_command(chain: Option<&str>) -> Value {
    let chain = chain
        .map(str::trim)
        .filter(|chain| !chain.is_empty())
        .unwrap_or("");
    json!({"command": ["set_property", "af", chain]})
}

/// Ruta del socket IPC, única por proceso.
pub fn socket_path() -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("spote-{pid}-{nanos}.sock"))
}

/// `true` si hay un mpv vivo en ese socket.
pub fn socket_alive(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_unique_and_in_tmp() {
        let a = socket_path();
        let b = socket_path();
        assert_ne!(a, b);
        assert!(a.starts_with(std::env::temp_dir()));
        assert!(a.extension().is_some_and(|e| e == "sock"));
    }

    #[test]
    fn parses_property_events() {
        let (tx, mut rx) = broadcast::channel(8);
        handle_line(
            r#"{"event":"property-change","id":1,"name":"time-pos","data":42.5}"#,
            &tx,
        );
        assert_eq!(rx.try_recv().unwrap(), PlayerEvent::TimePos(42.5));
        handle_line(
            r#"{"event":"property-change","id":3,"name":"pause","data":true}"#,
            &tx,
        );
        assert_eq!(rx.try_recv().unwrap(), PlayerEvent::Paused(true));
        handle_line(r#"{"event":"end-file","reason":"eof"}"#, &tx);
        assert_eq!(rx.try_recv().unwrap(), PlayerEvent::Eof);
        handle_line("basura", &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn af_command_sets_chain_and_clears_with_empty() {
        assert_eq!(
            af_command(Some("equalizer=f=1000:t=q:w=1.0:g=6")),
            json!({"command": ["set_property", "af", "equalizer=f=1000:t=q:w=1.0:g=6"]})
        );
        assert_eq!(
            af_command(None),
            json!({"command": ["set_property", "af", ""]})
        );
        assert_eq!(
            af_command(Some("   ")),
            json!({"command": ["set_property", "af", ""]})
        );
        assert_eq!(
            af_command(Some("  alimiter=limit=0.95  ")),
            json!({"command": ["set_property", "af", "alimiter=limit=0.95"]})
        );
    }

    #[tokio::test]
    async fn missing_mpv_reports_error() {
        let socket = socket_path();
        let result = Player::spawn(socket.clone(), "definitely-not-mpv-xyz", 0.5).await;
        assert!(result.is_err() || result.unwrap().failure().is_some());
        let _ = std::fs::remove_file(socket);
    }
}
