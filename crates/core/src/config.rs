//! Configuración y rutas. Todo tiene defaults: `spote` sin config funciona.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio_fx::AudioCfg;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub app: AppCfg,
    pub server: ServerCfg,
    pub player: PlayerCfg,
    pub playback: PlaybackCfg,
    pub ui: UiCfg,
    pub download: DownloadCfg,
    pub lyrics: LyricsCfg,
    pub discord: DiscordCfg,
    /// EQ, preamp, normalizador y limitador aplicados en caliente a mpv.
    pub audio: AudioCfg,
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
    /// Intérprete para el transcriptor ASR (faster-whisper).
    pub python: String,
    /// Afina las líneas estimadas contra el audio local con faster-whisper.
    /// La alineación es de spote; solo se delega la transcripción.
    pub asr: bool,
    /// Transcriptor auxiliar (ventanas → palabras). Relativo al repo.
    pub asr_script: String,
    pub asr_model: String,
    pub asr_device: String,
    /// `auto` deja que el modelo detecte el idioma.
    pub asr_language: String,
    /// Permite generar letra cuando no hay una fuente fiable: captions de
    /// YouTube como texto y transcripción ASR. Desactivado por defecto porque
    /// las auto-captions y el ASR pueden inventar líneas.
    pub allow_generated: bool,
}

impl Default for LyricsCfg {
    fn default() -> Self {
        Self {
            python: "python3".to_string(),
            asr: true,
            asr_script: "scripts/asr_words.py".to_string(),
            asr_model: "small".to_string(),
            asr_device: "auto".to_string(),
            asr_language: "auto".to_string(),
            allow_generated: false,
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
    /// Clave de cliente propia (`SPOTE_CLIENT_KEYS`). Sin ella la API responde 401.
    pub client_key: String,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            url: String::new(),
            token: String::new(),
            name: String::new(),
            invite: String::new(),
            client_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerCfg {
    pub volume: f32,
    pub prefetch: usize,
    pub mpv_path: String,
    /// Reproduce una URL resuelta mientras se descarga la pista en segundo plano.
    pub streaming: bool,
    /// Guarda en `media/` lo que se está reproduciendo en streaming.
    pub stream_cache: bool,
    /// `system` captura el monitor para el visualizador; `off` lo apaga.
    pub remote_viz: String,
}

impl Default for PlayerCfg {
    fn default() -> Self {
        Self {
            volume: 0.8,
            prefetch: 2,
            mpv_path: "mpv".to_string(),
            streaming: true,
            stream_cache: true,
            remote_viz: "system".to_string(),
        }
    }
}

/// Modos de reproducción persistidos. Son preferencias, no estado de sesión.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackCfg {
    /// `off` | `all` | `one`
    pub repeat: String,
    pub shuffle: bool,
}

impl Default for PlaybackCfg {
    fn default() -> Self {
        Self {
            repeat: "off".to_string(),
            shuffle: false,
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
    /// Límite de frames de la UI (20-60). Por encima de 60 una TUI no gana
    /// nada: el terminal repinta igual y el coste por frame se multiplica.
    pub fps: u32,
    /// Ritmo al que se regeneran los canvas de efectos (5-60). Es independiente
    /// de `fps`: el bucle de la UI sigue corriendo, pero reutiliza el canvas
    /// anterior hasta que toca el siguiente. Un fondo a pantalla completa a
    /// 60 Hz es lo que pone la iGPU al 100 %.
    pub fx_fps: u32,
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
    /// Altura del efecto en zen, en % del alto (30-100). El resto queda de
    /// fondo/mosaico para que la animación empiece más abajo.
    pub zen_viz_pct: u8,
    /// Número de bandas del ecualizador del header.
    pub show_bpm: bool,
    /// Tamaño/zoom de la letra: 0 compacta, 1 normal, 2 grande.
    pub lyrics_size: u8,
    /// Opacidad de los paneles, en % (55-100). El fondo se sigue viendo.
    pub panel_opacity: u8,
    /// Opacidad de las barras de chrome (top, navegación), en %.
    pub chrome_opacity: u8,
    /// Opacidad del pie (transporte, nowbar y hint), en %.
    pub bottom_opacity: u8,
    /// Velo de los overlays, en % (0-90).
    pub overlay_scrim: u8,
    /// `color` (portada reactiva), `line` (marco fino) o `none`.
    pub panel_border: String,
    /// Opacidad del borde (0-100), independiente del relleno del panel.
    pub border_opacity: u8,
    /// Intensidad del borde de color (0-100).
    pub rim_intensity: u8,
    /// El rim vibra con bombo/caja.
    pub rim_reactive: bool,
    /// `top`, `side`, `both` o `auto`. En `auto`, lateral = secciones y
    /// arriba = pestañas de documentos.
    pub nav_position: String,
    /// `docked`, `expand` o `auto` para artista/álbum.
    pub detail_mode: String,
    /// Efectos que entran en la rotación de `v`. El resto queda a mano.
    pub effects_enabled: Vec<String>,
    /// Rotación temporizada de efectos. Apagada por defecto: `v` manda.
    pub effects_auto: bool,
    /// `lateral`, `centered`, `behind` o `off`.
    pub lyrics_scenario: String,
    /// `line` o `word`.
    pub lyrics_scenario_unit: String,
    /// Variante de paleta derivada de la portada.
    pub palette_variant: String,
    /// Restaura las pestañas de la sesión anterior.
    pub restore_session: bool,
    /// Migración interna del rediseño: no se expone en la UI.
    pub ui_v2_migrated: bool,
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
    /// bandas braille), `lyrics` (letra en bandas) o `mosaic` (portada en
    /// mosaico detrás de toda la interfaz).
    pub ambient: String,
    /// Opacidad del fondo ambiental (0-50 %).
    pub ambient_opacity: u8,
    /// `mosaic`, `wallpaper`, `gradient` u `off`.
    pub backdrop: String,
    /// `half`, `plain`, `dither` o `braille`.
    pub mosaic_mode: String,
    /// Opacidad del mosaico (5-50 %).
    pub mosaic_opacity: u8,
    /// Reactividad automática del movimiento a la energía de la canción.
    pub mosaic_auto: bool,
    /// `auto`, `small`, `medium` o `large`.
    pub mosaic_tile: String,
    /// Wallpaper Kitty estático en vez del mosaico de celdas.
    pub mosaic_wallpaper: bool,
    /// Movimiento del mosaico de portada: `drift`, `rows`, `columns`,
    /// `diagonal`, `rotate` o `auto`.
    pub mosaic_motion: String,
    /// Velocidad del mosaico (1-10).
    pub mosaic_speed: u8,
    /// El mosaico salta al compás del bombo.
    pub mosaic_beat: bool,
    /// `mosaic` o `black`: fondo del visualizador a pantalla completa.
    pub fullscreen_backdrop: String,
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
            backdrop: "mosaic".into(),
            mosaic_mode: "half".into(),
            mosaic_opacity: 22,
            mosaic_auto: true,
            mosaic_tile: "auto".into(),
            mosaic_wallpaper: false,
            mosaic_motion: "auto".into(),
            mosaic_speed: 4,
            mosaic_beat: true,
            fullscreen_backdrop: "mosaic".into(),
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
            fps: 60,
            fx_fps: 60,
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
            zen_viz_pct: 55,
            show_bpm: true,
            lyrics_size: 1,
            panel_opacity: 72,
            chrome_opacity: 40,
            bottom_opacity: 92,
            overlay_scrim: 40,
            panel_border: "line".into(),
            border_opacity: 80,
            rim_intensity: 85,
            rim_reactive: true,
            nav_position: "auto".into(),
            detail_mode: "docked".into(),
            effects_enabled: vec![
                "clean".into(),
                "aurora".into(),
                "bloom".into(),
                "crt".into(),
                "braille".into(),
            ],
            effects_auto: false,
            lyrics_scenario: "off".into(),
            lyrics_scenario_unit: "line".into(),
            palette_variant: "cover".into(),
            restore_session: false,
            ui_v2_migrated: true,
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
                let parsed = toml::from_str::<Config>(&text);
                let mut valid = true;
                let mut cfg = match parsed {
                    Ok(cfg) => cfg,
                    Err(err) => {
                        valid = false;
                        eprintln!("spote: config inválida en {}: {err}", path.display());
                        // Se conserva una copia para que el usuario pueda
                        // recuperar sus ajustes si el siguiente save() escribe
                        // los defaults. Con timestamp para no pisar el backup
                        // bueno si hay dos cargas inválidas seguidas.
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let _ =
                            std::fs::copy(&path, path.with_extension(format!("toml.bak-{stamp}")));
                        Self::default()
                    }
                };
                cfg.audio = cfg.audio.sanitized();
                // Configs anteriores al rediseño: la rotación venía encendida y
                // no existía `[ui.layout]`. Solo se migra si el TOML era válido.
                if valid && !text.contains("[ui.layout]") {
                    cfg.ui.layout = LayoutCfg::default();
                    cfg.ui.layout.vcenter = true;
                    cfg.ui.art_rotate = false;
                    if cfg.ui.thumb_mode == "off" {
                        cfg.ui.thumb_mode = "plain".into();
                    }
                    let _ = cfg.save();
                }
                if valid && !cfg.ui.ui_v2_migrated {
                    // La primera versión del rediseño dejó el rim y el
                    // escenario de letra como defaults. Se restauran el marco
                    // fino y la letra normal una sola vez.
                    if cfg.ui.panel_border == "rim" {
                        cfg.ui.panel_border = "line".into();
                    }
                    if cfg.ui.lyrics_scenario != "off" {
                        cfg.ui.lyrics_scenario = "off".into();
                    }
                    if cfg.ui.chrome_opacity == 80 {
                        cfg.ui.chrome_opacity = 40;
                    }
                    cfg.ui.ui_v2_migrated = true;
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
        // Escritura atómica: un corte a mitad no deja la config truncada.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        // El fichero guarda token, invitación y clave de cliente: solo el
        // usuario que ejecuta spote debe poder leerlo.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path)
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

    use crate::audio_fx::NormalizerMode;

    #[test]
    fn defaults_are_usable() {
        let cfg = Config::default();
        assert_eq!(cfg.ui.art_mode, "plain");
        assert!(!cfg.server.url.is_empty() || cfg.server.url.is_empty());
        assert!(cfg.player.volume > 0.0);
        // Por defecto no hay limitador ni EQ, pero sí nivelación entre pistas.
        assert!(!cfg.audio.limiter);
        assert!(cfg.audio.normalize_tracks);
        assert_eq!(
            cfg.audio.filter_chain().as_deref(),
            Some("dynaudnorm=f=500:g=25:p=0.9:m=20")
        );
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
    fn old_audio_section_gets_leveling_enabled() {
        let cfg: Config = toml::from_str("[audio]\nnormalizer = \"off\"\n").unwrap();
        assert!(
            cfg.audio.normalize_tracks,
            "la nivelación debe venir activa"
        );
    }

    #[test]
    fn generated_lyrics_are_opt_in_and_roundtrip() {
        assert!(!Config::default().lyrics.allow_generated);
        let old: Config = toml::from_str(
            "[lyrics]
asr = true\n",
        )
        .unwrap();
        assert!(!old.lyrics.allow_generated, "config vieja = off");
        let mut cfg = Config::default();
        cfg.lyrics.allow_generated = true;
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(back.lyrics.allow_generated);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg: Config = toml::from_str("[ui]\nfps = 30\n").unwrap();
        assert_eq!(cfg.ui.fps, 30);
        assert_eq!(cfg.ui.art_mode, "plain");
        assert_eq!(cfg.player.volume, 0.8);
    }

    #[test]
    fn old_config_without_audio_section_uses_audio_defaults() {
        let cfg: Config = toml::from_str("[ui]\nfps = 30\n[player]\nvolume = 0.4\n").unwrap();
        assert_eq!(cfg.audio, AudioCfg::default());
        assert!(cfg.audio.enabled);
        assert!(cfg.audio.eq_enabled);
        assert_eq!(cfg.audio.normalizer, NormalizerMode::Off);
    }

    #[test]
    fn audio_section_roundtrips_through_toml() {
        let mut cfg = Config::default();
        cfg.audio.enabled = true;
        cfg.audio.preamp_db = -2.5;
        cfg.audio.bands = [1.0, 0.0, -1.5, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0, -4.0];
        cfg.audio.normalizer = NormalizerMode::Dynamic;
        cfg.audio.normalizer_strength = 0.75;
        cfg.audio.limiter = true;

        let text = toml::to_string(&cfg).unwrap();
        assert!(text.contains("[audio]"));
        assert!(text.contains("normalizer = \"dynamic\""));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.audio, cfg.audio);
        assert_eq!(back.ui.art_mode, cfg.ui.art_mode);
    }
}
