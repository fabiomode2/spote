//! Servidor MPRIS de spote.
//!
//! Publica `org.mpris.MediaPlayer2.spote` en el bus de sesión para que el
//! entorno (GNOME, KDE, playerctl, barras, llaveros multimedia…) vea lo que
//! suena y pueda mandar órdenes. Es la otra mitad de [`crate::spotify`] y
//! [`crate::system_media`]: allí spote es cliente de otros reproductores; aquí
//! es el reproductor.
//!
//! El hilo del servidor tiene su propio runtime current-thread. `update()`
//! solo deja un snapshot en una cola y vuelve; la emisión de
//! `PropertiesChanged` ocurre en ese hilo. `try_recv()` es síncrono y no
//! bloquea a la UI.
//!
//! El bus real no se puede ejercitar en tests headless: los tests de este
//! módulo cubren el cálculo puro de metadatos, el object path del trackid y el
//! diff entre snapshots.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedVariant, Variant};

use crate::util;

pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.spote";
pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK_PATH: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// Estado que spote publica por MPRIS. Lo rellena la capa que conoce al
/// reproductor (mpv) y se pasa a [`MprisServer::update`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MprisSnapshot {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub art_url: Option<String>,
    pub track_key: String,
    /// 0.0..=1.0. Valores fuera de rango se recortan.
    pub volume: f32,
    pub shuffle: bool,
    /// `"off"`, `"all"` o `"one"`. Cualquier otra cosa se degrada a `"off"`.
    pub repeat: String,
}

impl MprisSnapshot {
    /// Recorta los campos que MPRIS define con un dominio cerrado.
    fn normalized(mut self) -> Self {
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.repeat = match self.repeat.as_str() {
            "all" => "all".to_string(),
            "one" => "one".to_string(),
            _ => "off".to_string(),
        };
        self
    }

    fn playback_status(&self) -> &'static str {
        if self.playing {
            "Playing"
        } else {
            "Paused"
        }
    }

    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        let mut metadata = HashMap::new();
        metadata.insert(
            "mpris:trackid".to_string(),
            owned_object_path(&track_object_path(self)),
        );
        metadata.insert("xesam:title".to_string(), owned_str(&self.title));
        let artists = if self.artist.trim().is_empty() {
            Vec::new()
        } else {
            vec![self.artist.clone()]
        };
        metadata.insert("xesam:artist".to_string(), owned_strings(&artists));
        if let Some(album) = self
            .album
            .as_deref()
            .filter(|album| !album.trim().is_empty())
        {
            metadata.insert("xesam:album".to_string(), owned_str(album));
        }
        metadata.insert(
            "mpris:length".to_string(),
            OwnedVariant::from(micros(self.duration_ms)),
        );
        if let Some(art) = self.art_url.as_deref().filter(|art| !art.trim().is_empty()) {
            metadata.insert("mpris:artUrl".to_string(), owned_str(art));
        }
        metadata
    }

    fn volume_f64(&self) -> f64 {
        f64::from(self.volume.clamp(0.0, 1.0))
    }

    fn position_us(&self) -> i64 {
        micros(self.position_ms)
    }
}

/// Órdenes que llegan desde el bus. La UI las consume con
/// [`MprisServer::try_recv`] y las traduce a su motor de reproducción.
#[derive(Debug, Clone, PartialEq)]
pub enum MprisCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    /// Desplazamiento relativo en microsegundos (como el `Seek` de MPRIS).
    SeekRelative(i64),
    /// Posición absoluta en milisegundos.
    SetPosition(u64),
    SetVolume(f32),
    SetShuffle(bool),
    /// `"off"`, `"all"` o `"one"`.
    SetRepeat(String),
    OpenUri(String),
    Raise,
    Quit,
}

enum UpdateMsg {
    Snapshot(MprisSnapshot),
    Shutdown,
}

/// Handle del servidor. Vive mientras spote quiera aparecer en el bus; al
/// soltarlo se cierra la conexión y el hilo termina.
pub struct MprisServer {
    updates: tokio::sync::mpsc::UnboundedSender<UpdateMsg>,
    commands: Mutex<Receiver<MprisCommand>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MprisServer {
    /// Arranca el servidor y espera a que el nombre esté pedido.
    ///
    /// Devuelve un error claro si el bus de sesión no existe o si
    /// `org.mpris.MediaPlayer2.spote` ya está ocupado por otro proceso. La app
    /// decide si seguir sin MPRIS; este constructor no tumba nada por sí solo.
    pub fn start(identity: &str) -> Result<Self> {
        let identity = identity.trim();
        let identity = if identity.is_empty() {
            "spote".to_string()
        } else {
            identity.to_string()
        };

        let shared = Arc::new(SharedState::default());
        let (commands_tx, commands_rx) = mpsc::channel();
        let (updates_tx, updates_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("spote-mpris".to_string())
            .spawn(move || run_server(identity, shared, updates_rx, commands_tx, ready_tx))
            .context("no pude crear el hilo de MPRIS")?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                updates: updates_tx,
                commands: Mutex::new(commands_rx),
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                drop(updates_tx);
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                drop(updates_tx);
                Err(anyhow!(
                    "MPRIS no respondió en 5 s al publicar {BUS_NAME}; ¿el bus de sesión está disponible?"
                ))
            }
        }
    }

    /// Sustituye el snapshot visible para el bus. No bloquea: si el hilo del
    /// servidor ya murió, el envío se descarta.
    pub fn update(&self, snapshot: MprisSnapshot) {
        let _ = self
            .updates
            .send(UpdateMsg::Snapshot(snapshot.normalized()));
    }

    /// Saca la siguiente orden sin esperar. `None` si no hay ninguna.
    pub fn try_recv(&self) -> Option<MprisCommand> {
        let receiver = self
            .commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match receiver.try_recv() {
            Ok(command) => Some(command),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for MprisServer {
    fn drop(&mut self) {
        let _ = self.updates.send(UpdateMsg::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Hilo del servidor
// ---------------------------------------------------------------------------

fn run_server(
    identity: String,
    shared: Arc<SharedState>,
    mut updates: tokio::sync::mpsc::UnboundedReceiver<UpdateMsg>,
    commands: Sender<MprisCommand>,
    ready: Sender<Result<()>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = ready.send(Err(anyhow!("runtime de MPRIS: {err}")));
            return;
        }
    };

    runtime.block_on(async move {
        let root = RootInterface {
            identity,
            commands: commands.clone(),
        };
        let player = PlayerInterface {
            shared: Arc::clone(&shared),
            commands,
        };

        let connection = zbus::connection::Builder::session()
            .name(BUS_NAME)
            .allow_name_replacements(true)
            .replace_existing_names(false)
            .serve_at(OBJECT_PATH, root)
            .serve_at(OBJECT_PATH, player)
            .build()
            .await;

        let connection = match connection {
            Ok(connection) => connection,
            Err(err) => {
                let _ = ready.send(Err(anyhow!(
                    "no pude publicar {BUS_NAME} en el bus de sesión \
                     (¿otra instancia de spote ya lo ocupa?): {err}"
                )));
                return;
            }
        };

        let interface = match InterfaceName::from_static_str(PLAYER_IFACE) {
            Ok(interface) => interface,
            Err(err) => {
                let _ = ready.send(Err(anyhow!("nombre de interfaz MPRIS inválido: {err}")));
                return;
            }
        };
        let emitter = match SignalEmitter::new(&connection, OBJECT_PATH) {
            Ok(emitter) => emitter,
            Err(err) => {
                let _ = ready.send(Err(anyhow!("object path MPRIS inválido: {err}")));
                return;
            }
        };

        let _ = ready.send(Ok(()));

        while let Some(message) = updates.recv().await {
            match message {
                UpdateMsg::Shutdown => break,
                UpdateMsg::Snapshot(snapshot) => {
                    let changed = shared.replace(snapshot);
                    if changed.is_empty() {
                        continue;
                    }
                    let properties = changed
                        .into_iter()
                        .map(|(name, value)| (name, Variant::from(value)))
                        .collect::<HashMap<&'static str, Variant<'static>>>();
                    if let Err(err) = zbus::fdo::Properties::properties_changed(
                        &emitter,
                        interface.clone(),
                        properties,
                        Cow::Borrowed(&[]),
                    )
                    .await
                    {
                        tracing::debug!("MPRIS: no pude emitir PropertiesChanged: {err}");
                    }
                }
            }
        }
    });
}

#[derive(Default)]
struct SharedState {
    current: Mutex<MprisSnapshot>,
}

impl SharedState {
    fn snapshot(&self) -> MprisSnapshot {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn replace(&self, next: MprisSnapshot) -> HashMap<&'static str, OwnedVariant> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = snapshot_diff(&current, &next);
        *current = next;
        changed
    }
}

fn snapshot_diff(old: &MprisSnapshot, new: &MprisSnapshot) -> HashMap<&'static str, OwnedVariant> {
    let mut changed = HashMap::new();
    if old.playing != new.playing {
        changed.insert("PlaybackStatus", owned_str(new.playback_status()));
    }
    if old.repeat != new.repeat {
        changed.insert("LoopStatus", owned_str(loop_status_name(&new.repeat)));
    }
    if old.shuffle != new.shuffle {
        changed.insert("Shuffle", OwnedVariant::from(new.shuffle));
    }
    if (old.volume - new.volume).abs() > f32::EPSILON {
        changed.insert("Volume", OwnedVariant::from(new.volume_f64()));
    }
    if old.title != new.title
        || old.artist != new.artist
        || old.album != new.album
        || old.duration_ms != new.duration_ms
        || old.art_url != new.art_url
        || old.track_key != new.track_key
    {
        changed.insert("Metadata", metadata_variant(new));
    }
    changed
}

fn metadata_variant(snapshot: &MprisSnapshot) -> OwnedVariant {
    OwnedVariant::from(snapshot.metadata())
}

/// Object path estable para `mpris:trackid`: un hash del `track_key` de
/// spote. Sin pista se usa el path canónico de MPRIS.
fn track_object_path(snapshot: &MprisSnapshot) -> String {
    let key = snapshot.track_key.trim();
    if key.is_empty() {
        if snapshot.title.trim().is_empty() {
            return NO_TRACK_PATH.to_string();
        }
        let fallback = format!(
            "{}|{}|{}",
            snapshot.artist, snapshot.title, snapshot.duration_ms
        );
        return hashed_track_path(&fallback);
    }
    hashed_track_path(key)
}

fn hashed_track_path(key: &str) -> String {
    format!("/org/spote/track/{}", util::hex_sha1(key.as_bytes()))
}

fn micros(ms: u64) -> i64 {
    i64::try_from(ms.saturating_mul(1000)).unwrap_or(i64::MAX)
}

fn owned_str(value: &str) -> OwnedVariant {
    OwnedVariant::try_from(Variant::from(value)).expect("un string siempre es un variant válido")
}

fn owned_strings(values: &[String]) -> OwnedVariant {
    let array = zbus::Array::from(values.to_vec());
    OwnedVariant::try_from(Variant::from(array))
        .expect("un array de strings siempre es un variant válido")
}

fn owned_object_path(path: &str) -> OwnedVariant {
    let path = ObjectPath::from_str_unchecked(path);
    OwnedVariant::from(path)
}

// ---------------------------------------------------------------------------
// Interfaces D-Bus
// ---------------------------------------------------------------------------

struct RootInterface {
    identity: String,
    commands: Sender<MprisCommand>,
}

impl RootInterface {
    fn send(&self, command: MprisCommand) {
        let _ = self.commands.send(command);
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "spote".to_string()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".to_string(), "http".to_string(), "https".to_string()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![
            "audio/mpeg".to_string(),
            "audio/flac".to_string(),
            "audio/ogg".to_string(),
            "audio/opus".to_string(),
            "audio/mp4".to_string(),
            "audio/x-wav".to_string(),
        ]
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    fn raise(&self) {
        self.send(MprisCommand::Raise);
    }

    fn quit(&self) {
        self.send(MprisCommand::Quit);
    }
}

struct PlayerInterface {
    shared: Arc<SharedState>,
    commands: Sender<MprisCommand>,
}

/// MPRIS usa `None/Track/Playlist`, no los nombres internos `off/one/all`.
fn loop_status_name(repeat: &str) -> &'static str {
    match repeat {
        "one" => "Track",
        "all" => "Playlist",
        _ => "None",
    }
}

impl PlayerInterface {
    fn send(&self, command: MprisCommand) -> zbus::fdo::Result<()> {
        self.commands
            .send(command)
            .map_err(|_| zbus::fdo::Error::Failed("spote no está escuchando órdenes".to_string()))
    }

    fn current(&self) -> MprisSnapshot {
        self.shared.snapshot()
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.current().playback_status().to_string()
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        loop_status_name(&self.current().repeat).to_string()
    }

    #[zbus(property)]
    fn set_loop_status(&self, value: String) -> zbus::fdo::Result<()> {
        let repeat = match value.as_str() {
            "None" | "none" | "off" => "off",
            "Playlist" | "playlist" | "all" => "all",
            "Track" | "track" | "one" => "one",
            _ => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "LoopStatus inválido: {value}"
                )))
            }
        };
        self.send(MprisCommand::SetRepeat(repeat.to_string()))
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.current().shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, value: bool) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::SetShuffle(value))
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        self.current().metadata()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.current().volume_f64()
    }

    #[zbus(property)]
    fn set_volume(&self, value: f64) -> zbus::fdo::Result<()> {
        if !value.is_finite() {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Volume debe ser finito".to_string(),
            ));
        }
        let value = value.clamp(0.0, 1.0) as f32;
        self.send(MprisCommand::SetVolume(value))
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.current().position_us()
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    fn next(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Next)
    }

    fn previous(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Previous)
    }

    fn pause(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Pause)
    }

    fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::PlayPause)
    }

    fn stop(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Stop)
    }

    fn play(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Play)
    }

    fn seek(&self, offset: i64) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::SeekRelative(offset))
    }

    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) -> zbus::fdo::Result<()> {
        let snapshot = self.current();
        if track_id.as_str() != track_object_path(&snapshot) {
            // MPRIS: si el path no es la pista actual, la llamada se ignora.
            return Ok(());
        }
        let position_ms = u64::try_from(position).unwrap_or(0) / 1000;
        self.send(MprisCommand::SetPosition(position_ms))
    }

    fn open_uri(&self, uri: &str) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::OpenUri(uri.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Tests (lógica pura, sin bus)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn string(value: &OwnedVariant) -> String {
        String::try_from(value.try_clone().unwrap()).unwrap()
    }

    fn strings(value: &OwnedVariant) -> Vec<String> {
        Vec::<String>::try_from(value.try_clone().unwrap()).unwrap()
    }

    fn number(value: &OwnedVariant) -> i64 {
        i64::try_from(value.try_clone().unwrap()).unwrap()
    }

    fn object_path(value: &OwnedVariant) -> String {
        ObjectPath::try_from(value.try_clone().unwrap())
            .unwrap()
            .as_str()
            .to_string()
    }

    fn snapshot(track_key: &str) -> MprisSnapshot {
        MprisSnapshot {
            playing: true,
            title: "Windowlicker".to_string(),
            artist: "Aphex Twin".to_string(),
            album: Some("Windowlicker".to_string()),
            duration_ms: 366_000,
            position_ms: 12_000,
            art_url: Some("https://example.test/art.jpg".to_string()),
            track_key: track_key.to_string(),
            volume: 0.8,
            shuffle: false,
            repeat: "off".to_string(),
        }
    }

    #[test]
    fn track_path_is_stable_and_derived_from_key() {
        let first = track_object_path(&snapshot("sha1:abc"));
        let second = track_object_path(&snapshot("sha1:abc"));
        let other = track_object_path(&snapshot("sha1:def"));

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(first.starts_with("/org/spote/track/"));
        let hash = first.rsplit('/').next().unwrap();
        assert_eq!(hash.len(), 40);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn empty_key_uses_no_track_or_hash() {
        let mut empty = snapshot("");
        empty.title.clear();
        assert_eq!(track_object_path(&empty), NO_TRACK_PATH);

        let with_title = snapshot("");
        assert_ne!(track_object_path(&with_title), NO_TRACK_PATH);
        assert_eq!(
            track_object_path(&with_title),
            track_object_path(&with_title)
        );
    }

    #[test]
    fn metadata_maps_all_mpris_fields() {
        let snapshot = snapshot("spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        let metadata = snapshot.metadata();

        assert_eq!(
            object_path(metadata.get("mpris:trackid").unwrap()),
            track_object_path(&snapshot)
        );
        assert_eq!(string(metadata.get("xesam:title").unwrap()), "Windowlicker");
        assert_eq!(
            strings(metadata.get("xesam:artist").unwrap()),
            vec!["Aphex Twin".to_string()]
        );
        assert_eq!(string(metadata.get("xesam:album").unwrap()), "Windowlicker");
        assert_eq!(number(metadata.get("mpris:length").unwrap()), 366_000_000);
        assert_eq!(
            string(metadata.get("mpris:artUrl").unwrap()),
            "https://example.test/art.jpg"
        );
    }

    #[test]
    fn metadata_omits_optional_fields_and_keeps_artist_array() {
        let mut snapshot = snapshot("sha1:abc");
        snapshot.album = None;
        snapshot.art_url = None;
        snapshot.artist.clear();
        let metadata = snapshot.metadata();

        assert!(!metadata.contains_key("xesam:album"));
        assert!(!metadata.contains_key("mpris:artUrl"));
        assert!(strings(metadata.get("xesam:artist").unwrap()).is_empty());
    }

    #[test]
    fn snapshot_diff_only_emits_changed_properties() {
        let old = snapshot("sha1:abc");
        let mut next = old.clone();
        next.position_ms = 20_000;
        assert!(snapshot_diff(&old, &next).is_empty());

        next.playing = false;
        next.volume = 0.25;
        next.shuffle = true;
        next.repeat = "one".to_string();
        next.title = "Xtal".to_string();
        let diff = snapshot_diff(&old, &next);
        let mut keys = diff.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "LoopStatus",
                "Metadata",
                "PlaybackStatus",
                "Shuffle",
                "Volume"
            ]
        );
        assert_eq!(string(diff.get("PlaybackStatus").unwrap()), "Paused");
        assert_eq!(string(diff.get("LoopStatus").unwrap()), "Track");
        let metadata = <HashMap<String, OwnedVariant>>::try_from(
            diff.get("Metadata").unwrap().try_clone().unwrap(),
        )
        .unwrap();
        assert_eq!(number(metadata.get("mpris:length").unwrap()), 366_000_000);
    }

    #[test]
    fn shared_state_roundtrips_snapshot() {
        let state = SharedState::default();
        let first = snapshot("sha1:abc");
        assert!(!state.replace(first.clone()).is_empty());
        assert_eq!(state.snapshot(), first);
        assert!(state.replace(first.clone()).is_empty());

        let mut second = first;
        second.position_ms += 5_000;
        second.playing = false;
        let diff = state.replace(second.clone());
        assert_eq!(diff.len(), 1);
        assert!(diff.contains_key("PlaybackStatus"));
        assert_eq!(state.snapshot(), second);
    }

    #[test]
    fn normalization_clamps_volume_and_repeat() {
        let mut snapshot = snapshot("sha1:abc");
        snapshot.volume = 4.5;
        snapshot.repeat = "shuffle".to_string();
        let normalized = snapshot.clone().normalized();
        assert_eq!(normalized.volume, 1.0);
        assert_eq!(normalized.repeat, "off");

        snapshot.volume = f32::NAN;
        assert_eq!(snapshot.normalized().volume, 0.0);
    }

    #[test]
    fn snapshot_diff_treats_track_change_as_metadata() {
        let old = snapshot("sha1:abc");
        let mut next = old.clone();
        next.track_key = "sha1:def".to_string();
        let diff = snapshot_diff(&old, &next);
        assert_eq!(diff.len(), 1);
        assert!(diff.contains_key("Metadata"));
    }
}
