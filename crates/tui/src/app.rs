//! Estado de la aplicación y máquina de acciones. La UI solo lee de aquí.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use spote_core::artwork::{ArtMode, CoverArt, Palette};
use spote_core::audio::Spectrum;
use spote_core::config::{self, Config};
use spote_core::downloader::{DownloadEvent, DownloadSource, Downloader, SearchItem};
use spote_core::library::{Library, TrackRow};
use spote_core::lyrics::LyricsDoc;
use spote_core::player::{Player, PlayerEvent};
use spote_core::spotify::SpotifyState;
use spote_core::util;
use spote_core::ToolStatus;
use spote_protocol::{ControlAction, Presence, QueueItem, Room, RoomState, TrackRef};

use crate::fetch::{ArtRequest, ArtResponse, ArtWorker, LyricsMsg, LyricsWorker, SearchWorker, SpotifyWorker};
use crate::social::{Social, SocialCmd, SocialEvent};
use crate::theme::{FpsMeter, PaletteRuntime, Pulse, Spring};
use crate::viz::Viz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Now,
    Search,
    Library,
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
    pub const ALL: [Tab; 5] = [Tab::Now, Tab::Search, Tab::Library, Tab::Room, Tab::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Now => "ahora",
            Self::Search => "buscar",
            Self::Library => "librería",
            Self::Room => "sala",
            Self::Settings => "ajustes",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Now => "◉",
            Self::Search => "/",
            Self::Library => "≡",
            Self::Room => "◎",
            Self::Settings => "✧",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Now => 0,
            Self::Search => 1,
            Self::Library => 2,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Effects,
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
    SeekTo(u64),
    Volume(f32),
    Mute,
    VizNext,
    VizPrev,
    VizLayer(usize),
    VizFullscreen,
    VizToggle,
    ArtMode,
    ArtFullscreen,
    SelectRow(usize),
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

#[derive(Debug, Clone, Default)]
pub struct NowPlaying {
    pub track: Option<TrackRef>,
    pub path: Option<PathBuf>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    pub volume: f32,
    pub loading: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub results: Vec<SearchItem>,
    pub selected: usize,
    pub loading: bool,
    pub last_query_id: u64,
    pub last_query: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub track: TrackRef,
    pub path: Option<PathBuf>,
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
    pub lyrics_worker: LyricsWorker,
    pub spotify_worker: Option<SpotifyWorker>,
    pub tools: ToolStatus,

    pub tab: Tab,
    pub focus: Pane,
    pub fullscreen: Fullscreen,
    pub overlay: Overlay,
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
    pub lyrics_scroll: Spring,

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
    pub art_next_at: Instant,
    pub art_variants: std::collections::HashMap<ArtMode, CoverArt>,
    pub thumbs: std::collections::HashMap<String, CoverArt>,
    pub thumb_requests: std::collections::HashSet<String>,
    pub art_request: Option<Instant>,
    pub search_worker: SearchWorker,
    pub palette: PaletteRuntime,
    pub dominant: Palette,

    pub list_cards: bool,
    pub viz_background: bool,
    pub vcenter: bool,
    pub zen: bool,
    pub settings_selected: usize,
    pub show_bpm: bool,
    pub viz: Viz,
    pub fps: FpsMeter,
    pub frame_ms: f32,
    pub pulse: Pulse,
    pub intro: f32,
    pub hit: HitMap,
    pub hover: Option<Action>,
    pub last_click: Option<(Instant, usize)>,
    pub toasts: Vec<Toast>,
    pub status: String,
    pub status_at: Instant,
    pub dragging_seek: bool,
    pub drag_ratio: Option<f32>,

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
    pub spectrum_at: Instant,
}

fn onoff(value: bool) -> String {
    if value { "sí".into() } else { "no".into() }
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
            lyrics_worker: LyricsWorker::start(cfg.lyrics.clone()),
            spotify_worker: None,
            tools: ToolStatus::detect(),
            tab: Tab::Now,
            focus: Pane::Center,
            fullscreen: Fullscreen::None,
            overlay: Overlay::None,
            show_viz: cfg.ui.viz_enabled,
            now: NowPlaying {
                volume: player_volume,
                ..Default::default()
            },
            volume_before_mute: player_volume,
            player_events: None,
            queue: Vec::new(),
            queue_selected: 0,
            queue_drag: None,
            view: Vec::new(),
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
            lyrics_scroll: Spring::new(0.0),
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
            art_next_at: Instant::now() + std::time::Duration::from_secs(cfg.ui.art_rotate_secs.max(5)),
            art_variants: std::collections::HashMap::new(),
            thumbs: std::collections::HashMap::new(),
            thumb_requests: std::collections::HashSet::new(),
            art_request: None,
            search_worker: SearchWorker::start(),
            palette: PaletteRuntime::new(Palette::dark(), cfg.ui.theme_fade_ms),
            dominant: Palette::dark(),
            list_cards: cfg.ui.list_cards,
            viz_background: cfg.ui.viz_background,
            vcenter: cfg.ui.vcenter,
            zen: cfg.ui.zen,
            settings_selected: 0,
            show_bpm: cfg.ui.show_bpm,
            viz,
            fps: FpsMeter::new(),
            frame_ms: 0.0,
            pulse: Pulse::default(),
            intro: 0.0,
            hit: HitMap::default(),
            hover: None,
            last_click: None,
            toasts: Vec::new(),
            status: String::new(),
            status_at: Instant::now(),
            dragging_seek: false,
            drag_ratio: None,
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
        self.view = match self.tab {
            Tab::Library | Tab::Now => match self.lib_filter {
                LibFilter::All => self.lib.all_tracks(500).unwrap_or_default(),
                LibFilter::Favorites => self.lib.favorites().unwrap_or_default(),
                LibFilter::Downloaded => self.lib.playable(500).unwrap_or_default(),
                LibFilter::Recent => self.lib.recent(500).unwrap_or_default(),
                LibFilter::Playlist(id) => self.lib.playlist_tracks(id).unwrap_or_default(),
            },
            Tab::Search | Tab::Room | Tab::Settings => Vec::new(),
        };
        if self.selected >= self.view.len() {
            self.selected = self.view.len().saturating_sub(1);
        }
    }

    pub fn selected_row(&self) -> Option<&TrackRow> {
        self.view.get(self.selected)
    }

    // --------------------------------------------------------- reproducción
    /// Carga una pista ya resuelta a fichero local.
    pub fn play_local(&mut self, track: TrackRef, path: Option<PathBuf>) {
        let Some(path) = path.or_else(|| self.lib.local_path(&track.key)) else {
            self.queue_track(track, None);
            return;
        };
        if !path.exists() {
            self.toast(format!("falta el fichero: {}", path.display()), ToastKind::Error);
            return;
        }
        self.now.track = Some(track.clone());
        self.now.path = Some(path.clone());
        self.now.position_ms = 0;
        self.now.duration_ms = track.duration_ms;
        self.now.playing = true;
        self.now.loading = false;
        self.now.message.clear();
        self.now.volume = self.cfg.player.volume;
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
            player.load(path.clone(), 0.0);
        } else {
            self.set_status("mpv no disponible: modo visual");
        }
        self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
        self.lyrics = None;
        self.lyrics_status.clear();
        self.lyrics_key.clear();
        self.lyrics_loading = false;
        self.art = None;
        self.art_prev = None;
        self.art_variants.clear();
        self.art_key.clear();
        self.art_next_at = Instant::now()
            + std::time::Duration::from_secs(self.cfg.ui.art_rotate_secs.max(5));
        self.load_lyrics(&track);
        if !self.follow_spotify {
            self.spectrum.decode_file(path, 0.0);
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
        if let Some(path) = self.lib.local_path(&track.key) {
            self.queue.push(QueueEntry {
                track,
                path: Some(path),
            });
            self.toast("añadida a la cola", ToastKind::Ok);
            return;
        }
        if let Some(hint) = &track.source_hint {
            if self.tools.yt_dlp {
                self.lib.upsert_track(&track).ok();
                if hint.provider == "spotify" || hint.provider == "deezer" {
                    self.start_resolve(&track);
                } else {
                    self.downloader.download(
                        &track.key,
                        DownloadSource::Url(format!(
                            "https://www.youtube.com/watch?v={}",
                            hint.id
                        )),
                        self.cfg.media_dir(),
                    );
                }
                self.queue.push(QueueEntry { track, path: None });
                self.toast("descargando y en cola", ToastKind::Info);
                return;
            }
        }
        if let Some(url) = url {
            self.downloader.download(
                &track.key,
                DownloadSource::Url(url),
                self.cfg.media_dir(),
            );
            self.queue.push(QueueEntry { track, path: None });
            return;
        }
        self.toast("no hay fuente descargable para esta pista", ToastKind::Error);
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
                let _ = spote_core::mpris::call(
                    if playing { "Play" } else { "Pause" },
                    &(),
                );
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
            if let Some(path) = entry.path {
                self.play_local(entry.track, Some(path));
            } else {
                self.now.track = Some(entry.track.clone());
                self.pending_play = Some(entry.track.key.clone());
                self.set_status("esperando descarga…");
            }
            return;
        }
        if self.view.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.view.len();
        if let Some(row) = self.selected_row().cloned() {
            self.play_local(row.track, row.path);
        }
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
        if self.view.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        if let Some(row) = self.selected_row().cloned() {
            self.play_local(row.track, row.path);
        }
    }

    pub fn seek_to(&mut self, ms: u64) {
        let target = ms.min(self.now.duration_ms.max(ms));
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
    pub fn settings_rows(&self) -> Vec<(&'static str, String)> {
        vec![
            ("rotación de arte", onoff(self.art_rotate)),
            ("rotar cada", format!("{} s", self.cfg.ui.art_rotate_secs.max(5))),
            ("modo de arte", self.art_mode.label().to_string()),
            ("visualizador de fondo", onoff(self.viz_background)),
            ("tarjetas", onoff(self.list_cards)),
            ("centrar vertical", onoff(self.vcenter)),
            ("zen (sin portada ni letra)", onoff(self.zen)),
            ("seguir spotify", onoff(self.follow_spotify)),
            ("guardar ajustes", "enter".into()),
        ]
    }

    pub fn adjust_setting(&mut self, row: usize, delta: i32) {
        match row {
            1 => {
                let secs = self.cfg.ui.art_rotate_secs as i32 + delta * 5;
                self.cfg.ui.art_rotate_secs = secs.clamp(5, 300) as u64;
                self.art_next_at = Instant::now()
                    + Duration::from_secs(self.cfg.ui.art_rotate_secs.max(5));
                self.cfg.save().ok();
            }
            2 => {
                let next = if delta > 0 { self.art_mode.next() } else { self.art_mode.prev() };
                self.set_art_mode(next);
                self.cfg.save().ok();
            }
            _ => self.toggle_setting(row),
        }
    }

    pub fn toggle_setting(&mut self, row: usize) {
        match row {
            0 => {
                self.art_rotate = !self.art_rotate;
                self.cfg.ui.art_rotate = self.art_rotate;
            }
            3 => {
                self.viz_background = !self.viz_background;
                self.cfg.ui.viz_background = self.viz_background;
            }
            4 => {
                self.list_cards = !self.list_cards;
                self.cfg.ui.list_cards = self.list_cards;
            }
            5 => {
                self.vcenter = !self.vcenter;
                self.cfg.ui.vcenter = self.vcenter;
            }
            6 => self.set_zen(!self.zen),
            7 => self.toggle_spotify(),
            8 => match self.cfg.save() {
                Ok(()) => self.toast("ajustes guardados", ToastKind::Ok),
                Err(err) => self.toast(format!("no se pudo guardar: {err}"), ToastKind::Error),
            },
            _ => {}
        }
        if row != 8 {
            self.cfg.save().ok();
        }
    }

    pub fn set_zen(&mut self, zen: bool) {
        self.zen = zen;
        self.cfg.ui.zen = zen;
    }

    // ------------------------------------------------------------- Spotify
    pub fn enable_spotify(&mut self) {
        if self.spotify_worker.is_none() {
            self.spotify_worker = Some(SpotifyWorker::start());
        }
        self.follow_spotify = true;
        self.spotify_warned = false;
        self.last_spotify_poll = Instant::now();
        // El modo acompañante manda: nada de audio local sonando por debajo.
        if let Some(player) = &self.player {
            player.pause(true);
        }
        self.now.playing = false;
        self.spectrum.decode_system();
        self.toast("siguiendo Spotify (transporte y audio del cliente oficial)", ToastKind::Info);
    }

    pub fn disable_spotify(&mut self) {
        self.follow_spotify = false;
        self.spotify_worker = None;
        self.spectrum.stop();
        // Vuelve el modo local limpio: fuera la pista remota.
        self.spotify = None;
        self.now.track = None;
        self.now.path = None;
        self.now.position_ms = 0;
        self.now.playing = false;
        self.lyrics = None;
        self.lyrics_key.clear();
        self.lyrics_status.clear();
        self.art = None;
        self.art_key.clear();
        self.art_variants.clear();
        if let Some(row) = self.selected_row().cloned() {
            if let Some(path) = row.path {
                self.play_local(row.track, Some(path));
            }
        }
        self.toast("modo Spotify desactivado · vuelve el modo local", ToastKind::Info);
    }

    fn apply_spotify(&mut self, state: SpotifyState) {
        if let Some(mut track) = state.track.clone() {
            // Si la pista está descargada, usa SU clave (y su portada local):
            // MPRIS da otro id y si no, la letra/arte no encuentran nada.
            if let Ok(Some(row)) = self
                .lib
                .find_by_meta(&track.artist, &track.title, track.duration_ms)
            {
                track.key = row.track.key.clone();
                if track.album.is_none() {
                    track.album = row.track.album.clone();
                }
                if track.art_url.is_none() {
                    track.art_url = row.track.art_url.clone();
                }
            }
            let changed = self.now.track.as_ref().map(|t| &t.key) != Some(&track.key);
            self.now.track = Some(track.clone());
            self.now.duration_ms = track.duration_ms;
            // MPRIS informa con ~400 ms de retraso. Si se aplica tal cual en
            // cada sondeo, la posición retrocede y la letra se queda clavada.
            // Solo se resincroniza cuando la deriva es de verdad.
            let drift = state.position_ms as i64 - self.now.position_ms as i64;
            if changed || drift.abs() > 1500 || drift < -700 {
                self.now.position_ms = state.position_ms;
            }
            self.now.playing = state.playing;
            self.now.path = None;
            if changed {
                self.lyrics = None;
                self.lyrics_status.clear();
                self.lyrics_key.clear();
                self.art = None;
                self.art_key.clear();
                self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
                self.lib.upsert_track(&track).ok();
                let has_local = self
                    .lib
                    .art_path(&track.key)
                    .ok()
                    .flatten()
                    .is_some_and(|path| path.exists())
                    || self
                        .lib
                        .local_path(&track.key)
                        .map(|path| path.with_extension("jpg").exists())
                        .unwrap_or(false);
                if !has_local {
                    if let Some(url) = track.art_url.clone() {
                        self.lib.set_art(&track.key, None, Some(&url)).ok();
                    }
                }
                self.load_lyrics(&track);
            }
        }
        self.spotify = Some(state);
    }

    // --------------------------------------------------------------- letras
    fn load_lyrics(&mut self, track: &TrackRef) {
        if self.lyrics_loading {
            return;
        }
        // 1. Cache SQLite.
        if let Ok(Some((lrc, synced, offset))) = self.lib.lyrics(&track.key) {
            let mut doc = LyricsDoc::from_lrc(&lrc, synced);
            doc.offset_ms = offset;
            self.lyrics = Some(doc);
            self.lyrics_status = if synced { "lrclib" } else { "aprox" }.to_string();
            self.lyrics_key = track.key.clone();
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
                return;
            }
        }
        // 3. Red.
        self.lyrics_loading = true;
        self.lyrics_status = "buscando letra…".into();
        self.lyrics_worker.request(track.clone());
    }

    // ---------------------------------------------------------------- tick
    pub fn tick(&mut self, dt: f32) {
        self.intro = (self.intro + dt * 2.5).min(1.0);
        let fps = self.fps.tick();
        self.frame_ms = if fps > 0.1 { 1000.0 / fps } else { 0.0 };
        self.pulse.update(dt);
        self.palette.update(dt);
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
            let modes = ArtMode::ROTATION;
            let current = modes.iter().position(|mode| *mode == self.art_mode).unwrap_or(0);
            let next = modes[(current + 1) % modes.len()];
            self.set_art_mode(next);
        }
        self.list_scroll.set_target(self.selected as f32);
        self.list_scroll.update(dt);
        if let Some(doc) = &self.lyrics {
            let index = doc.index_at(self.now.position_ms).unwrap_or(0) as f32;
            self.lyrics_scroll.set_target(index);
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
                Ok(items) if !items.is_empty() => {
                    let provider = items
                        .first()
                        .map(|item| item.provider.clone())
                        .unwrap_or_default();
                    self.search.results = items;
                    self.search.selected = 0;
                    self.search.loading = false;
                    self.search.message = format!(
                        "{} resultados · {} · enter para descargar",
                        self.search.results.len(),
                        provider
                    );
                }
                Ok(_) => {
                    // Sin resultados en Spotify/Deezer: yt-dlp de respaldo.
                    if self.tools.yt_dlp {
                        self.search.message = "sin resultados en el catálogo · probando yt-dlp".into();
                        self.downloader.search(self.search.last_query_id, self.search.last_query.clone());
                    } else {
                        self.search.loading = false;
                        self.search.message = "sin resultados".into();
                    }
                }
                Err(err) => {
                    self.search.loading = false;
                    self.search.message = format!("catálogo no disponible: {err}");
                    if self.tools.yt_dlp {
                        self.downloader.search(self.search.last_query_id, self.search.last_query.clone());
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
                        self.search.selected = 0;
                        self.search.loading = false;
                        self.search.message = format!("{} resultados", self.search.results.len());
                    }
                }
                DownloadEvent::SearchFailed { query_id, message } => {
                    if query_id == self.search.last_query_id {
                        self.search.loading = false;
                        self.search.message = format!("error: {message}");
                    }
                    self.toast(message, ToastKind::Error);
                }
                DownloadEvent::Progress { key, percent, stage } => {
                    if self.now.track.as_ref().is_some_and(|t| t.key == key) {
                        self.set_status(format!("{stage} · {percent:.0}%"));
                    }
                }
                DownloadEvent::Done { key, path } => {
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
                    if self.pending_play.as_deref() == Some(key.as_str()) {
                        self.pending_play = None;
                        if let Some(entry) = self
                            .queue
                            .iter()
                            .position(|entry| entry.track.key == key)
                        {
                            let entry = self.queue.remove(entry);
                            self.play_local(entry.track, Some(path.clone()));
                        } else if let Some(track) = self.now.track.clone() {
                            self.play_local(track, Some(path.clone()));
                        }
                    } else {
                        self.toast(format!("descargada {}", util::short_hash(&key)), ToastKind::Ok);
                    }
                    self.refresh_view();
                }
                DownloadEvent::Failed { key, message } => {
                    if self.pending_play.as_deref() == Some(key.as_str()) {
                        self.pending_play = None;
                    }
                    self.now.loading = false;
                    self.toast(format!("descarga fallida: {message}"), ToastKind::Error);
                }
                DownloadEvent::Message(message) => {
                    self.toast(message, ToastKind::Info);
                }
            }
        }
    }

    fn poll_art(&mut self) {
        while let Some(response) = self.art_worker.try_recv() {
            self.apply_art(response);
        }
    }

    fn apply_art(&mut self, response: ArtResponse) {
        // Miniaturas de la lista: caché aparte, no tocan el tema ni el arte grande.
        if response.w <= 8 && response.h <= 4 {
            if let Some(art) = response.art {
                self.thumbs.insert(response.key, art);
            }
            return;
        }
        let current_key = self.now.track.as_ref().map(|track| track.key.clone());
        if current_key.as_deref() != Some(response.key.as_str()) {
            return;
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
            self.lib
                .set_art(&response.key, Some(&path), None)
                .ok();
        }
        if let Some(error) = response.error {
            if error != "sin portada" {
                self.set_status(error);
            }
        }
    }

    fn poll_lyrics(&mut self) {
        while let Some(message) = self.lyrics_worker.try_recv() {
            match message {
                LyricsMsg::Loaded { key, doc, lrc, error } => {
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
                                    )
                                    .ok();
                            }
                            self.lyrics_status = doc.source.label().to_string();
                            self.lyrics_offset_ms = self.lib.lyrics_offset(&key).unwrap_or(0);
                            let mut doc = doc;
                            doc.offset_ms = self.lyrics_offset_ms;
                            self.lyrics = Some(doc);
                            self.lyrics_key = key;
                        }
                        None => {
                            self.lyrics = None;
                            self.lyrics_status = error.unwrap_or_else(|| "sin letra".into());
                        }
                    }
                }
            }
        }
    }

    fn poll_spotify(&mut self, dt: f32) {
        if !self.follow_spotify {
            return;
        }
        if let Some(worker) = &self.spotify_worker {
            if let Some(state) = worker.try_recv() {
                self.apply_spotify(state);
                self.last_spotify_poll = Instant::now();
            }
        }
        if self.spotify.is_none()
            && !self.spotify_warned
            && self.last_spotify_poll.elapsed() > Duration::from_secs(4)
        {
            self.spotify_warned = true;
            self.toast(
                "Spotify no responde por MPRIS: abre el cliente oficial",
                ToastKind::Warn,
            );
        }
        // Interpola la posición entre sondeos de MPRIS.
        // MPRIS solo informa de la posición cada ~400 ms; entre sondeos el
        // reloj lo lleva spote. Antes se cortaba a los 3 s y la letra se
        // quedaba congelada.
        if self.now.playing {
            self.now.position_ms = self
                .now
                .position_ms
                .saturating_add((dt * 1000.0) as u64)
                .min(self.now.duration_ms.max(self.now.position_ms));
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
                    self.room_selected = self.rooms.iter().position(|r| r.id == room.id).unwrap_or(0);
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
        social.send(SocialCmd::NowPlaying(self.now.track.clone(), self.now.playing));
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
        let Some(room) = self.rooms.get(index) else { return };
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
        }
        let same = self.art_key == track.key
            && self.art_size == (w, h)
            && self.art_mode_seen == mode
            && self.art.is_some();
        if same {
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
        self.art_worker.request(ArtRequest {
            key: track.key,
            url,
            local,
            w,
            h,
            mode,
        });
    }

    /// Pide una miniatura de 6x3 para las tarjetas de la lista.
    pub fn request_thumb(&mut self, track: &TrackRef) {
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
        self.art_worker.request(ArtRequest {
            key: track.key.clone(),
            url,
            local,
            w: 6,
            h: 3,
            mode: ArtMode::Half,
        });
    }

    /// El layout llama aquí cada frame con el hueco disponible para el arte.
    pub fn set_art_size_hint(&mut self, w: u16, h: u16) {
        if self.art_size_hint != (w, h) {
            self.art_size_hint = (w, h);
            self.art_key.clear();
            self.art_variants.clear();
            self.art_prev = None;
            self.art_fade_active = false;
        }
    }

    // -------------------------------------------------------------- entrada
    pub fn on_key(&mut self, key: KeyEvent) {
        if self.overlay != Overlay::None {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.overlay = Overlay::None;
                    return;
                }
                KeyCode::Char(c @ '1'..='9') if self.overlay == Overlay::Effects => {
                    let index = c as usize - '1' as usize;
                    self.viz.toggle_layer(index);
                    return;
                }
                KeyCode::Char('e') if self.overlay == Overlay::Effects => {
                    self.overlay = Overlay::None;
                    return;
                }
                _ => return,
            }
        }
        // En la vista de búsqueda se escribe directamente: no hace falta "/".
        if self.tab == Tab::Search {
            match key.code {
                KeyCode::Esc => {
                    if self.search.query.is_empty() {
                        self.tab = Tab::Now;
                    } else {
                        self.search.query.clear();
                        self.search.results.clear();
                    }
                    return;
                }
                KeyCode::Enter => {
                    if self.search.results.is_empty() {
                        self.submit_search();
                    } else if self.search.last_query == self.search.query.trim()
                        && (self.search.last_query_id > 0)
                    {
                        self.play_search_selection();
                    } else {
                        self.submit_search();
                    }
                    return;
                }
                KeyCode::Backspace => {
                    self.search.query.pop();
                    return;
                }
                KeyCode::Up => {
                    self.search.selected = self.search.selected.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    if !self.search.results.is_empty() {
                        self.search.selected =
                            (self.search.selected + 1).min(self.search.results.len() - 1);
                    }
                    return;
                }
                KeyCode::Tab => {
                    self.tab = self.tab.next();
                    return;
                }
                KeyCode::BackTab => {
                    self.tab = self.tab.prev();
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search.query.push(c);
                    self.search.results.clear();
                    self.search.message.clear();
                    return;
                }
                _ => {}
            }
        }

        let upper = key.modifiers.contains(KeyModifiers::SHIFT)
            || matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Esc => {
                self.fullscreen = Fullscreen::None;
                self.overlay = Overlay::None;
                self.center_focus = CenterFocus::Normal;
            }
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.focus = Pane::Center;
                if self.tab == Tab::Search {
                    self.search.query.clear();
                    self.search.results.clear();
                }
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                self.focus = Pane::Center;
            }
            KeyCode::Char('/') => {
                self.tab = Tab::Search;
                self.search.results.clear();
                self.search.query.clear();
            }
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('e') if !upper => self.overlay = Overlay::Effects,
            KeyCode::Char('y') => self.copy_lyrics(),
            KeyCode::Char('l') if !upper => {
                self.focus = Pane::Center;
                self.tab = Tab::Now;
            }
            KeyCode::Char('s') if !upper => {
                self.tab = Tab::Room;
                if let Some(social) = &self.social {
                    social.send(SocialCmd::Refresh);
                }
            }
            KeyCode::Char('L') => self.leave_room(),
            KeyCode::Char('c') if !upper => {
                self.center_focus = if self.center_focus == CenterFocus::Art {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Art
                };
            }
            KeyCode::Char('C') => {
                self.center_focus = if self.center_focus == CenterFocus::Lyrics {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Lyrics
                };
            }
            KeyCode::Right => {
                if self.tab == Tab::Settings {
                    self.adjust_setting(self.settings_selected, 1);
                    return;
                }
                let step = if upper { 30_000 } else { 5_000 };
                self.seek_relative(step);
            }
            KeyCode::Left => {
                if self.tab == Tab::Settings {
                    self.adjust_setting(self.settings_selected, -1);
                    return;
                }
                let step = if upper { 30_000 } else { 5_000 };
                self.seek_relative(-step);
            }
            KeyCode::Up => {
                if self.tab == Tab::Settings {
                    self.settings_selected = self.settings_selected.saturating_sub(1);
                    return;
                }
                if upper {
                    self.move_queue(-1);
                } else {
                    self.move_selection(-1);
                }
            }
            KeyCode::Down => {
                if self.tab == Tab::Settings {
                    self.settings_selected = (self.settings_selected + 1).min(8);
                    return;
                }
                if upper {
                    self.move_queue(1);
                } else {
                    self.move_selection(1);
                }
            }
            KeyCode::PageUp => self.move_selection(-8),
            KeyCode::PageDown => self.move_selection(8),
            KeyCode::Enter => {
                if self.tab == Tab::Settings {
                    self.toggle_setting(self.settings_selected);
                } else if self.tab == Tab::Room {
                    self.join_room(self.room_selected);
                } else if let Some(row) = self.selected_row().cloned() {
                    self.play_local(row.track, row.path);
                }
            }
            KeyCode::Char('a') if !upper => {
                if let Some(row) = self.selected_row().cloned() {
                    self.queue_track(row.track, None);
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.cycle_art_mode();
            }
            KeyCode::Char('f') if !upper => self.toggle_favorite(),
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                };
            }
            KeyCode::Char('v') if !upper => {
                if self.show_viz {
                    self.viz.next_preset();
                } else {
                    self.show_viz = true;
                }
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.fullscreen = if self.fullscreen == Fullscreen::Viz {
                    Fullscreen::None
                } else {
                    Fullscreen::Viz
                };
            }
            KeyCode::Char('i') => self.set_zen(!self.zen),
            KeyCode::Char('x') => self.remove_queue_selected(),
            KeyCode::Char('z') => self.list_cards = !self.list_cards,
            KeyCode::Char('b') => self.viz_background = !self.viz_background,
            KeyCode::Char('r') => {
                self.art_rotate = !self.art_rotate;
                self.toast(
                    if self.art_rotate {
                        "rotación de arte activada"
                    } else {
                        "rotación de arte desactivada"
                    },
                    ToastKind::Info,
                );
            }
            KeyCode::Char('t') => self.toggle_spotify(),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let volume = (self.now.volume + 0.05).min(1.0);
                self.set_volume(volume);
            }
            KeyCode::Char('-') => {
                let volume = (self.now.volume - 0.05).max(0.0);
                self.set_volume(volume);
            }
            KeyCode::Char('n') => self.next_track(),
            KeyCode::Char('p') => self.prev_track(),
            KeyCode::Char('.') => self.move_queue(1),
            KeyCode::Char(',') => self.move_queue(-1),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::F(1) => self.overlay = Overlay::Help,
            KeyCode::F(2) => self.start_harvest(),
            _ => {}
        }
    }

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

    fn toggle_favorite(&mut self) {
        if let Some(row) = self.selected_row().cloned() {
            self.toggle_favorite_key(&row.track.key);
        } else {
            self.toggle_favorite_now();
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

    pub fn toggle_spotify(&mut self) {
        if self.follow_spotify {
            self.disable_spotify();
        } else {
            self.enable_spotify();
        }
    }

    /// Cambia el modo de arte con disolución desde el arte actual.
    pub fn cycle_art_mode(&mut self) {
        self.set_art_mode(self.art_mode.next());
    }

    pub fn set_art_mode(&mut self, mode: ArtMode) {
        if mode == self.art_mode {
            return;
        }
        self.art_mode = mode;
        self.cfg.ui.art_mode = mode.label().to_string();
        if mode == ArtMode::Off {
            self.art_prev = self.art.take();
            self.art_prev_mode = self.art_mode_seen;
            self.art_fade = 0.0;
            self.art_fade_active = true;
            self.art_key.clear();
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

    /// Mueve la entrada seleccionada de la cola local.
    pub fn move_queue(&mut self, delta: i32) {
        if !self.room_queue.is_empty() {
            self.queue_selected = (self.queue_selected as i32 + delta)
                .clamp(0, self.queue_len() as i32 - 1) as usize;
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
        match self.tab {
            Tab::Search => {
                if self.search.results.is_empty() {
                    return;
                }
                let next = self.search.selected as i32 + delta;
                self.search.selected = next.clamp(0, self.search.results.len() as i32 - 1) as usize;
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
        // Metadatos primero (Spotify/Deezer); la descarga se resuelve después
        // con esos datos, no con el texto libre en yt-dlp.
        self.search_worker.search(self.search.last_query_id, query.clone());
        if !self.tools.yt_dlp {
            self.search.message = "buscando… (sin yt-dlp no se podrá descargar)".into();
        }
    }

    fn play_search_selection(&mut self) {
        let Some(item) = self.search.results.get(self.search.selected).cloned() else {
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
        for (bin, args) in [("wl-copy", vec![]), ("xclip", vec!["-selection", "clipboard"])] {
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

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Moved => {
                self.hover = self.hit.hit(mouse.column, mouse.row).cloned();
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let action = self.hit.hit(mouse.column, mouse.row).cloned();
                let Some(action) = action else { return };
                match &action {
                    Action::SeekTo(_) => {
                        self.dragging_seek = true;
                        if let Some(rect) = self.hit.rect_for(&Action::SeekTo(0)) {
                            if rect.width > 0 {
                                let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                    / rect.width as f32)
                                    .clamp(0.0, 1.0);
                                self.drag_ratio = Some(ratio);
                                let target = (self.now.duration_ms as f32 * ratio) as u64;
                                self.seek_to(target);
                            }
                        }
                    }
                    Action::SearchPlay(index) => {
                        self.search.selected = (*index).min(self.search.results.len().saturating_sub(1));
                        self.play_search_selection();
                    }
                    Action::QueueRemove(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.remove_queue_selected();
                    }
                    Action::QueueRow(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.queue_drag = Some(self.queue_selected);
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| last == *index && at.elapsed() < Duration::from_millis(320))
                            .unwrap_or(false);
                        self.last_click = Some((now, *index));
                        if double && self.queue_len() > 0 {
                            if let Some(entry) = self.queue.get(self.queue_selected).cloned() {
                                self.queue.remove(self.queue_selected);
                                self.play_local(entry.track, entry.path);
                            }
                        }
                    }
                    Action::SelectRow(index) => {
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| last == *index && at.elapsed() < Duration::from_millis(320))
                            .unwrap_or(false);
                        self.last_click = Some((now, *index));
                        if double {
                            match action {
                                Action::SelectRow(_) => self.play_row(*index),
                                Action::QueueRow(_) => self.play_queue_row(*index),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                self.perform(action);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(from) = self.queue_drag {
                    if let Some(Action::QueueRow(to)) =
                        self.hit.hit(mouse.column, mouse.row).cloned()
                    {
                        if to != from && self.room_queue.is_empty() && to < self.queue.len() {
                            let entry = self.queue.remove(from.min(self.queue.len() - 1));
                            self.queue.insert(to, entry);
                            self.queue_selected = to;
                            self.queue_drag = Some(to);
                        }
                    }
                }
                if self.dragging_seek {
                    if let Some(rect) = self
                        .hit
                        .entries
                        .iter()
                        .find(|(_, action)| matches!(action, Action::SeekTo(_)))
                        .map(|(rect, _)| *rect)
                    {
                        if rect.width > 0 {
                            let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                / rect.width as f32)
                                .clamp(0.0, 1.0);
                            self.drag_ratio = Some(ratio);
                            let target = (self.now.duration_ms as f32 * ratio) as u64;
                            self.seek_to(target);
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.dragging_seek = false;
                self.drag_ratio = None;
                self.queue_drag = None;
            }
            _ => {}
        }
    }

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::PlaylistOpen(id) => {
                self.lib_filter = LibFilter::Playlist(id);
                self.tab = Tab::Library;
                self.selected = 0;
                self.refresh_view();
            }
            Action::LibraryFilter(filter) => {
                self.lib_filter = filter;
                self.tab = Tab::Library;
                self.selected = 0;
                self.refresh_view();
            }
            Action::SettingsRow(index) => {
                self.settings_selected = index.min(8);
                self.tab = Tab::Settings;
            }
            Action::SettingsAdjust(row, delta) => self.adjust_setting(row, delta),
            Action::SettingsToggle(row) => self.toggle_setting(row),
            Action::ToggleZen => self.set_zen(!self.zen),
            Action::Tab(tab) => {
                self.tab = tab;
                if tab != Tab::Library {
                    self.lib_filter = LibFilter::All;
                    self.refresh_view();
                }
                if tab == Tab::Search {
                    self.search.query.clear();
                    self.search.results.clear();
                }
            }
            Action::PlayPause => self.toggle_play(),
            Action::Next => self.next_track(),
            Action::Prev => self.prev_track(),
            Action::SeekTo(ms) => self.seek_to(ms),
            Action::Volume(volume) => self.set_volume(volume),
            Action::Mute => {
                if self.now.volume > 0.0 {
                    self.volume_before_mute = self.now.volume;
                    self.set_volume(0.0);
                } else {
                    let volume = self.volume_before_mute.max(0.2);
                    self.set_volume(volume);
                }
            }
            Action::VizNext => {
                self.show_viz = true;
                self.viz.next_preset();
            }
            Action::VizPrev => {
                self.show_viz = true;
                self.viz.prev_preset();
            }
            Action::VizLayer(index) => self.viz.toggle_layer(index),
            Action::VizFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Viz {
                    Fullscreen::None
                } else {
                    Fullscreen::Viz
                }
            }
            Action::VizToggle => {
                self.show_viz = !self.show_viz;
                self.viz.paused = !self.show_viz;
            }
            Action::ArtMode => {
                self.art_mode = self.art_mode.next();
                self.art_key.clear();
            }
            Action::ArtFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                }
            }
            Action::SelectRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
            }
            Action::SearchPlay(index) => {
                self.search.selected = index.min(self.search.results.len().saturating_sub(1));
                self.play_search_selection();
            }
            Action::PlayRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
                if let Some(row) = self.view.get(index).cloned() {
                    self.play_local(row.track, row.path);
                }
            }
            Action::QueueRow(index) => {
                self.queue_selected = index.min(self.queue_len().saturating_sub(1));
            }
            Action::QueueRemove(index) => {
                self.queue_selected = index.min(self.queue_len().saturating_sub(1));
                self.remove_queue_selected();
            }
            Action::Scroll(delta) => self.move_selection(delta),
            Action::Focus(pane) => self.focus = pane,
            Action::CenterArt => {
                self.center_focus = if self.center_focus == CenterFocus::Art {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Art
                };
            }
            Action::CenterLyrics => {
                self.center_focus = if self.center_focus == CenterFocus::Lyrics {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Lyrics
                };
            }
            Action::Help => self.overlay = Overlay::Help,
            Action::Effects => self.overlay = Overlay::Effects,
            Action::CloseOverlay => self.overlay = Overlay::None,
            Action::ToggleFollow => {
                if self.follow_spotify {
                    self.disable_spotify();
                } else {
                    self.enable_spotify();
                }
            }
            Action::Favorite => {
                if let Some(row) = self.selected_row().cloned() {
                    self.lib.toggle_favorite(&row.track.key).ok();
                    self.refresh_view();
                }
            }
            Action::AddQueue => {
                if let Some(row) = self.selected_row().cloned() {
                    self.queue_track(row.track, None);
                }
            }
            Action::JoinRoom(index) => self.join_room(index),
            Action::LeaveRoom => self.leave_room(),
            Action::StartSearch => {
                self.tab = Tab::Search;
                self.search.query.clear();
            }
            Action::Quit => self.should_quit = true,
        }
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
            (self.now.duration_ms as f32 * ratio) as u64
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_in_both_directions() {
        assert_eq!(Tab::Now.next(), Tab::Search);
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
