//! Protocolo del mini reproductor de escritorio.
//!
//! El mini reproductor (`spote-mini`) habla con la app principal por un socket
//! Unix local. Cada mensaje es una línea JSON terminada en `\n` (NDJSON):
//!
//! - el servidor manda [`MiniSnapshot`], el estado completo de reproducción;
//! - el cliente manda [`MiniCommand`], una acción puntual.
//!
//! Ambos extremos deben resolver la ruta con [`mini_socket_path`] para que el
//! cliente y el servidor no se desincronicen. El servidor escucha, el cliente
//! conecta y reintenta cada segundo.
//!
//! Unidades y rangos:
//! - `position_ms` y `duration_ms`: milisegundos.
//! - `volume`: `0.0..=1.0` (la UI lo muestra como porcentaje).
//! - `bands`: 32 valores `0.0..=1.0`, de grave a agudo.
//! - `repeat`: `off`, `all` o `one`.
//! - `accent`: componente RGB de la portada.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Número de bandas del espectro que espera el mini reproductor.
pub const MINI_BAND_COUNT: usize = 32;

/// Nombre del archivo del socket. El directorio sale de [`mini_socket_path`].
pub const MINI_SOCKET_NAME: &str = "spote-mini.sock";

pub const MINI_REPEAT_OFF: &str = "off";
pub const MINI_REPEAT_ALL: &str = "all";
pub const MINI_REPEAT_ONE: &str = "one";

/// Modos de repetición, en el orden en que cicla la tecla de repeat.
pub const MINI_REPEAT_MODES: [&str; 3] = [MINI_REPEAT_OFF, MINI_REPEAT_ALL, MINI_REPEAT_ONE];

/// Estado completo que el servidor envía al mini reproductor.
///
/// El servidor manda un snapshot cada vez que cambia algo relevante (pista,
/// play/pausa, posición, volumen, espectro...) y también periódicamente para
/// refrescar la posición. El cliente puede pintar sin pedir nada más.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MiniSnapshot {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// `0.0..=1.0`.
    pub volume: f32,
    pub shuffle: bool,
    /// `off` | `all` | `one`.
    pub repeat: String,
    /// Ruta local de la portada, si ya está descargada.
    pub art_path: Option<String>,
    /// Color dominante de la portada.
    pub accent: [u8; 3],
    /// 32 bandas `0.0..=1.0`. Si viene vacío, el cliente dibuja su animación.
    pub bands: Vec<f32>,
    /// Texto libre del reproductor (`playing`, `paused`, `buffering`...).
    pub status: String,
    /// Identidad canónica de la pista; cambia cuando cambia la canción.
    pub track_key: String,
    /// Preset de visualización activo en la TUI.
    pub viz_preset: String,
    /// Presets disponibles para ciclar desde el mini.
    pub viz_presets: Vec<String>,
    /// Frame del visual de la TUI ya compuesto, en una rejilla pequeña.
    pub viz_cols: u16,
    pub viz_rows: u16,
    pub viz: Vec<MiniVizCell>,
}

/// Celda del visual de la TUI reenviada al mini.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MiniVizCell {
    pub x: u16,
    pub y: u16,
    /// Carácter de la celda. El mini lo dibuja con fuente monoespaciada.
    pub ch: char,
    pub fg: [u8; 3],
    /// 0 = transparente, 255 = opaco.
    pub alpha: u8,
}

impl MiniSnapshot {
    pub fn new(title: impl Into<String>, artist: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            artist: artist.into(),
            ..Self::default()
        }
    }

    pub fn with_playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    pub fn with_position_ms(mut self, position_ms: u64) -> Self {
        self.position_ms = position_ms;
        if self.duration_ms > 0 {
            self.position_ms = self.position_ms.min(self.duration_ms);
        }
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        if duration_ms > 0 {
            self.position_ms = self.position_ms.min(duration_ms);
        }
        self
    }

    /// Volumen en `0.0..=1.0`; fuera de rango se recorta.
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = clean_volume(volume);
        self
    }

    pub fn with_shuffle(mut self, shuffle: bool) -> Self {
        self.shuffle = shuffle;
        self
    }

    pub fn with_repeat(mut self, repeat: impl AsRef<str>) -> Self {
        self.repeat = normalize_repeat(repeat.as_ref()).to_owned();
        self
    }

    pub fn with_art_path(mut self, art_path: impl Into<String>) -> Self {
        self.art_path = Some(art_path.into());
        self
    }

    pub fn with_accent(mut self, accent: [u8; 3]) -> Self {
        self.accent = accent;
        self
    }

    /// Guarda exactamente 32 bandas, recortadas a `0.0..=1.0`.
    ///
    /// Los valores no finitos se convierten en `0.0`; si faltan bandas se
    /// rellenan con ceros. Así el cliente siempre puede indexar `0..32`.
    pub fn with_bands(mut self, bands: impl IntoIterator<Item = f32>) -> Self {
        self.set_bands(bands);
        self
    }

    pub fn set_bands(&mut self, bands: impl IntoIterator<Item = f32>) {
        self.bands = bands
            .into_iter()
            .take(MINI_BAND_COUNT)
            .map(|band| {
                if band.is_finite() {
                    band.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            })
            .collect();
        self.bands.resize(MINI_BAND_COUNT, 0.0);
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    pub fn with_track_key(mut self, track_key: impl Into<String>) -> Self {
        self.track_key = track_key.into();
        self
    }

    /// Modo de repetición saneado; cualquier valor desconocido cae en `off`.
    pub fn repeat_mode(&self) -> &'static str {
        normalize_repeat(&self.repeat)
    }

    /// Progreso `0.0..=1.0`. Sin duración conocida devuelve `0.0`.
    pub fn progress(&self) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        (self.position_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
    }

    /// Volumen como porcentaje entero, para la UI.
    pub fn volume_percent(&self) -> u8 {
        (clean_volume(self.volume) * 100.0).round() as u8
    }

    pub fn is_idle(&self) -> bool {
        self.track_key.is_empty() && self.title.is_empty()
    }

    /// Serializa el snapshot como una línea NDJSON terminada en `\n`.
    pub fn to_line(&self) -> serde_json::Result<String> {
        crate::encode_line(self)
    }

    /// Decodifica una línea, aceptando `\n`, `\r\n` y espacios alrededor.
    pub fn from_line(line: &str) -> serde_json::Result<Self> {
        serde_json::from_str(line.trim())
    }
}

/// Acción del mini reproductor hacia la app principal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MiniCommand {
    PlayPause,
    Next,
    Previous,
    Seek(u64),
    /// `0.0..=1.0`.
    SetVolume(f32),
    Shuffle,
    CycleRepeat,
    /// Cambia al siguiente preset de visualización.
    CycleViz,
    Quit,
}

impl MiniCommand {
    /// Seek con la posición ya saneada.
    pub fn seek_to(position_ms: u64) -> Self {
        Self::Seek(position_ms)
    }

    /// `SetVolume` con el volumen recortado a `0.0..=1.0`.
    pub fn set_volume(volume: f32) -> Self {
        Self::SetVolume(clean_volume(volume))
    }

    pub fn is_quit(&self) -> bool {
        matches!(self, Self::Quit)
    }

    /// Serializa el comando como una línea NDJSON terminada en `\n`.
    pub fn to_line(&self) -> serde_json::Result<String> {
        crate::encode_line(self)
    }

    /// Decodifica una línea, aceptando `\n`, `\r\n` y espacios alrededor.
    pub fn from_line(line: &str) -> serde_json::Result<Self> {
        serde_json::from_str(line.trim())
    }
}

/// Normaliza un modo de repetición al conjunto cerrado del protocolo.
pub fn normalize_repeat(repeat: &str) -> &'static str {
    match repeat {
        MINI_REPEAT_ALL => MINI_REPEAT_ALL,
        MINI_REPEAT_ONE => MINI_REPEAT_ONE,
        _ => MINI_REPEAT_OFF,
    }
}

/// Siguiente modo al pulsar repeat: `off -> all -> one -> off`.
pub fn next_repeat(repeat: &str) -> &'static str {
    match normalize_repeat(repeat) {
        MINI_REPEAT_ALL => MINI_REPEAT_ONE,
        MINI_REPEAT_ONE => MINI_REPEAT_OFF,
        _ => MINI_REPEAT_ALL,
    }
}

/// Formatea milisegundos como `m:ss` o `h:mm:ss`.
pub fn format_time(ms: u64) -> String {
    let total_secs = ms / 1_000;
    let hours = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Ruta del socket del mini reproductor, compartida por cliente y servidor.
///
/// Precedencia:
/// 1. `$XDG_RUNTIME_DIR/spote-mini.sock` si el directorio existe.
/// 2. `$SPOTE_HOME/cache/spote-mini.sock`.
/// 3. `$XDG_CACHE_HOME/spote/spote-mini.sock`.
/// 4. `/tmp/spote-mini-$UID.sock`.
///
/// No crea directorios: el servidor es quien debe crearlos antes de escuchar.
pub fn mini_socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let spote_home = std::env::var_os("SPOTE_HOME").map(PathBuf::from);
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    mini_socket_path_from(
        runtime.as_deref(),
        spote_home.as_deref(),
        cache.as_deref(),
        current_uid(),
    )
}

fn mini_socket_path_from(
    runtime: Option<&Path>,
    spote_home: Option<&Path>,
    cache: Option<&Path>,
    uid: u32,
) -> PathBuf {
    if let Some(dir) = runtime {
        if dir.is_dir() {
            return dir.join(MINI_SOCKET_NAME);
        }
    }
    if let Some(home) = spote_home {
        return home.join("cache").join(MINI_SOCKET_NAME);
    }
    if let Some(cache) = cache {
        return cache.join("spote").join(MINI_SOCKET_NAME);
    }
    PathBuf::from("/tmp").join(format!("spote-mini-{uid}.sock"))
}

fn clean_volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// UID real del proceso sin añadir dependencias; `/proc` es suficiente en Linux.
fn current_uid() -> u32 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                if let Some(uid) = rest.split_whitespace().next() {
                    if let Ok(uid) = uid.parse() {
                        return uid;
                    }
                }
            }
        }
    }
    std::env::var("UID")
        .ok()
        .and_then(|uid| uid.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> MiniSnapshot {
        MiniSnapshot::new("Hyperballad", "Björk")
            .with_album("Post")
            .with_playing(true)
            .with_duration_ms(326_000)
            .with_position_ms(42_500)
            .with_volume(0.8)
            .with_shuffle(true)
            .with_repeat("all")
            .with_art_path("/tmp/post.png")
            .with_accent([32, 180, 164])
            .with_bands([0.1, 0.4, 0.9])
            .with_status("playing")
            .with_track_key("isrc:GBBKS9500001")
    }

    #[test]
    fn zero_duration_does_not_erase_the_position() {
        let snapshot = MiniSnapshot::new("Song", "Artist")
            .with_position_ms(42_500)
            .with_duration_ms(0);
        assert_eq!(snapshot.position_ms, 42_500);
        assert_eq!(snapshot.duration_ms, 0);
    }

    #[test]
    fn snapshot_roundtrips_as_ndjson() {
        let snapshot = sample_snapshot();
        let line = snapshot.to_line().unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.trim_end().lines().count(), 1);
        assert_eq!(MiniSnapshot::from_line(&line).unwrap(), snapshot);

        let crlf = line.replace('\n', "\r\n");
        assert_eq!(MiniSnapshot::from_line(&crlf).unwrap(), snapshot);
    }

    #[test]
    fn snapshot_tolerates_missing_fields() {
        let parsed: MiniSnapshot =
            serde_json::from_str(r#"{"title":"Solo","artist":"Alguien"}"#).unwrap();
        assert_eq!(parsed.title, "Solo");
        assert_eq!(parsed.artist, "Alguien");
        assert_eq!(parsed.album, None);
        assert!(!parsed.playing);
        assert_eq!(parsed.bands, Vec::<f32>::new());
        assert_eq!(parsed.repeat_mode(), MINI_REPEAT_OFF);
        assert_eq!(parsed.volume_percent(), 0);
    }

    #[test]
    fn commands_roundtrip() {
        let commands = [
            MiniCommand::PlayPause,
            MiniCommand::Next,
            MiniCommand::Previous,
            MiniCommand::Seek(42_000),
            MiniCommand::SetVolume(0.8),
            MiniCommand::Shuffle,
            MiniCommand::CycleRepeat,
            MiniCommand::Quit,
        ];
        for command in commands {
            let line = command.to_line().unwrap();
            assert!(line.ends_with('\n'));
            assert_eq!(MiniCommand::from_line(&line).unwrap(), command);
        }

        assert_eq!(
            serde_json::to_string(&MiniCommand::PlayPause).unwrap(),
            "\"PlayPause\""
        );
        assert_eq!(
            serde_json::to_string(&MiniCommand::Seek(42_000)).unwrap(),
            "{\"Seek\":42000}"
        );
        assert_eq!(
            serde_json::to_string(&MiniCommand::SetVolume(0.5)).unwrap(),
            "{\"SetVolume\":0.5}"
        );
    }

    #[test]
    fn helpers_clamp_and_normalize() {
        let snapshot = MiniSnapshot::default()
            .with_position_ms(500)
            .with_duration_ms(100)
            .with_volume(3.0)
            .with_repeat("raro")
            .with_bands([-1.0, 0.5, 2.0, f32::NAN]);
        assert_eq!(snapshot.position_ms, 100);
        assert_eq!(snapshot.volume, 1.0);
        assert_eq!(snapshot.repeat_mode(), MINI_REPEAT_OFF);
        assert_eq!(snapshot.bands.len(), MINI_BAND_COUNT);
        assert_eq!(snapshot.bands[0], 0.0);
        assert_eq!(snapshot.bands[1], 0.5);
        assert_eq!(snapshot.bands[2], 1.0);
        assert_eq!(snapshot.bands[3], 0.0);
        assert_eq!(snapshot.bands[31], 0.0);
        assert_eq!(MiniCommand::set_volume(9.0), MiniCommand::SetVolume(1.0));
    }

    #[test]
    fn progress_volume_and_idle_helpers() {
        let snapshot = MiniSnapshot::new("Tema", "Artista")
            .with_duration_ms(200_000)
            .with_position_ms(50_000)
            .with_volume(0.8);
        assert!((snapshot.progress() - 0.25).abs() < 1e-6);
        assert_eq!(snapshot.volume_percent(), 80);
        assert!(!snapshot.is_idle());
        assert!(MiniSnapshot::default().is_idle());
    }

    #[test]
    fn repeat_cycles_off_all_one() {
        assert_eq!(next_repeat("off"), MINI_REPEAT_ALL);
        assert_eq!(next_repeat("all"), MINI_REPEAT_ONE);
        assert_eq!(next_repeat("one"), MINI_REPEAT_OFF);
        assert_eq!(next_repeat("desconocido"), MINI_REPEAT_ALL);
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(1_999), "0:01");
        assert_eq!(format_time(65_000), "1:05");
        assert_eq!(format_time(3_600_000), "1:00:00");
        assert_eq!(format_time(3_665_000), "1:01:05");
        assert_eq!(format_time(59_999), "0:59");
    }

    #[test]
    fn socket_path_respects_precedence() {
        let existing_runtime = Path::new(env!("CARGO_MANIFEST_DIR"));
        let missing_runtime =
            std::env::temp_dir().join(format!("spote-mini-no-existe-{}", std::process::id()));

        assert_eq!(
            mini_socket_path_from(
                Some(existing_runtime),
                Some(Path::new("/spote-home")),
                Some(Path::new("/xdg-cache")),
                1000
            ),
            existing_runtime.join(MINI_SOCKET_NAME)
        );

        assert_eq!(
            mini_socket_path_from(
                Some(&missing_runtime),
                Some(Path::new("/spote-home")),
                Some(Path::new("/xdg-cache")),
                1000
            ),
            PathBuf::from("/spote-home/cache").join(MINI_SOCKET_NAME)
        );

        assert_eq!(
            mini_socket_path_from(
                Some(&missing_runtime),
                None,
                Some(Path::new("/xdg-cache")),
                1000
            ),
            PathBuf::from("/xdg-cache/spote").join(MINI_SOCKET_NAME)
        );

        assert_eq!(
            mini_socket_path_from(Some(&missing_runtime), None, None, 1000),
            PathBuf::from("/tmp/spote-mini-1000.sock")
        );
    }
}
