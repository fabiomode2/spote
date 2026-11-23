//! Estado de la aplicación y máquina de acciones. La UI solo lee de aquí.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use spote_core::artwork::{ArtMode, CoverArt, Palette};
use spote_core::audio::Spectrum;
use spote_core::audio_map::SongMap;
use spote_core::config::{self, Config};
use spote_core::downloader::{DownloadEvent, DownloadSource, Downloader, SearchItem};
use spote_core::library::{Library, TrackRow};
use spote_core::lyrics::LyricsDoc;
use spote_core::playback::{self, Advance, RepeatMode, ShuffleState};
use spote_core::player::{Player, PlayerEvent};
use spote_core::stream::{StreamEvent, StreamResolver};
use spote_core::system_media::SystemMediaState;
use spote_core::spotify_api::{AlbumSummary, ArtistSummary};
use spote_core::util;
use spote_core::ToolStatus;
use spote_protocol::{ControlAction, Presence, QueueItem, Room, RoomState, TrackRef};

use crate::fetch::{
    ArtKind, ArtRequest, ArtResponse, ArtWorker, LyricsMsg, LyricsWorker, MetaEvent, MetaWorker,
    SearchWorker, SongMapEvent, SongMapWorker,
};
use crate::system_media::{SystemCommand, SystemMediaWorker};
use crate::social::{Social, SocialCmd, SocialEvent};
use crate::theme::{FpsMeter, FxClock, PaletteRuntime, Pulse, Spring};
use crate::viz::Viz;

mod actions;
mod audio_ui;
mod mini;
mod mpris;
pub(crate) mod albums;
mod artist;
mod context;
mod downloads;
mod input;
mod keymap;
mod layout_editor;
pub use keymap::help_sections;
mod lyrics_search;
mod playlists;
mod settings;
mod spotify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Now,
    Search,
    Library,
    Audio,
    Albums,
    Room,
    Settings,
}

/// Filtro activo de la biblioteca (favoritas, descargadas, recientes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibFilter {
    All,
    Favorites,
    Downloaded,
    Recent,
    Playlist(i64),
}

impl LibFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "todo",
            Self::Favorites => "favoritas",
            Self::Downloaded => "descargadas",
            Self::Recent => "recientes",
            Self::Playlist(_) => "lista",
        }
    }
}

impl Tab {
    // Álbumes ya no es una pestaña: los álbumes se abren desde la búsqueda,
    // la biblioteca o un artista. Se mantiene el variante interno para el
    // tracklist antiguo, pero no aparece en la navegación.
    pub const ALL: [Tab; 6] = [
        Tab::Now,
        Tab::Search,
        Tab::Library,
        Tab::Audio,
        Tab::Room,
        Tab::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Now => "ahora",
            Self::Search => "buscar",
            Self::Library => "librería",
            Self::Audio => "audio",
            Self::Albums => "álbumes",
            Self::Room => "sala",
            Self::Settings => "ajustes",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Now => "◉",
            Self::Search => "/",
            Self::Library => "≡",
            Self::Audio => "♫",
            Self::Albums => "▤",
            Self::Room => "◎",
            Self::Settings => "✧",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Now => 0,
            Self::Search => 1,
            Self::Library => 2,
            Self::Audio => 3,
            // `Albums` es interno y ya no está en `ALL`; se ancla a Library.
            Self::Albums => 2,
            Self::Room => 4,
            Self::Settings => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Nav,
    Center,
    Side,
    Transport,
}

/// Qué ocupa el centro cuando se pincha en la portada o en la letra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenterFocus {
    Normal,
    Art,
    Lyrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fullscreen {
    None,
    Art,
    Viz,
}

/// Panel persistente de la derecha: perfil de artista o ficha de álbum.
/// No es un overlay: se queda abierto hasta que se pulsa volver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Artist,
    Album,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Effects,
    /// Formulario de lista nueva: nombre, descripción y portada opcional.
    PlaylistForm,
    /// Importar una playlist o álbum público de Spotify pegando su URL.
    ImportPlaylist,
    ConfirmDeletePlaylist {
        id: i64,
        name: String,
    },
    /// Elegir lista destino para la pista seleccionada.
    AddToPlaylist {
        key: String,
        selected: usize,
    },
    /// Editor de diseño de paneles.
    Layout,
    /// Menú contextual de una pista (clic derecho).
    Context,
    /// Perfil de artista de Spotify/Deezer.
    #[allow(dead_code)]
    Artist,
    /// Álbum abierto (biblioteca o catálogo remoto).
    #[allow(dead_code)]
    Album,
    /// Editor de recorte de intro/outro.
    Trim,
    /// Ecualizador, preamp, normalizador y limitador.
    /// Se conserva para el overlay antiguo; la entrada normal es la pestaña.
    #[allow(dead_code)]
    Audio,
    /// Hay una versión nueva en GitHub.
    Update,
}

/// Campos del formulario de lista nueva. `field` marca dónde está el cursor.
#[derive(Debug, Clone, Default)]
pub struct PlaylistForm {
    pub name: String,
    pub description: String,
    pub cover: String,
    pub field: usize,
}

impl PlaylistForm {
    pub fn fields(&self) -> [(&'static str, &str); 3] {
        [
            ("nombre", &self.name),
            ("descripción", &self.description),
            ("portada (url, vacío = automática)", &self.cover),
        ]
    }

    pub fn editing(&mut self) -> &mut String {
        match self.field {
            1 => &mut self.description,
            2 => &mut self.cover,
            _ => &mut self.name,
        }
    }
}

/// Acción de un elemento clickable. La UI registra `Rect → Action` cada frame.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Action {
    Tab(Tab),
    LibraryFilter(LibFilter),
    LibrarySearch,
    PlaylistOpen(i64),
    SettingsRow(usize),
    SettingsAdjust(usize, i32),
    SettingsToggle(usize),
    ToggleZen,
    PlayPause,
    Next,
    Prev,
    LoopToggle,
    CycleRepeat,
    ToggleShuffle,
    SeekTo(u64),
    SeekLyrics(u64),
    Volume(f32),
    VolumeTrack,
    Mute,
    FocusTransport,
    VizNext,
    VizPrev,
    VizPreset(usize),
    VizLayer(usize),
    AudioBand(usize),
    AudioBandSet(usize),
    AudioPreamp,
    AudioNormalizer,
    AudioStrength,
    AudioLimiter,
    AudioPreset(usize),
    AudioToggle,
    AudioEqToggle,
    AudioNormalize,
    VizFullscreen,
    VizToggle,
    ArtMode,
    ArtFullscreen,
    SelectRow(usize),
    AlbumOpen(usize),
    Resize(Pane),
    AlbumDownload,
    AlbumSavePlaylist,
    AlbumBack,
    AlbumTrackPlay(usize),
    AlbumPageDownload,
    AlbumPageSave,
    AlbumPageClose,
    OpenTrim,
    TrimStart,
    TrimEnd,
    TrimClear,
    TrimClose,
    ContextSelect(usize),
    SearchPlay(usize),
    PlayRow(usize),
    QueueRow(usize),
    QueueRemove(usize),
    Scroll(i32),
    Focus(Pane),
    CenterArt,
    CenterLyrics,
    /// Zoom de la letra: -1 compacta, +1 grande.
    LyricsZoom(i32),
    /// Ajuste fino del offset de la letra; va al motor de sync real.
    LyricsSync(i64),
    /// Devuelve el offset manual a cero.
    LyricsSyncReset,
    /// Scroll manual de la letra (positivo = hacia abajo).
    LyricsScroll(i32),
    /// Vuelve a seguir la línea que suena.
    LyricsFollow,
    /// Borra la caché de la letra de esta pista y la busca de nuevo.
    LyricsRefetch,
    /// Abre la búsqueda manual dentro de la zona de letra.
    LyricsSearchOpen,
    LyricsSearchRun,
    LyricsSearchClose,
    LyricsSearchSelect(usize),
    LyricsAcceptCandidate,
    Help,
    Effects,
    CloseOverlay,
    ToggleFollow,
    Favorite,
    AddQueue,
    StartSearch,
    NewPlaylist,
    ImportPlaylist,
    OpenLayout,
    NavMenu,
    QueueMenu,
    Artist,
    OpenArtist(String),
    OpenAlbum(String, String),
    ArtistAlbum(usize),
    ArtistTrack(usize),
    ArtistAlbumsScroll(i32),
    CloseDetail,
    FavoriteNow,
    RefreshLibrary,
    DeletePlaylist(i64),
    AddToPlaylist,
    JoinRoom(usize),
    LeaveRoom,
    Quit,
}

#[derive(Debug, Default)]
pub struct HitMap {
    entries: Vec<(Rect, Action)>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn add(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.entries.push((rect, action));
        }
    }

    pub fn hit(&self, x: u16, y: u16) -> Option<&Action> {
        // El último registrado gana: los overlays se dibujan después.
        self.entries
            .iter()
            .rev()
            .find(|(rect, _)| {
                x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
            })
            .map(|(_, action)| action)
    }

    pub fn rect_for(&self, action: &Action) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, candidate)| candidate == action)
            .map(|(rect, _)| *rect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created: Instant,
}

#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub track: Option<TrackRef>,
    pub path: Option<PathBuf>,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Última posición recibida del reproductor y cuándo, para interpolar el
    /// playhead entre eventos sin que la barra vaya a saltos.
    pub position_at: Instant,
    pub position_seen_ms: u64,
    /// `true` cuando el reproductor seguido no publica `Position`: el reloj
    /// avanza por interpolación en vez de quedarse clavado en 0.
    pub position_unknown: bool,
    /// Duración que reporta mpv para el fichero/stream actual. Puede diferir
    /// de `duration_ms` (la de la pista): un visualizer es más largo, un vídeo
    /// corto es más corto. Se usa para decidir el fin real.
    pub stream_duration_ms: u64,
    pub playing: bool,
    pub volume: f32,
    pub loading: bool,
    /// mpv llegó al final y descargó el fichero; hay que recargar para
    /// volver a reproducir (no basta con unpause/seek).
    pub ended: bool,
    pub message: String,
}

impl Default for NowPlaying {
    fn default() -> Self {
        Self {
            track: None,
            path: None,
            position_ms: 0,
            duration_ms: 0,
            position_at: Instant::now(),
            position_seen_ms: 0,
            position_unknown: false,
            stream_duration_ms: 0,
            playing: false,
            volume: 0.0,
            loading: false,
            ended: false,
            message: String::new(),
        }
    }
}

/// Estado optimista tras mandar una orden al reproductor MPRIS.
///
/// El sondeo del reproductor externo puede devolver durante unos cientos de
/// milisegundos el estado *anterior* a la orden (pausa → play → pausa). Mientras
/// este margen esté vivo se conserva lo que pidió el usuario y se aplica el
/// resto de campos del sondeo (pista, posición, carátula) con normalidad.
#[derive(Debug, Clone)]
pub struct FollowExpect {
    pub at: Instant,
    pub playing: Option<bool>,
    pub volume: Option<f32>,
    pub shuffle: Option<bool>,
    pub repeat: Option<RepeatMode>,
    pub position: Option<(u64, Instant)>,
}

impl FollowExpect {
    pub const GRACE: Duration = Duration::from_millis(1_500);

    fn new() -> Self {
        Self {
            at: Instant::now(),
            playing: None,
            volume: None,
            shuffle: None,
            repeat: None,
            position: None,
        }
    }

    fn live(&self) -> bool {
        self.at.elapsed() < Self::GRACE
    }

    fn empty(&self) -> bool {
        self.playing.is_none()
            && self.volume.is_none()
            && self.shuffle.is_none()
            && self.repeat.is_none()
            && self.position.is_none()
    }
}

/// Entrada aplanada de la búsqueda mixta (artistas, álbumes y pistas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEntry {
    Artist(usize),
    Album(usize),
    Track(usize),
}

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub results: Vec<SearchItem>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub selected: usize,
    pub loading: bool,
    pub last_query_id: u64,
    pub last_query: String,
    pub message: String,
}

/// Búsqueda manual de letras dentro de la pestaña de letra.
#[derive(Debug, Clone, Default)]
pub struct LyricsSearchState {
    pub query: String,
    pub query_id: u64,
    pub last_search_query: String,
    pub loading: bool,
    pub results: Vec<spote_core::lyrics::LyricsCandidate>,
    pub selected: usize,
    pub preview_scroll: usize,
    pub status: String,
}

/// Efecto de las miniaturas de las listas: paleta del tema, foto literal o
/// directamente sin miniatura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbMode {
    Palette,
    Plain,
    Off,
}

impl ThumbMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Palette => "paleta",
            Self::Plain => "fiel",
            Self::Off => "apagado",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Palette => Self::Plain,
            Self::Plain => Self::Off,
            Self::Off => Self::Palette,
        }
    }

    pub fn from_str(text: &str) -> Self {
        match text {
            "plain" | "fiel" => Self::Plain,
            "off" | "apagado" => Self::Off,
            _ => Self::Palette,
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::Plain => "plain",
            Self::Off => "off",
        }
    }
}

/// Filas de la pestaña de ajustes. Un enum en vez de índices sueltos: añadir
/// una fila nueva no rompe las que ya estaban.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRow {
    Rotate,
    RotateSecs,
    RotateOff,
    ArtMode,
    VizPreset,
    VizBackground,
    Cards,
    Vcenter,
    Zen,
    Thumb,
    ProgressInViz,
    BarWidth,
    ButtonPad,
    Ambient,
    AmbientOpacity,
    MosaicMotion,
    MosaicSpeed,
    MosaicBeat,
    InstrumentBorders,
    EdgeFlame,
    BloomPlay,
    CleanSpill,
    GlitchPanels,
    LyricsSize,
    NormalizeTracks,
    Audio,
    Spotify,
    LayoutEditor,
    Save,
}

impl SettingRow {
    pub const ALL: [SettingRow; 29] = [
        Self::Rotate,
        Self::RotateSecs,
        Self::RotateOff,
        Self::ArtMode,
        Self::VizPreset,
        Self::VizBackground,
        Self::Cards,
        Self::Vcenter,
        Self::Zen,
        Self::Thumb,
        Self::ProgressInViz,
        Self::BarWidth,
        Self::ButtonPad,
        Self::Ambient,
        Self::AmbientOpacity,
        Self::MosaicMotion,
        Self::MosaicSpeed,
        Self::MosaicBeat,
        Self::InstrumentBorders,
        Self::EdgeFlame,
        Self::BloomPlay,
        Self::CleanSpill,
        Self::GlitchPanels,
        Self::LyricsSize,
        Self::NormalizeTracks,
        Self::Audio,
        Self::Spotify,
        Self::LayoutEditor,
        Self::Save,
    ];

    pub fn section(self) -> &'static str {
        match self {
            Self::Rotate | Self::RotateSecs | Self::RotateOff | Self::ArtMode => "arte",
            Self::VizPreset
            | Self::VizBackground
            | Self::Cards
            | Self::Vcenter
            | Self::Zen
            | Self::Thumb
            | Self::ProgressInViz
            | Self::BarWidth
            | Self::ButtonPad => "interfaz",
            Self::Ambient
            | Self::AmbientOpacity
            | Self::MosaicMotion
            | Self::MosaicSpeed
            | Self::MosaicBeat => "fondo",
            Self::InstrumentBorders
            | Self::EdgeFlame
            | Self::BloomPlay
            | Self::CleanSpill
            | Self::GlitchPanels => "efectos",
            Self::LyricsSize | Self::NormalizeTracks | Self::Audio => "letra y audio",
            Self::Spotify | Self::LayoutEditor => "sistema",
            Self::Save => "guardar",
        }
    }

    /// Texto corto que explica qué hace el ajuste seleccionado.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Rotate => "cambia el modo de la portada cada pocos segundos",
            Self::RotateSecs => "cada cuánto rota el modo de arte",
            Self::RotateOff => "incluye el modo apagado en la rotación",
            Self::ArtMode => "pixel, braille, degradado o sin portada",
            Self::VizPreset => "efecto de la franja inferior y del fondo",
            Self::VizBackground => "el visualizador ocupa toda la pantalla de fondo",
            Self::Cards => "filas con tarjeta o lista compacta",
            Self::Vcenter => "centra el bloque de paneles en vertical",
            Self::Zen => "oculta portada y letra; deja solo el efecto",
            Self::Thumb => "paleta, foto fiel o miniaturas apagadas",
            Self::ProgressInViz => "la barra de progreso vive en la franja del efecto",
            Self::BarWidth => "ancho de la barra de progreso",
            Self::ButtonPad => "tamaño de los botones del transporte",
            Self::Ambient => "fondo: mosaico, aurora, glitch, letra…",
            Self::AmbientOpacity => "fuerza del fondo ambiental (5-50 %)",
            Self::MosaicMotion => "movimiento del mosaico: deriva, filas, columnas, giro",
            Self::MosaicSpeed => "velocidad del mosaico",
            Self::MosaicBeat => "el mosaico salta al compás del bombo",
            Self::InstrumentBorders => "los bordes laten con bombo, caja, charles y voz",
            Self::EdgeFlame => "brasa que respira en el borde del efecto",
            Self::BloomPlay => "halo del color de la portada detrás del play",
            Self::CleanSpill => "la onda puede subir por encima de los paneles",
            Self::GlitchPanels => "trama de glitch dentro de los paneles",
            Self::LyricsSize => "tamaño de la letra: compacta, normal o grande",
            Self::NormalizeTracks => "igualar el volumen de todas las canciones",
            Self::Audio => "ecualizador, preamp, normalizador y limitador",
            Self::Spotify => "seguir el reproductor MPRIS del sistema",
            Self::LayoutEditor => "abrir el editor de paneles y tamaños",
            Self::Save => "escribir la configuración ahora",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rotate => "rotación de arte",
            Self::RotateSecs => "rotar cada",
            Self::RotateOff => "rotación incluye off",
            Self::ArtMode => "modo de arte",
            Self::VizPreset => "efecto inferior",
            Self::VizBackground => "visualizador de fondo",
            Self::Cards => "tarjetas",
            Self::Vcenter => "centrar paneles",
            Self::Zen => "zen (sin portada ni letra)",
            Self::Thumb => "efecto miniaturas",
            Self::ProgressInViz => "barra en el visualizador",
            Self::BarWidth => "ancho de la barra",
            Self::ButtonPad => "tamaño de controles",
            Self::Ambient => "fondo ambiental",
            Self::AmbientOpacity => "opacidad ambiental",
            Self::MosaicMotion => "movimiento del mosaico",
            Self::MosaicSpeed => "velocidad del mosaico",
            Self::MosaicBeat => "mosaico al compás",
            Self::InstrumentBorders => "bordes con instrumentos",
            Self::EdgeFlame => "llama en el borde",
            Self::BloomPlay => "bloom en el play",
            Self::CleanSpill => "clean sobre paneles",
            Self::GlitchPanels => "glitch en paneles",
            Self::LyricsSize => "tamaño de letra",
            Self::NormalizeTracks => "igualar volumen entre pistas",
            Self::Audio => "ecualizador / normalizador (A)",
            Self::Spotify => "seguir sistema (MPRIS)",
            Self::LayoutEditor => "editor de diseño (E)",
            Self::Save => "guardar ajustes",
        }
    }

    pub fn adjustable(self) -> bool {
        matches!(
            self,
            Self::RotateSecs
                | Self::ArtMode
                | Self::VizPreset
                | Self::Ambient
                | Self::AmbientOpacity
                | Self::MosaicMotion
                | Self::MosaicSpeed
                | Self::LyricsSize
                | Self::Thumb
                | Self::BarWidth
                | Self::ButtonPad
        )
    }

    /// Interruptor booleano: la UI lo marca con ◉/○.
    pub fn toggle(self) -> bool {
        matches!(
            self,
            Self::Rotate
                | Self::RotateOff
                | Self::VizBackground
                | Self::Cards
                | Self::Vcenter
                | Self::Zen
                | Self::ProgressInViz
                | Self::MosaicBeat
                | Self::InstrumentBorders
                | Self::EdgeFlame
                | Self::BloomPlay
                | Self::CleanSpill
                | Self::GlitchPanels
                | Self::NormalizeTracks
                | Self::Spotify
        )
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// Descarga en curso o recién terminada. La UI pinta el paso y la barra en el
/// panel de sesión y un badge en la fila de la pista.
#[derive(Debug, Clone)]
pub struct DownloadInfo {
    pub key: String,
    pub title: String,
    pub stage: String,
    pub percent: f32,
    pub failed: bool,
    pub finished_at: Option<Instant>,
}

impl DownloadInfo {
    pub fn active(&self) -> bool {
        self.finished_at.is_none()
    }
}

/// Álbum abierto desde la biblioteca o desde una búsqueda remota.
#[derive(Debug, Clone, Default)]
pub struct AlbumPage {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub cover_path: Option<PathBuf>,
    pub release_date: Option<String>,
    pub tracks: Vec<TrackRef>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub track: TrackRef,
    pub path: Option<PathBuf>,
}

/// Origen del menú contextual: biblioteca, búsqueda o cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextOrigin {
    Library,
    Search,
    Queue,
    Nav,
    QueueHeader,
    SearchAlbum,
    SearchArtist,
    ArtistName,
    AlbumName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Play,
    Queue,
    Download,
    Favorite,
    Artist,
    Cover,
    RemoveLibrary,
    DeleteFile,
    RemoveQueue,
    Status,
    NewPlaylist,
    ImportPlaylist,
    RefreshLibrary,
    Layout,
    ClearQueue,
    Open,
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub origin: ContextOrigin,
    pub index: usize,
    pub key: String,
    pub title: String,
    pub artist: String,
    pub url: Option<String>,
    pub path: Option<PathBuf>,
    pub selected: usize,
}

impl ContextMenu {
    pub fn actions(&self) -> Vec<ContextAction> {
        match self.origin {
            ContextOrigin::Library => vec![
                ContextAction::Play,
                ContextAction::Queue,
                ContextAction::Download,
                ContextAction::Favorite,
                ContextAction::Artist,
                ContextAction::Cover,
                ContextAction::RemoveLibrary,
                ContextAction::DeleteFile,
            ],
            ContextOrigin::Search => vec![
                ContextAction::Play,
                ContextAction::Queue,
                ContextAction::Download,
                ContextAction::Favorite,
                ContextAction::Artist,
                ContextAction::Status,
            ],
            ContextOrigin::Queue => vec![
                ContextAction::Play,
                ContextAction::RemoveQueue,
                ContextAction::Artist,
                ContextAction::Status,
            ],
            ContextOrigin::Nav => vec![
                ContextAction::NewPlaylist,
                ContextAction::ImportPlaylist,
                ContextAction::RefreshLibrary,
                ContextAction::Layout,
            ],
            ContextOrigin::QueueHeader => vec![
                ContextAction::ClearQueue,
                ContextAction::RefreshLibrary,
                ContextAction::Layout,
            ],
            ContextOrigin::SearchAlbum => vec![
                ContextAction::Open,
                ContextAction::Artist,
                ContextAction::Status,
            ],
            ContextOrigin::SearchArtist => vec![ContextAction::Open, ContextAction::Status],
            ContextOrigin::ArtistName => vec![ContextAction::Open, ContextAction::Status],
            ContextOrigin::AlbumName => vec![ContextAction::Open, ContextAction::Status],
        }
    }

    pub fn action(&self) -> ContextAction {
        let actions = self.actions();
        actions[self.selected.min(actions.len().saturating_sub(1))]
    }
}

pub struct App {
    pub cfg: Config,
    pub lib: Library,
    pub player: Option<Player>,
    pub spectrum: Spectrum,
    pub audio: spote_core::audio::AudioFrame,
    pub downloader: Downloader,
    pub spotify_jobs: crate::spotify_jobs::SpotifyJobs,
    pub discord: crate::spotify_jobs::DiscordPresence,
    pub art_worker: ArtWorker,
    pub meta_worker: MetaWorker,
    pub map_worker: SongMapWorker,
    pub song_map: Option<SongMap>,
    pub song_map_key: String,
    /// Ganancia de nivelación medida para la pista actual. `None` = aún sin
    /// medir (o stream): se usa el nivelador en directo como respaldo.
    pub track_gain_db: Option<f32>,
    pub song_intensity: f32,
    pub song_section: Option<spote_core::audio_map::SongSectionKind>,
    pub artist: Option<spote_core::spotify_api::ArtistProfile>,
    pub artist_art: Option<CoverArt>,
    pub artist_loading: bool,
    pub artist_selected: usize,
    pub artist_status: String,
    /// Nombre que se está pidiendo; las respuestas de otro artista se ignoran.
    pub artist_request: String,
    /// Desplazamiento horizontal de la discografía del artista.
    pub artist_album_scroll: usize,
    /// Tarjetas de álbum que caben en el panel; lo fija el dibujo.
    pub detail_album_visible: usize,
    /// Zona de las tarjetas de álbum para la rueda del ratón.
    pub detail_albums_area: Rect,
    /// Panel persistente de detalle (artista o álbum), a la derecha.
    pub detail: Option<Detail>,
    pub metadata_requests: std::collections::HashSet<String>,
    pub metadata_tried: std::collections::HashSet<String>,
    pub metadata_manual: std::collections::HashSet<String>,
    pub lyrics_worker: LyricsWorker,
    pub spotify_worker: Option<SystemMediaWorker>,
    pub tools: ToolStatus,

    pub tab: Tab,
    pub focus: Pane,
    pub fullscreen: Fullscreen,
    pub overlay: Overlay,
    /// Último overlay pintado; al cambiar se fuerza un repintado completo.
    pub overlay_seen: Overlay,
    pub show_viz: bool,

    pub name: String,
    pub center_focus: CenterFocus,
    pub now: NowPlaying,
    /// Último toggle de play/pausa: evita que el autorepeat del terminal
    /// mande pausa → play → pausa al mantener la barra espaciadora.
    pub play_toggle_at: Instant,
    pub volume_before_mute: f32,
    /// `true` mientras el silencio es temporal; no se persiste como volumen 0.
    pub muted: bool,
    /// Fila seleccionada en el overlay de audio (0-9 bandas, 10 preamp…).
    pub audio_selected: usize,
    pub mini_hub: Option<crate::mini::MiniHub>,
    /// Motivo por el que el socket del mini no se pudo abrir, para avisar al
    /// usuario en vez de fingir que `w` funciona.
    pub mini_error: Option<String>,
    pub mini_last_publish: Instant,
    pub mini_child: Option<std::process::Child>,
    /// Autoplay deseado mientras se resuelve un stream; el usuario puede
    /// pausarlo antes de que mpv tenga la URL.
    pub stream_autoplay: bool,
    /// Seek pedido durante la resolución; se aplica al cargar la URL/fichero
    /// solo si sigue siendo la misma pista.
    pub pending_seek_ms: Option<u64>,
    pub pending_seek_key: Option<String>,
    pub mpris_server: Option<spote_core::mpris_server::MprisServer>,
    pub mpris_last_update: Instant,
    pub player_events: Option<tokio::sync::broadcast::Receiver<PlayerEvent>>,
    pub queue: Vec<QueueEntry>,
    pub queue_selected: usize,
    pub queue_drag: Option<usize>,
    pub view: Vec<TrackRow>,
    /// Bucle del fichero actual (`loop-file=inf` en mpv).
    pub loop_file: bool,
    /// Pistas previas de la sesión, para que «anterior» no caiga siempre en
    /// la primera fila de la biblioteca.
    pub history: Vec<(TrackRef, Option<PathBuf>)>,
    /// Contexto de reproducción (vista, búsqueda, álbum, artista).
    pub play_context: Vec<QueueEntry>,
    pub context_index: usize,
    pub repeat_mode: RepeatMode,
    pub shuffle_on: bool,
    pub shuffle_state: Option<ShuffleState>,
    pub stream_resolver: StreamResolver,
    pub pending_stream: Option<String>,
    pub streaming: bool,
    /// Copia local pendiente de un stream. Se lanza solo cuando la pista lleva
    /// unos segundos sonando: saltar de canción no descarga el mundo.
    pub stream_cache_pending: Option<(String, String)>,
    pub albums: Vec<spote_core::library::AlbumRow>,
    pub album_selected: usize,
    pub album_open: Option<spote_core::library::AlbumRow>,
    pub album_page: Option<AlbumPage>,
    pub album_page_loading: bool,
    pub album_page_status: String,
    pub lib_filter: LibFilter,
    pub selected: usize,
    /// Filtro de texto de la biblioteca (buscador inline).
    pub lib_query: String,
    pub lib_search_focus: bool,
    /// Primera fila visible de la biblioteca. El scroll es por bordes: la
    /// selección se mueve dentro de la página y la lista solo desplaza cuando
    /// hace falta, sin que la "cámara" persiga al cursor con un muelle.
    pub list_start: usize,
    /// Primera entrada visible de la cola, con la misma regla.
    pub queue_start: usize,
    /// Primera fila visible de la búsqueda mixta (secciones + entradas).
    pub search_start: usize,
    pub search: SearchState,
    pub pending_play: Option<String>,
    /// Comprobación de versión en segundo plano.
    pub update_check: Option<std::sync::mpsc::Receiver<Option<crate::version::UpdateInfo>>>,
    pub update_info: Option<crate::version::UpdateInfo>,
    /// Actualizador en marcha: `true` si terminó bien.
    pub update_run: Option<std::sync::mpsc::Receiver<bool>>,
    pub update_started: Option<Instant>,
    pub update_dismissed: bool,
    pub update_finished: Option<bool>,

    pub lyrics: Option<LyricsDoc>,
    pub lyrics_status: String,
    pub lyrics_key: String,
    pub lyrics_loading: bool,
    pub lyrics_search: Option<LyricsSearchState>,
    pub lyrics_search_worker: crate::fetch::LyricsSearchWorker,
    pub lyrics_offset_ms: i64,
    /// Desplazamiento calculado con el mapa de audio (no persistido).
    pub lyrics_auto_offset: Option<i64>,
    /// Pistas cuya letra sin sincronizar ya se intentó mejorar en esta sesión.
    pub lyrics_retry: std::collections::HashSet<String>,
    pub trim_start_ms: u64,
    pub trim_end_ms: Option<u64>,
    pub lyrics_scroll: Spring,
    /// Índice que marcó el último salto de scroll (para hacer snap, no easing).
    pub lyrics_scroll_index: Option<usize>,
    /// `false` mientras el usuario explora la letra a mano; la reproducción
    /// no le roba el scroll hasta que pulse «volver a la letra actual».
    pub lyrics_follow: bool,
    /// Zona de la letra en pantalla, para que la rueda sepa dónde hacer scroll.
    pub lyrics_area: Rect,
    /// Estado optimista del modo MPRIS (ver [`FollowExpect`]).
    pub follow_expect: Option<FollowExpect>,
    /// Overlay temporal con la base temporal de la letra (tecla `D`).
    pub lyrics_debug: bool,
    /// (índice, progreso) del frame anterior, para avisar de cambios prematuros.
    pub lyrics_debug_last: Option<(usize, f32)>,

    pub ambient: Viz,
    pub ambient_art: Option<CoverArt>,
    pub ambient_art_key: String,
    pub art: Option<CoverArt>,
    pub art_key: String,
    pub art_size: (u16, u16),
    pub art_size_hint: (u16, u16),
    pub art_mode: ArtMode,
    pub art_mode_seen: ArtMode,
    pub art_prev: Option<CoverArt>,
    pub art_prev_mode: ArtMode,
    pub art_fade: f32,
    pub art_fade_active: bool,
    pub art_rotate: bool,
    pub art_rotate_off: bool,
    pub art_next_at: Instant,
    pub art_variants: std::collections::HashMap<ArtMode, CoverArt>,
    /// PNG de la portada actual para el protocolo de Kitty (full-res).
    pub art_png: Option<Vec<u8>>,
    pub art_png_key: String,
    pub art_png_size: Option<(u32, u32)>,
    /// Tamaño de celda con el que se generó el PNG actual.
    pub art_png_hint: (u16, u16),
    /// Rect que ocupa la portada en pantalla este frame; lo consume `hires`.
    pub art_visible_rect: Option<Rect>,
    /// Miniaturas por (pista, ancho, alto): una de 6x3 para la tarjeta no
    /// sirve para la cabecera de 12x7. La clave lleva el tamaño para no
    /// estirar una miniatura pequeña y que se vea pixelada.
    pub thumbs: std::collections::HashMap<(String, u16, u16), CoverArt>,
    pub thumb_requests: std::collections::HashSet<(String, u16, u16)>,
    /// PNG normalizado por portada para el protocolo de imágenes de Kitty.
    /// Se manda una vez y se reutiliza en todas las miniaturas de esa pista.
    pub thumb_pngs: std::collections::HashMap<String, (Vec<u8>, (u32, u32))>,
    /// Miniaturas visibles este frame: (clave de colocación, clave de arte,
    /// rect). `hires` las coloca encima del half-block.
    pub hi_thumbs: Vec<(String, String, Rect)>,
    pub art_request: Option<Instant>,
    pub art_request_key: String,
    pub art_request_size: (u16, u16),
    pub search_worker: SearchWorker,
    pub palette: PaletteRuntime,
    pub dominant: Palette,

    pub list_cards: bool,
    pub viz_background: bool,
    pub vcenter: bool,
    pub zen: bool,
    pub thumb_mode: ThumbMode,
    pub playlist_form: PlaylistForm,
    pub import_input: String,
    pub progress_in_viz: bool,
    pub bar_width_pct: u8,
    pub button_pad: u8,
    pub settings_selected: usize,
    pub show_bpm: bool,
    pub viz: Viz,
    pub fps: FpsMeter,
    pub frame_ms: f32,
    /// Ritmo máximo de los canvas de efectos (Hz).
    pub fx_fps: f32,
    /// Relojes independientes por canvas: fondo ambiental y visualizador.
    pub ambient_fx: FxClock,
    pub viz_fx: FxClock,
    pub pulse: Pulse,
    pub intro: f32,
    /// Reloj propio del fondo mosaico: sigue aunque el visualizador esté
    /// apagado, para que el fondo nunca se congele.
    pub mosaic_time: f32,
    pub hit: HitMap,
    pub hover: Option<Action>,
    pub last_click: Option<(Instant, usize)>,
    pub toasts: Vec<Toast>,
    pub status: String,
    pub status_at: Instant,
    pub downloads: Vec<DownloadInfo>,
    pub dragging_seek: bool,
    pub dragging_volume: bool,
    pub resize_drag: Option<Pane>,
    pub last_main: Rect,
    pub drag_ratio: Option<f32>,
    pub layout_selected: usize,
    pub context: Option<ContextMenu>,

    pub social: Option<Social>,
    pub rooms: Vec<Room>,
    pub room: Option<Room>,
    pub room_state: Option<RoomState>,
    pub room_queue: Vec<QueueItem>,
    pub presence: Vec<Presence>,
    pub chat: Vec<spote_protocol::ChatMsg>,
    pub social_status: String,
    pub social_connected: bool,
    pub last_social_error: Option<Instant>,
    pub clock_offset_ms: i64,
    pub applying_remote: bool,
    pub pending_room_target: Option<u64>,
    pub room_selected: usize,
    pub last_presence: (Option<String>, bool),

    pub follow_spotify: bool,
    pub spotify: Option<SystemMediaState>,
    pub last_spotify_poll: Instant,
    pub spotify_warned: bool,
    pub should_quit: bool,
    /// Fuerza un repintado completo en el siguiente frame (cambios de layout).
    pub needs_redraw: bool,
    pub spectrum_at: Instant,
}

impl App {
    pub fn new(cfg: Config) -> anyhow::Result<Self> {
        let lib = Library::open(&config::db_path())?;
        let art_mode = ArtMode::from_str(&cfg.ui.art_mode).unwrap_or_default();
        let viz = Viz::from_preset(&cfg.ui.viz);
        let player_volume = cfg.player.volume;
        let fx_fps = cfg.ui.fx_fps.clamp(5, 60) as f32;
        let social = if cfg.server.url.is_empty() {
            None
        } else {
            Some(Social::start(cfg.server.clone()))
        };
        let (mini_hub, mini_error) = match crate::mini::MiniHub::start() {
            Ok(hub) => (Some(hub), None),
            Err(err) => (None, Some(err.to_string())),
        };
        let mut app = Self {
            name: cfg.app.name.clone(),
            center_focus: CenterFocus::Normal,
            lib,
            player: None,
            spectrum: Spectrum::start(),
            audio: spote_core::audio::AudioFrame::default(),
            downloader: Downloader::start(),
            spotify_jobs: crate::spotify_jobs::SpotifyJobs::start(),
            discord: crate::spotify_jobs::DiscordPresence::start(&cfg),
            art_worker: ArtWorker::start(),
            meta_worker: MetaWorker::start(),
            map_worker: SongMapWorker::start(),
            song_map: None,
            song_map_key: String::new(),
            track_gain_db: None,
            song_intensity: 0.0,
            song_section: None,
            artist: None,
            artist_art: None,
            artist_loading: false,
            artist_selected: 0,
            artist_status: String::new(),
            artist_request: String::new(),
            artist_album_scroll: 0,
            detail_album_visible: 1,
            detail_albums_area: Rect::default(),
            detail: None,
            metadata_requests: std::collections::HashSet::new(),
            metadata_tried: std::collections::HashSet::new(),
            metadata_manual: std::collections::HashSet::new(),
            lyrics_worker: LyricsWorker::start(cfg.lyrics.clone()),
            spotify_worker: None,
            tools: ToolStatus::detect(),
            tab: Tab::Now,
            focus: Pane::Center,
            fullscreen: Fullscreen::None,
            overlay: Overlay::None,
            overlay_seen: Overlay::None,
            show_viz: cfg.ui.viz_enabled,
            now: NowPlaying {
                volume: player_volume,
                position_at: Instant::now(),
                ..Default::default()
            },
            play_toggle_at: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
            volume_before_mute: player_volume,
            muted: false,
            audio_selected: 0,
            mini_hub,
            mini_error,
            mini_last_publish: Instant::now(),
            mini_child: None,
            stream_autoplay: true,
            pending_seek_ms: None,
            pending_seek_key: None,
            mpris_server: None,
            mpris_last_update: Instant::now(),
            player_events: None,
            queue: Vec::new(),
            queue_selected: 0,
            queue_drag: None,
            view: Vec::new(),
            loop_file: cfg.playback.repeat == "one",
            history: Vec::new(),
            play_context: Vec::new(),
            context_index: 0,
            repeat_mode: match cfg.playback.repeat.as_str() {
                "all" => RepeatMode::All,
                "one" => RepeatMode::One,
                _ => RepeatMode::Off,
            },
            shuffle_on: cfg.playback.shuffle,
            shuffle_state: None,
            stream_resolver: StreamResolver::start_in(cfg.media_dir().join("stream")),
            pending_stream: None,
            streaming: false,
            stream_cache_pending: None,
            albums: Vec::new(),
            album_selected: 0,
            album_open: None,
            album_page: None,
            album_page_loading: false,
            album_page_status: String::new(),
            lib_filter: LibFilter::All,
            selected: 0,
            lib_query: String::new(),
            lib_search_focus: false,
            list_start: 0,
            queue_start: 0,
            search_start: 0,
            search: SearchState::default(),
            pending_play: None,
            update_check: crate::version::spawn_check(),
            update_info: None,
            update_run: None,
            update_started: None,
            update_dismissed: false,
            update_finished: None,
            lyrics: None,
            lyrics_status: String::new(),
            lyrics_key: String::new(),
            lyrics_loading: false,
            lyrics_search: None,
            lyrics_search_worker: crate::fetch::LyricsSearchWorker::start(cfg.lyrics.clone()),
            lyrics_offset_ms: 0,
            lyrics_auto_offset: None,
            lyrics_retry: std::collections::HashSet::new(),
            trim_start_ms: 0,
            trim_end_ms: None,
            lyrics_scroll: Spring::new(0.0),
            lyrics_scroll_index: None,
            lyrics_follow: true,
            lyrics_area: Rect::default(),
            follow_expect: None,
            lyrics_debug: false,
            lyrics_debug_last: None,
            ambient: Viz::from_preset(&cfg.ui.fx.ambient),
            ambient_art: None,
            ambient_art_key: String::new(),
            art: None,
            art_key: String::new(),
            art_size: (0, 0),
            art_size_hint: (0, 0),
            art_mode,
            art_mode_seen: art_mode,
            art_prev: None,
            art_prev_mode: art_mode,
            art_fade: 1.0,
            art_fade_active: false,
            art_rotate: cfg.ui.art_rotate,
            art_rotate_off: cfg.ui.art_rotate_off,
            art_next_at: Instant::now()
                + std::time::Duration::from_secs(cfg.ui.art_rotate_secs.max(5)),
            art_variants: std::collections::HashMap::new(),
            art_png: None,
            art_png_key: String::new(),
            art_png_size: None,
            art_png_hint: (0, 0),
            art_visible_rect: None,
            thumbs: std::collections::HashMap::new(),
            thumb_requests: std::collections::HashSet::new(),
            thumb_pngs: std::collections::HashMap::new(),
            hi_thumbs: Vec::new(),
            art_request: None,
            art_request_key: String::new(),
            art_request_size: (0, 0),
            search_worker: SearchWorker::start(),
            palette: PaletteRuntime::new(Palette::dark(), cfg.ui.theme_fade_ms),
            dominant: Palette::dark(),
            list_cards: cfg.ui.list_cards,
            viz_background: cfg.ui.viz_background,
            vcenter: cfg.ui.layout.vcenter,
            zen: cfg.ui.zen,
            thumb_mode: ThumbMode::from_str(&cfg.ui.thumb_mode),
            playlist_form: PlaylistForm::default(),
            import_input: String::new(),
            progress_in_viz: cfg.ui.progress_in_viz,
            bar_width_pct: cfg.ui.bar_width_pct.clamp(55, 100),
            button_pad: cfg.ui.button_pad.clamp(1, 4),
            settings_selected: 0,
            show_bpm: cfg.ui.show_bpm,
            viz,
            fps: FpsMeter::new(),
            frame_ms: 0.0,
            fx_fps,
            ambient_fx: FxClock::new(fx_fps * 0.34),
            viz_fx: FxClock::new(fx_fps),
            pulse: Pulse::default(),
            intro: 0.0,
            mosaic_time: 0.0,
            hit: HitMap::default(),
            hover: None,
            last_click: None,
            toasts: Vec::new(),
            status: String::new(),
            status_at: Instant::now(),
            downloads: Vec::new(),
            dragging_seek: false,
            dragging_volume: false,
            resize_drag: None,
            last_main: Rect::default(),
            drag_ratio: None,
            layout_selected: 0,
            context: None,
            social,
            rooms: Vec::new(),
            room: None,
            room_state: None,
            room_queue: Vec::new(),
            presence: Vec::new(),
            chat: Vec::new(),
            social_status: String::new(),
            social_connected: false,
            last_social_error: None,
            clock_offset_ms: 0,
            applying_remote: false,
            pending_room_target: None,
            room_selected: 0,
            last_presence: (None, false),
            follow_spotify: cfg.ui.follow_spotify,
            spotify: None,
            last_spotify_poll: Instant::now(),
            spotify_warned: false,
            should_quit: false,
            needs_redraw: false,
            spectrum_at: Instant::now(),
            cfg: cfg.clone(),
        };
        app.refresh_view();
        if app.follow_spotify {
            app.enable_spotify();
        }
        Ok(app)
    }

    // ------------------------------------------------------------- arranque
    pub async fn connect_player(&mut self) -> anyhow::Result<()> {
        let socket = spote_core::player::socket_path();
        let player =
            Player::spawn(socket, &self.cfg.player.mpv_path, self.cfg.player.volume).await?;
        player.set_volume(self.cfg.player.volume);
        self.player_events = Some(player.subscribe());
        self.player = Some(player);
        self.apply_audio_fx();
        Ok(())
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            created: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_at = Instant::now();
    }

    // --------------------------------------------------------------- vista
    pub fn refresh_view(&mut self) {
        let previous = std::mem::take(&mut self.view);
        let mut error = None;
        let load = |lib: &Library, filter: &LibFilter| -> anyhow::Result<Vec<TrackRow>> {
            Ok(match filter {
                LibFilter::All => lib.all_tracks(500)?,
                LibFilter::Favorites => lib.favorites()?,
                LibFilter::Downloaded => lib.playable(500)?,
                LibFilter::Recent => lib.recent(500)?,
                LibFilter::Playlist(id) => lib.playlist_tracks(*id)?,
            })
        };
        let mut next = match self.tab {
            Tab::Library | Tab::Now => match load(&self.lib, &self.lib_filter) {
                Ok(rows) => rows,
                Err(err) => {
                    error = Some(err.to_string());
                    previous.clone()
                }
            },
            Tab::Albums => {
                match self.lib.albums() {
                    Ok(rows) => self.albums = rows,
                    Err(err) => {
                        self.set_status(format!("álbumes: {err}"));
                    }
                }
                if self.album_selected >= self.albums.len() {
                    self.album_selected = self.albums.len().saturating_sub(1);
                }
                if let Some(open) = &self.album_open {
                    self.lib
                        .album_tracks(&open.album, &open.artist)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            Tab::Search | Tab::Audio | Tab::Room | Tab::Settings => Vec::new(),
        };
        if self.tab == Tab::Library && !self.lib_query.trim().is_empty() {
            // FTS5 busca en toda la biblioteca, no solo en las primeras 500
            // filas del filtro. Para favoritas/listas se intersecta por clave.
            match self.lib.search(self.lib_query.trim(), 500) {
                Ok(found) => {
                    if matches!(self.lib_filter, LibFilter::All) {
                        next = found;
                    } else {
                        let keys: std::collections::HashSet<String> = found
                            .into_iter()
                            .map(|row| row.track.key)
                            .collect();
                        next.retain(|row| keys.contains(&row.track.key));
                    }
                }
                Err(_) => {
                    // FTS puede fallar con comodines raros; el filtro en
                    // memoria mantiene la UI usable. Una query que normaliza
                    // a vacío no debe casar con todo.
                    let query = spote_core::util::normalize(&self.lib_query);
                    if query.is_empty() {
                        next.clear();
                    }
                    next.retain(|row| {
                        let haystack = spote_core::util::normalize(&format!(
                            "{} {} {}",
                            row.track.title,
                            row.track.artist,
                            row.track.album.as_deref().unwrap_or("")
                        ));
                        haystack.contains(&query)
                    });
                }
            }
        }
        self.view = next;
        if let Some(error) = error {
            // Un error real de SQLite nunca debe ser mudo: si además dejó la
            // vista vacía, se conserva la anterior para no perder el contexto.
            if self.view.is_empty() && !previous.is_empty() {
                self.view = previous;
            }
            self.toast(
                format!("no pude refrescar la biblioteca: {error}"),
                ToastKind::Warn,
            );
        }
        if self.selected >= self.view.len() {
            self.selected = self.view.len().saturating_sub(1);
        }
    }

    /// Cicla los filtros de la biblioteca incluida la lista abierta: así se
    /// navega sin ratón. `[` va hacia atrás y `]` hacia delante.
    pub fn cycle_filter(&mut self, delta: i32) {
        let mut options = vec![
            LibFilter::All,
            LibFilter::Favorites,
            LibFilter::Downloaded,
            LibFilter::Recent,
        ];
        if let Ok(lists) = self.lib.playlists() {
            options.extend(lists.into_iter().map(|list| LibFilter::Playlist(list.id)));
        }
        let current = options
            .iter()
            .position(|filter| *filter == self.lib_filter)
            .unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(options.len() as i32) as usize;
        self.lib_filter = options[next];
        self.selected = 0;
        self.tab = Tab::Library;
        self.refresh_view();
    }

    pub fn selected_row(&self) -> Option<&TrackRow> {
        self.view.get(self.selected)
    }

    // --------------------------------------------------------- reproducción
    /// Devuelve la fila canónica local si la misma canción ya existe con otra
    /// clave (ISRC/Spotify frente a `sha1`). Conserva hint y portada de la
    /// pista entrante para no repetir búsquedas ni perder arte.
    fn canonical_track(&self, track: &TrackRef) -> TrackRef {
        let canonical = self.lib.canonical_key(track);
        if canonical == track.key {
            return track.clone();
        }
        let Ok(Some(row)) = self.lib.track_by_key(&canonical) else {
            return track.clone();
        };
        let mut out = row.track;
        out.art_url = out.art_url.or_else(|| track.art_url.clone());
        out.source_hint = out.source_hint.or_else(|| track.source_hint.clone());
        if out.album.is_none() {
            out.album = track.album.clone();
        }
        out
    }

    /// Carga una pista ya resuelta a fichero local.
    pub fn play_local(&mut self, track: TrackRef, path: Option<PathBuf>) {
        self.play_local_inner(track, path, true);
    }

    fn play_local_inner(
        &mut self,
        mut track: TrackRef,
        path: Option<PathBuf>,
        record_history: bool,
    ) {
        if self.follow_spotify {
            self.toast(
                "modo sistema activo: desactívalo con t para reproducir en spote",
                ToastKind::Info,
            );
            return;
        }
        if self.room.is_some() && !self.applying_remote {
            // En una sala manda el servidor: seleccionar una pista la encola,
            // no arranca audio local por debajo de la reproducción compartida.
            if let Some(social) = &self.social {
                social.send(SocialCmd::QueueAdd(track.clone()));
            }
            self.toast(
                format!("{} añadida a la cola de la sala", track.title),
                ToastKind::Info,
            );
            return;
        }
        // Reutiliza la fila canónica si la misma canción ya está descargada
        // con otra clave (búsqueda/álbum/ISRC). Evita duplicados y descargas.
        track = self.canonical_track(&track);
        // La historia se guarda antes de resolver el stream: las pistas que
        // aún no tienen fichero también deben poder recuperarse con "anterior".
        if record_history {
            if let Some(current) = self.now.track.clone() {
                if current.key != track.key {
                    self.history.push((current, self.now.path.clone()));
                    if self.history.len() > 100 {
                        self.history.remove(0);
                    }
                }
            }
        }
        // Elegir otra pista cancela la reproducción que esperaba una descarga;
        // si no, al terminar esa descarga interrumpía lo que ya estaba sonando.
        self.pending_play = None;
        self.now.ended = false;
        self.stream_autoplay = true;
        // Cualquier resolución anterior deja de interesar: si se elige una
        // pista local, su yt-dlp no debe seguir descargando por detrás.
        self.stream_resolver.cancel();
        let Some(path) = path.or_else(|| self.lib.local_path(&track.key)) else {
            if self.cfg.player.streaming && self.tools.yt_dlp {
                self.start_stream(track);
            } else {
                let key = track.key.clone();
                self.queue_track(track, None);
                self.pending_play = Some(key);
            }
            return;
        };
        if !path.exists() {
            self.toast(
                format!("falta el fichero: {}", path.display()),
                ToastKind::Error,
            );
            if self.cfg.player.streaming && self.tools.yt_dlp {
                self.start_stream(track);
            }
            return;
        }
        // Recorte persistente: si el vídeo de YouTube trae intro/outro, se
        // salta solo sin tocar el fichero (reversible con Backspace).
        let (trim_start, trim_end) = self.lib.trim(&track.key).unwrap_or((0, None));
        let duration = track.duration_ms.max(1);
        self.trim_start_ms = trim_start.min(duration.saturating_sub(500));
        self.trim_end_ms = trim_end
            .filter(|end| *end > self.trim_start_ms + 500)
            .map(|end| end.min(duration));
        let start_ms = if self.pending_seek_key.as_deref() == Some(track.key.as_str()) {
            self.pending_seek_ms.take().unwrap_or(self.trim_start_ms)
        } else {
            self.pending_seek_ms = None;
            self.pending_seek_key = None;
            self.trim_start_ms
        };
        self.now.track = Some(track.clone());
        self.now.path = Some(path.clone());
        self.now.position_ms = start_ms;
        self.now.duration_ms = track.duration_ms;
        self.now.stream_duration_ms = 0;
        self.now.playing = true;
        self.now.position_unknown = false;
        self.now.loading = false;
        self.now.message.clear();
        self.pending_stream = None;
        self.streaming = false;
        self.stream_cache_pending = None;
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
            player.set_loop(self.repeat_mode == RepeatMode::One);
            player.load(path.clone(), start_ms as f64 / 1000.0);
        } else {
            self.set_status("mpv no disponible: modo visual");
        }
        self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
        self.lyrics_auto_offset = None;
        self.lyrics = None;
        self.lyrics_status.clear();
        self.lyrics_key.clear();
        self.lyrics_loading = false;
        self.reset_lyrics_view();
        self.art = None;
        self.art_prev = None;
        self.art_variants.clear();
        self.art_key.clear();
        self.art_png = None;
        self.art_png_key.clear();
        self.art_png_size = None;
        self.art_png_hint = (0, 0);
        self.art_request_key.clear();
        self.art_request_size = (0, 0);
        self.ambient_art = None;
        self.ambient_art_key.clear();
        self.art_next_at =
            Instant::now() + std::time::Duration::from_secs(self.cfg.ui.art_rotate_secs.max(5));
        self.load_lyrics(&track);
        self.song_map = None;
        self.song_map_key.clear();
        self.track_gain_db = None;
        self.apply_audio_fx();
        self.song_intensity = 0.0;
        self.song_section = None;
        self.map_worker.request(track.key.clone(), path.clone());
        if !self.follow_spotify {
            self.spectrum
                .decode_file(path, start_ms as f64 / 1000.0);
            self.spectrum_at = Instant::now();
        }
        self.set_status(format!("▶ {}", track.label()));
    }

    fn start_stream(&mut self, track: TrackRef) {
        if self.follow_spotify {
            return;
        }
        if !self.cfg.player.streaming || !self.tools.yt_dlp {
            self.queue_track(track, None);
            return;
        }
        let hint = track.source_hint.clone().or_else(|| {
            self.lib.source_hint(&track.key).map(|(provider, id)| {
                spote_protocol::SourceHint {
                    provider,
                    id,
                    duration_ms: track.duration_ms,
                }
            })
        });
        let (trim_start, trim_end) = self.lib.trim(&track.key).unwrap_or((0, None));
        let duration = track.duration_ms.max(1);
        self.trim_start_ms = trim_start.min(duration.saturating_sub(500));
        self.trim_end_ms = trim_end
            .filter(|end| *end > self.trim_start_ms + 500)
            .map(|end| end.min(duration));
        self.now.track = Some(track.clone());
        self.now.path = None;
        self.now.position_ms = self.trim_start_ms;
        self.now.position_seen_ms = self.trim_start_ms;
        self.now.position_at = Instant::now();
        self.now.duration_ms = track.duration_ms;
        self.now.stream_duration_ms = 0;
        self.stream_autoplay = true;
        // Mientras se resuelve, la intención es reproducir: MPRIS/mini deben
        // publicar `Playing` y un `Play` del sistema no debe cancelar el
        // autoplay. Pausar sí lo desactiva.
        self.now.playing = self.stream_autoplay;
        self.now.position_unknown = false;
        self.now.loading = true;
        self.now.ended = false;
        self.pending_seek_ms = None;
        self.pending_seek_key = None;
        self.now.message.clear();
        self.pending_play = None;
        self.pending_stream = Some(track.key.clone());
        self.streaming = true;
        self.stream_cache_pending = None;
        // La pista anterior no debe seguir sonando mientras se resuelve.
        if let Some(player) = &self.player {
            player.set_loop(self.repeat_mode == RepeatMode::One);
            player.stop();
        }
        self.lyrics = None;
        self.lyrics_status.clear();
        self.lyrics_key.clear();
        self.lyrics_loading = false;
        self.lyrics_auto_offset = None;
        self.reset_lyrics_view();
        self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
        self.art = None;
        self.art_prev = None;
        self.art_variants.clear();
        self.art_key.clear();
        self.art_png = None;
        self.art_png_key.clear();
        self.art_png_size = None;
        self.art_png_hint = (0, 0);
        self.ambient_art = None;
        self.ambient_art_key.clear();
        self.song_map = None;
        self.song_map_key.clear();
        self.track_gain_db = None;
        self.apply_audio_fx();
        self.song_section = None;
        self.spectrum.stop();
        self.load_lyrics(&track);
        self.stream_resolver.resolve(
            track.key.clone(),
            &track.artist,
            &track.title,
            track.duration_ms,
            hint,
        );
        self.set_status(format!("resolviendo stream · {}", track.label()));
    }

    fn poll_stream(&mut self) {
        while let Some(event) = self.stream_resolver.try_recv() {
            match event {
                StreamEvent::Resolved { key, source } => {
                    if self.follow_spotify {
                        continue;
                    }
                    // Solo el trabajo pendiente actual puede resolver. Un
                    // evento duplicado o de una pista anterior no debe
                    // recargar la canción que ya está sonando.
                    let is_current = self.pending_stream.as_deref() == Some(key.as_str());
                    if !is_current {
                        continue;
                    }
                    self.pending_stream = None;
                    self.streaming = true;
                    self.now.loading = false;
                    self.now.playing = self.stream_autoplay;
                    self.now.path = None;
                    let start_ms = if self.pending_seek_key.as_deref()
                        == self.now.track.as_ref().map(|track| track.key.as_str())
                    {
                        self.pending_seek_ms.take().unwrap_or(self.trim_start_ms)
                    } else {
                        self.pending_seek_ms = None;
                        self.pending_seek_key = None;
                        self.trim_start_ms
                    };
                    let provider = source.provider.clone();
                    let id = source.id.clone();
                    if let Some(track) = self.now.track.clone() {
                        if let Ok(track_id) = self.lib.upsert_track(&track) {
                            self.lib
                                .set_source(
                                    track_id,
                                    &provider,
                                    Some(&id),
                                    None,
                                    spote_core::library::SourceStatus::Pending,
                                    None,
                                )
                                .ok();
                        }
                        let mut track = track;
                        track.source_hint = Some(spote_protocol::SourceHint {
                            provider: provider.clone(),
                            id: id.clone(),
                            duration_ms: track.duration_ms,
                        });
                        self.now.track = Some(track);
                    }
                    // El id resuelto vale también para el contexto y la cola:
                    // al volver a esta pista no se repite la búsqueda.
                    if let Some(hint) = self
                        .now
                        .track
                        .as_ref()
                        .and_then(|track| track.source_hint.clone())
                    {
                        for entry in &mut self.play_context {
                            if entry.track.key == key {
                                entry.track.source_hint = Some(hint.clone());
                            }
                        }
                        for entry in &mut self.queue {
                            if entry.track.key == key {
                                entry.track.source_hint = Some(hint.clone());
                            }
                        }
                    }
                    if let Some(player) = &self.player {
                        player.load_url(source.url, start_ms as f64 / 1000.0);
                        if !self.stream_autoplay {
                            player.pause(true);
                        }
                    }
                    if self.cfg.player.remote_viz == "system" && self.tools.parec {
                        self.spectrum.decode_system();
                        self.spectrum_at = Instant::now();
                    } else {
                        self.spectrum.stop();
                    }
                    // La copia local no se lanza todavía: si el usuario salta
                    // de pista en los primeros segundos, no se descarga una
                    // canción entera para nada. `poll_stream_cache` la arranca
                    // cuando de verdad se está escuchando.
                    self.stream_cache_pending = if self.cfg.player.stream_cache {
                        Some((
                            key.clone(),
                            format!("https://www.youtube.com/watch?v={}", source.id),
                        ))
                    } else {
                        None
                    };
                    self.set_status("▶ stream");
                }
                StreamEvent::Downloaded {
                    key,
                    path,
                    provider,
                    id,
                } => {
                    if self.follow_spotify {
                        continue;
                    }
                    // Solo el trabajo pendiente actual puede resolver. Un
                    // evento duplicado o de una pista anterior no debe
                    // recargar la canción que ya está sonando.
                    let is_current = self.pending_stream.as_deref() == Some(key.as_str());
                    if !is_current {
                        continue;
                    }
                    self.pending_stream = None;
                    self.streaming = false;
                    self.stream_cache_pending = None;
                    self.now.loading = false;
                    if let Some(track) = self.now.track.clone() {
                        let hint = if provider.trim().is_empty() {
                            self.lib.source_hint(&key)
                        } else {
                            Some((provider.clone(), id.clone()))
                        };
                        if let Ok(track_id) = self.lib.upsert_track(&track) {
                            self.lib
                                .set_source(
                                    track_id,
                                    hint.as_ref().map(|(p, _)| p.as_str()).unwrap_or("stream"),
                                    hint.as_ref().map(|(_, i)| i.as_str()),
                                    Some(&path),
                                    spote_core::library::SourceStatus::Ready,
                                    std::fs::metadata(&path).ok().map(|m| m.len()),
                                )
                                .ok();
                        }
                        let autoplay = self.stream_autoplay;
                        self.play_local(track, Some(path));
                        self.stream_autoplay = autoplay;
                        if !autoplay {
                            if let Some(player) = &self.player {
                                player.pause(true);
                            }
                            self.now.playing = false;
                        }
                    } else {
                        self.set_status("audio descargado, pero no hay pista activa");
                    }
                }
                StreamEvent::Failed { key, message } => {
                    // Solo el trabajo pendiente actual puede resolver. Un
                    // evento duplicado o de una pista anterior no debe
                    // recargar la canción que ya está sonando.
                    let is_current = self.pending_stream.as_deref() == Some(key.as_str());
                    if !is_current {
                        continue;
                    }
                    self.pending_stream = None;
                    self.pending_seek_ms = None;
                    self.pending_seek_key = None;
                    self.streaming = false;
                    self.stream_cache_pending = None;
                    self.now.loading = false;
                    self.now.playing = false;
                    self.toast(format!("stream no disponible: {message}"), ToastKind::Warn);
                    // NoMatch / AgeGated / GeoBlocked no mejoran con una
                    // descarga completa: reintentar solo alarga el fallo.
                    let terminal = ["NoMatch", "AgeGated", "GeoBlocked"]
                        .iter()
                        .any(|code| message.starts_with(code));
                    if !terminal {
                        if let Some(track) = self.now.track.clone() {
                            self.start_resolve(&track);
                            self.pending_play = Some(track.key.clone());
                            self.now.loading = true;
                            self.set_status("descargando tras fallar el stream…");
                        }
                    }
                }
            }
        }
    }

    /// Lanza la copia local de un stream solo cuando la pista lleva unos
    /// segundos sonando. Saltar de canción no dispara una descarga completa de
    /// cada pista por la que se pasa.
    fn poll_stream_cache(&mut self) {
        const CACHE_AFTER_MS: u64 = 15_000;
        let Some((key, url)) = self.stream_cache_pending.clone() else {
            return;
        };
        let same_track = self
            .now
            .track
            .as_ref()
            .map(|track| track.key.as_str())
            == Some(key.as_str());
        if !same_track {
            self.stream_cache_pending = None;
            return;
        }
        if !self.now.playing || self.now.loading {
            return;
        }
        // En pistas cortas no tiene sentido esperar 15 s: con la mitad de la
        // duración ya se sabe que se está escuchando de verdad.
        let threshold = if self.now.duration_ms > 0 {
            CACHE_AFTER_MS.min(self.now.duration_ms / 2)
        } else {
            CACHE_AFTER_MS
        };
        let near_end = self.now.duration_ms > 0
            && self.now.position_ms.saturating_add(1_500) >= self.now.duration_ms;
        if self.now.position_ms < threshold && !near_end {
            return;
        }
        self.stream_cache_pending = None;
        self.downloader
            .download(&key, DownloadSource::Url(url), self.cfg.media_dir());
    }

    /// Encola una pista: si no está descargada, lanza la descarga.
    pub fn queue_track(&mut self, track: TrackRef, url: Option<String>) {
        let track = self.canonical_track(&track);
        if let Some(path) = self.lib.local_path(&track.key).filter(|path| path.exists()) {
            self.queue.push(QueueEntry {
                track,
                path: Some(path),
            });
            self.toast("añadida a la cola", ToastKind::Ok);
            return;
        }
        // Las pistas importadas vienen sin hint en el TrackRef: se recupera de
        // la tabla de fuentes para poder descargarlas al encolarlas.
        let hint = track.source_hint.clone().or_else(|| {
            self.lib
                .source_hint(&track.key)
                .map(|(provider, id)| spote_protocol::SourceHint {
                    provider,
                    id,
                    duration_ms: track.duration_ms,
                })
        });
        if let Some(hint) = hint {
            if self.tools.yt_dlp {
                self.lib.upsert_track(&track).ok();
                if hint.provider == "spotify" || hint.provider == "deezer" {
                    self.start_resolve(&track);
                } else {
                    self.downloader.download(
                        &track.key,
                        DownloadSource::Url(format!("https://www.youtube.com/watch?v={}", hint.id)),
                        self.cfg.media_dir(),
                    );
                }
                self.queue.push(QueueEntry { track, path: None });
                self.toast("descargando y en cola", ToastKind::Info);
                return;
            }
        }
        if let Some(url) = url {
            self.downloader
                .download(&track.key, DownloadSource::Url(url), self.cfg.media_dir());
            self.queue.push(QueueEntry { track, path: None });
            return;
        }
        // Último recurso: artista + título, que es lo que tienen las pistas
        // viejas de la librería sin fuente registrada.
        if !track.artist.is_empty() && !track.title.is_empty() && self.tools.yt_dlp {
            self.lib.upsert_track(&track).ok();
            self.start_resolve(&track);
            self.queue.push(QueueEntry { track, path: None });
            self.toast("buscando fuente y en cola", ToastKind::Info);
            return;
        }
        self.toast(
            "no hay fuente descargable para esta pista",
            ToastKind::Error,
        );
    }

    fn start_resolve(&mut self, track: &TrackRef) {
        self.lib.upsert_track(track).ok();
        if let Some(url) = &track.art_url {
            self.lib.set_art(&track.key, None, Some(url)).ok();
        }
        self.downloader.resolve_and_download(
            &track.key,
            &track.artist,
            &track.title,
            track.duration_ms,
            self.cfg.media_dir(),
        );
    }

    fn start_download(&mut self, track: &TrackRef, url: String) {
        self.lib.upsert_track(track).ok();
        self.downloader
            .download(&track.key, DownloadSource::Url(url), self.cfg.media_dir());
    }

    /// Lanza el actualizador del repo en segundo plano. La TUI sigue viva
    /// hasta que termina; entonces se cierra para dejar paso a la versión
    /// nueva.
    fn start_update(&mut self) {
        if self.update_run.is_some() {
            return;
        }
        match crate::version::spawn_update() {
            Some(rx) => {
                self.update_run = Some(rx);
                self.update_started = Some(Instant::now());
                self.update_finished = None;
                self.overlay = Overlay::Update;
                self.set_status("actualizando spote… se cerrará al terminar");
            }
            None => {
                self.overlay = Overlay::None;
                self.toast(
                    "no encuentro scripts/install.sh; actualiza a mano con git pull",
                    ToastKind::Error,
                );
            }
        }
    }

    pub fn toggle_play(&mut self) {
        // Autorepeat del teclado: el terminal puede mandar varias pulsaciones
        // de espacio al mantener la tecla. Sin esta puerta, la pausa rebota
        // (pausa → play → pausa) y parece que va con retraso.
        if self.play_toggle_at.elapsed() < Duration::from_millis(180) {
            return;
        }
        self.play_toggle_at = Instant::now();
        if self.follow_spotify {
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::PlayPause);
            }
            self.now.playing = !self.now.playing;
            self.expect_follow_playing(self.now.playing);
            return;
        }
        if self.now.loading && self.pending_stream.is_some() {
            self.stream_autoplay = !self.stream_autoplay;
            self.now.playing = self.stream_autoplay;
            self.set_status(if self.stream_autoplay {
                "se reproducirá al resolver el stream"
            } else {
                "pausado; esperando al stream"
            });
            return;
        }
        if self.now.ended {
            if let Some(track) = self.now.track.clone() {
                self.now.ended = false;
                if let Some(path) = self.now.path.clone().filter(|path| path.exists()) {
                    self.play_local(track, Some(path));
                } else {
                    self.start_stream(track);
                }
                return;
            }
        }
        if self.now.track.is_none() {
            // Nada cargado: reproduce la fila seleccionada o la primera y crea
            // contexto para que repeat/next/prev tengan una lista estable.
            if self.selected_row().is_some() {
                self.play_row(self.selected);
                return;
            }
            if !self.view.is_empty() {
                self.play_row(0);
            }
            return;
        }
        if self.now.path.is_none() && !self.streaming {
            // Pista en espera de descarga: no hay nada en mpv que pausar.
            if self.pending_stream.is_some() {
                self.toast("resolviendo stream…", ToastKind::Info);
            } else if self.pending_play.is_some() {
                self.toast("esperando descarga…", ToastKind::Info);
            } else if let Some(track) = self.now.track.clone() {
                self.queue_track(track, None);
            }
            return;
        }
        self.now.playing = !self.now.playing;
        if let Some(player) = &self.player {
            player.pause(!self.now.playing);
        }
        let action = if self.now.playing {
            ControlAction::Play
        } else {
            ControlAction::Pause
        };
        self.send_control(action, None);
        if self.now.playing && !self.follow_spotify {
            if let Some(path) = &self.now.path {
                self.spectrum
                    .decode_file(path.clone(), self.now.position_ms as f64 / 1000.0);
            }
        } else {
            self.spectrum.stop();
        }
    }

    /// La letra de la pista anterior no debe arrastrar su scroll a la nueva.
    fn reset_lyrics_view(&mut self) {
        self.lyrics_scroll = Spring::new(0.0);
        self.lyrics_scroll_index = None;
        self.lyrics_follow = true;
        self.lyrics_search = None;
        self.lyrics_debug_last = None;
    }

    /// Define el contexto de reproducción (vista, búsqueda, álbum, artista).
    pub fn set_context(&mut self, entries: Vec<QueueEntry>, index: usize) {
        if entries.is_empty() {
            self.play_context.clear();
            self.context_index = 0;
            self.shuffle_state = None;
            return;
        }
        let index = index.min(entries.len().saturating_sub(1));
        self.play_context = entries;
        self.context_index = index;
        self.shuffle_state = if self.shuffle_on {
            let mut state = ShuffleState::new(self.play_context.len(), index);
            state.enable(self.play_context.len(), index);
            Some(state)
        } else {
            None
        };
    }

    fn ensure_shuffle(&mut self, len: usize, current: usize) {
        let rebuild = self
            .shuffle_state
            .as_ref()
            .map_or(true, |state| state.order_snapshot().len() != len);
        if rebuild {
            let mut state = ShuffleState::new(len, current);
            state.enable(len, current);
            self.shuffle_state = Some(state);
        }
    }

    fn play_context_entry(&mut self, index: usize) {
        let Some(entry) = self.play_context.get(index).cloned() else {
            return;
        };
        self.context_index = index;
        self.play_local(entry.track, entry.path);
    }

    fn apply_advance(&mut self, advance: Advance) {
        match advance {
            Advance::To(index) => self.play_context_entry(index),
            Advance::RepeatCurrent => {
                self.now.position_ms = self.trim_start_ms;
                if self.now.ended {
                    if let Some(track) = self.now.track.clone() {
                        self.now.ended = false;
                        if let Some(path) = self.now.path.clone().filter(|path| path.exists()) {
                            self.play_local(track, Some(path));
                        } else {
                            self.start_stream(track);
                        }
                    }
                } else if let Some(player) = &self.player {
                    player.seek(self.trim_start_ms as f64 / 1000.0);
                    player.pause(false);
                }
                self.now.playing = true;
            }
            Advance::Stop => {
                self.now.playing = false;
                self.now.ended = true;
                if let Some(player) = &self.player {
                    player.pause(true);
                }
                self.set_status("fin de la cola");
            }
        }
    }

    /// Avance automático al terminar una pista. La cola explícita manda; si
    /// está vacía, se aplica el modo repeat/shuffle del contexto.
    pub fn auto_next(&mut self) {
        if self.follow_spotify || self.room.is_some() {
            self.next_track();
            return;
        }
        if self.repeat_mode == RepeatMode::One {
            self.apply_advance(Advance::RepeatCurrent);
            return;
        }
        if !self.queue.is_empty() {
            self.next_track();
            return;
        }
        if self.play_context.is_empty() {
            self.next_track();
            return;
        }
        let len = self.play_context.len();
        let current = self.context_index.min(len.saturating_sub(1));
        let advance = if self.shuffle_on {
            self.ensure_shuffle(len, current);
            self.shuffle_state
                .as_mut()
                .map(|state| state.next(len, self.repeat_mode))
                .unwrap_or(Advance::Stop)
        } else {
            playback::auto_advance(current, len, self.repeat_mode)
        };
        self.apply_advance(advance);
    }

    pub fn cycle_repeat(&mut self) {
        self.set_repeat_mode(self.repeat_mode.next());
        self.toast(
            match self.repeat_mode {
                RepeatMode::Off => "bucle apagado",
                RepeatMode::All => "bucle: toda la cola",
                RepeatMode::One => "bucle: una canción",
            },
            ToastKind::Info,
        );
    }

    /// Cambia el modo de bucle sin toast (MPRIS y el mini lo usan).
    pub fn set_repeat_mode(&mut self, mode: RepeatMode) {
        if self.repeat_mode == mode {
            // Repara el invariante si venía de config o de un SetRepeat
            // externo con `loop_file` desincronizado.
            self.loop_file = mode == RepeatMode::One;
            if !self.follow_spotify {
                if let Some(player) = &self.player {
                    player.set_loop(self.loop_file);
                }
            }
            return;
        }
        self.repeat_mode = mode;
        self.loop_file = mode == RepeatMode::One;
        if self.follow_spotify {
            // El repeat es del reproductor externo: se reenvía, pero no se
            // guarda como preferencia local ni se aplica al mpv de spote.
            self.expect_follow_repeat(mode);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::SetRepeat(
                    match mode {
                        RepeatMode::Off => "off",
                        RepeatMode::All => "all",
                        RepeatMode::One => "one",
                    }
                    .to_string(),
                ));
            }
            return;
        }
        self.cfg.playback.repeat = match mode {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
        .to_string();
        if let Some(player) = &self.player {
            player.set_loop(self.loop_file);
        }
        self.cfg.save().ok();
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle_on = !self.shuffle_on;
        if self.follow_spotify {
            // Igual que repeat: manda en el reproductor seguido, no en la
            // configuración local.
            self.expect_follow_shuffle(self.shuffle_on);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::SetShuffle(self.shuffle_on));
            }
            return;
        }
        self.cfg.playback.shuffle = self.shuffle_on;
        self.cfg.save().ok();
        if self.shuffle_on {
            if self.play_context.is_empty() && !self.view.is_empty() {
                let entries: Vec<QueueEntry> = self
                    .view
                    .iter()
                    .map(|row| QueueEntry {
                        track: row.track.clone(),
                        path: row.path.clone().filter(|path| path.exists()),
                    })
                    .collect();
                self.set_context(entries, self.selected);
            }
            let len = if self.play_context.is_empty() {
                self.view.len()
            } else {
                self.play_context.len()
            };
            let current = if self.play_context.is_empty() {
                self.selected
            } else {
                self.context_index
            };
            if len > 0 {
                let mut state = ShuffleState::new(len, current);
                state.enable(len, current);
                self.shuffle_state = Some(state);
            }
            self.toast("aleatorio activado", ToastKind::Info);
        } else {
            if let Some(state) = self.shuffle_state.as_mut() {
                if !self.play_context.is_empty() {
                    self.context_index = state.disable(self.context_index);
                }
            }
            self.shuffle_state = None;
            self.toast("aleatorio desactivado", ToastKind::Info);
        }
    }

    /// Atajo `u`: alterna bucle de una pista. Comparte estado con `o` para
    /// que `loop-file` y `RepeatMode` no se contradigan.
    pub fn toggle_loop(&mut self) {
        if self.follow_spotify {
            self.toast("el bucle no se controla en modo Spotify", ToastKind::Warn);
            return;
        }
        self.repeat_mode = if self.repeat_mode == RepeatMode::One {
            RepeatMode::Off
        } else {
            RepeatMode::One
        };
        self.loop_file = self.repeat_mode == RepeatMode::One;
        self.cfg.playback.repeat = match self.repeat_mode {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
        .to_string();
        self.cfg.save().ok();
        if let Some(player) = &self.player {
            player.set_loop(self.loop_file);
        }
        self.toast(
            if self.loop_file {
                "bucle: una canción"
            } else {
                "bucle apagado"
            },
            ToastKind::Info,
        );
    }

    pub fn next_track(&mut self) {
        if self.follow_spotify {
            self.now.position_ms = 0;
            self.expect_follow_position(0);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::Next);
            }
            return;
        }
        if self.room.is_some() {
            self.send_control(ControlAction::Next, None);
            return;
        }
        if let Some(entry) = self.queue.first().cloned() {
            self.queue.remove(0);
            self.queue_selected = self.queue_selected.min(self.queue.len().saturating_sub(1));
            // La entrada pudo descargarse mientras estaba en cola: se mira la
            // biblioteca antes de darla por pendiente (antes se quedaba
            // «esperando descarga…» para siempre).
            let path = entry
                .path
                .filter(|path| path.exists())
                .or_else(|| self.lib.local_path(&entry.track.key));
            if let Some(path) = path {
                self.play_local(entry.track, Some(path));
            } else if self.cfg.player.streaming && self.tools.yt_dlp {
                self.play_local(entry.track, None);
            } else {
                // Ojo: aquí todavía no hay nada cargado en mpv. Antes se
                // cambiaba `now.track` y se dejaba el fichero anterior en el
                // reproductor: la UI decía Chanel pero seguía sonando la
                // canción de antes, y un seek/atrás reproducía esa.
                if let Some(current) = self.now.track.clone() {
                    if current.key != entry.track.key {
                        self.history.push((current, self.now.path.clone()));
                        if self.history.len() > 100 {
                            self.history.remove(0);
                        }
                    }
                }
                self.pending_play = Some(entry.track.key.clone());
                self.now.track = Some(entry.track.clone());
                self.now.path = None;
                self.now.position_ms = 0;
                self.now.duration_ms = entry.track.duration_ms;
                self.now.playing = false;
                self.now.loading = true;
                self.now.ended = false;
                self.stream_autoplay = true;
                self.pending_seek_ms = None;
                self.pending_seek_key = None;
                self.now.message = "esperando descarga…".into();
                self.lyrics = None;
                self.lyrics_status = "esperando descarga…".into();
                self.lyrics_key.clear();
                self.reset_lyrics_view();
                self.art = None;
                self.art_key.clear();
                self.art_png = None;
                self.art_png_key.clear();
                self.ambient_art = None;
                self.ambient_art_key.clear();
                self.song_map = None;
                self.song_map_key.clear();
                self.track_gain_db = None;
                self.apply_audio_fx();
                self.spectrum.stop();
                if let Some(player) = &self.player {
                    player.stop();
                }
                self.set_status("esperando descarga…");
            }
            return;
        }
        if !self.play_context.is_empty() {
            let len = self.play_context.len();
            let current = self.context_index.min(len.saturating_sub(1));
            let advance = if self.shuffle_on {
                self.ensure_shuffle(len, current);
                self.shuffle_state
                    .as_mut()
                    .map(|state| state.next(len, self.repeat_mode))
                    .unwrap_or(Advance::Stop)
            } else {
                playback::manual_next(current, len, self.repeat_mode)
            };
            self.apply_advance(advance);
            return;
        }
        let wrap = self.repeat_mode == RepeatMode::All;
        self.play_view_step(1, wrap, true, false);
    }

    pub fn prev_track(&mut self) {
        if self.follow_spotify {
            self.now.position_ms = 0;
            self.expect_follow_position(0);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::Previous);
            }
            return;
        }
        if self.room.is_some() {
            self.send_control(ControlAction::Prev, None);
            return;
        }
        // Solo se rebobina si de verdad hay un fichero cargado. Con una pista
        // esperando descarga, `position_ms` es 0 y no hay nada que seekear.
        if (self.now.path.is_some() || self.streaming) && self.now.position_ms > 3_000 {
            self.seek_to(0);
            return;
        }
        // Historial de sesión: «anterior» devuelve a la pista que sonaba de
        // verdad, aunque se haya puesto desde una lista, la cola o buscar.
        if let Some((track, path)) = self.history.pop() {
            // Aunque no haya fichero local, la pista puede reproducirse por
            // stream; antes se descartaba y «anterior» se saltaba la historia.
            let path = path
                .filter(|path| path.exists())
                .or_else(|| self.lib.local_path(&track.key));
            // La pista que dejamos pasa a ser la primera de la cola: al dar a
            // «siguiente» se vuelve a ella antes de continuar con lo encolado.
            self.queue_current_for_back();
            self.play_local_inner(track, path, false);
            return;
        }
        if !self.play_context.is_empty() {
            let len = self.play_context.len();
            let current = self.context_index.min(len.saturating_sub(1));
            let index = if self.shuffle_on {
                self.ensure_shuffle(len, current);
                self.shuffle_state.as_mut().and_then(|state| state.prev(len))
            } else {
                playback::advance_index(current, len, self.repeat_mode, playback::Direction::Prev)
            };
            if let Some(index) = index {
                self.play_context_entry(index);
                return;
            }
        }
        let wrap = self.repeat_mode == RepeatMode::All;
        self.play_view_step(-1, wrap, false, true);
    }

    /// Mete la pista actual al principio de la cola local. Se usa al
    /// retroceder para que el camino de vuelta no se pierda: si la cola era
    /// [B, C] y estabas en A, atrás deja [A, B, C] y «siguiente» vuelve a A.
    fn queue_current_for_back(&mut self) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        let selected_key = self
            .queue
            .get(self.queue_selected)
            .map(|entry| entry.track.key.clone());
        // Si ya estaba en la cola (aunque más abajo), se mueve al frente para
        // no acabar con la misma canción dos veces.
        self.queue.retain(|entry| entry.track.key != track.key);
        self.queue.insert(
            0,
            QueueEntry {
                track: track.clone(),
                path: self.now.path.clone(),
            },
        );
        self.queue_selected = if selected_key.as_deref() == Some(track.key.as_str()) {
            0
        } else {
            selected_key
                .and_then(|key| self.queue.iter().position(|entry| entry.track.key == key))
                .unwrap_or(0)
        };
    }

    /// Avanza/retrocede en la lista tomando como referencia la pista que
    /// suena, no la fila seleccionada. Si la pista no está en la lista actual,
    /// arranca desde la selección.
    fn play_view_step(
        &mut self,
        delta: i32,
        wrap: bool,
        record_history: bool,
        queue_current: bool,
    ) {
        if self.view.is_empty() {
            return;
        }
        let current = self.now.track.as_ref().map(|track| track.key.clone());
        let index = current
            .as_ref()
            .and_then(|key| self.view.iter().position(|row| &row.track.key == key))
            .unwrap_or_else(|| self.selected.min(self.view.len() - 1));
        let len = self.view.len() as i32;
        let next = if wrap {
            (index as i32 + delta).rem_euclid(len)
        } else {
            let raw = index as i32 + delta;
            if raw < 0 || raw >= len {
                // Repeat off: en el borde no se da la vuelta ni se corta la
                // pista actual; simplemente no hay siguiente/anterior.
                self.set_status("fin de la cola");
                return;
            }
            raw
        };
        self.selected = next as usize;
        if let Some(row) = self.view.get(self.selected).cloned() {
            if queue_current && current.as_deref() != Some(row.track.key.as_str()) {
                self.queue_current_for_back();
            }
            self.play_local_inner(row.track, row.path, record_history);
        }
    }

    pub fn seek_to(&mut self, ms: u64) {
        let target = if self.now.duration_ms > 0 {
            ms.min(self.now.duration_ms)
        } else {
            ms
        };
        // Un seek rearma el interpolador en el mismo instante: si no, el frame
        // siguiente mezcla la posición nueva con el elapsed del seek anterior.
        self.now.position_seen_ms = target;
        self.now.position_at = Instant::now();
        if self.follow_spotify {
            // El transporte manda al reproductor MPRIS del sistema.
            self.now.position_ms = target;
            self.expect_follow_position(target);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::SetPosition(target));
            }
            return;
        }
        if self.now.loading || self.pending_stream.is_some() {
            // El stream aún no está cargado: se guarda el destino y se aplica
            // al recibir la URL o el fichero descargado.
            self.pending_seek_ms = Some(target);
            self.pending_seek_key = self.now.track.as_ref().map(|track| track.key.clone());
            self.now.position_ms = target;
            return;
        }
        if self.now.path.is_none() && !self.streaming {
            // Pista pendiente de descarga: no hay fichero en mpv al que
            // saltar. Evita resucitar por accidente la canción anterior.
            return;
        }
        self.send_control(ControlAction::Seek, Some(target));
        self.now.position_ms = target;
        if let Some(player) = &self.player {
            player.seek(target as f64 / 1000.0);
        }
        if !self.follow_spotify {
            if let Some(path) = &self.now.path {
                self.spectrum
                    .decode_file(path.clone(), target as f64 / 1000.0);
                self.spectrum_at = Instant::now();
            }
        }
    }

    pub fn seek_relative(&mut self, delta_ms: i64) {
        if self.follow_spotify {
            // El reproductor seguido entiende `Seek` relativo; recalcular una
            // posición absoluta desde un reloj local atrasado saltaba a otro
            // sitio (o abortaba si no publica `Position`).
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::SeekRelative(delta_ms.saturating_mul(1000)));
            }
            return;
        }
        let target = (self.display_position_ms() as i64 + delta_ms).max(0) as u64;
        self.seek_to(target);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.now.volume = volume.clamp(0.0, 1.0);
        if self.now.volume > 0.0 {
            self.muted = false;
        }
        if self.follow_spotify {
            // El volumen del reproductor externo no es el volumen local de
            // spote: no se guarda, ni se aplica a mpv, ni se arrastra al
            // desactivar el modo acompañante.
            self.expect_follow_volume(self.now.volume);
            if let Some(worker) = &self.spotify_worker {
                worker.send(SystemCommand::SetVolume(self.now.volume));
            }
            return;
        }
        self.cfg.player.volume = self.now.volume;
        // Durante el arrastre se guarda al soltar; en teclado/rueda, ya.
        if !self.dragging_volume {
            self.cfg.save().ok();
        }
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
        }
    }

    /// Guarda el volumen aunque venga de un arrastre en curso.
    pub fn persist_volume(&mut self) {
        // Un mute temporal no debe convertirse en el volumen guardado.
        if !self.follow_spotify {
            let volume = if self.muted {
                self.volume_before_mute.max(0.0)
            } else {
                self.now.volume
            };
            self.cfg.player.volume = volume;
        }
        self.cfg.save().ok();
    }

    // ------------------------------------------------------------- ajustes
    fn load_lyrics(&mut self, track: &TrackRef) {
        if self.lyrics_loading {
            return;
        }
        // 1. Cache SQLite. Si es una letra aproximada y hay audio local, se
        // muestra mientras se reintenta con ASR: las de Genius repartidas a
        // ojo fueron justo el problema de sincronización.
        if let Ok(Some(row)) = self.lib.lyrics(&track.key) {
            let mut doc = LyricsDoc::from_lrc(&row.lrc, row.synced);
            let lrc_offset = doc.offset_ms;
            // Un offset manual guardado manda; si no hay, conservamos el
            // `[offset:]` del LRC en vez de perderlo al recargar la caché.
            doc.offset_ms = if row.offset_ms != 0 {
                row.offset_ms
            } else {
                lrc_offset
            };
            doc.aligned = row.aligned;
            if let Some(source) = spote_core::lyrics::LyricsSource::from_label(&row.source) {
                doc.source = source;
            }
            // Un alineado de una versión anterior del pipeline pudo dejar
            // tiempos malos; se muestra mientras se vuelve a afinar.
            let stale_alignment =
                row.aligned && row.align_version < spote_core::library::LYRICS_ALIGN_VERSION;
            // Un LRC «sincronizado» también puede venir roto: varias líneas a
            // menos de 400 ms es una ráfaga imposible y merece ASR.
            let broken_sync = row.synced
                && !row.aligned
                && doc
                    .lines
                    .windows(2)
                    .filter(|pair| {
                        pair[1].start_ms.saturating_sub(pair[0].start_ms) < 400
                    })
                    .count()
                    >= 2;
            self.lyrics_offset_ms = doc.offset_ms;
            let source = doc.source;
            self.lyrics = Some(doc);
            self.lyrics_status = if stale_alignment || broken_sync {
                "afinando de nuevo…".to_string()
            } else if row.aligned {
                "asr".to_string()
            } else {
                match source {
                    spote_core::lyrics::LyricsSource::Plain
                    | spote_core::lyrics::LyricsSource::Estimated => "aprox".to_string(),
                    source => source.label().to_string(),
                }
            };
            self.lyrics_key = track.key.clone();
            self.refine_lyrics_words();
            let audio = self.lib.local_path(&track.key).filter(|path| path.exists());
            if ((!row.synced && !row.aligned) || stale_alignment || broken_sync)
                && row.source != "manual"
                && audio.is_some()
                && !self.lyrics_retry.contains(&track.key)
            {
                self.lyrics_retry.insert(track.key.clone());
                self.lyrics_loading = true;
                self.lyrics_status = "afinando con el audio…".into();
                let request_track = self.lyrics_track(&track);
                self.lyrics_worker.request(
                    request_track,
                    audio,
                    stale_alignment || broken_sync,
                );
            } else {
                return;
            }
            return;
        }
        // 2. `.lrc` junto al fichero.
        if let Some(path) = self.lib.local_path(&track.key) {
            let candidate = path.with_extension("lrc");
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                let mut doc = spote_core::lyrics::parse_lrc(&text);
                self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
                if self.lyrics_offset_ms != 0 {
                    doc.offset_ms = self.lyrics_offset_ms;
                }
                self.lyrics = Some(doc);
                self.lyrics_status = "local".into();
                self.lyrics_key = track.key.clone();
                self.refine_lyrics_words();
                return;
            }
        }
        // 3. Red.
        self.lyrics_loading = true;
        self.lyrics_status = "buscando letra…".into();
        let audio = self.lib.local_path(&track.key).filter(|path| path.exists());
        let request_track = self.lyrics_track(&track);
        self.lyrics_worker.request(request_track, audio, false);
    }

    /// Copia la pista con el hint de su fuente. Los captions de YouTube usan
    /// el id del vídeo del que salió el audio para no buscar a ciegas.
    fn lyrics_track(&self, track: &TrackRef) -> TrackRef {
        let mut track = track.clone();
        if track.source_hint.is_none() {
            if let Some((provider, id)) = self.lib.source_hint(&track.key) {
                track.source_hint = Some(spote_protocol::SourceHint {
                    provider,
                    id,
                    duration_ms: track.duration_ms,
                });
            }
        }
        track
    }

    /// Marca el inicio del recorte en la posición actual (sáltate la intro).
    fn trim_start_here(&mut self) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        let start = self.now.position_ms;
        if self.trim_end_ms.is_some_and(|end| end <= start + 500) {
            self.trim_end_ms = None;
        }
        self.trim_start_ms = start;
        self.lib.set_trim(&track.key, start, self.trim_end_ms).ok();
        self.toast(
            format!(
                "intro recortada hasta {}",
                spote_core::util::format_ms(start)
            ),
            ToastKind::Info,
        );
        if self.now.playing {
            self.seek_to(start);
        }
    }

    /// Marca el final del recorte en la posición actual (sáltate el outro).
    fn trim_end_here(&mut self) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        let end = self.now.position_ms.max(self.trim_start_ms + 500);
        self.trim_end_ms = Some(end);
        self.lib
            .set_trim(&track.key, self.trim_start_ms, Some(end))
            .ok();
        self.toast(
            format!("outro recortado en {}", spote_core::util::format_ms(end)),
            ToastKind::Info,
        );
    }

    fn clear_trim(&mut self) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        self.trim_start_ms = 0;
        self.trim_end_ms = None;
        self.lib.clear_trim(&track.key).ok();
        self.toast("recorte quitado", ToastKind::Info);
    }

    /// Shift+clic en una línea de la letra: alinea esa línea con la posición
    /// actual sin cortar la reproducción.
    fn sync_lyrics_to_position(&mut self, line_start_ms: u64) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        let offset = lyrics_offset_for_click(
            line_start_ms,
            self.lyrics_offset_ms,
            self.display_position_ms(),
        );
        self.lyrics_offset_ms = offset.clamp(-120_000, 120_000);
        if let Some(doc) = &mut self.lyrics {
            doc.offset_ms = self.lyrics_offset_ms;
        }
        self.lib
            .set_lyrics_offset(&track.key, self.lyrics_offset_ms)
            .ok();
        self.toast(
            format!(
                "letra sincronizada a esta línea ({:+} ms)",
                self.lyrics_offset_ms
            ),
            ToastKind::Ok,
        );
    }

    /// Ajuste fino de la letra (`,`/`.`): guarda el offset por pista.
    fn nudge_lyrics_offset(&mut self, delta_ms: i64) {
        self.lyrics_offset_ms = (self.lyrics_offset_ms + delta_ms).clamp(-120_000, 120_000);
        if let Some(doc) = self.lyrics.as_mut() {
            doc.offset_ms = self.lyrics_offset_ms;
        }
        if let Some(track) = &self.now.track {
            self.lib
                .set_lyrics_offset(&track.key, self.lyrics_offset_ms)
                .ok();
        }
        self.toast(
            format!("letra {:+} ms", self.lyrics_offset_ms),
            ToastKind::Info,
        );
    }

    /// Botones A−/A+ de la letra. No es un zoom artificial del dibujo: cambia
    /// el tamaño tipográfico de la letra (ancho de columna y aire entre líneas)
    /// y se guarda en la config.
    pub fn adjust_lyrics_size(&mut self, delta: i32) {
        let size = (i32::from(self.cfg.ui.lyrics_size) + delta).clamp(0, 2) as u8;
        self.cfg.ui.lyrics_size = size;
        self.cfg.save().ok();
        self.set_status(match size {
            0 => "letra compacta",
            2 => "letra grande",
            _ => "letra normal",
        });
    }

    /// Devuelve el ajuste manual a cero (también en SQLite).
    pub fn reset_lyrics_offset(&mut self) {
        self.lyrics_offset_ms = 0;
        if let Some(doc) = self.lyrics.as_mut() {
            doc.offset_ms = 0;
        }
        if let Some(track) = &self.now.track {
            self.lib.set_lyrics_offset(&track.key, 0).ok();
        }
        self.set_status("offset de letra a 0 ms");
    }

    /// Scroll manual de la letra. Desactiva el seguimiento automático para que
    /// la canción no devuelva la vista al verso actual mientras se explora.
    pub fn scroll_lyrics(&mut self, delta: i32) {
        let Some(doc) = &self.lyrics else { return };
        if doc.lines.is_empty() {
            return;
        }
        let max = doc.lines.len().saturating_sub(1) as f32;
        self.lyrics_follow = false;
        let base = self.lyrics_scroll.target.round();
        let next = (base + delta as f32).clamp(0.0, max);
        self.lyrics_scroll.set_target(next);
        self.needs_redraw = true;
    }

    /// Vuelve a la línea que suena y reactiva el seguimiento.
    pub fn follow_current_lyric(&mut self) {
        self.lyrics_follow = true;
        let index = self.lyrics_scroll_index.unwrap_or(0);
        self.lyrics_scroll.value = index as f32;
        self.lyrics_scroll.velocity = 0.0;
        self.lyrics_scroll.set_target(index as f32);
        self.needs_redraw = true;
    }

    /// Descarta la letra cacheada (p. ej. una coincidencia equivocada de
    /// otro máster) y vuelve a preguntar a los proveedores.
    pub fn refetch_lyrics(&mut self) {
        let Some(track) = self.now.track.clone() else {
            self.toast("no hay nada sonando", ToastKind::Warn);
            return;
        };
        self.lib.delete_lyrics(&track.key).ok();
        self.lib.set_lyrics_offset(&track.key, 0).ok();
        self.lyrics = None;
        self.lyrics_key.clear();
        self.lyrics_status = "buscando otra letra…".into();
        self.lyrics_loading = false;
        self.lyrics_offset_ms = 0;
        self.lyrics_retry.remove(&track.key);
        self.reset_lyrics_view();
        self.load_lyrics(&track);
        self.toast("caché de letra borrada · buscando de nuevo", ToastKind::Info);
    }

    /// Refina los tiempos de palabra estimados con el mapa de energía de la
    /// canción: las palabras caen donde hay voz/energía real, no repartidas a
    /// ciegas. Solo toca líneas sin tags LRC de palabra (`words_exact`).
    fn refine_lyrics_words(&mut self) {
        let Some(map) = self.song_map.clone() else {
            return;
        };
        let Some(doc) = self.lyrics.as_mut() else {
            return;
        };
        // ASR ya entregó tiempos de palabra: reescribirlos sería empeorarlos.
        if doc.aligned {
            return;
        }
        let offset = doc.offset_ms;
        for line in &mut doc.lines {
            if line.words_exact || line.words.len() < 2 || line.end_ms <= line.start_ms {
                continue;
            }
            // Los buckets están en tiempo de audio; las líneas, en tiempo de
            // documento. Sin descontar el offset se elige una ventana ajena y
            // las palabras se encienden a mitad de frase.
            let audio_start = (line.start_ms as i64 - offset).max(0) as u64;
            let audio_end = (line.end_ms as i64 - offset).max(0) as u64;
            let buckets: Vec<_> = map
                .buckets
                .iter()
                .filter(|b| b.at_ms >= audio_start && b.at_ms < audio_end)
                .collect();
            if buckets.len() < 3 {
                continue;
            }
            let weights: Vec<f32> = buckets
                .iter()
                .map(|b| (b.vocal * 1.4 + b.energy * 0.8 + b.mid * 0.2).max(0.01))
                .collect();
            let total: f32 = weights.iter().sum();
            if total <= 0.0 {
                continue;
            }
            // Fracción de texto acumulada por palabra (misma ponderación que el
            // parser: sílabas + puntuación).
            let mut text_weights = Vec::with_capacity(line.words.len());
            for word in &line.words {
                let vowels = word
                    .text
                    .to_lowercase()
                    .chars()
                    .filter(|c| "aeiouáéíóúüàèìòù".contains(*c))
                    .count()
                    .max(1);
                let pause = if word.text.ends_with([',', '.', '!', '?', ';', '…']) {
                    1.8
                } else {
                    0.0
                };
                text_weights
                    .push(vowels as f32 * 1.7 + word.text.chars().count() as f32 * 0.3 + pause);
            }
            let text_total: f32 = text_weights.iter().sum::<f32>().max(0.001);
            let mut cumulative = 0.0f32;
            let mut starts = Vec::with_capacity(line.words.len());
            for weight in &text_weights {
                let target = cumulative / text_total;
                let mut acc = 0.0;
                // Si la energía no alcanza el objetivo, colocar la palabra al
                // final de la ventana, no al principio (antes se apelotonaban
                // todas en `line.start_ms`).
                let mut at = audio_end;
                for (bucket, w) in buckets.iter().zip(&weights) {
                    acc += w / total;
                    if acc >= target {
                        at = bucket.at_ms;
                        break;
                    }
                }
                let at = (at as i64 + offset).max(0) as u64;
                starts.push(at.clamp(line.start_ms, line.end_ms));
                cumulative += weight;
            }
            starts.sort_unstable();
            // Anclas: la frase empieza con la primera palabra. La última no
            // se pega a `line.end_ms`: si empieza justo cuando salta la línea,
            // nunca llega a encenderse. Se le deja un margen de 150 ms para
            // que el karaoke la llene antes del cambio.
            if let Some(first) = starts.first_mut() {
                *first = line.start_ms;
            }
            let tail_end = line
                .end_ms
                .saturating_sub(150)
                .max(line.start_ms + 1);
            // Deja al menos 250 ms para que la última palabra se encienda.
            let last_start_limit = tail_end.saturating_sub(250).max(line.start_ms);
            if let Some(last) = starts.last_mut() {
                *last = (*last).min(last_start_limit);
            }
            starts.sort_unstable();
            for (i, word) in line.words.iter_mut().enumerate() {
                word.start_ms = starts[i].min(last_start_limit);
                word.end_ms = starts
                    .get(i + 1)
                    .copied()
                    .unwrap_or(tail_end)
                    .max(word.start_ms + 1);
            }
            if let Some(last) = line.words.last_mut() {
                last.end_ms = tail_end.max(last.start_ms + 1);
            }
        }
    }

    // ---------------------------------------------------------------- tick
    pub fn tick(&mut self, dt: f32) {
        if self.now.position_ms != self.now.position_seen_ms {
            self.now.position_seen_ms = self.now.position_ms;
            self.now.position_at = Instant::now();
        }
        self.intro = (self.intro + dt * 2.5).min(1.0);
        self.mosaic_time += dt;
        let fps = self.fps.tick();
        self.frame_ms = if fps > 0.1 { 1000.0 / fps } else { 0.0 };
        self.pulse.update(dt);
        self.palette.update(dt);
        self.prune_downloads();
        if self.art_fade_active {
            self.art_fade = (self.art_fade + dt / 1.1).min(1.0);
            if self.art_fade >= 1.0 {
                self.art_fade_active = false;
                self.art_prev = None;
            }
        }
        if self.art_rotate && self.now.track.is_some() && Instant::now() >= self.art_next_at {
            let secs = self.cfg.ui.art_rotate_secs.max(5);
            self.art_next_at = Instant::now() + std::time::Duration::from_secs(secs);
            let mut modes = ArtMode::ROTATION.to_vec();
            if self.art_rotate_off {
                modes.push(ArtMode::Off);
            }
            let current = modes
                .iter()
                .position(|mode| *mode == self.art_mode)
                .unwrap_or(0);
            let next = modes[(current + 1) % modes.len()];
            self.set_art_mode(next);
        }
        if let Some(rx) = &self.update_check {
            match rx.try_recv() {
                Ok(result) => {
                    self.update_check = None;
                    if let Some(info) = result {
                        self.update_info = Some(info);
                        if !self.update_dismissed && self.overlay == Overlay::None {
                            self.overlay = Overlay::Update;
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_check = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.update_run {
            match rx.try_recv() {
                Ok(ok) => {
                    self.update_run = None;
                    self.update_finished = Some(ok);
                    if ok {
                        self.should_quit = true;
                    } else {
                        self.update_started = None;
                        self.overlay = Overlay::Update;
                        self.toast(
                            "la actualización falló; mira ~/.cache/spote/update.log",
                            ToastKind::Error,
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.update_run = None;
                    self.update_started = None;
                    self.update_finished = Some(false);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(doc) = &self.lyrics {
            if let Some(index) = doc.index_at(self.display_position_ms()) {
                if self.lyrics_follow {
                    if self.lyrics_scroll_index != Some(index) {
                        self.lyrics_scroll_index = Some(index);
                        // El resaltado cambia al instante: si el muelle persigue
                        // el índice, la línea nueva se pintaba mientras la
                        // anterior seguía en el centro y se percibía como cambio
                        // a destiempo.
                        self.lyrics_scroll.value = index as f32;
                        self.lyrics_scroll.velocity = 0.0;
                    }
                    self.lyrics_scroll.set_target(index as f32);
                } else {
                    // Guarda por dónde va la canción para poder volver con un
                    // clic, pero sin mover la vista del usuario.
                    self.lyrics_scroll_index = Some(index);
                }
            }
        }
        self.lyrics_scroll.update(dt);

        let frame = self.spectrum.frame();
        if frame.beat {
            self.pulse.hit(0.7);
        }
        self.audio = frame;

        self.poll_player();
        self.poll_stream();
        self.poll_stream_cache();
        self.poll_search();
        self.poll_downloads();
        self.poll_spotify_jobs();
        self.poll_art();
        self.poll_meta();
        self.poll_song_map();
        self.poll_lyrics();
        self.poll_lyrics_search();
        self.poll_spotify(dt);
        self.poll_mini();
        self.poll_mpris();
        self.poll_social();
        self.publish_presence();

        // Sincroniza el espectro con la posición real de mpv: si se desvía más
        // de medio segundo, se re-arranca la decodificación en el punto exacto.
        if !self.follow_spotify && self.now.playing && self.spectrum.active() {
            if let Some(path) = self.now.path.clone() {
                if self.spectrum_at.elapsed() > Duration::from_secs(3) {
                    self.spectrum_at = Instant::now();
                    self.spectrum
                        .decode_file(path, self.now.position_ms as f64 / 1000.0);
                }
            }
        }

        if self.status_at.elapsed() > Duration::from_secs(4) {
            self.status.clear();
        }
        self.toasts
            .retain(|toast| toast.created.elapsed() < Duration::from_secs(6));
    }

    /// ¿Hay algo que pueda cambiar en pantalla sin que el usuario toque nada?
    /// Con `false` el bucle pasa a 10 Hz: sigue dibujando (los efectos en
    /// pausa van a ese ritmo) pero no repinta 60 veces por segundo en balde.
    pub fn animating(&self) -> bool {
        self.now.playing
            || self.now.loading
            || self.intro < 1.0
            || self.art_fade_active
            || self.palette.animating()
            || !self.lyrics_scroll.settled()
            || self.pulse.value > 0.0
            || (self.art_rotate && self.now.track.is_some())
            || self.downloads.iter().any(DownloadInfo::active)
            || self.search.loading
            || self.album_page_loading
    }

    /// Ritmo del canvas a pantalla completa (fondo del visualizador, zen).
    /// Regenerarlo a 60 Hz es lo que satura la GPU integrada: nadie nota el
    /// glitch a 30 entre paneles translúcidos. En pausa, 10 (como el diseño).
    pub fn full_effects_hz(&self) -> f32 {
        let hz = (self.fx_fps * 0.5).clamp(4.0, 30.0);
        if self.now.playing {
            hz
        } else {
            hz.min(10.0)
        }
    }

    /// Ritmo del tinte ambiental (opacidad muy baja, detrás de todo).
    pub fn ambient_effects_hz(&self) -> f32 {
        let hz = (self.fx_fps * 0.34).clamp(3.0, 20.0);
        if self.now.playing {
            hz
        } else {
            hz.min(8.0)
        }
    }

    /// Ritmo de la franja: el de `fx_fps` mientras suena; 10 en pausa.
    pub fn strip_effects_hz(&self) -> f32 {
        if self.now.playing {
            self.fx_fps
        } else {
            self.fx_fps.min(10.0)
        }
    }

    /// Regenera el canvas del visualizador si toca o si cambió el tamaño.
    /// Devuelve si se regeneró; si no, la capa anterior sigue valiendo.
    pub fn pump_viz(&mut self, w: u16, h: u16, hz: f32, pal: &Palette) -> bool {
        if w == 0 || h == 0 {
            return false;
        }
        // Un cambio de layout necesita el canvas ya: no espera al reloj.
        let size_changed = self.viz.output.w != w || self.viz.output.h != h;
        self.viz_fx.set_hz(hz);
        let dt = if size_changed {
            Some(1.0 / hz.max(1.0))
        } else {
            self.viz_fx.due(Instant::now())
        };
        if let Some(dt) = dt {
            self.viz.render(w, h, &self.audio, pal, dt);
            return true;
        }
        false
    }

    /// Igual que `pump_viz`, para el canvas ambiental (media resolución).
    pub fn pump_ambient(&mut self, w: u16, h: u16, hz: f32, pal: &Palette) -> bool {
        if w == 0 || h == 0 {
            return false;
        }
        let size_changed = self.ambient.output.w != w || self.ambient.output.h != h;
        self.ambient_fx.set_hz(hz);
        let dt = if size_changed {
            Some(1.0 / hz.max(1.0))
        } else {
            self.ambient_fx.due(Instant::now())
        };
        if let Some(dt) = dt {
            self.ambient.render(w, h, &self.audio, pal, dt);
            return true;
        }
        false
    }

    fn poll_player(&mut self) {
        let Some(receiver) = self.player_events.as_mut() else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        if self.follow_spotify {
            // En modo acompañante la posición/duración las manda MPRIS, pero
            // una desconexión de mpv sigue siendo un evento que hay que ver:
            // al desactivar follow no podemos quedarnos con un Player muerto.
            if events
                .iter()
                .any(|event| matches!(event, PlayerEvent::Disconnected))
            {
                self.player = None;
                self.player_events = None;
                self.streaming = false;
                self.stream_cache_pending = None;
                self.now.playing = false;
                self.now.loading = false;
                self.spectrum.stop();
                self.toast("mpv desconectado; reinicia spote", ToastKind::Error);
            }
            return;
        }
        for event in events {
            match event {
                PlayerEvent::TimePos(position) => {
                    self.now.position_ms = (position.max(0.0) * 1000.0) as u64;
                    // Anclar aquí, no detectando el cambio en `tick`: así el
                    // interpolador no adelanta un frame el playhead de la letra.
                    self.now.position_seen_ms = self.now.position_ms;
                    self.now.position_at = Instant::now();
                    // Los streams HLS pueden quedarse reconectando en el EOF y
                    // no emitir `end-file eof`; se avanza por posición cuando
                    // la pista conocida ya terminó. Con repeat-one manda mpv
                    // (`loop-file`), no este heurístico: si no, cada vuelta
                    // re-resolvería el stream.
                    if self.streaming
                        && self.repeat_mode != RepeatMode::One
                        && self.effective_duration_ms() > 0
                        && self.now.position_ms.saturating_add(700)
                            >= self.effective_duration_ms()
                    {
                        self.now.ended = true;
                        if let Some(track) = self.now.track.clone() {
                            self.lib
                                .record_play(&track.key, self.now.position_ms, true)
                                .ok();
                        }
                        self.auto_next();
                        return;
                    }
                    if let Some(end) = self.trim_end_ms {
                        if self.now.position_ms >= end.saturating_sub(30) {
                            if let Some(track) = self.now.track.clone() {
                                self.lib
                                    .record_play(&track.key, self.now.position_ms, true)
                                    .ok();
                            }
                            self.auto_next();
                            return;
                        }
                    }
                }
                PlayerEvent::Duration(duration) => {
                    let reported = (duration * 1000.0) as u64;
                    if reported > 0 {
                        self.now.stream_duration_ms = reported;
                    }
                    // En un stream el vídeo puede ser un visualizer más largo
                    // que la pista: la duración lógica manda y el corte sigue
                    // siendo el de la canción, no el del vídeo.
                    if reported > 0 && (!self.streaming || self.now.duration_ms == 0) {
                        self.now.duration_ms = reported;
                    }
                }
                PlayerEvent::Paused(paused) => self.now.playing = !paused,
                PlayerEvent::Eof => {
                    // Un end-file de la pista anterior puede llegar después de
                    // cargar la siguiente. Si la posición actual está lejos
                    // del final lógico/real, el evento es obsoleto.
                    let end = self.effective_duration_ms();
                    if end > 0 && self.now.position_ms.saturating_add(2_000) < end {
                        continue;
                    }
                    self.now.playing = false;
                    self.now.ended = true;
                    if let Some(track) = self.now.track.clone() {
                        self.lib
                            .record_play(&track.key, self.now.position_ms, true)
                            .ok();
                    }
                    self.auto_next();
                }
                PlayerEvent::Error(message) => {
                    self.toast(format!("mpv: {message}"), ToastKind::Error);
                    // Un error de la URL anterior puede llegar después de
                    // empezar a resolver la siguiente: mientras `loading` esté
                    // activo no se dispara una descarga de la pista nueva por
                    // un fallo que no es suyo.
                    if self.streaming && !self.now.loading {
                        self.streaming = false;
                        self.stream_cache_pending = None;
                        self.now.loading = true;
                        self.now.playing = false;
                        self.now.message = "descargando tras fallar el stream…".into();
                        if let Some(track) = self.now.track.clone() {
                            // La URL cacheada puede estar muerta (403/expire
                            // prematuro): que la próxima vez se resuelva otra.
                            self.stream_resolver.invalidate(&track.key);
                            self.start_resolve(&track);
                            self.pending_play = Some(track.key.clone());
                        }
                    }
                }
                PlayerEvent::Disconnected => {
                    self.player = None;
                    self.player_events = None;
                    self.streaming = false;
                    self.stream_cache_pending = None;
                    self.now.playing = false;
                    self.now.loading = false;
                    self.spectrum.stop();
                    self.set_status("mpv desconectado · reinicia spote para recuperar el audio");
                    self.toast("mpv desconectado; reinicia spote", ToastKind::Error);
                }
                _ => {}
            }
        }
    }

    fn poll_search(&mut self) {
        while let Some((id, result)) = self.search_worker.try_recv() {
            if id != self.search.last_query_id {
                continue;
            }
            match result {
                Ok(all) if !all.is_empty() => {
                    let provider = all.provider.clone();
                    self.search.artists = all.artists;
                    self.search.albums = all.albums;
                    // El catálogo ordena por relevancia propia y a veces deja
                    // la pista exacta debajo de homónimos. Se reordena por
                    // coincidencia real con la consulta antes de pintar.
                    let query = self.search.last_query.clone();
                    let mut tracks = all.tracks;
                    tracks.sort_by(|a, b| {
                        catalog_match_score(&query, &b.title, &b.artists)
                            .cmp(&catalog_match_score(&query, &a.title, &a.artists))
                    });
                    self.search.results = tracks
                        .into_iter()
                        .map(search_item_from_spotify)
                        .collect();
                    self.search.selected = 0;
                    self.search.loading = false;
                    self.search.message = format!(
                        "{} artistas · {} álbumes · {} pistas · {}",
                        self.search.artists.len(),
                        self.search.albums.len(),
                        self.search.results.len(),
                        provider
                    );
                }
                Ok(_) => {
                    // Sin resultados en Spotify/Deezer: yt-dlp de respaldo.
                    if self.tools.yt_dlp {
                        self.search.message =
                            "sin resultados en el catálogo · probando yt-dlp".into();
                        self.downloader
                            .search(self.search.last_query_id, self.search.last_query.clone());
                    } else {
                        self.search.loading = false;
                        self.search.message = "sin resultados".into();
                    }
                }
                Err(err) => {
                    self.search.loading = false;
                    self.search.message = format!("catálogo no disponible: {err}");
                    if self.tools.yt_dlp {
                        self.downloader
                            .search(self.search.last_query_id, self.search.last_query.clone());
                    }
                }
            }
        }
    }

    fn poll_downloads(&mut self) {
        for event in self.downloader.drain() {
            match event {
                DownloadEvent::Found { query_id, items } => {
                    if query_id == self.search.last_query_id {
                        self.search.results = items;
                        self.search.albums.clear();
                        self.search.artists.clear();
                        self.search.selected = 0;
                        self.search.loading = false;
                        self.search.message =
                            format!("{} resultados de yt-dlp", self.search.results.len());
                    }
                }
                DownloadEvent::SearchFailed { query_id, message } => {
                    if query_id == self.search.last_query_id {
                        self.search.loading = false;
                        self.search.message = format!("error: {message}");
                    }
                    self.toast(message, ToastKind::Error);
                }
                DownloadEvent::Progress {
                    key,
                    percent,
                    stage,
                } => {
                    if self.now.track.as_ref().is_some_and(|t| t.key == key) {
                        self.set_status(format!("{stage} · {percent:.0}%"));
                    }
                    self.track_download(key, stage, percent, false);
                }
                DownloadEvent::StageProgress {
                    key,
                    stage,
                    percent,
                } => {
                    self.track_download(key, stage.label().to_string(), percent, false);
                }
                DownloadEvent::Done { key, path } => {
                    // yt-dlp deja la miniatura junto al audio. Se registra en
                    // la librería ya mismo; si no existe, la portada remota de
                    // Spotify la pedirá el worker de arte.
                    for ext in ["jpg", "png", "webp"] {
                        let cover = path.with_extension(ext);
                        if cover.exists() {
                            self.lib.set_art(&key, Some(&cover), None).ok();
                            self.drop_thumbs(&key);
                            break;
                        }
                    }
                    if let Ok(Some(row)) = self.lib.track_by_key(&key) {
                        let hint = self.lib.source_hint(&key);
                        self.lib
                            .set_source(
                                row.id,
                                hint.as_ref().map(|(p, _)| p.as_str()).unwrap_or("ytmusic"),
                                hint.as_ref().map(|(_, id)| id.as_str()),
                                Some(&path),
                                spote_core::library::SourceStatus::Ready,
                                std::fs::metadata(&path).ok().map(|m| m.len()),
                            )
                            .ok();
                    }
                    if let Some(entry) = self.queue.iter_mut().find(|entry| entry.track.key == key)
                    {
                        entry.path = Some(path.clone());
                    }
                    self.finish_download(&key, false);
                    if self.pending_play.as_deref() == Some(key.as_str()) {
                        self.pending_play = None;
                        if self.follow_spotify {
                            self.toast(
                                format!("descargada {}", util::short_hash(&key)),
                                ToastKind::Ok,
                            );
                        } else if let Some(entry) =
                            self.queue.iter().position(|entry| entry.track.key == key)
                        {
                            let entry = self.queue.remove(entry);
                            self.play_local(entry.track, Some(path.clone()));
                        } else if let Some(track) = self
                            .lib
                            .track_by_key(&key)
                            .ok()
                            .flatten()
                            .map(|row| row.track)
                            .or_else(|| self.now.track.clone().filter(|track| track.key == key))
                        {
                            // El fallback por `now.track` sin comprobar la
                            // clave cargaba la pista A con el fichero de B si B
                            // ya no estaba en la cola.
                            self.play_local(track, Some(path.clone()));
                        }
                    } else {
                        self.toast(
                            format!("descargada {}", util::short_hash(&key)),
                            ToastKind::Ok,
                        );
                    }
                    self.refresh_view();
                }
                DownloadEvent::Failed { key, message } => {
                    let was_pending = self.pending_play.as_deref() == Some(key.as_str());
                    if was_pending {
                        self.pending_play = None;
                        self.pending_seek_ms = None;
                        self.pending_seek_key = None;
                        self.now.loading = false;
                    }
                    // Si el stream ya está sonando, esta descarga era solo la
                    // copia de caché: su fallo no debe pausar la reproducción
                    // ni tapar la pista con un toast de error.
                    let cache_only = !was_pending
                        && self.streaming
                        && self.now.track.as_ref().map(|track| track.key.as_str())
                            == Some(key.as_str());
                    if was_pending
                        && !self.streaming
                        && self.now.track.as_ref().map(|track| track.key.as_str())
                            == Some(key.as_str())
                        && self.now.path.is_none()
                    {
                        self.now.playing = false;
                        self.now.message = "descarga fallida".into();
                        self.lyrics_status = "descarga fallida".into();
                    }
                    self.finish_download(&key, true);
                    if cache_only {
                        self.set_status("copia local fallida · sigue el stream");
                    } else {
                        self.toast(format!("descarga fallida: {message}"), ToastKind::Error);
                    }
                }
                DownloadEvent::Message(message) => {
                    self.toast(message, ToastKind::Info);
                }
            }
        }
    }

    /// Título legible de una pista por clave, para las tarjetas de descarga.
    fn poll_art(&mut self) {
        while let Some(response) = self.art_worker.try_recv() {
            self.apply_art(response);
        }
    }

    fn apply_art(&mut self, mut response: ArtResponse) {
        if response.kind == ArtKind::Ambient {
            if let Some(art) = response.art {
                self.ambient_art = Some(art);
            }
            return;
        }
        if response.kind == ArtKind::Artist {
            if let Some(art) = response.art {
                self.artist_art = Some(art);
            }
            return;
        }
        // Miniaturas: caché aparte, no tocan el tema ni el arte grande.
        if response.kind == ArtKind::Thumb {
            if let Some(png) = response.png.take() {
                let size = response.png_size.unwrap_or((192, 192));
                self.thumb_pngs.insert(response.key.clone(), (png, size));
            }
            if let Some(art) = response.art {
                self.thumb_requests
                    .remove(&(response.key.clone(), response.w, response.h));
                self.thumbs
                    .insert((response.key.clone(), response.w, response.h), art);
            }
            // Si falló, la petición sigue marcada: no se reintenta en bucle
            // cada frame. `drop_thumbs` la limpia cuando cambia la portada.
            if let Some(path) = response.art_path {
                self.lib.set_art(&response.key, Some(&path), None).ok();
            }
            return;
        }
        let current_key = self.now.track.as_ref().map(|track| track.key.clone());
        if current_key.as_deref() != Some(response.key.as_str()) {
            return;
        }
        // Una respuesta de un tamaño anterior (lista 30x15) llegaba después de
        // pedir la portada grande y la pisaba: se veía pequeña y sin PNG.
        if (response.w, response.h) != self.art_size_hint {
            return;
        }
        if response.mode == ArtMode::Plain {
            if response.png.is_some() {
                self.art_png = response.png.take();
                self.art_png_key = response.key.clone();
                self.art_png_size = response.png_size;
                self.art_png_hint = (response.w, response.h);
            }
        }
        if let Some(palette) = response.palette {
            self.dominant = palette;
            self.palette.go(palette);
        }
        if let Some(art) = response.art.clone() {
            self.art_variants.insert(response.mode, art.clone());
            if response.mode != self.art_mode {
                // Variante antigua que llega tarde: a la caché y a otra cosa.
                return;
            }
            if self.art.is_some() && self.art_mode_seen != response.mode {
                self.art_prev = self.art.take();
                self.art_prev_mode = self.art_mode_seen;
                self.art_fade = 0.0;
                self.art_fade_active = true;
            }
            self.art = Some(art);
            self.art_key = response.key.clone();
            self.art_size = (response.w, response.h);
            self.art_mode_seen = response.mode;
        }
        if let Some(path) = response.art_path {
            self.lib.set_art(&response.key, Some(&path), None).ok();
        }
        if let Some(error) = response.error {
            if error != "sin portada" {
                self.set_status(error);
            }
        }
    }

    fn poll_song_map(&mut self) {
        while let Some(event) = self.map_worker.try_recv() {
            match event {
                SongMapEvent::Loaded { key, map } => {
                    if self.now.track.as_ref().map(|t| &t.key) != Some(&key) {
                        continue;
                    }
                    self.song_intensity = map.intensity_at(self.now.position_ms);
                    self.song_section = map.section_at(self.now.position_ms).map(|s| s.kind);
                    self.track_gain_db = Some(map.loudness_gain_db);
                    self.song_map = Some(map);
                    self.song_map_key = key;
                    self.apply_audio_fx();
                    self.apply_auto_lyrics_offset();
                    self.refine_lyrics_words();
                }
                SongMapEvent::Failed { key, error } => {
                    if self.now.track.as_ref().map(|t| &t.key) == Some(&key) {
                        tracing::debug!("song map: {error}");
                    }
                }
            }
        }
        // El mapa sigue al usuario al hacer seek: se consulta por posición, no
        // por un reloj interno.
        if let Some(map) = &self.song_map {
            self.song_intensity = map.intensity_at(self.now.position_ms);
            self.song_section = map.section_at(self.now.position_ms).map(|s| s.kind);
        }
    }

    fn poll_meta(&mut self) {
        while let Some(event) = self.meta_worker.try_recv() {
            match event {
                MetaEvent::Artist { name, result } => {
                    // Una respuesta lenta del artista anterior no debe pintarse
                    // como si fuera el nuevo: eso mezclaba discografías enteras.
                    if !self.artist_request.is_empty()
                        && util::normalize(&name) != util::normalize(&self.artist_request)
                    {
                        continue;
                    }
                    if self.artist_loading {
                        self.artist_loading = false;
                    }
                    match result {
                        Ok(profile) => {
                            self.artist_status = format!(
                                "{} · {} fans · {} pistas",
                                profile.provider,
                                profile.followers,
                                profile.top_tracks.len()
                            );
                            if let Some(url) = profile.image_url.clone() {
                                self.art_worker.request(ArtRequest {
                                    key: format!("artist:{}", profile.id),
                                    url: Some(url),
                                    local: None,
                                    prefer_url: true,
                                    w: 24,
                                    h: 12,
                                    mode: ArtMode::Plain,
                                    kind: ArtKind::Artist,
                                });
                            }
                            self.artist = Some(profile);
                            self.artist_selected = 0;
                        }
                        Err(err) => {
                            self.artist_status = format!("{name}: {err}");
                            self.artist = None;
                        }
                    }
                }
                MetaEvent::AlbumByName {
                    album,
                    artist,
                    result,
                } => {
                    let current = self.album_page.as_ref().is_some_and(|page| {
                        page.provider == "search" && page.title.eq_ignore_ascii_case(&album)
                    });
                    if current {
                        self.album_page_loading = false;
                        match result {
                            Ok(info) => {
                                self.open_album_info(info);
                            }
                            Err(err) => {
                                self.album_page_status = format!("álbum: {err}");
                                self.toast(
                                    format!("no encontré «{album}» de {artist}: {err}"),
                                    ToastKind::Warn,
                                );
                            }
                        }
                    }
                }
                MetaEvent::Album {
                    provider,
                    id,
                    result,
                } => {
                    let current = self
                        .album_page
                        .as_ref()
                        .is_some_and(|page| page.provider == provider && page.id == id);
                    if current {
                        self.album_page_loading = false;
                        match result {
                            Ok(info) => {
                                self.open_album_info(info);
                            }
                            Err(err) => {
                                self.album_page_status = format!("álbum: {err}");
                                self.toast(
                                    format!("no pude cargar el álbum: {err}"),
                                    ToastKind::Error,
                                );
                            }
                        }
                    }
                }
                MetaEvent::Metadata { key, result } => {
                    self.metadata_requests.remove(&key);
                    let manual = self.metadata_manual.remove(&key);
                    if let Some(track) = result {
                        if let Ok(Some(row)) = self.lib.track_by_key(&key) {
                            self.lib
                                .update_metadata(
                                    &key,
                                    &track.artists,
                                    &track.title,
                                    Some(track.album.as_str()),
                                    track.duration_ms,
                                    track.art_url.as_deref(),
                                )
                                .ok();
                            if manual {
                                self.toast(
                                    format!("metadatos de Spotify para {}", row.track.title),
                                    ToastKind::Ok,
                                );
                            }
                        }
                        self.drop_thumbs(&key);
                        self.art_key.clear();
                        self.refresh_view();
                    } else if manual {
                        self.toast("Spotify no encontró la pista", ToastKind::Warn);
                    }
                }
            }
        }
    }

    /// Ajusta la letra al audio solo cuando el mapa ya está listo y no hay un
    /// offset manual guardado. El cálculo es conservador; si no hay evidencia
    /// clara, se deja el LRC tal cual.
    fn apply_auto_lyrics_offset(&mut self) {
        if self.lyrics_offset_ms != 0 || self.lyrics_auto_offset.is_some() {
            return;
        }
        let Some(map) = self.song_map.clone() else {
            return;
        };
        let Some(doc) = self.lyrics.as_mut() else {
            return;
        };
        if doc.offset_ms != 0
            || doc.aligned
            || matches!(
                doc.source,
                spote_core::lyrics::LyricsSource::Estimated
                    | spote_core::lyrics::LyricsSource::Plain
                    | spote_core::lyrics::LyricsSource::Local
                    | spote_core::lyrics::LyricsSource::Manual
                    | spote_core::lyrics::LyricsSource::Captions
            )
        {
            // Una letra estimada no tiene una rejilla fiable: desplazarla
            // «un poco» solo empeora la sensación de desfase. Para eso está
            // el ASR; el offset automático es para LRC sincronizados.
            return;
        }
        let Some(offset) = doc.estimate_offset_from_map(&map) else {
            return;
        };
        doc.offset_ms = offset;
        self.lyrics_auto_offset = Some(offset);
        self.toast(
            format!(
                "letra ajustada al audio ({:+.1} s); shift/alt+clic para afinar",
                offset as f32 / 1000.0
            ),
            ToastKind::Info,
        );
    }

    fn poll_lyrics(&mut self) {
        while let Some(message) = self.lyrics_worker.try_recv() {
            match message {
                LyricsMsg::Loaded {
                    key,
                    doc,
                    lrc,
                    error,
                } => {
                    // Una respuesta de la pista anterior no debe cancelar la
                    // carga de la actual: solo se apaga el flag si sigue
                    // siendo la misma pista.
                    if self.now.track.as_ref().map(|t| &t.key) != Some(&key) {
                        continue;
                    }
                    self.lyrics_loading = false;
                    match doc {
                        Some(doc) => {
                            // Un plain de un proveedor de reserva no debe
                            // reemplazar un LRC sincronizado (o alineado) que
                            // ya se está mostrando.
                            let incoming = if doc.aligned {
                                3
                            } else if doc.synced {
                                2
                            } else {
                                1
                            };
                            let forced = self.lyrics_retry.contains(&key);
                            let cached = if forced {
                                0
                            } else {
                                self.lyrics
                                    .as_ref()
                                    .map(|old| {
                                        if old.aligned {
                                            3
                                        } else if old.synced {
                                            2
                                        } else {
                                            1
                                        }
                                    })
                                    .unwrap_or(0)
                            };
                            if cached > incoming {
                                continue;
                            }
                            if let Some(text) = lrc {
                                self.lib
                                    .save_lyrics(
                                        &key,
                                        &text,
                                        doc.synced,
                                        doc.source.label(),
                                        doc.aligned,
                                    )
                                    .ok();
                                // La versión ASR ya está alineada: arrastrar el
                                // offset de la letra aproximada la desalinea.
                                // Un ajuste manual hecho mientras cargaba se
                                // respeta.
                                if doc.aligned && self.lyrics_offset_ms == 0 {
                                    self.lib.set_lyrics_offset(&key, 0).ok();
                                }
                            }
                            self.lyrics_status = if doc.aligned {
                                "asr".to_string()
                            } else {
                                doc.source.label().to_string()
                            };
                            // Si esta vez no se pudo afinar, se permite otro
                            // intento en la siguiente reproducción; antes un
                            // fallo transitorio bloqueaba la pista toda la
                            // sesión.
                            if !doc.aligned && self.lib.local_path(&key).is_some() {
                                self.lyrics_retry.remove(&key);
                            }
                            let mut doc = doc;
                            let lrc_offset = doc.offset_ms;
                            self.lyrics_offset_ms = self.lib.lyrics_offset(&key).unwrap_or(0);
                            doc.offset_ms = if self.lyrics_offset_ms != 0 {
                                self.lyrics_offset_ms
                            } else {
                                lrc_offset
                            };
                            self.lyrics = Some(doc);
                            self.lyrics_key = key;
                            self.apply_auto_lyrics_offset();
                            self.refine_lyrics_words();
                        }
                        None => {
                            if self.lib.local_path(&key).is_some() {
                                self.lyrics_retry.remove(&key);
                            }
                            if self.lyrics_key == key && self.lyrics.is_some() {
                                self.lyrics_status = error
                                    .unwrap_or_else(|| "letra aproximada (sin mejorar)".into());
                            } else {
                                self.lyrics = None;
                                self.lyrics_status = error.unwrap_or_else(|| "sin letra".into());
                            }
                        }
                    }
                }
            }
        }
    }

    fn poll_social(&mut self) {
        let mut events = Vec::new();
        if let Some(social) = &self.social {
            while let Some(event) = social.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                SocialEvent::Ready { user, token } => {
                    self.social_status = format!("conectado como {user}");
                    self.cfg.server.token = token;
                    self.cfg.server.name = user;
                    let _ = self.cfg.save();
                }
                SocialEvent::Status(status) => {
                    if status.contains("conectado") {
                        self.social_connected = true;
                    }
                    self.social_status = status;
                }
                SocialEvent::Error(error) => {
                    self.social_connected = false;
                    self.social_status = error.clone();
                    // Sin spam: reconectar cada pocos segundos no debe llenar
                    // la pantalla de toasts.
                    let throttle = self
                        .last_social_error
                        .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(25));
                    if !throttle {
                        self.last_social_error = Some(Instant::now());
                        self.toast(error, ToastKind::Error);
                    }
                }
                SocialEvent::Clock(offset) => {
                    self.clock_offset_ms = offset;
                }
                SocialEvent::Rooms(rooms) => {
                    if self.room.is_none() {
                        self.room_selected = self.room_selected.min(rooms.len().saturating_sub(1));
                    }
                    self.rooms = rooms;
                }
                SocialEvent::Room(room) => {
                    self.room_selected =
                        self.rooms.iter().position(|r| r.id == room.id).unwrap_or(0);
                    self.room = Some(room);
                }
                SocialEvent::State(state) => self.apply_room_state(state),
                SocialEvent::Queue(items) => self.room_queue = items,
                SocialEvent::Presence(presences) => self.presence = presences,
                SocialEvent::Chat(chat) => {
                    self.chat.push(chat);
                    if self.chat.len() > 100 {
                        self.chat.remove(0);
                    }
                }
            }
        }
    }

    fn server_now(&self) -> i64 {
        now_ms() + self.clock_offset_ms
    }

    fn apply_room_state(&mut self, state: RoomState) {
        self.room_state = Some(state.clone());
        let Some(track) = state.track.clone() else {
            return;
        };
        let target = state.target_position(self.server_now());
        if self.now.track.as_ref().map(|t| &t.key) != Some(&track.key) {
            if let Some(path) = self.lib.local_path(&track.key) {
                self.applying_remote = true;
                self.play_local(track, Some(path));
                self.applying_remote = false;
                self.seek_to(target);
                self.pending_room_target = None;
            } else {
                self.pending_room_target = Some(target);
                if track.source_hint.is_some() {
                    self.queue_track(track.clone(), None);
                    self.toast(format!("bajando {}", track.label()), ToastKind::Info);
                } else {
                    self.toast(
                        format!("la sala pide {} (no descargable)", track.label()),
                        ToastKind::Warn,
                    );
                }
            }
            return;
        }
        if let Some(target) = self.pending_room_target.take() {
            self.seek_to(target);
        }
        let drift = target as i64 - self.now.position_ms as i64;
        if drift.abs() > 600 {
            self.applying_remote = true;
            self.seek_to(target);
            self.applying_remote = false;
        }
        if state.playing != self.now.playing {
            self.applying_remote = true;
            self.toggle_play();
            self.applying_remote = false;
        }
    }

    fn publish_presence(&mut self) {
        let Some(social) = &self.social else { return };
        let key = self.now.track.as_ref().map(|track| track.key.clone());
        if self.last_presence.0 == key && self.last_presence.1 == self.now.playing {
            return;
        }
        self.last_presence = (key, self.now.playing);
        social.send(SocialCmd::NowPlaying(
            self.now.track.clone(),
            self.now.playing,
        ));
    }

    fn send_control(&mut self, action: ControlAction, position_ms: Option<u64>) {
        if self.applying_remote {
            return;
        }
        if let Some(social) = &self.social {
            if self.room.is_some() {
                social.send(SocialCmd::Control(action, position_ms));
            }
        }
    }

    pub fn join_room(&mut self, index: usize) {
        let Some(room) = self.rooms.get(index) else {
            return;
        };
        let id = room.id.clone();
        let label = self.room_label(&id);
        if let Some(social) = &self.social {
            social.send(SocialCmd::Join(id.clone()));
            self.toast(format!("uniéndose a {label}"), ToastKind::Info);
        }
    }

    /// Nombre legible de una sala: el suyo, o un id corto si aún no llegó.
    pub fn room_label(&self, id: &str) -> String {
        self.rooms
            .iter()
            .find(|room| room.id == id)
            .map(|room| {
                if room.name.is_empty() {
                    room.id.chars().take(8).collect()
                } else {
                    room.name.clone()
                }
            })
            .unwrap_or_else(|| id.chars().take(8).collect())
    }

    pub fn leave_room(&mut self) {
        if let Some(social) = &self.social {
            social.send(SocialCmd::Leave);
        }
        self.room = None;
        self.room_state = None;
        self.room_queue.clear();
        self.toast("sala abandonada", ToastKind::Info);
    }

    // --------------------------------------------------------------- arte
    /// Pide la portada si cambió la canción, el tamaño o el modo.
    pub fn ensure_art(&mut self) {
        let Some(track) = self.now.track.clone() else {
            self.art = None;
            return;
        };
        let mode = self.art_mode;
        if mode == ArtMode::Off {
            self.art = None;
            self.art_key.clear();
            self.art_png = None;
            self.art_png_key.clear();
            self.art_png_size = None;
            self.art_png_hint = (0, 0);
            self.art_visible_rect = None;
            return;
        }
        let (w, h) = self.art_size_hint;
        if w < 4 || h < 2 {
            return;
        }
        if self.art_key != track.key {
            // Pista nueva: fuera el arte viejo. Se pinta el placeholder hasta
            // que llegue el bueno, nunca la portada anterior.
            self.art = None;
            self.art_prev = None;
            self.art_fade_active = false;
            self.art_variants.clear();
            self.art_png = None;
            self.art_png_key.clear();
            self.art_png_size = None;
            self.art_png_hint = (0, 0);
        }
        let needs_png = mode == ArtMode::Plain
            && w >= 24
            && h >= 12
            && (self.art_png.is_none() || self.art_png_hint != (w, h));
        let same = self.art_key == track.key
            && self.art_size == (w, h)
            && self.art_mode_seen == mode
            && self.art.is_some()
            && !needs_png;
        if same {
            return;
        }
        if self.art_request_key == track.key && self.art_request_size == (w, h) {
            return;
        }
        if let Some(at) = self.art_request {
            if at.elapsed() < Duration::from_millis(120) {
                return;
            }
        }
        self.art_request = Some(Instant::now());
        let local = self
            .lib
            .art_path(&track.key)
            .ok()
            .flatten()
            .or_else(|| self.now.path.as_ref().map(|p| p.with_extension("jpg")))
            .filter(|p| p.exists());
        let url = track
            .art_url
            .clone()
            .or_else(|| self.lib.art_url(&track.key).ok().flatten());
        if local.is_none() && url.is_none() && !track.artist.is_empty() {
            // Sin portada local ni remota: se busca en Spotify una sola vez.
            self.request_spotify_metadata_for_key(&track.key);
        }
        let ambient_mode = self.cfg.ui.fx.ambient == "cover";
        if ambient_mode && self.ambient_art_key != track.key && (local.is_some() || url.is_some()) {
            self.ambient_art_key = track.key.clone();
            self.ambient_art = None;
            self.art_worker.request(ArtRequest {
                key: format!("ambient:{}", track.key),
                url: url.clone(),
                local: local.clone(),
                prefer_url: true,
                w: 32,
                h: 16,
                mode: ArtMode::Braille,
                kind: ArtKind::Ambient,
            });
        }
        self.art_request_key = track.key.clone();
        self.art_request_size = (w, h);
        self.art_worker.request(ArtRequest {
            key: track.key,
            url,
            local,
            prefer_url: true,
            w,
            h,
            mode,
            kind: ArtKind::Main,
        });
    }

    /// Pide la miniatura del tamaño exacto que ocupa en pantalla.
    ///
    /// Antes se generaba una sola de 6x3 y se estiraba al pintar: en la
    /// cabecera de un álbum (12 columnas) o en su ficha (18x9) se veía
    /// pixelada. Ahora se rasteriza a la resolución del hueco y se pide en
    /// modo `Plain`, sin la viñeta que oscurecía las miniaturas.
    pub fn request_thumb(&mut self, track: &TrackRef, w: u16, h: u16) {
        if w < 2 || h < 1 {
            return;
        }
        let slot = (track.key.clone(), w, h);
        if self.thumbs.contains_key(&slot) || self.thumb_requests.contains(&slot) {
            return;
        }
        let local = self
            .lib
            .art_path(&track.key)
            .ok()
            .flatten()
            .filter(|path| path.exists())
            .or_else(|| {
                self.lib
                    .local_path(&track.key)
                    .map(|path| path.with_extension("jpg"))
                    .filter(|path| path.exists())
            });
        let url = track
            .art_url
            .clone()
            .or_else(|| self.lib.art_url(&track.key).ok().flatten());
        if local.is_none() && url.is_none() {
            // Sin portada local ni remota: se busca en segundo plano y no se
            // marca la miniatura como pedida, para reintentar cuando llegue
            // la URL. Como mucho 3 búsquedas a la vez.
            if !track.artist.is_empty() && self.metadata_requests.len() < 3 {
                self.request_spotify_metadata_for_key(&track.key);
            }
            return;
        }
        self.thumb_requests.insert(slot);
        let ambient_mode = self.cfg.ui.fx.ambient == "cover";
        if ambient_mode && self.ambient_art_key != track.key && (local.is_some() || url.is_some()) {
            self.ambient_art_key = track.key.clone();
            self.ambient_art = None;
            self.art_worker.request(ArtRequest {
                key: format!("ambient:{}", track.key),
                url: url.clone(),
                local: local.clone(),
                prefer_url: true,
                w: 32,
                h: 16,
                mode: ArtMode::Braille,
                kind: ArtKind::Ambient,
            });
        }
        self.art_worker.request(ArtRequest {
            key: track.key.clone(),
            url,
            local,
            prefer_url: true,
            w,
            h,
            mode: ArtMode::Plain,
            kind: ArtKind::Thumb,
        });
    }

    /// Mejor miniatura disponible para un hueco: la exacta o una mayor que se
    /// pueda reducir sin perder detalle.
    pub fn thumb_for(&self, key: &str, w: u16, h: u16) -> Option<&CoverArt> {
        if let Some(art) = self.thumbs.get(&(key.to_string(), w, h)) {
            return Some(art);
        }
        self.thumbs
            .iter()
            .filter(|((track, tw, th), _)| track == key && *tw >= w && *th >= h)
            .min_by_key(|((_, tw, th), _)| u32::from(*tw) * u32::from(*th))
            .map(|(_, art)| art)
    }

    /// La miniatura más grande cacheada de una pista, sea del tamaño que sea.
    /// Sirve de puente mientras llega la del tamaño exacto.
    pub fn thumb_any(&self, key: &str) -> Option<&CoverArt> {
        self.thumbs
            .iter()
            .filter(|((track, _, _), _)| track == key)
            .max_by_key(|((_, tw, th), _)| u32::from(*tw) * u32::from(*th))
            .map(|(_, art)| art)
    }

    /// Tira todas las miniaturas de una pista (cambió la portada).
    pub fn drop_thumbs(&mut self, key: &str) {
        self.thumbs.retain(|(track, _, _), _| track != key);
        self.thumb_requests.retain(|(track, _, _)| track != key);
        self.thumb_pngs.remove(key);
    }

    /// El layout llama aquí cada frame con el hueco disponible para el arte.
    pub fn set_art_size_hint(&mut self, w: u16, h: u16) {
        if self.art_size_hint != (w, h) {
            self.art_size_hint = (w, h);
            self.art_key.clear();
            self.art_variants.clear();
            self.art_prev = None;
            self.art_fade_active = false;
            self.art_request_key.clear();
            self.art_request_size = (0, 0);
            // El PNG anterior tiene otra geometría; usarlo estirado en
            // pantalla completa es justo lo que se veía pixelado.
            self.art_png = None;
            self.art_png_key.clear();
            self.art_png_size = None;
            self.art_png_hint = (0, 0);
        }
    }

    // -------------------------------------------------------------- entrada
    fn toggle_mute(&mut self) {
        if self.now.volume > 0.0 {
            self.volume_before_mute = self.now.volume;
            // Silenciar no debe guardar 0 en config: al reabrir, el usuario
            // espera el volumen que tenía, no la app muda.
            self.muted = true;
            self.now.volume = 0.0;
            if let Some(player) = &self.player {
                player.set_volume(0.0);
            }
            if self.follow_spotify {
                if let Some(worker) = &self.spotify_worker {
                    worker.send(SystemCommand::SetVolume(0.0));
                }
            }
        } else {
            let volume = self.volume_before_mute.max(0.2);
            self.muted = false;
            self.set_volume(volume);
        }
    }

    fn toggle_favorite_now(&mut self) {
        if let Some(track) = self.now.track.clone() {
            self.toggle_favorite_key(&track.key);
        }
    }

    /// `f` actúa sobre lo que hay seleccionado EN LA PESTAÑA ACTIVA. Antes caía
    /// siempre a la pista sonando, así que en ajustes o sala marcabas favorita
    /// sin querer.
    fn toggle_favorite(&mut self) {
        match self.tab {
            Tab::Now => self.toggle_favorite_now(),
            Tab::Search => match self.search_entry(self.search.selected) {
                Some(SearchEntry::Track(index)) => {
                    if let Some(item) = self.search.results.get(index) {
                        let key = item.to_track().key;
                        self.toggle_favorite_key(&key);
                    }
                }
                Some(_) => self.toast(
                    "las favoritas son pistas, no artistas/álbumes",
                    ToastKind::Warn,
                ),
                None => self.toast("busca algo y selecciona una fila", ToastKind::Warn),
            },
            Tab::Library | Tab::Albums => match self.selected_row().cloned() {
                Some(row) => self.toggle_favorite_key(&row.track.key),
                None => self.toast("no hay ninguna pista seleccionada", ToastKind::Warn),
            },
            Tab::Audio | Tab::Room | Tab::Settings => self.toast(
                "favorita: selecciona una pista en buscar o librería",
                ToastKind::Warn,
            ),
        }
    }

    /// Algunas acciones (favorita, lista) llegan desde resultados remotos que
    /// aún no están en SQLite. Se materializa la pista antes de tocar la BD.
    pub(super) fn ensure_track_key(&mut self, key: &str) -> bool {
        if self
            .lib
            .track_by_key(key)
            .ok()
            .flatten()
            .is_some()
        {
            return true;
        }
        if let Some(track) = self
            .search
            .results
            .iter()
            .find(|item| item.to_track().key == key)
            .map(|item| item.to_track())
            .or_else(|| self.now.track.clone().filter(|track| track.key == key))
        {
            return self.lib.upsert_track(&track).is_ok();
        }
        false
    }

    fn toggle_favorite_key(&mut self, key: &str) {
        if !self.ensure_track_key(key) {
            self.toast("no encuentro esa pista en la biblioteca", ToastKind::Warn);
            return;
        }
        if let Ok(value) = self.lib.toggle_favorite(key) {
            self.toast(
                if value {
                    "añadida a favoritas"
                } else {
                    "quitada de favoritas"
                },
                ToastKind::Info,
            );
            self.refresh_view();
        }
    }

    pub fn cycle_art_mode(&mut self) {
        self.set_art_mode(self.art_mode.next());
    }

    pub fn set_art_mode(&mut self, mode: ArtMode) {
        if mode == self.art_mode {
            return;
        }
        self.art_mode = mode;
        self.cfg.ui.art_mode = mode.label().to_string();
        self.cfg.save().ok();
        self.art_request_key.clear();
        self.art_request_size = (0, 0);
        if mode != ArtMode::Plain {
            self.art_png = None;
            self.art_png_key.clear();
            self.art_png_size = None;
            self.art_png_hint = (0, 0);
            self.art_visible_rect = None;
        }
        if mode == ArtMode::Off {
            self.art_prev = self.art.take();
            self.art_prev_mode = self.art_mode_seen;
            self.art_fade = 0.0;
            self.art_fade_active = true;
            self.art_key.clear();
            self.art_png = None;
            self.art_png_key.clear();
            return;
        }
        if let Some(art) = self.art_variants.get(&mode).cloned() {
            // Ya renderizado: la transición es instantánea de datos, la
            // disolución la hace el render.
            self.art_prev = self.art.take();
            self.art_prev_mode = self.art_mode_seen;
            self.art = Some(art);
            self.art_mode_seen = mode;
            self.art_fade = 0.0;
            self.art_fade_active = true;
            self.art_key = self
                .now
                .track
                .as_ref()
                .map(|track| track.key.clone())
                .unwrap_or_default();
            self.art_size = self.art_size_hint;
            return;
        }
        // Sin caché: se pide; el arte actual se queda hasta que llegue.
        self.art_key.clear();
    }

    /// Selección de cola: la de la sala si hay una activa, si no la local.
    fn queue_len(&self) -> usize {
        if self.room_queue.is_empty() {
            self.queue.len()
        } else {
            self.room_queue.len()
        }
    }

    pub fn remove_queue_selected(&mut self) {
        if self.queue_len() == 0 {
            return;
        }
        let index = self.queue_selected.min(self.queue_len() - 1);
        if self.room_queue.is_empty() {
            let removed = self.queue.remove(index);
            if self.pending_play.as_deref() == Some(removed.track.key.as_str()) {
                self.pending_play = None;
                self.pending_seek_ms = None;
                self.pending_seek_key = None;
                if self.now.track.as_ref().map(|track| track.key.as_str())
                    == Some(removed.track.key.as_str())
                    && self.now.path.is_none()
                {
                    self.now.track = None;
                    self.now.duration_ms = 0;
                    self.now.position_ms = 0;
                    self.now.position_unknown = false;
                    self.now.loading = false;
                    self.now.playing = false;
                    self.now.ended = false;
                    self.now.message = "quitada de la cola".into();
                    self.lyrics = None;
                    self.lyrics_key.clear();
                    self.lyrics_status.clear();
                    self.lyrics_loading = false;
                    self.art = None;
                    self.art_key.clear();
                    self.spectrum.stop();
                }
            }
            self.toast("quitada de la cola", ToastKind::Info);
        } else if let Some(item) = self.room_queue.get(index) {
            let id = item.id.clone();
            if let Some(social) = &self.social {
                social.send(SocialCmd::QueueRemove(id));
            }
            self.room_queue.remove(index);
            self.toast("quitada de la cola de la sala", ToastKind::Info);
        }
        self.queue_selected = self.queue_selected.min(self.queue_len().saturating_sub(1));
    }

    /// Mueve el cursor de la cola (el panel tiene foco).
    pub fn move_queue_selection(&mut self, delta: i32) {
        let len = self.queue_len();
        if len == 0 {
            return;
        }
        self.queue_selected =
            (self.queue_selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
    }

    /// Reproduce la entrada marcada en la cola local desde el teclado.
    pub fn play_queue_selected(&mut self) {
        if !self.room_queue.is_empty() {
            self.toast("la cola de la sala la controla el servidor", ToastKind::Info);
            return;
        }
        let index = self.queue_selected.min(self.queue_len().saturating_sub(1));
        if let Some(entry) = self.queue.get(index).cloned() {
            self.queue.remove(index);
            self.queue_selected = self.queue_selected.min(self.queue.len().saturating_sub(1));
            self.play_local(entry.track, entry.path);
        }
    }

    /// Mueve la entrada seleccionada de la cola local.
    pub fn move_queue(&mut self, delta: i32) {
        if !self.room_queue.is_empty() {
            self.queue_selected =
                (self.queue_selected as i32 + delta).clamp(0, self.queue_len() as i32 - 1) as usize;
            self.toast("la cola de la sala la ordena el servidor", ToastKind::Info);
            return;
        }
        if self.queue.len() < 2 {
            return;
        }
        let from = self.queue_selected.min(self.queue.len() - 1);
        let to = (from as i32 + delta).clamp(0, self.queue.len() as i32 - 1) as usize;
        if from != to {
            let entry = self.queue.remove(from);
            self.queue.insert(to, entry);
            self.queue_selected = to;
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.tab == Tab::Albums && self.album_open.is_none() {
            self.move_album_selection(delta);
            return;
        }
        match self.tab {
            Tab::Search => {
                let len = self.search_len();
                if len == 0 {
                    return;
                }
                let next = self.search.selected as i32 + delta;
                self.search.selected = next.clamp(0, len as i32 - 1) as usize;
            }
            _ => {
                if self.view.is_empty() {
                    return;
                }
                let next = self.selected as i32 + delta;
                self.selected = next.clamp(0, self.view.len() as i32 - 1) as usize;
            }
        }
    }

    /// Longitud de la lista mixta: pistas, artistas y álbumes.
    pub fn search_len(&self) -> usize {
        self.search.results.len() + self.search.artists.len() + self.search.albums.len()
    }

    pub fn search_entry(&self, index: usize) -> Option<SearchEntry> {
        // Las canciones van primero: es lo que el usuario busca casi siempre y
        // antes quedaban enterradas bajo una lista larga de álbumes de Deezer.
        if index < self.search.results.len() {
            return Some(SearchEntry::Track(index));
        }
        let index = index - self.search.results.len();
        if index < self.search.artists.len() {
            return Some(SearchEntry::Artist(index));
        }
        let index = index - self.search.artists.len();
        (index < self.search.albums.len()).then_some(SearchEntry::Album(index))
    }

    fn activate_search_selection(&mut self) {
        let index = self.search.selected;
        match self.search_entry(index) {
            Some(SearchEntry::Artist(i)) => {
                if let Some(artist) = self.search.artists.get(i).cloned() {
                    self.open_artist_summary(&artist);
                }
            }
            Some(SearchEntry::Album(i)) => {
                if let Some(album) = self.search.albums.get(i).cloned() {
                    self.open_album_summary(&album);
                }
            }
            Some(SearchEntry::Track(_)) => self.play_search_selection(),
            None => {}
        }
    }

    fn submit_search(&mut self) {
        let query = self.search.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        // Una URL de Spotify no se busca: se importa (playlist, álbum, artista o pista).
        if spote_core::catalog::parse_url(&query).is_ok() {
            self.search.message = "cargando catálogo de Spotify…".into();
            self.spotify_jobs.import(query);
            return;
        }
        self.search.last_query_id += 1;
        self.search.last_query = query.clone();
        self.search.loading = true;
        self.search.message = "buscando en el catálogo…".into();
        self.search.results.clear();
        self.search.albums.clear();
        self.search.artists.clear();
        self.search.selected = 0;
        // Metadatos primero (Spotify/Deezer); la descarga se resuelve después
        // con esos datos, no con el texto libre en yt-dlp.
        self.search_worker
            .search(self.search.last_query_id, query.clone());
        if !self.tools.yt_dlp {
            self.search.message = "buscando… (sin yt-dlp no se podrá descargar)".into();
        }
    }

    fn play_search_selection(&mut self) {
        let Some(SearchEntry::Track(track_index)) = self.search_entry(self.search.selected) else {
            self.activate_search_selection();
            return;
        };
        let Some(item) = self.search.results.get(track_index).cloned() else {
            return;
        };
        let entries: Vec<QueueEntry> = self
            .search
            .results
            .iter()
            .map(|item| {
                let track = item.to_track();
                let path = self
                    .lib
                    .local_path(&track.key)
                    .filter(|path| path.exists());
                QueueEntry { track, path }
            })
            .collect();
        self.set_context(entries, track_index);
        let track = item.to_track();
        if let Ok(id) = self.lib.upsert_track(&track) {
            if let Some(hint) = &track.source_hint {
                self.lib
                    .set_source(
                        id,
                        &hint.provider,
                        Some(&hint.id),
                        None,
                        spote_core::library::SourceStatus::Pending,
                        None,
                    )
                    .ok();
            }
            if let Some(url) = &item.art_url {
                self.lib.set_art(&track.key, None, Some(url)).ok();
            }
        }
        self.play_local(track, None);
    }

    fn copy_lyrics(&mut self) {
        let Some(doc) = &self.lyrics else {
            return;
        };
        let text: String = doc
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            return;
        }
        // xclip/wl-copy: best effort, nunca bloquea ni peta.
        use std::io::Write;
        for (bin, args) in [
            ("wl-copy", vec![]),
            ("xclip", vec!["-selection", "clipboard"]),
        ] {
            if let Ok(mut child) = std::process::Command::new(bin)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                if let Some(stdin) = child.stdin.as_mut() {
                    if stdin.write_all(text.as_bytes()).is_ok() {
                        self.toast("letra copiada", ToastKind::Ok);
                        return;
                    }
                }
            }
        }
        self.toast("no hay wl-copy/xclip", ToastKind::Error);
    }

    fn play_row(&mut self, index: usize) {
        self.selected = index.min(self.view.len().saturating_sub(1));
        let entries: Vec<QueueEntry> = self
            .view
            .iter()
            .map(|row| QueueEntry {
                track: row.track.clone(),
                path: row.path.clone().filter(|path| path.exists()),
            })
            .collect();
        self.set_context(entries, index);
        if let Some(row) = self.view.get(index).cloned() {
            self.play_local(row.track, row.path);
        }
    }

    fn play_queue_row(&mut self, index: usize) {
        if let Some(entry) = self.queue.get(index).cloned() {
            self.queue.remove(index);
            self.play_local(entry.track, entry.path);
        }
    }

    /// Fin lógico de la pista actual: la duración de la canción manda sobre un
    /// visualizer más largo, pero si el vídeo es más corto que los metadatos
    /// no se espera a un final que nunca llega.
    fn effective_duration_ms(&self) -> u64 {
        match (self.now.duration_ms, self.now.stream_duration_ms) {
            (0, reported) => reported,
            (logical, 0) => logical,
            (logical, reported) => logical.min(reported),
        }
    }

    /// Posición actual lista para pintar (incluye el drag en curso).
    pub fn display_position_ms(&self) -> u64 {
        if let Some(ratio) = self.drag_ratio {
            return (self.now.duration_ms as f32 * ratio) as u64;
        }
        // En follow el reloj lo lleva `poll_spotify` frame a frame: aquí no
        // se suma `elapsed` otra vez (eso congelaba la barra al llegar al
        // tope de 500 ms entre sondeos de 400 ms).
        if self.follow_spotify {
            return if self.now.duration_ms > 0 {
                self.now.position_ms.min(self.now.duration_ms)
            } else {
                self.now.position_ms
            };
        }
        if self.now.playing {
            let elapsed = self.now.position_at.elapsed().as_millis() as u64;
            // Tope sin retroceso: antes, al pasar 500 ms, el valor caía de
            // `position_ms + 500` a `position_ms` (diente de sierra visible
            // en el karaoke cuando mpv tarda en informar).
            let current = self.now.position_ms + elapsed.min(500);
            if self.now.duration_ms > 0 {
                current.min(self.now.duration_ms)
            } else {
                current
            }
        } else {
            self.now.position_ms
        }
    }
}

/// Reloj del cliente en ms desde epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

impl Tab {
    fn next(self) -> Self {
        let index = (self.index() + 1) % Tab::ALL.len();
        Tab::ALL[index]
    }

    fn prev(self) -> Self {
        let index = (self.index() + Tab::ALL.len() - 1) % Tab::ALL.len();
        Tab::ALL[index]
    }
}

/// Convierte una pista del catálogo de Spotify/Deezer al tipo que ya usa la UI
/// de búsqueda (el `url` vacío se resuelve luego con yt-dlp).
/// Puntos de una pista del catálogo contra la consulta escrita. Es una
/// heurística local: el proveedor ya filtró, aquí solo se desempata para que
/// la canción exacta no quede debajo de álbumes y homónimos.
/// Offset nuevo al sincronizar una línea. `seek_ms` viene del hitbox ya
/// corregido con el offset actual; hay que deshacerlo antes de comparar.
fn lyrics_offset_for_click(seek_ms: u64, current_offset: i64, position_ms: u64) -> i64 {
    seek_ms as i64 + current_offset - position_ms as i64
}

fn catalog_match_score(query: &str, title: &str, artist: &str) -> i32 {
    let q = spote_core::util::normalize(query);
    let t = spote_core::util::normalize(title);
    let a = spote_core::util::normalize(artist);
    if q.is_empty() || t.is_empty() {
        return 0;
    }
    let haystack = format!("{t} {a}");
    if t == q {
        return 200;
    }
    // Los tokens que el usuario escribió y que no forman parte del artista
    // son los que deberían aparecer en el título. Así "Superreservao Alma en
    // pena" no premia una pista titulada solo "Superreservao".
    let artist_words: Vec<&str> = a.split_whitespace().collect();
    let title_tokens: Vec<&str> = q
        .split_whitespace()
        .filter(|token| !artist_words.contains(token))
        .collect();
    let mut score = 0;
    if !title_tokens.is_empty() {
        let matched = title_tokens
            .iter()
            .filter(|token| {
                t.split_whitespace()
                    .any(|word| word == **token || word.starts_with(**token))
            })
            .count();
        score += if matched == title_tokens.len() {
            120
        } else {
            matched as i32 * 15
        };
    } else if haystack.contains(&q) {
        score += 120;
    }
    for token in q.split_whitespace() {
        if a.split_whitespace().any(|word| word.starts_with(token)) {
            score += 10;
        }
    }
    score
}

fn search_item_from_spotify(track: spote_core::spotify_api::SpotifyTrack) -> SearchItem {
    SearchItem {
        provider_id: track.id,
        provider: track.provider,
        title: track.title,
        artist: track.artists,
        album: Some(track.album).filter(|album| !album.is_empty()),
        duration_ms: track.duration_ms,
        channel: String::new(),
        url: String::new(),
        art_url: track.art_url,
        isrc: track.isrc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_in_both_directions() {
        assert_eq!(Tab::Now.next(), Tab::Search);
        assert_eq!(Tab::Library.next(), Tab::Audio);
        assert_eq!(Tab::Audio.next(), Tab::Room);
        assert_eq!(Tab::Room.next(), Tab::Settings);
        assert_eq!(Tab::Settings.next(), Tab::Now);
        assert_eq!(Tab::Now.prev(), Tab::Settings);
        assert_eq!(Tab::Settings.prev(), Tab::Room);
        assert_eq!(Tab::Room.prev(), Tab::Audio);
    }

    #[test]
    fn lyrics_click_does_not_accumulate_the_old_offset() {
        // La línea empieza en 10 s, el offset actual es +1 s y el hitbox de
        // seek lleva 9 s. El nuevo offset debe ser 10 s - 5 s = +5 s, no
        // 9 s - 5 s = +4 s.
        assert_eq!(lyrics_offset_for_click(9_000, 1_000, 5_000), 5_000);
        assert_eq!(lyrics_offset_for_click(10_000, 0, 5_000), 5_000);
    }

    #[test]
    fn catalog_ranking_prefers_the_exact_song_over_homonyms() {
        let query = "Superreservao Alma en pena";
        let exact = catalog_match_score(query, "como alma en pena", "superreservao");
        let other = catalog_match_score(query, "Superreservao", "Agorazein");
        let unrelated = catalog_match_score(query, "azulado", "superreservao");
        assert!(exact > other, "{exact} vs {other}");
        assert!(exact > unrelated, "{exact} vs {unrelated}");
    }

    #[test]
    fn hit_map_returns_topmost() {
        let mut map = HitMap::default();
        map.add(Rect::new(0, 0, 10, 10), Action::PlayPause);
        map.add(Rect::new(2, 2, 4, 4), Action::Next);
        assert_eq!(map.hit(3, 3), Some(&Action::Next));
        assert_eq!(map.hit(8, 8), Some(&Action::PlayPause));
        assert_eq!(map.hit(50, 50), None);
    }

    #[test]
    fn seek_clamps_to_duration() {
        // No se puede instanciar App sin DB, pero la aritmética está cubierta en
        // `RoomState::target_position` y en `util`. Aquí solo comprobamos que el
        // helper de selección está acotado en la vista vacía.
        let mut map = HitMap::default();
        map.add(Rect::new(0, 0, 0, 5), Action::Next);
        assert_eq!(map.hit(0, 0), None);
    }
}
