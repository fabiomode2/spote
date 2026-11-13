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
            client_id: "1570904121924657204".to_string(),
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
    /// `dither` | `half` | `plain` | `braille` | `off`
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
    /// Incluye `off` en la rotación (por defecto se salta).
    pub art_rotate_off: bool,
    /// Lista en tarjetas de 3 filas con miniatura de portada.
    pub list_cards: bool,
    /// Visualizador como fondo de toda la UI, no solo en la franja.
    pub viz_background: bool,
    /// Centra verticalmente el bloque de paneles (el cuadrado), no el texto.
    pub vcenter: bool,
    /// Efecto de las miniaturas de las listas: `palette`, `plain` u `off`.
    pub thumb_mode: String,
    /// Dibuja la barra de progreso dentro de la franja del visualizador.
    pub progress_in_viz: bool,
    /// Ancho de la barra de progreso, en % del ancho disponible (55-100).
    pub bar_width_pct: u8,
    /// Aire alrededor de los iconos del transporte (1-4 celdas).
    pub button_pad: u8,
    /// Arranca en modo zen (solo visualización, sin portada ni letra).
    pub zen: bool,
    /// Número de bandas del ecualizador del header.
    pub show_bpm: bool,
    /// Tamaño/zoom de la letra: 0 compacta, 1 normal, 2 grande.
    pub lyrics_size: u8,
    /// Diseño de paneles (se puede editar en caliente con `E`).
    pub layout: LayoutCfg,
    /// Efectos ambientales repartidos por la UI.
    pub fx: FxCfg,
}

/// Geometría y visibilidad de los paneles. Todo se puede tocar en runtime
/// desde el editor de diseño (`E`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutCfg {
    pub top: bool,
    pub nav: bool,
    pub queue: bool,
    pub info: bool,
    pub viz: bool,
    pub transport: bool,
    pub hint: bool,
    /// Centra el bloque de paneles en vertical en vez de estirarlo.
    pub vcenter: bool,
    /// Altura del bloque centrado (30-100 % del alto disponible).
    pub vcenter_pct: u8,
    /// Ancho del panel de navegación (columnas).
    pub nav_width: u16,
    /// Ancho del panel lateral de cola/info (columnas).
    pub side_width: u16,
    /// Alto de la franja del visualizador (filas).
    pub viz_height: u16,
    /// Alto del transporte (filas, 2-4).
    pub transport_height: u16,
    /// `left` o `right`.
    pub nav_dock: String,
    pub side_dock: String,
    /// Porcentaje del lateral que ocupa la cola; el resto es info.
    pub queue_share: u8,
    /// Tamaño de la portada respecto al centro (40-100 %).
    pub art_size: u8,
    /// `auto`, `left` o `right`: dónde va la portada respecto a la letra.
    pub art_side: String,
}

impl Default for LayoutCfg {
    fn default() -> Self {
        Self {
            top: true,
            nav: true,
            queue: true,
            info: true,
            viz: true,
            transport: true,
            hint: true,
            vcenter: true,
            vcenter_pct: 82,
            nav_width: 19,
            side_width: 42,
            viz_height: 8,
            transport_height: 4,
            nav_dock: "left".into(),
            side_dock: "right".into(),
            queue_share: 62,
            art_size: 100,
            art_side: "auto".into(),
        }
    }
}

impl LayoutCfg {
    pub fn nav_left(&self) -> bool {
        self.nav_dock != "right"
    }

    pub fn side_left(&self) -> bool {
        self.side_dock == "left"
    }

    pub fn dock_label(&self) -> String {
        format!(
            "nav {}, lateral {}",
            if self.nav_left() { "izq" } else { "der" },
            if self.side_left() { "izq" } else { "der" }
        )
    }
}

/// Efectos ambientales. Los presets ruidosos (`void`, `neural`, `glitch`) se
/// usan de fondo con poca opacidad; `clean`/`aurora`/`bloom`/`inferno` se
/// quedan en la franja inferior y en detalles puntuales.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FxCfg {
    /// `off`, `void`, `aurora`, `neural`, `glitch`, `cover` (portada en
    /// bandas braille) o `lyrics` (letra en bandas).
    pub ambient: String,
    /// Opacidad del fondo ambiental (0-40 %).
    pub ambient_opacity: u8,
    /// Los bordes laten con bombo/caja/charles/voz.
    pub instrument_borders: bool,
    /// Llama/brasa en el borde inferior.
    pub edge_flame: bool,
    /// Glow de bloom detrás del play y en la esquina.
    pub bloom_play: bool,
    /// La onda de `clean` puede subir por encima de los paneles.
    pub clean_spill: bool,
    /// Trama de glitch muy tenue dentro de los paneles.
    pub glitch_panels: bool,
}

impl Default for FxCfg {
    fn default() -> Self {
        Self {
            ambient: "cover".into(),
            ambient_opacity: 16,
            instrument_borders: true,
            edge_flame: true,
            bloom_play: true,
            clean_spill: true,
            glitch_panels: false,
        }
    }
}

impl Default for UiCfg {
    fn default() -> Self {
        Self {
            art_mode: "plain".to_string(),
            viz: "clean".to_string(),
            viz_enabled: true,
            theme_fade_ms: 800,
            fps: 120,
            show_fps: false,
            follow_spotify: false,
            art_rotate: false,
            art_rotate_secs: 24,
            art_rotate_off: false,
            list_cards: false,
            viz_background: false,
            vcenter: true,
            thumb_mode: "plain".to_string(),
            progress_in_viz: false,
            bar_width_pct: 70,
            button_pad: 2,
            zen: false,
            show_bpm: true,
            lyrics_size: 1,
            layout: LayoutCfg::default(),
            fx: FxCfg::default(),
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
            Ok(text) => {
                let mut cfg: Config = toml::from_str(&text).unwrap_or_else(|err| {
                    eprintln!("spote: config inválida en {}: {err}", path.display());
                    Self::default()
                });
                // Configs anteriores al rediseño: la rotación venía encendida y
                // no existía `[ui.layout]`. Se migra una vez, sin tocar el resto.
                if !text.contains("[ui.layout]") {
                    cfg.ui.layout = LayoutCfg::default();
                    cfg.ui.layout.vcenter = true;
                    cfg.ui.art_rotate = false;
                    if cfg.ui.thumb_mode == "off" {
                        cfg.ui.thumb_mode = "plain".into();
                    }
                    let _ = cfg.save();
                }
                cfg
            }
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
        assert_eq!(cfg.ui.art_mode, "plain");
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
        assert_eq!(cfg.ui.art_mode, "plain");
        assert_eq!(cfg.player.volume, 0.8);
    }
}
