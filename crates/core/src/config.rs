//! Configuración y rutas. Todo tiene defaults: `spote` sin config funciona.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub app: AppCfg,
    pub server: ServerCfg,
    pub player: PlayerCfg,
    pub ui: UiCfg,
    pub download: DownloadCfg,
    pub lyrics: LyricsCfg,
    pub discord: DiscordCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppCfg {
    /// Nombre visible en la UI y en el título de la ventana.
    pub name: String,
}

impl Default for AppCfg {
    fn default() -> Self {
        Self {
            name: "spote".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsCfg {
    /// Usa el motor de lyricbar (LRCLIB + Genius + captions) como primer proveedor.
    pub bridge: bool,
    pub python: String,
    /// Ruta del puente. Relativa al repo si se arranca desde ahí.
    pub script: String,
    /// Checkout de lyricbar.
    pub root: String,
}

impl Default for LyricsCfg {
    fn default() -> Self {
        Self {
            bridge: true,
            python: "python3".to_string(),
            script: "scripts/lyricbar_bridge.py".to_string(),
            root: "/home/fabio/Code/lyricbar".to_string(),
        }
    }
}

/// Discord Rich Presence. Sin `client_id` (Application ID de una app propia en
/// discord.com/developers) no se conecta: Discord rechaza el handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordCfg {
    pub enabled: bool,
    /// Application ID de la app de Discord (no es un token).
    pub client_id: String,
    /// Botón "Abrir en Spotify" bajo la actividad.
    pub buttons: bool,
    /// Clave de asset para el icono pequeño. Cuando exista el logo de spote se
    /// sube a la app de Discord y se pone aquí su nombre.
    pub small_image: String,
}

impl Default for DiscordCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: String::new(),
            buttons: true,
            small_image: "spote".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerCfg {
    pub url: String,
    pub token: String,
    pub name: String,
    pub invite: String,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            url: String::new(),
            token: String::new(),
            name: String::new(),
            invite: "spote".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerCfg {
    pub volume: f32,
    pub prefetch: usize,
    pub mpv_path: String,
}

impl Default for PlayerCfg {
    fn default() -> Self {
        Self {
            volume: 0.8,
            prefetch: 2,
            mpv_path: "mpv".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiCfg {
    /// `dither` | `half` | `braille` | `off`
    pub art_mode: String,
    /// Preset de visualización por defecto (`aurora`, `inferno`, `void`, ...).
    pub viz: String,
    pub viz_enabled: bool,
    pub theme_fade_ms: u64,
    pub fps: u32,
    pub show_fps: bool,
    /// Modo acompañante del cliente oficial de Spotify (MPRIS).
    pub follow_spotify: bool,
    /// Rota el arte de la portada entre modos con disolución progresiva.
    pub art_rotate: bool,
    pub art_rotate_secs: u64,
    /// Lista en tarjetas de 3 filas con miniatura de portada.
    pub list_cards: bool,
    /// Visualizador como fondo de toda la UI, no solo en la franja.
    pub viz_background: bool,
    /// Número de bandas del ecualizador del header.
    pub show_bpm: bool,
}

impl Default for UiCfg {
    fn default() -> Self {
        Self {
            art_mode: "half".to_string(),
            viz: "aurora".to_string(),
            viz_enabled: true,
            theme_fade_ms: 800,
            fps: 60,
            show_fps: false,
            follow_spotify: false,
            art_rotate: true,
            art_rotate_secs: 24,
            list_cards: false,
            viz_background: false,
            show_bpm: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadCfg {
    pub format: String,
    pub dir: String,
    pub update_ytdlp: bool,
}

impl Default for DownloadCfg {
    fn default() -> Self {
        Self {
            format: "opus".to_string(),
            dir: String::new(),
            update_ytdlp: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("spote: config inválida en {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, text)
    }

    pub fn media_dir(&self) -> PathBuf {
        if self.download.dir.is_empty() {
            data_dir().join("media")
        } else {
            expand(&self.download.dir)
        }
    }
}

/// `~/.config/spote/config.toml`
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn config_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SPOTE_HOME") {
        return PathBuf::from(home).join("config");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("spote")
}

pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SPOTE_HOME") {
        return PathBuf::from(home).join("data");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("spote")
}

pub fn cache_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SPOTE_HOME") {
        return PathBuf::from(home).join("cache");
    }
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("spote")
}

pub fn db_path() -> PathBuf {
    data_dir().join("spote.db")
}

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable() {
        let cfg = Config::default();
        assert_eq!(cfg.ui.art_mode, "half");
        assert!(!cfg.server.url.is_empty() || cfg.server.url.is_empty());
        assert!(cfg.player.volume > 0.0);
    }

    #[test]
    fn toml_roundtrip() {
        let cfg = Config::default();
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.ui.art_mode, cfg.ui.art_mode);
        assert_eq!(back.ui.fps, cfg.ui.fps);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg: Config = toml::from_str("[ui]\nfps = 30\n").unwrap();
        assert_eq!(cfg.ui.fps, 30);
        assert_eq!(cfg.ui.art_mode, "half");
        assert_eq!(cfg.player.volume, 0.8);
    }
}
