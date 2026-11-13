//! Trabajos en segundo plano que nunca deben bloquear el render: portada,
//! letra y sondeo de MPRIS.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use spote_core::artwork::{decode_image, render_cover, ArtMode, CoverArt, Palette};
use spote_core::config;
use spote_core::lyrics::{LyricsDoc, LyricsService};
use spote_core::spotify::{SpotifyState, SpotifyWatcher};
use spote_core::spotify_api;
use spote_core::downloader::SearchItem;
use spote_core::util;
use spote_protocol::TrackRef;

// ---------------------------------------------------------------------------
// Búsqueda (Spotify con respaldo Deezer)
// ---------------------------------------------------------------------------

pub struct SearchWorker {
    requests: Sender<(u64, String)>,
    responses: Receiver<(u64, Result<Vec<SearchItem>, String>)>,
}

impl SearchWorker {
    pub fn start() -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::channel::<(u64, String)>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-search".into())
            .spawn(move || {
                while let Ok((id, query)) = request_rx.recv() {
                    let result = spotify_api::search(&query, 12)
                        .map(|tracks| {
                            tracks
                                .into_iter()
                                .map(|track| SearchItem {
                                    provider_id: track.id,
                                    provider: track.provider,
                                    title: track.title,
                                    artist: track.artists,
                                    album: Some(track.album),
                                    duration_ms: track.duration_ms,
                                    channel: String::new(),
                                    url: String::new(),
                                    art_url: track.art_url,
                                    isrc: track.isrc,
                                })
                                .collect()
                        })
                        .map_err(|err| err.to_string());
                    if response_tx.send((id, result)).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Self {
            requests: request_tx,
            responses: response_rx,
        }
    }

    pub fn search(&self, id: u64, query: String) {
        let _ = self.requests.send((id, query));
    }

    pub fn try_recv(&self) -> Option<(u64, Result<Vec<SearchItem>, String>)> {
        self.responses.try_recv().ok()
    }
}

// ---------------------------------------------------------------------------
// Portada
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ArtRequest {
    pub key: String,
    pub url: Option<String>,
    pub local: Option<PathBuf>,
    pub w: u16,
    pub h: u16,
    pub mode: ArtMode,
}

#[derive(Debug)]
pub struct ArtResponse {
    pub key: String,
    pub palette: Option<Palette>,
    pub art: Option<CoverArt>,
    pub art_path: Option<PathBuf>,
    pub w: u16,
    pub h: u16,
    pub mode: ArtMode,
    pub error: Option<String>,
}

pub struct ArtWorker {
    big: Sender<ArtRequest>,
    thumbs: Sender<ArtRequest>,
    responses: Receiver<ArtResponse>,
}

impl ArtWorker {
    pub fn start() -> Self {
        let (big_tx, big_rx) = std::sync::mpsc::channel::<ArtRequest>();
        let (thumb_tx, thumb_rx) = std::sync::mpsc::channel::<ArtRequest>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-art".into())
            .spawn(move || {
                let client = reqwest::blocking::Client::builder()
                    .user_agent(concat!("spote/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .ok();
                // Dos colas: la portada grande manda. Antes, una lista en
                // tarjetas encolaba 8 miniaturas y la portada de la canción
                // nueva llegaba segundos tarde (o no llegaba).
                loop {
                    let request = match big_rx.recv_timeout(std::time::Duration::from_millis(20)) {
                        Ok(request) => request,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            match thumb_rx.try_recv() {
                                Ok(request) => request,
                                Err(_) => continue,
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    };
                    let response = process(&request, client.as_ref());
                    if response_tx.send(response).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Self {
            big: big_tx,
            thumbs: thumb_tx,
            responses: response_rx,
        }
    }

    pub fn request(&self, request: ArtRequest) {
        if request.w <= 8 && request.h <= 4 {
            let _ = self.thumbs.send(request);
        } else {
            let _ = self.big.send(request);
        }
    }

    pub fn try_recv(&self) -> Option<ArtResponse> {
        match self.responses.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

fn process(request: &ArtRequest, client: Option<&reqwest::blocking::Client>) -> ArtResponse {
    let mut response = ArtResponse {
        key: request.key.clone(),
        palette: None,
        art: None,
        art_path: None,
        w: request.w,
        h: request.h,
        mode: request.mode,
        error: None,
    };

    let mut image = request
        .local
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| decode_image(&bytes).ok());

    if image.is_none() {
        if let Some(url) = request.url.as_deref() {
            let path = cache_art_path(&request.key, url);
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    image = decode_image(&bytes).ok();
                    response.art_path = Some(path.clone());
                }
            }
            if image.is_none() {
                if let Some(client) = client {
                    match download(client, url) {
                        Ok(bytes) => {
                            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/tmp")));
                            let _ = std::fs::write(&path, &bytes);
                            image = decode_image(&bytes).ok();
                            response.art_path = Some(path.clone());
                        }
                        Err(err) => response.error = Some(err),
                    }
                }
            }
            if response.art_path.is_none() && path.exists() {
                response.art_path = Some(path);
            }
        }
    }

    let Some(image) = image else {
        if response.error.is_none() {
            response.error = Some("sin portada".into());
        }
        return response;
    };

    response.palette = Some(Palette::from_image(&image));
    if request.mode != ArtMode::Off && request.w > 2 && request.h > 1 {
        response.art = Some(render_cover(
            &image,
            request.w,
            request.h,
            request.mode,
            response.palette.as_ref().unwrap(),
            0.0,
        ));
    }
    response
}

fn download(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|err| err.to_string());
    }
    let response = client.get(url).send().map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!("http {}", response.status()));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|err| err.to_string())
}

/// Ruta de caché estable por canción. `jpg` siempre: la imagen se reencoda al
/// pedirla y para la paleta da igual el formato original.
pub fn cache_art_path(key: &str, url: &str) -> PathBuf {
    let name = util::short_hash(&format!("{key}|{url}"));
    config::cache_dir().join("art").join(format!("{name}.img"))
}

// ---------------------------------------------------------------------------
// Letras
// ---------------------------------------------------------------------------

pub enum LyricsMsg {
    Loaded {
        key: String,
        doc: Option<LyricsDoc>,
        lrc: Option<String>,
        error: Option<String>,
    },
}

pub struct LyricsWorker {
    requests: Sender<TrackRef>,
    responses: Receiver<LyricsMsg>,
}

impl LyricsWorker {
    pub fn start(cfg: spote_core::config::LyricsCfg) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<TrackRef>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-lyrics".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let service = LyricsService::from_config(&cfg);
                while let Ok(track) = rx.recv() {
                    let result = runtime.block_on(service.fetch(&track));
                    let message = match result {
                        Ok(Some(doc)) => LyricsMsg::Loaded {
                            key: track.key.clone(),
                            lrc: Some(doc_to_lrc(&doc)),
                            doc: Some(doc),
                            error: None,
                        },
                        Ok(None) => LyricsMsg::Loaded {
                            key: track.key.clone(),
                            doc: None,
                            lrc: None,
                            error: None,
                        },
                        Err(err) => LyricsMsg::Loaded {
                            key: track.key.clone(),
                            doc: None,
                            lrc: None,
                            error: Some(err.to_string()),
                        },
                    };
                    if response_tx.send(message).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Self {
            requests: tx,
            responses: response_rx,
        }
    }

    pub fn request(&self, track: TrackRef) {
        let _ = self.requests.send(track);
    }

    pub fn try_recv(&self) -> Option<LyricsMsg> {
        self.responses.try_recv().ok()
    }
}

/// Serializa de vuelta a LRC para cachear y para el `.lrc` junto al audio.
pub fn doc_to_lrc(doc: &LyricsDoc) -> String {
    let mut out = String::new();
    for line in &doc.lines {
        let minutes = line.start_ms / 60_000;
        let seconds = (line.start_ms % 60_000) as f32 / 1000.0;
        if line.words.is_empty() {
            out.push_str(&format!("[{minutes:02}:{seconds:05.2}] {}\n", line.text));
        } else {
            let mut body = String::new();
            for word in &line.words {
                let wm = word.start_ms / 60_000;
                let ws = (word.start_ms % 60_000) as f32 / 1000.0;
                body.push_str(&format!("<{wm:02}:{ws:05.2}>{}\n", word.text));
            }
            out.push_str(&format!("[{minutes:02}:{seconds:05.2}] {}\n", body.replace('\n', " ")));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Spotify (MPRIS)
// ---------------------------------------------------------------------------

pub struct SpotifyWorker {
    states: Receiver<SpotifyState>,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SpotifyWorker {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = std::sync::Arc::clone(&active);
        std::thread::Builder::new()
            .name("spote-spotify".into())
            .spawn(move || {
                let mut watcher = SpotifyWatcher::new();
                while flag.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(state) = watcher.poll() {
                        // Se manda SIEMPRE, también cuando solo cambia la
                        // posición: de esto depende la letra sincronizada.
                        if tx.send(state).is_err() {
                            return;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
            })
            .ok();
        Self {
            states: rx,
            active,
        }
    }

    pub fn try_recv(&self) -> Option<SpotifyState> {
        self.states.try_recv().ok()
    }
}

impl Drop for SpotifyWorker {
    fn drop(&mut self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_worker_renders_from_a_local_file() {
        let path = std::path::Path::new("/tmp/spote-demo/Tame Impala/The Slow Rush/cover.jpg");
        if !path.exists() {
            return;
        }
        let worker = ArtWorker::start();
        worker.request(ArtRequest {
            key: "test".into(),
            url: None,
            local: Some(path.to_path_buf()),
            w: 60,
            h: 20,
            mode: ArtMode::Dither,
        });
        let start = std::time::Instant::now();
        let mut response = None;
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if let Some(value) = worker.try_recv() {
                response = Some(value);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let response = response.expect("el worker no respondió");
        assert!(response.error.is_none(), "error: {:?}", response.error);
        assert!(response.palette.is_some());
        let art = response.art.expect("sin arte");
        assert_eq!(art.w, 60);
        assert_eq!(art.h, 20);
        let drawn = art.cells.iter().filter(|cell| cell.ch != ' ').count();
        assert!(drawn > 100, "drawn={drawn}");
    }
}
