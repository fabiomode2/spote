//! Créditos de artista para la UI.

use super::*;

pub fn join_artist_credit(artists: &[String], fallback: &str) -> String {
    if artists.is_empty() {
        fallback.to_string()
    } else {
        artists.join(", ")
    }
}

pub fn row_artist_credit(row: &TrackRow) -> String {
    join_artist_credit(&row.artists, &row.track.artist)
}

pub fn row_primary_artist(row: &TrackRow) -> String {
    row.artists.first().cloned().unwrap_or_else(|| {
        spote_core::library::split_artist_credit(&row.track.artist)
            .into_iter()
            .next()
            .unwrap_or_default()
    })
}

impl App {
    /// Lanza una pasada de backfill al arrancar. Es de fondo: el worker va a
    /// una petición por segundo y la UI sigue respondiendo.
    pub fn start_artist_backfill(&mut self) {
        let Ok(rows) = self.lib.tracks_without_artists(200) else {
            return;
        };
        for row in rows {
            let fallback = if row.artists.is_empty() {
                spote_core::library::split_artist_credit(&row.track.artist)
            } else {
                row.artists.clone()
            };
            if row.track.artist.trim().is_empty() && row.track.title.trim().is_empty() {
                let _ = self.lib.update_artists(&row.track.key, &fallback, 1);
                continue;
            }
            self.artist_backfill.request(ArtistJob {
                key: row.track.key.clone(),
                artist: row.track.artist.clone(),
                title: row.track.title.clone(),
                fallback,
            });
        }
    }

    pub fn poll_artist_backfill(&mut self) {
        while let Some(event) = self.artist_backfill.try_recv() {
            let artists = if event.artists.is_empty() {
                event.fallback
            } else {
                event.artists
            };
            if !artists.is_empty() || event.error.is_some() {
                let _ = self.lib.update_artists(&event.key, &artists, event.version);
                if let Some(primary) = artists.first() {
                    // El álbum se agrupa por el artista principal, no por el
                    // crédito completo de cada pista con features distintos.
                    let _ = self.lib.set_album_artist(&event.key, primary);
                }
                if self.now.track.as_ref().map(|track| track.key.as_str())
                    == Some(event.key.as_str())
                {
                    self.refresh_now_artists();
                }
                self.refresh_view();
            }
        }
    }

    pub fn refresh_now_artists(&mut self) {
        let Some(track) = self.now.track.clone() else {
            self.now_artists.clear();
            return;
        };
        self.now_artists = self
            .lib
            .track_by_key(&track.key)
            .ok()
            .flatten()
            .map(|row| row.artists)
            .filter(|artists| !artists.is_empty())
            .unwrap_or_else(|| spote_core::library::split_artist_credit(&track.artist));
    }

    pub fn now_artist_credit(&self) -> String {
        let fallback = self
            .now
            .track
            .as_ref()
            .map(|track| track.artist.as_str())
            .unwrap_or("");
        join_artist_credit(&self.now_artists, fallback)
    }

    pub fn now_primary_artist(&self) -> String {
        self.now_artists.first().cloned().unwrap_or_else(|| {
            self.now
                .track
                .as_ref()
                .map(|track| track.artist.clone())
                .unwrap_or_default()
        })
    }
}
