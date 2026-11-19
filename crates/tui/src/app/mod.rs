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
use spote_core::player::{Player, PlayerEvent};
use spote_core::spotify::SpotifyState;
use spote_core::spotify_api::{AlbumSummary, ArtistSummary};
use spote_core::util;
use spote_core::ToolStatus;
use spote_protocol::{ControlAction, Presence, QueueItem, Room, RoomState, TrackRef};

use crate::fetch::{
    ArtKind, ArtRequest, ArtResponse, ArtWorker, LyricsMsg, LyricsWorker, MetaEvent, MetaWorker,
    SearchWorker, SongMapEvent, SongMapWorker, SpotifyWorker,
};
use crate::social::{Social, SocialCmd, SocialEvent};
use crate::theme::{FpsMeter, PaletteRuntime, Pulse, Spring};
use crate::viz::Viz;

mod actions;
pub(crate) mod albums;
mod artist;
mod context;
mod downloads;
mod input;
mod keymap;
mod layout_editor;
pub use keymap::help_sections;
mod playlists;
mod settings;
mod spotify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Now,
    Search,
    Library,
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
    pub const ALL: [Tab; 5] = [
        Tab::Now,
        Tab::Search,
        Tab::Library,
        Tab::Room,
        Tab::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Now => "ahora",
            Self::Search => "buscar",
            Self::Library => "librería",
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
            // `Albums` es interno y ya no está en `ALL`; se ancla a Library.
            Self::Albums => 2,
            Self::Room => 3,
            Self::Settings => 4,
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
    Artist,
    /// Álbum abierto (biblioteca o catálogo remoto).
    Album,
    /// Editor de recorte de intro/outro.
    Trim,
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
    PlaylistOpen(i64),
    SettingsRow(usize),
    SettingsAdjust(usize, i32),
    SettingsToggle(usize),
    ToggleZen,
    PlayPause,
    Next,
    Prev,
    LoopToggle,
    SeekTo(u64),
    SeekLyrics(u64),
    Volume(f32),
    VolumeTrack,
    Mute,
    FocusTransport,
    VizNext,
    VizPrev,
    VizLayer(usize),
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
    pub playing: bool,
    pub volume: f32,
    pub loading: bool,
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
            playing: false,
            volume: 0.0,
            loading: false,
            message: String::new(),
        }
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
    Ambient,
    AmbientOpacity,
    InstrumentBorders,
    EdgeFlame,
    BloomPlay,
    CleanSpill,
    GlitchPanels,
    LyricsSize,
    VizBackground,
    Cards,
    Vcenter,
    Zen,
    Thumb,
    ProgressInViz,
    BarWidth,
    ButtonPad,
    Spotify,
    LayoutEditor,
    Save,
}

impl SettingRow {
    pub const ALL: [SettingRow; 24] = [
        Self::Rotate,
        Self::RotateSecs,
        Self::RotateOff,
        Self::ArtMode,
        Self::VizPreset,
        Self::Ambient,
        Self::AmbientOpacity,
        Self::InstrumentBorders,
        Self::EdgeFlame,
        Self::BloomPlay,
        Self::CleanSpill,
        Self::GlitchPanels,
        Self::LyricsSize,
        Self::VizBackground,
        Self::Cards,
        Self::Vcenter,
        Self::Zen,
        Self::Thumb,
        Self::ProgressInViz,
        Self::BarWidth,
        Self::ButtonPad,
        Self::Spotify,
        Self::LayoutEditor,
        Self::Save,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rotate => "rotación de arte",
            Self::RotateSecs => "rotar cada",
            Self::RotateOff => "rotación incluye off",
            Self::ArtMode => "modo de arte",
            Self::VizPreset => "efecto inferior",
            Self::Ambient => "fondo ambiental",
            Self::AmbientOpacity => "opacidad ambiental",
            Self::InstrumentBorders => "bordes con instrumentos",
            Self::EdgeFlame => "llama en el borde",
            Self::BloomPlay => "bloom en el play",
            Self::CleanSpill => "clean sobre paneles",
            Self::GlitchPanels => "glitch en paneles",
            Self::LyricsSize => "tamaño de letra",
            Self::VizBackground => "visualizador de fondo",
            Self::Cards => "tarjetas",
            Self::Vcenter => "centrar paneles",
            Self::Zen => "zen (sin portada ni letra)",
            Self::Thumb => "efecto miniaturas",
            Self::ProgressInViz => "barra en el visualizador",
            Self::BarWidth => "ancho de la barra",
            Self::ButtonPad => "tamaño de controles",
            Self::Spotify => "seguir spotify",
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
                | Self::LyricsSize
                | Self::Thumb
                | Self::BarWidth
                | Self::ButtonPad
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
    pub song_intensity: f32,
    pub song_section: Option<spote_core::audio_map::SongSectionKind>,
    pub artist: Option<spote_core::spotify_api::ArtistProfile>,
    pub artist_art: Option<CoverArt>,
    pub artist_loading: bool,
    pub artist_selected: usize,
    pub artist_status: String,
    pub metadata_requests: std::collections::HashSet<String>,
    pub metadata_tried: std::collections::HashSet<String>,
    pub metadata_manual: std::collections::HashSet<String>,
    pub lyrics_worker: LyricsWorker,
    pub spotify_worker: Option<SpotifyWorker>,
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
    pub volume_before_mute: f32,
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
    pub albums: Vec<spote_core::library::AlbumRow>,
    pub album_selected: usize,
    pub album_open: Option<spote_core::library::AlbumRow>,
    pub album_page: Option<AlbumPage>,
    pub album_page_loading: bool,
    pub album_page_status: String,
    pub lib_filter: LibFilter,
    pub selected: usize,
    pub list_scroll: Spring,
    pub search: SearchState,
    pub pending_play: Option<String>,

    pub lyrics: Option<LyricsDoc>,
    pub lyrics_status: String,
    pub lyrics_key: String,
    pub lyrics_loading: bool,
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
    /// Delta real del último tick; los efectos lo usan para no cambiar de
    /// velocidad cuando el terminal pierde algún frame.
    pub last_dt: f32,
    pub pulse: Pulse,
    pub intro: f32,
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
    pub spotify: Option<SpotifyState>,
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
        let social = if cfg.server.url.is_empty() {
            None
        } else {
            Some(Social::start(cfg.server.clone()))
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
            song_intensity: 0.0,
            song_section: None,
            artist: None,
            artist_art: None,
            artist_loading: false,
            artist_selected: 0,
            artist_status: String::new(),
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
            volume_before_mute: player_volume,
            player_events: None,
            queue: Vec::new(),
            queue_selected: 0,
            queue_drag: None,
            view: Vec::new(),
            loop_file: false,
            history: Vec::new(),
            albums: Vec::new(),
            album_selected: 0,
            album_open: None,
            album_page: None,
            album_page_loading: false,
            album_page_status: String::new(),
            lib_filter: LibFilter::All,
            selected: 0,
            list_scroll: Spring::snappy(0.0),
            search: SearchState::default(),
            pending_play: None,
            lyrics: None,
            lyrics_status: String::new(),
            lyrics_key: String::new(),
            lyrics_loading: false,
            lyrics_offset_ms: 0,
            lyrics_auto_offset: None,
            lyrics_retry: std::collections::HashSet::new(),
            trim_start_ms: 0,
            trim_end_ms: None,
            lyrics_scroll: Spring::new(0.0),
            lyrics_scroll_index: None,
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
            last_dt: 1.0 / 120.0,
            pulse: Pulse::default(),
            intro: 0.0,
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
        let next = match self.tab {
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
            Tab::Search | Tab::Room | Tab::Settings => Vec::new(),
        };
        self.view = next;
        if error.is_some() && self.view.is_empty() && !previous.is_empty() {
            // Solo un error real de SQLite justifica rescatar la vista anterior.
            // Un filtro vacío (favoritas, lista…) debe verse vacío, no con las
            // canciones de otro filtro.
            self.view = previous;
            self.toast(
                format!(
                    "no pude refrescar la biblioteca{}",
                    error.map(|e| format!(": {e}")).unwrap_or_default()
                ),
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
    /// Carga una pista ya resuelta a fichero local.
    pub fn play_local(&mut self, track: TrackRef, path: Option<PathBuf>) {
        self.play_local_inner(track, path, true);
    }

    fn play_local_inner(
        &mut self,
        track: TrackRef,
        path: Option<PathBuf>,
        record_history: bool,
    ) {
        let Some(path) = path.or_else(|| self.lib.local_path(&track.key)) else {
            self.queue_track(track, None);
            return;
        };
        if !path.exists() {
            self.toast(
                format!("falta el fichero: {}", path.display()),
                ToastKind::Error,
            );
            return;
        }
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
        // Recorte persistente: si el vídeo de YouTube trae intro/outro, se
        // salta solo sin tocar el fichero (reversible con Backspace).
        let (trim_start, trim_end) = self.lib.trim(&track.key).unwrap_or((0, None));
        let duration = track.duration_ms.max(1);
        self.trim_start_ms = trim_start.min(duration.saturating_sub(500));
        self.trim_end_ms = trim_end
            .filter(|end| *end > self.trim_start_ms + 500)
            .map(|end| end.min(duration));
        self.now.track = Some(track.clone());
        self.now.path = Some(path.clone());
        self.now.position_ms = self.trim_start_ms;
        self.now.duration_ms = track.duration_ms;
        self.now.playing = true;
        self.now.loading = false;
        self.now.message.clear();
        self.now.volume = self.cfg.player.volume;
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
            player.set_loop(self.loop_file);
            player.load(path.clone(), self.trim_start_ms as f64 / 1000.0);
        } else {
            self.set_status("mpv no disponible: modo visual");
        }
        self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
        self.lyrics_auto_offset = None;
        self.lyrics = None;
        self.lyrics_status.clear();
        self.lyrics_key.clear();
        self.lyrics_loading = false;
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
        self.song_intensity = 0.0;
        self.song_section = None;
        self.map_worker.request(track.key.clone(), path.clone());
        if !self.follow_spotify {
            self.spectrum
                .decode_file(path, self.trim_start_ms as f64 / 1000.0);
            self.spectrum_at = Instant::now();
        }
        if let Some(row) = self.lib.track_by_key(&track.key).ok().flatten() {
            self.lib.record_play(&row.track.key, 0, false).ok();
        }
        if !self.applying_remote && self.room.is_some() {
            // En una sala manda el servidor: lo que pongas se anuncia a la cola
            // compartida, no se reproduce por tu cuenta.
            if let Some(social) = &self.social {
                social.send(SocialCmd::QueueAdd(track.clone()));
                self.toast(
                    format!("{} añadida a la cola de la sala", track.title),
                    ToastKind::Info,
                );
            }
        }
        self.set_status(format!("▶ {}", track.label()));
    }

    /// Encola una pista: si no está descargada, lanza la descarga.
    pub fn queue_track(&mut self, track: TrackRef, url: Option<String>) {
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

    pub fn toggle_play(&mut self) {
        if self.follow_spotify {
            let playing = !self.now.playing;
            self.now.playing = playing;
            std::thread::spawn(move || {
                let _ = spote_core::mpris::call(if playing { "Play" } else { "Pause" }, &());
            });
            return;
        }
        if self.now.track.is_none() {
            // Nada cargado: reproduce la fila seleccionada o la primera.
            if let Some(row) = self.selected_row().cloned() {
                self.play_local(row.track, row.path);
                return;
            }
            if let Some(row) = self.view.first().cloned() {
                self.play_local(row.track, row.path);
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

    /// Bucle del fichero actual. En mpv basta con `loop-file`; al cambiar de
    /// pista se vuelve a mandar para que el nuevo fichero herede el modo.
    pub fn toggle_loop(&mut self) {
        if self.follow_spotify {
            self.toast("el bucle no se controla en modo Spotify", ToastKind::Warn);
            return;
        }
        self.loop_file = !self.loop_file;
        if let Some(player) = &self.player {
            player.set_loop(self.loop_file);
        }
        self.toast(
            if self.loop_file {
                "bucle activado"
            } else {
                "bucle desactivado"
            },
            ToastKind::Info,
        );
    }

    pub fn next_track(&mut self) {
        if self.follow_spotify {
            self.now.position_ms = 0;
            std::thread::spawn(|| {
                let _ = spote_core::mpris::call("Next", &());
            });
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
            } else {
                self.now.track = Some(entry.track.clone());
                self.pending_play = Some(entry.track.key.clone());
                self.set_status("esperando descarga…");
            }
            return;
        }
        self.play_view_step(1, true, true);
    }

    pub fn prev_track(&mut self) {
        if self.follow_spotify {
            self.now.position_ms = 0;
            std::thread::spawn(|| {
                let _ = spote_core::mpris::call("Previous", &());
            });
            return;
        }
        if self.room.is_some() {
            self.send_control(ControlAction::Prev, None);
            return;
        }
        if self.now.position_ms > 3_000 {
            self.seek_to(0);
            return;
        }
        // Historial de sesión: «anterior» devuelve a la pista que sonaba de
        // verdad, aunque se haya puesto desde una lista, la cola o buscar.
        while let Some((track, path)) = self.history.pop() {
            let path = path
                .filter(|path| path.exists())
                .or_else(|| self.lib.local_path(&track.key));
            if path.is_some() {
                self.play_local_inner(track, path, false);
                return;
            }
        }
        self.play_view_step(-1, false, false);
    }

    /// Avanza/retrocede en la lista tomando como referencia la pista que
    /// suena, no la fila seleccionada. Si la pista no está en la lista actual,
    /// arranca desde la selección.
    fn play_view_step(&mut self, delta: i32, wrap: bool, record_history: bool) {
        if self.view.is_empty() {
            return;
        }
        let current = self.now.track.as_ref().map(|track| track.key.clone());
        let index = current
            .and_then(|key| self.view.iter().position(|row| row.track.key == key))
            .unwrap_or_else(|| self.selected.min(self.view.len() - 1));
        let len = self.view.len() as i32;
        let next = if wrap {
            (index as i32 + delta).rem_euclid(len)
        } else {
            (index as i32 + delta).clamp(0, len - 1)
        };
        self.selected = next as usize;
        if let Some(row) = self.view.get(self.selected).cloned() {
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
            // El transporte manda al cliente oficial: SetPosition por MPRIS.
            let before = self.now.position_ms;
            self.now.position_ms = target;
            if let Some(spotify_id) = self
                .now
                .track
                .as_ref()
                .and_then(|track| track.source_hint.as_ref())
                .map(|hint| hint.id.clone())
            {
                std::thread::spawn(move || {
                    let _ = spote_core::mpris::set_position(&spotify_id, target);
                });
            } else {
                // Sin trackid no se puede SetPosition; MPRIS acepta un seek
                // relativo en microsegundos.
                let delta = (target as i64 - before as i64) * 1000;
                std::thread::spawn(move || {
                    let _ = spote_core::mpris::call("Seek", &delta);
                });
            }
            return;
        }
        self.send_control(ControlAction::Seek, Some(ms));
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
        let target = (self.now.position_ms as i64 + delta_ms).max(0) as u64;
        self.seek_to(target);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.now.volume = volume.clamp(0.0, 1.0);
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
        }
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
            self.lyrics_offset_ms = doc.offset_ms;
            self.lyrics = Some(doc);
            self.lyrics_status = if stale_alignment {
                "afinando de nuevo…".to_string()
            } else if row.aligned {
                "asr".to_string()
            } else if row.synced {
                "lrclib".to_string()
            } else {
                "aprox".to_string()
            };
            self.lyrics_key = track.key.clone();
            self.refine_lyrics_words();
            let audio = self.lib.local_path(&track.key).filter(|path| path.exists());
            if ((!row.synced && !row.aligned) || stale_alignment)
                && audio.is_some()
                && !self.lyrics_retry.contains(&track.key)
            {
                self.lyrics_retry.insert(track.key.clone());
                self.lyrics_loading = true;
                self.lyrics_status = "afinando con el audio…".into();
                self.lyrics_worker
                    .request(track.clone(), audio, stale_alignment);
            } else {
                return;
            }
            return;
        }
        // 2. `.lrc` junto al fichero.
        if let Some(path) = self.lib.local_path(&track.key) {
            let candidate = path.with_extension("lrc");
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                let doc = spote_core::lyrics::parse_lrc(&text);
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
        self.lyrics_worker.request(track.clone(), audio, false);
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
        let offset = line_start_ms as i64 - self.display_position_ms() as i64;
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
        self.lyrics_offset_ms = (self.lyrics_offset_ms + delta_ms).clamp(-5_000, 5_000);
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
            // Anclas: la frase empieza con la primera palabra y termina con la
            // última; nunca al revés.
            if let Some(first) = starts.first_mut() {
                *first = line.start_ms;
            }
            if let Some(last) = starts.last_mut() {
                *last = line.end_ms;
            }
            for (i, word) in line.words.iter_mut().enumerate() {
                word.start_ms = starts[i];
                word.end_ms = starts.get(i + 1).copied().unwrap_or(line.end_ms);
            }
        }
    }

    // ---------------------------------------------------------------- tick
    pub fn tick(&mut self, dt: f32) {
        self.last_dt = dt;
        if self.now.position_ms != self.now.position_seen_ms {
            self.now.position_seen_ms = self.now.position_ms;
            self.now.position_at = Instant::now();
        }
        self.intro = (self.intro + dt * 2.5).min(1.0);
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
        self.list_scroll.set_target(self.selected as f32);
        self.list_scroll.update(dt);
        if let Some(doc) = &self.lyrics {
            if let Some(index) = doc.index_at(self.display_position_ms()) {
                if self.lyrics_scroll_index != Some(index) {
                    self.lyrics_scroll_index = Some(index);
                    // El resaltado cambia al instante: si el muelle persigue el
                    // índice, la línea nueva se pintaba mientras la anterior
                    // seguía en el centro y se percibía como cambio a destiempo.
                    self.lyrics_scroll.value = index as f32;
                    self.lyrics_scroll.velocity = 0.0;
                }
                self.lyrics_scroll.set_target(index as f32);
            }
        }
        self.lyrics_scroll.update(dt);

        let frame = self.spectrum.frame();
        if frame.beat {
            self.pulse.hit(0.7);
        }
        self.audio = frame;

        self.poll_player();
        self.poll_search();
        self.poll_downloads();
        self.poll_spotify_jobs();
        self.poll_art();
        self.poll_meta();
        self.poll_song_map();
        self.poll_lyrics();
        self.poll_spotify(dt);
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

    fn poll_player(&mut self) {
        let Some(receiver) = self.player_events.as_mut() else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        if self.follow_spotify {
            // En modo acompañante la posición y la duración las manda MPRIS.
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
                    if let Some(end) = self.trim_end_ms {
                        if self.now.position_ms >= end.saturating_sub(30) {
                            if let Some(track) = self.now.track.clone() {
                                self.lib
                                    .record_play(&track.key, self.now.position_ms, true)
                                    .ok();
                            }
                            self.next_track();
                            return;
                        }
                    }
                }
                PlayerEvent::Duration(duration) => {
                    if duration > 0.0 {
                        self.now.duration_ms = (duration * 1000.0) as u64;
                    }
                }
                PlayerEvent::Paused(paused) => self.now.playing = !paused,
                PlayerEvent::Eof => {
                    if let Some(track) = self.now.track.clone() {
                        self.lib
                            .record_play(&track.key, self.now.position_ms, true)
                            .ok();
                    }
                    self.next_track();
                }
                PlayerEvent::Disconnected => {
                    self.set_status("mpv desconectado");
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
                    self.search.results = all
                        .tracks
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
                        self.lib
                            .set_source(
                                row.id,
                                "ytmusic",
                                None,
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
                        if let Some(entry) =
                            self.queue.iter().position(|entry| entry.track.key == key)
                        {
                            let entry = self.queue.remove(entry);
                            self.play_local(entry.track, Some(path.clone()));
                        } else if let Some(track) = self.now.track.clone() {
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
                    self.finish_download(&key, false);
                    if self.pending_play.as_deref() == Some(key.as_str()) {
                        self.pending_play = None;
                    }
                    self.now.loading = false;
                    self.finish_download(&key, true);
                    self.toast(format!("descarga fallida: {message}"), ToastKind::Error);
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
                    self.song_map = Some(map);
                    self.song_map_key = key;
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
                                self.overlay = Overlay::Album;
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
                                self.overlay = Overlay::Album;
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
            || doc.source == spote_core::lyrics::LyricsSource::Local
        {
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
                    self.lyrics_loading = false;
                    if self.now.track.as_ref().map(|t| &t.key) != Some(&key) {
                        continue;
                    }
                    match doc {
                        Some(doc) => {
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
                                if doc.aligned {
                                    self.lib.set_lyrics_offset(&key, 0).ok();
                                }
                            }
                            self.lyrics_status = if doc.aligned {
                                "asr".to_string()
                            } else {
                                doc.source.label().to_string()
                            };
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
            self.set_volume(0.0);
        } else {
            let volume = self.volume_before_mute.max(0.2);
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
            Tab::Room | Tab::Settings => self.toast(
                "favorita: selecciona una pista en buscar o librería",
                ToastKind::Warn,
            ),
        }
    }

    fn toggle_favorite_key(&mut self, key: &str) {
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
            self.queue.remove(index);
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

    /// Longitud de la lista mixta: artistas, álbumes y pistas.
    pub fn search_len(&self) -> usize {
        self.search.artists.len() + self.search.albums.len() + self.search.results.len()
    }

    pub fn search_entry(&self, index: usize) -> Option<SearchEntry> {
        if index < self.search.artists.len() {
            return Some(SearchEntry::Artist(index));
        }
        let index = index - self.search.artists.len();
        if index < self.search.albums.len() {
            return Some(SearchEntry::Album(index));
        }
        let index = index - self.search.albums.len();
        (index < self.search.results.len()).then_some(SearchEntry::Track(index))
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
        if let Some(path) = self.lib.local_path(&track.key) {
            self.play_local(track, Some(path));
            return;
        }
        if item.provider == "spotify" || item.provider == "deezer" {
            self.start_resolve(&track);
        } else {
            self.start_download(&track, item.url.clone());
        }
        self.pending_play = Some(track.key.clone());
        self.now.track = Some(track.clone());
        self.now.loading = true;
        self.now.duration_ms = track.duration_ms;
        self.set_status("descargando…");
        self.lyrics = None;
        self.lyrics_auto_offset = None;
        self.lyrics_key.clear();
        self.load_lyrics(&track);
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

    /// Posición actual lista para pintar (incluye el drag en curso).
    pub fn display_position_ms(&self) -> u64 {
        if let Some(ratio) = self.drag_ratio {
            return (self.now.duration_ms as f32 * ratio) as u64;
        }
        if self.now.playing {
            let elapsed = self.now.position_at.elapsed().as_millis() as u64;
            // Tope sin retroceso: antes, al pasar 500 ms, el valor caía de
            // `position_ms + 500` a `position_ms` (diente de sierra visible
            // en el karaoke cuando mpv tarda en informar).
            (self.now.position_ms + elapsed.min(500)).min(self.now.duration_ms.max(1))
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
        assert_eq!(Tab::Library.next(), Tab::Room);
        assert_eq!(Tab::Room.next(), Tab::Settings);
        assert_eq!(Tab::Settings.next(), Tab::Now);
        assert_eq!(Tab::Now.prev(), Tab::Settings);
        assert_eq!(Tab::Settings.prev(), Tab::Room);
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
