//! Modo acompañante del cliente oficial de Spotify, vía MPRIS (D-Bus).
//!
//! No descarga nada: lee qué suena para pintar portada, letra y visualizador.
//! Es la demo instantánea y el banco de pruebas del render.
//!
//! zbus 6 es async: el watcher mantiene un runtime current-thread propio y
//! expone `poll()` síncrono, que es lo que consume el hilo de la UI.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use spote_protocol::TrackRef;
use zbus::OwnedVariant;
use zbus::Proxy;

const PLAYER_NAME: &str = "org.mpris.MediaPlayer2.spotify";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone, Default)]
pub struct SpotifyState {
    pub playing: bool,
    pub track: Option<TrackRef>,
    pub position_ms: u64,
    pub art_url: Option<String>,
}

/// Sondeo de MPRIS. `poll()` bloquea unos milisegundos y se llama desde un
/// hilo dedicado, nunca desde el render.
pub struct SpotifyWatcher {
    runtime: tokio::runtime::Runtime,
    connection: Option<zbus::Connection>,
    last_attempt: Instant,
}

impl SpotifyWatcher {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime de zbus");
        Self {
            runtime,
            connection: None,
            last_attempt: Instant::now() - Duration::from_secs(10),
        }
    }

    /// `true` si el bus de sesión responde. No garantiza que Spotify esté vivo.
    pub fn session_bus_available() -> bool {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        runtime
            .block_on(async { zbus::Connection::session().await.is_ok() })
    }

    async fn read(connection: zbus::Connection) -> Option<(zbus::Connection, SpotifyState)> {
        let proxy = Proxy::new(&connection, PLAYER_NAME, PLAYER_PATH, PLAYER_IFACE)
            .await
            .ok()?;
        let status: String = proxy.get_property("PlaybackStatus").await.ok()?;
        let metadata: HashMap<String, OwnedVariant> = proxy.get_property("Metadata").await.ok()?;
        let position: i64 = proxy.get_property("Position").await.unwrap_or(0);

        let title = metadata
            .get("xesam:title")
            .and_then(as_string)
            .unwrap_or_default();
        if title.trim().is_empty() {
            return Some((
                connection,
                SpotifyState {
                    playing: status == "Playing",
                    ..Default::default()
                },
            ));
        }
        let artists = metadata
            .get("xesam:artist")
            .map(as_strings)
            .unwrap_or_default();
        let artist = artists.join(", ");
        let album = metadata.get("xesam:album").and_then(as_string);
        let duration_us = metadata.get("mpris:length").and_then(as_i64).unwrap_or(0);
        let art_url = metadata.get("mpris:artUrl").and_then(as_string);
        let spotify_id = metadata
            .get("mpris:trackid")
            .and_then(as_string)
            .and_then(|track_id| spotify_id_from_trackid(&track_id));

        let key = spotify_id
            .clone()
            .map(|id| format!("spotify:track:{id}"))
            .unwrap_or_else(|| {
                crate::util::fallback_key(&artist, &title, (duration_us / 1000).max(0) as u64)
            });

        let track = TrackRef {
            key,
            title,
            artist,
            album,
            duration_ms: (duration_us / 1000).max(0) as u64,
            art_url: art_url.clone(),
            source_hint: spotify_id.map(|id| spote_protocol::SourceHint {
                provider: "spotify".to_string(),
                id,
                duration_ms: (duration_us / 1000).max(0) as u64,
            }),
        };

        Some((
            connection,
            SpotifyState {
                playing: status == "Playing",
                track: Some(track),
                position_ms: (position / 1000).max(0) as u64,
                art_url,
            },
        ))
    }

    /// Lee el estado actual. `None` si Spotify no está corriendo.
    pub fn poll(&mut self) -> Option<SpotifyState> {
        let existing = self.connection.clone();
        let attempt = if existing.is_none() && self.last_attempt.elapsed() < Duration::from_secs(3)
        {
            return None;
        } else {
            self.last_attempt = Instant::now();
            existing
        };
        let result = self.runtime.block_on(async move {
            let connection = match attempt {
                Some(connection) => connection,
                None => zbus::Connection::session().await.ok()?,
            };
            Self::read(connection).await
        });
        match result {
            Some((connection, state)) => {
                if self.connection.is_none() {
                    self.connection = Some(connection);
                }
                Some(state)
            }
            None => {
                self.connection = None;
                None
            }
        }
    }
}

impl Default for SpotifyWatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn spotify_id_from_trackid(track_id: &str) -> Option<String> {
    // Formatos vistos: "spotify:track:4uLU6hMCjMI75M1A2tKUQC" y
    // "/com/spotify/track/4uLU6hMCjMI75M1A2tKUQC".
    let id = track_id.rsplit([':', '/']).next().unwrap_or(track_id).trim();
    if id.len() >= 16 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(id.to_string())
    } else {
        None
    }
}

fn as_string(value: &OwnedVariant) -> Option<String> {
    let owned = value.try_clone().ok()?;
    String::try_from(owned).ok()
}

fn as_i64(value: &OwnedVariant) -> Option<i64> {
    // MPRIS declara `mpris:length` como int64, pero según el reproductor puede
    // llegar como unsigned o int32: se prueban los tres.
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
    as_string(value).map(|s| vec![s]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_spotify_ids() {
        assert_eq!(
            spotify_id_from_trackid("spotify:track:4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(
            spotify_id_from_trackid("/com/spotify/track/4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(spotify_id_from_trackid("spotify:ad:abc"), None);
    }

    #[test]
    fn session_bus_check_does_not_panic() {
        let _ = SpotifyWatcher::session_bus_available();
    }
}
