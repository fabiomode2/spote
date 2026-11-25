//! Backfill de artistas completos para pistas antiguas.
//!
//! Consulta Spotify por título+artista, elige la coincidencia con mejor
//! puntuación y devuelve la lista estructurada. La app escribe el resultado en
//! SQLite con `Library::update_artists`; este worker no toca la base de datos.

use std::sync::mpsc::{Receiver, Sender};

use spote_core::spotify_api;

#[derive(Debug, Clone)]
pub struct ArtistJob {
    pub key: String,
    pub artist: String,
    pub title: String,
    pub fallback: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ArtistEvent {
    pub key: String,
    pub artists: Vec<String>,
    pub version: i64,
    pub error: Option<String>,
    pub fallback: Vec<String>,
}

pub struct ArtistBackfill {
    jobs: Sender<ArtistJob>,
    events: Receiver<ArtistEvent>,
}

impl ArtistBackfill {
    pub fn start() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<ArtistJob>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<ArtistEvent>();
        std::thread::Builder::new()
            .name("spote-artists".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    // Una petición por segundo como máximo; el backfill es de
                    // fondo y no debe competir con la reproducción.
                    std::thread::sleep(std::time::Duration::from_millis(900));
                    let event = match lookup(&job) {
                        Ok(artists) if !artists.is_empty() => ArtistEvent {
                            key: job.key,
                            artists,
                            version: 2,
                            error: None,
                            fallback: job.fallback,
                        },
                        Ok(_) => ArtistEvent {
                            key: job.key,
                            artists: job.fallback,
                            version: 1,
                            error: Some("sin coincidencia".into()),
                            fallback: Vec::new(),
                        },
                        Err(err) => ArtistEvent {
                            key: job.key,
                            artists: job.fallback,
                            version: 1,
                            error: Some(err),
                            fallback: Vec::new(),
                        },
                    };
                    if event_tx.send(event).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Self {
            jobs: job_tx,
            events: event_rx,
        }
    }

    pub fn request(&self, job: ArtistJob) {
        let _ = self.jobs.send(job);
    }

    pub fn try_recv(&self) -> Option<ArtistEvent> {
        self.events.try_recv().ok()
    }
}

pub fn find_artists(artist: &str, title: &str) -> Result<Vec<String>, String> {
    lookup(&ArtistJob {
        key: String::new(),
        artist: artist.to_string(),
        title: title.to_string(),
        fallback: Vec::new(),
    })
}

fn lookup(job: &ArtistJob) -> Result<Vec<String>, String> {
    let query = format!("{} {}", job.artist, job.title);
    let results = spotify_api::search(&query, 5).map_err(|err| err.to_string())?;
    let wanted_title = spote_core::util::normalize(&job.title);
    let wanted_artist = spote_core::util::normalize(&job.artist);
    let best = results
        .into_iter()
        .filter(|track| !track.artist_list.is_empty())
        .max_by_key(|track| {
            let title = spote_core::util::normalize(&track.title);
            let artist = spote_core::util::normalize(&track.artists);
            let title_score = if title == wanted_title {
                4
            } else if title.contains(&wanted_title) || wanted_title.contains(&title) {
                2
            } else {
                0
            };
            let artist_score = if artist.contains(&wanted_artist) || wanted_artist.contains(&artist)
            {
                3
            } else {
                0
            };
            title_score + artist_score
        });
    match best {
        Some(track)
            if {
                let title = spote_core::util::normalize(&track.title);
                title == wanted_title
                    || title.contains(&wanted_title)
                    || wanted_title.contains(&title)
            } =>
        {
            Ok(track.artist_list)
        }
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_metadata_is_representable() {
        let job = ArtistJob {
            key: "x".into(),
            artist: String::new(),
            title: String::new(),
            fallback: Vec::new(),
        };
        assert!(job.artist.is_empty());
        assert!(job.fallback.is_empty());
    }
}
