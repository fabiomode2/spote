//! Watcher genérico de reproductores MPRIS del sistema.
//!
//! [`crate::spotify`] sigue exclusivamente al cliente de Spotify. Este módulo
//! hace lo mismo con cualquier reproductor que publique
//! `org.mpris.MediaPlayer2.*` (Spotify, Firefox, VLC, mpv, Elisa, Rhythmbox…):
//! descubre los nombres del bus, elige uno activo y lee su estado. También
//! puede mandarle órdenes.
//!
//! Se excluye a spote (`org.mpris.MediaPlayer2.spote` y sus sufijos de
//! instancia) para no acabar leyéndonos a nosotros mismos.
//!
//! Igual que `SpotifyWatcher`, el runtime de zbus es current-thread y
//! `poll()` bloquea unos milisegundos: se llama desde un hilo dedicado, nunca
//! desde el render. Las acciones (`call`, `seek`, …) son puntuales.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use spote_protocol::{SourceHint, TrackRef};
use zbus::zvariant::OwnedObjectPath;
use zbus::{OwnedVariant, Proxy};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const SESSION_BUS: &str = "org.freedesktop.DBus";

/// Estado de un reproductor externo en un instante.
#[derive(Debug, Clone, Default)]
pub struct SystemMediaState {
    /// Nombre MPRIS completo, por ejemplo `org.mpris.MediaPlayer2.firefox`.
    pub player: String,
    pub playing: bool,
    pub track: Option<TrackRef>,
    pub position_ms: u64,
    /// `false` si el reproductor no publica `Position`; en ese caso no hay
    /// que resincronizar el reloj a 0.
    pub position_known: bool,
    pub volume: Option<f32>,
    pub shuffle: Option<bool>,
    /// Valor literal de `LoopStatus`: `"None"`, `"Track"` o `"Playlist"`.
    pub repeat: Option<String>,
    pub art_url: Option<String>,
}

struct Reading {
    state: SystemMediaState,
    trackid: Option<String>,
    track_key: Option<String>,
}

/// Sondeo bloqueante de MPRIS. Mantiene la conexión y el último reproductor
/// elegido para que la selección no baile entre polls.
pub struct SystemMediaWatcher {
    runtime: tokio::runtime::Runtime,
    connection: Option<zbus::Connection>,
    last_attempt: Instant,
    active: Option<String>,
    active_trackid: Option<String>,
    active_track_key: Option<String>,
    active_position_ms: u64,
    active_position_known: bool,
}

impl SystemMediaWatcher {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime de zbus");
        Self {
            runtime,
            connection: None,
            last_attempt: Instant::now() - Duration::from_secs(10),
            active: None,
            active_trackid: None,
            active_track_key: None,
            active_position_ms: 0,
            active_position_known: false,
        }
    }

    /// `true` si el bus de sesión responde. No dice nada de los reproductores.
    pub fn session_bus_available() -> bool {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        runtime.block_on(async { zbus::Connection::session().await.is_ok() })
    }

    /// Lee el reproductor activo. `None` si no hay bus o no queda ningún
    /// reproductor MPRIS vivo.
    pub fn poll(&mut self) -> Option<SystemMediaState> {
        let existing = self.connection.clone();
        let attempt = if existing.is_none() && self.last_attempt.elapsed() < Duration::from_secs(3)
        {
            return None;
        } else {
            self.last_attempt = Instant::now();
            existing
        };
        let current = self.active.clone();
        let discovered = self.runtime.block_on(discover(attempt, current.as_deref()));

        match discovered {
            Some((connection, Some(reading))) => {
                self.connection = Some(connection);
                self.active = Some(reading.state.player.clone());
                self.active_position_ms = reading.state.position_ms;
                self.active_position_known = reading.state.position_known;
                self.active_trackid = reading.trackid;
                self.active_track_key = reading.track_key;
                Some(reading.state)
            }
            Some((connection, None)) => {
                // El bus sigue bien, simplemente no hay reproductores.
                self.connection = Some(connection);
                self.clear_active();
                None
            }
            None => {
                self.connection = None;
                self.clear_active();
                None
            }
        }
    }

    /// Nombre MPRIS del reproductor elegido en el último `poll()`.
    pub fn player_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// `Play`, `Pause`, `PlayPause`, `Next`, `Previous` o `Stop`.
    pub fn call(&self, method: &str) -> Result<()> {
        if !matches!(
            method,
            "Play" | "Pause" | "PlayPause" | "Next" | "Previous" | "Stop"
        ) {
            bail!("método MPRIS no permitido: {method}");
        }
        let player = self.require_player()?.to_string();
        self.call_method(&player, method, &())
    }

    /// Salto relativo en microsegundos (el `Seek` de MPRIS).
    pub fn seek(&self, offset_us: i64) -> Result<()> {
        let player = self.require_player()?.to_string();
        self.call_method(&player, "Seek", &(offset_us,))
    }

    /// Posición absoluta en milisegundos.
    ///
    /// `track_key_or_path` puede ser el object path MPRIS (`/...`) de la pista
    /// actual o la clave que devolvió `state.track.key` (por ejemplo
    /// `spotify:track:…`). Cualquier otra cosa se rechaza: `SetPosition` con un
    /// trackid ajeno no tiene sentido.
    pub fn set_position(&self, track_key_or_path: &str, position_ms: u64) -> Result<()> {
        let known_track = track_key_or_path.starts_with('/')
            || self.active_track_key.as_deref() == Some(track_key_or_path);
        if !known_track {
            bail!(
                "pista desconocida para SetPosition: {track_key_or_path} \
                 (esperaba el object path o la clave de la pista activa)"
            );
        }
        let path = if track_key_or_path.starts_with('/') {
            Some(track_key_or_path.to_string())
        } else {
            self.active_trackid.clone()
        };
        let player = self.require_player()?.to_string();
        if let Some(path) = path {
            let object = zbus::ObjectPath::try_from(path.as_str())
                .map_err(|err| anyhow!("trackid MPRIS inválido: {err}"))?;
            let position_us = i64::try_from(position_ms.saturating_mul(1000)).unwrap_or(i64::MAX);
            return self.call_method(&player, "SetPosition", &(object, position_us));
        }
        // Sin mpris:trackid (Stremio, algunos navegadores) se usa Seek
        // relativo, que sí entienden todos los reproductores MPRIS.
        let current = if self.active_position_known {
            self.active_position_ms
        } else {
            0
        };
        let delta_ms = position_ms as i64 - current as i64;
        let delta_us = delta_ms.saturating_mul(1000);
        self.call_method(&player, "Seek", &(delta_us,))
    }

    pub fn set_volume(&self, volume: f32) -> Result<()> {
        if !volume.is_finite() {
            bail!("volumen inválido");
        }
        self.set_property("Volume", f64::from(volume.clamp(0.0, 1.0)))
    }

    pub fn set_shuffle(&self, on: bool) -> Result<()> {
        self.set_property("Shuffle", on)
    }

    /// `off`, `all` o `one`; se traduce al `LoopStatus` de MPRIS.
    pub fn set_repeat(&self, mode: &str) -> Result<()> {
        let loop_status = match mode {
            "off" | "None" => "None",
            "all" | "Playlist" => "Playlist",
            "one" | "Track" => "Track",
            other => bail!("modo de repetición inválido: {other}"),
        };
        self.set_property("LoopStatus", loop_status.to_string())
    }

    pub fn open_uri(&self, uri: &str) -> Result<()> {
        let player = self.require_player()?.to_string();
        self.call_method(&player, "OpenUri", &(uri,))
    }

    fn require_player(&self) -> Result<&str> {
        self.active
            .as_deref()
            .ok_or_else(|| anyhow!("no hay reproductor MPRIS activo (llama antes a poll())"))
    }

    fn clear_active(&mut self) {
        self.active = None;
        self.active_trackid = None;
        self.active_track_key = None;
    }

    fn call_method<B>(&self, player: &str, method: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        self.runtime.block_on(async {
            let connection = match self.connection.clone() {
                Some(connection) => connection,
                None => session_connection().await?,
            };
            let proxy = Proxy::new(&connection, player, OBJECT_PATH, PLAYER_IFACE)
                .await
                .with_context(|| format!("proxy MPRIS de {player}"))?;
            proxy
                .call_method(method, body)
                .await
                .with_context(|| format!("{method} en {player}"))?;
            Ok(())
        })
    }

    fn set_property<T>(&self, property: &str, value: T) -> Result<()>
    where
        T: serde::Serialize + zbus::Type,
    {
        let player = self.require_player()?.to_string();
        self.runtime.block_on(async {
            let connection = match self.connection.clone() {
                Some(connection) => connection,
                None => session_connection().await?,
            };
            let proxy = Proxy::new(&connection, player.as_str(), OBJECT_PATH, PLAYER_IFACE)
                .await
                .with_context(|| format!("proxy MPRIS de {player}"))?;
            proxy
                .set_property(property, value)
                .await
                .with_context(|| format!("{property} en {player}"))?;
            Ok(())
        })
    }
}

impl Default for SystemMediaWatcher {
    fn default() -> Self {
        Self::new()
    }
}

async fn session_connection() -> Result<zbus::Connection> {
    zbus::Connection::session()
        .await
        .context("bus de sesión no disponible")
}

/// `None` = el bus falló; `Some((conn, None))` = bus vivo sin reproductores.
async fn discover(
    existing: Option<zbus::Connection>,
    current: Option<&str>,
) -> Option<(zbus::Connection, Option<Reading>)> {
    let connection = match existing {
        Some(connection) => connection,
        None => zbus::Connection::session().await.ok()?,
    };
    let bus = Proxy::new(
        &connection,
        SESSION_BUS,
        "/org/freedesktop/DBus",
        SESSION_BUS,
    )
    .await
    .ok()?;
    let names: Vec<String> = bus
        .call_method("ListNames", &())
        .await
        .ok()?
        .body()
        .deserialize()
        .ok()?;

    let candidates = player_names(names);
    if candidates.is_empty() {
        return Some((connection, None));
    }

    let mut alive = Vec::new();
    for name in &candidates {
        if let Some(status) = read_status(&connection, name).await {
            alive.push((name.clone(), status));
        }
    }
    if alive.is_empty() {
        return Some((connection, None));
    }

    let playing = alive
        .iter()
        .filter(|(_, status)| status == "Playing")
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();

    loop {
        let names = alive
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let Some(chosen) = choose_player(&names, &playing, current) else {
            return Some((connection, None));
        };
        let status = alive
            .iter()
            .find(|(name, _)| name == &chosen)
            .map(|(_, status)| status.clone())
            .unwrap_or_default();
        if let Some(reading) = read_player(&connection, &chosen, &status).await {
            return Some((connection, Some(reading)));
        }
        // El proxy se cayó entre el sondeo y la lectura: probar el siguiente.
        alive.retain(|(name, _)| name != &chosen);
        if alive.is_empty() {
            return Some((connection, None));
        }
    }
}

async fn read_status(connection: &zbus::Connection, player: &str) -> Option<String> {
    let proxy = Proxy::new(connection, player, OBJECT_PATH, PLAYER_IFACE)
        .await
        .ok()?;
    proxy.get_property("PlaybackStatus").await.ok()
}

async fn read_player(
    connection: &zbus::Connection,
    player: &str,
    probed_status: &str,
) -> Option<Reading> {
    let proxy = Proxy::new(connection, player, OBJECT_PATH, PLAYER_IFACE)
        .await
        .ok()?;
    let status: String = proxy
        .get_property("PlaybackStatus")
        .await
        .unwrap_or_else(|_| probed_status.to_string());
    let metadata: HashMap<String, OwnedVariant> =
        proxy.get_property("Metadata").await.unwrap_or_default();
    let position = proxy.get_property::<i64>("Position").await.ok();
    let position_known = position.is_some();
    let position_us = position.unwrap_or(0);
    let volume = proxy
        .get_property::<f64>("Volume")
        .await
        .ok()
        .filter(|volume| volume.is_finite())
        .map(|volume| volume as f32);
    let shuffle = proxy.get_property::<bool>("Shuffle").await.ok();
    let repeat = proxy.get_property::<String>("LoopStatus").await.ok();
    let trackid = trackid_from_metadata(&metadata);
    let track = track_from_metadata(&metadata);
    let art_url = metadata
        .get("mpris:artUrl")
        .and_then(as_string)
        .filter(|art| !art.trim().is_empty());
    let position_ms = (position_us.max(0) as u64) / 1000;
    let track_key = track.as_ref().map(|track| track.key.clone());

    Some(Reading {
        state: SystemMediaState {
            player: player.to_string(),
            playing: status == "Playing",
            track,
            position_ms,
            position_known,
            volume,
            shuffle,
            repeat,
            art_url,
        },
        trackid,
        track_key,
    })
}

// ---------------------------------------------------------------------------
// Helpers puros
// ---------------------------------------------------------------------------

/// Nombres MPRIS utilizables, ordenados y sin spote.
fn player_names<I>(names: I) -> Vec<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut players = names
        .into_iter()
        .filter_map(|name| {
            let name = name.as_ref();
            let instance = name.strip_prefix(MPRIS_PREFIX)?;
            if instance.is_empty() || is_spote_instance(instance) {
                return None;
            }
            Some(name.to_string())
        })
        .collect::<Vec<_>>();
    players.sort();
    players.dedup();
    players
}

fn is_spote_instance(instance: &str) -> bool {
    instance == "spote" || instance.starts_with("spote.") || instance.starts_with("spote-")
}

/// Elige el reproductor activo.
///
/// - Mantiene el actual mientras siga vivo y nadie más esté sonando.
/// - Prefiere al primero que esté en `Playing`.
/// - Si nadie suena, mantiene el actual; si murió, el último de la lista.
fn choose_player(
    candidates: &[String],
    playing: &[String],
    current: Option<&str>,
) -> Option<String> {
    let is_playing = |name: &str| playing.iter().any(|playing| playing == name);
    let is_alive = |name: &str| candidates.iter().any(|candidate| candidate == name);

    if let Some(current) = current {
        if is_alive(current) && (is_playing(current) || playing.is_empty()) {
            return Some(current.to_string());
        }
    }
    if let Some(first_playing) = candidates.iter().find(|name| is_playing(name)) {
        return Some(first_playing.clone());
    }
    candidates.last().cloned()
}

fn trackid_from_metadata(metadata: &HashMap<String, OwnedVariant>) -> Option<String> {
    metadata
        .get("mpris:trackid")
        .and_then(as_string)
        .map(|trackid| trackid.trim().to_string())
        .filter(|trackid| !trackid.is_empty())
}

struct RecognizedId {
    provider: &'static str,
    id: String,
    key: String,
}

/// Solo reconocemos IDs que sepamos reconstruir sin ambigüedad (Spotify).
fn recognized_track_id(trackid: &str) -> Option<RecognizedId> {
    let lower = trackid.to_ascii_lowercase();
    let is_spotify = lower.starts_with("spotify:track:")
        || lower.contains("/com/spotify/track/")
        || lower.contains("/spotify/track/");
    if !is_spotify {
        return None;
    }
    let id = trackid.rsplit([':', '/']).next()?.trim();
    if id.len() < 16 || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(RecognizedId {
        provider: "spotify",
        id: id.to_string(),
        key: format!("spotify:track:{id}"),
    })
}

fn track_from_metadata(metadata: &HashMap<String, OwnedVariant>) -> Option<TrackRef> {
    let title = metadata
        .get("xesam:title")
        .and_then(as_string)
        .unwrap_or_default();
    if title.trim().is_empty() {
        return None;
    }
    let artist = metadata
        .get("xesam:artist")
        .map(as_strings)
        .unwrap_or_default()
        .join(", ");
    let album = metadata
        .get("xesam:album")
        .and_then(as_string)
        .filter(|album| !album.trim().is_empty());
    let duration_ms = metadata
        .get("mpris:length")
        .and_then(as_i64)
        .map(|micros| (micros.max(0) as u64) / 1000)
        .unwrap_or(0);
    let art_url = metadata
        .get("mpris:artUrl")
        .and_then(as_string)
        .filter(|art| !art.trim().is_empty());
    let trackid = trackid_from_metadata(metadata);
    let recognized = trackid.as_deref().and_then(recognized_track_id);

    let (key, source_hint) = match recognized {
        Some(recognized) => (
            recognized.key,
            Some(SourceHint {
                provider: recognized.provider.to_string(),
                id: recognized.id,
                duration_ms,
            }),
        ),
        None => (
            crate::util::fallback_key(&artist, &title, duration_ms),
            None,
        ),
    };

    Some(TrackRef {
        key,
        title,
        artist,
        album,
        duration_ms,
        art_url,
        source_hint,
    })
}

fn as_string(value: &OwnedVariant) -> Option<String> {
    let owned = value.try_clone().ok()?;
    if let Ok(text) = String::try_from(owned.clone()) {
        return Some(text);
    }
    // MPRIS declara `mpris:trackid` como object path (`o`), no como string.
    // VLC, Firefox, mpv y Elisa lo mandan así; antes se descartaba.
    OwnedObjectPath::try_from(owned)
        .ok()
        .map(|path| path.to_string())
}

fn as_i64(value: &OwnedVariant) -> Option<i64> {
    // `mpris:length` es int64 por especificación, pero algunos reproductores
    // lo publican como unsigned o int32.
    let owned = value.try_clone().ok()?;
    if let Ok(number) = i64::try_from(owned.clone()) {
        return Some(number);
    }
    if let Ok(number) = u64::try_from(owned.clone()) {
        return Some(number as i64);
    }
    i32::try_from(owned).ok().map(|number| number as i64)
}

fn as_strings(value: &OwnedVariant) -> Vec<String> {
    if let Ok(owned) = value.try_clone() {
        if let Ok(list) = <Vec<String>>::try_from(owned) {
            return list;
        }
    }
    as_string(value)
        .map(|string| vec![string])
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests (sin bus)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::{Array, Variant};

    fn owned_str(value: &str) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(value)).unwrap()
    }

    fn owned_strings(values: &[&str]) -> OwnedVariant {
        let values = values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        OwnedVariant::try_from(Variant::from(Array::from(values))).unwrap()
    }

    fn metadata(entries: Vec<(&str, OwnedVariant)>) -> HashMap<String, OwnedVariant> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    #[test]
    fn trackid_accepts_object_path_and_string() {
        use zbus::zvariant::ObjectPath;
        let path = ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap();
        let value = OwnedVariant::from(path);
        assert_eq!(
            as_string(&value).as_deref(),
            Some("/org/mpris/MediaPlayer2/TrackList/NoTrack")
        );
        assert_eq!(
            as_string(&owned_str("spotify:track:x")).as_deref(),
            Some("spotify:track:x")
        );
    }

    fn spotify_metadata() -> HashMap<String, OwnedVariant> {
        metadata(vec![
            (
                "mpris:trackid",
                owned_str("/com/spotify/track/4uLU6hMCjMI75M1A2tKUQC"),
            ),
            ("xesam:title", owned_str("Never Gonna Give You Up")),
            ("xesam:artist", owned_strings(&["Rick Astley"])),
            ("xesam:album", owned_str("Whenever You Need Somebody")),
            ("mpris:length", OwnedVariant::from(213_000_000i64)),
            ("mpris:artUrl", owned_str("https://example.test/rick.jpg")),
        ])
    }

    #[test]
    fn filters_spote_and_keeps_other_players() {
        let names = [
            "org.freedesktop.DBus",
            ":1.42",
            "org.mpris.MediaPlayer2.spote",
            "org.mpris.MediaPlayer2.spote.instance1234",
            "org.mpris.MediaPlayer2.spotify",
            "org.mpris.MediaPlayer2.firefox",
            "org.mpris.MediaPlayer2.vlc",
        ];
        assert_eq!(
            player_names(names),
            vec![
                "org.mpris.MediaPlayer2.firefox",
                "org.mpris.MediaPlayer2.spotify",
                "org.mpris.MediaPlayer2.vlc",
            ]
        );
    }

    #[test]
    fn spote_variants_are_excluded_but_spotify_is_not() {
        assert!(is_spote_instance("spote"));
        assert!(is_spote_instance("spote.instance42"));
        assert!(is_spote_instance("spote-2"));
        assert!(!is_spote_instance("spotify"));
    }

    #[test]
    fn chooses_playing_player() {
        let candidates = vec!["a".to_string(), "b".to_string()];
        let playing = vec!["b".to_string()];
        assert_eq!(
            choose_player(&candidates, &playing, Some("a")).as_deref(),
            Some("b")
        );
    }

    #[test]
    fn keeps_current_player_when_multiple_are_playing() {
        let candidates = vec!["a".to_string(), "b".to_string()];
        let playing = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            choose_player(&candidates, &playing, Some("b")).as_deref(),
            Some("b")
        );
    }

    #[test]
    fn keeps_paused_current_when_nothing_plays() {
        let candidates = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            choose_player(&candidates, &[], Some("a")).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn falls_back_to_last_alive_when_current_died() {
        let candidates = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            choose_player(&candidates, &[], Some("gone")).as_deref(),
            Some("b")
        );
        assert_eq!(choose_player(&[], &[], Some("gone")), None);
    }

    #[test]
    fn converts_spotify_metadata_to_trackref() {
        let track = track_from_metadata(&spotify_metadata()).expect("pista");
        assert_eq!(track.key, "spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(track.title, "Never Gonna Give You Up");
        assert_eq!(track.artist, "Rick Astley");
        assert_eq!(track.album.as_deref(), Some("Whenever You Need Somebody"));
        assert_eq!(track.duration_ms, 213_000);
        assert_eq!(
            track.art_url.as_deref(),
            Some("https://example.test/rick.jpg")
        );
        let hint = track.source_hint.expect("source hint");
        assert_eq!(hint.provider, "spotify");
        assert_eq!(hint.id, "4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(hint.duration_ms, 213_000);
    }

    #[test]
    fn falls_back_to_stable_key_without_recognized_id() {
        let metadata = metadata(vec![
            (
                "mpris:trackid",
                owned_str("/org/mpris/MediaPlayer2/TrackList/42"),
            ),
            ("xesam:title", owned_str("Windowlicker")),
            ("xesam:artist", owned_strings(&["Aphex Twin"])),
            ("mpris:length", OwnedVariant::from(366_000_000i64)),
        ]);
        let first = track_from_metadata(&metadata).expect("pista");
        let second = track_from_metadata(&metadata).expect("pista");
        assert_eq!(first.key, second.key);
        assert_eq!(
            first.key,
            crate::util::fallback_key("Aphex Twin", "Windowlicker", 366_000)
        );
        assert!(first.source_hint.is_none());
    }

    #[test]
    fn joins_artist_arrays_and_accepts_scalar_artist() {
        let mut metadata = spotify_metadata();
        metadata.insert(
            "xesam:artist".to_string(),
            owned_strings(&["Daft Punk", "Pharrell Williams"]),
        );
        let track = track_from_metadata(&metadata).unwrap();
        assert_eq!(track.artist, "Daft Punk, Pharrell Williams");

        metadata.insert("xesam:artist".to_string(), owned_str("Solo"));
        let track = track_from_metadata(&metadata).unwrap();
        assert_eq!(track.artist, "Solo");
    }

    #[test]
    fn ignores_metadata_without_title() {
        let metadata = metadata(vec![("xesam:artist", owned_strings(&["Nobody"]))]);
        assert!(track_from_metadata(&metadata).is_none());
    }

    #[test]
    fn accepts_length_as_unsigned_or_int32() {
        let mut metadata = spotify_metadata();
        metadata.insert(
            "mpris:length".to_string(),
            OwnedVariant::from(213_000_000u64),
        );
        assert_eq!(track_from_metadata(&metadata).unwrap().duration_ms, 213_000);

        metadata.insert(
            "mpris:length".to_string(),
            OwnedVariant::from(213_000_000i32),
        );
        assert_eq!(track_from_metadata(&metadata).unwrap().duration_ms, 213_000);
    }

    #[test]
    fn recognizes_spotify_trackid_variants() {
        let colon = recognized_track_id("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap();
        assert_eq!(colon.key, "spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        let path = recognized_track_id("/com/spotify/track/4uLU6hMCjMI75M1A2tKUQC").unwrap();
        assert_eq!(path.id, "4uLU6hMCjMI75M1A2tKUQC");
        assert!(recognized_track_id("spotify:ad:abc").is_none());
        assert!(recognized_track_id("/org/mpris/MediaPlayer2/TrackList/1").is_none());
    }

    #[test]
    fn session_bus_check_does_not_panic() {
        let _ = SystemMediaWatcher::session_bus_available();
    }
}
