//! Trabajos en segundo plano que nunca deben bloquear el render: portada,
//! letra y sondeo de MPRIS.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use spote_core::artwork::{decode_image, render_cover, ArtMode, CoverArt, Palette};
use spote_core::config;
use spote_core::lyrics::{LyricsCandidate, LyricsDoc, LyricsService};
use spote_core::spotify_api;
use spote_core::util;
use spote_protocol::TrackRef;

// ---------------------------------------------------------------------------
// Búsqueda (Spotify con respaldo Deezer)
// ---------------------------------------------------------------------------

pub struct SearchWorker {
    requests: Sender<(u64, String)>,
    responses: Receiver<(u64, Result<spote_core::spotify_api::SearchAll, String>)>,
}

impl SearchWorker {
    pub fn start() -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::channel::<(u64, String)>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-search".into())
            .spawn(move || {
                while let Ok((id, query)) = request_rx.recv() {
                    let result = spotify_api::search_all(&query, 10).map_err(|err| err.to_string());
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

    pub fn try_recv(&self) -> Option<(u64, Result<spote_core::spotify_api::SearchAll, String>)> {
        self.responses.try_recv().ok()
    }
}

// ---------------------------------------------------------------------------
// Portada
// ---------------------------------------------------------------------------

/// Para qué se pide el arte. La clasificación va explícita y no por tamaño:
/// una miniatura de álbum puede ser 18x9 y no debe pisar la portada grande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtKind {
    Main,
    Thumb,
    Ambient,
    Artist,
}

#[derive(Debug, Clone)]
pub struct ArtRequest {
    pub key: String,
    pub url: Option<String>,
    pub local: Option<PathBuf>,
    /// Prefiere la portada remota (Spotify) sobre la miniatura local de
    /// yt-dlp. Si la remota falla, cae al fichero local.
    pub prefer_url: bool,
    pub w: u16,
    pub h: u16,
    pub mode: ArtMode,
    pub kind: ArtKind,
}

#[derive(Debug)]
pub struct ArtResponse {
    pub key: String,
    pub palette: Option<Palette>,
    pub art: Option<CoverArt>,
    pub art_path: Option<PathBuf>,
    /// PNG original para el protocolo de imágenes de Kitty (portada full-res).
    pub png: Option<Vec<u8>>,
    pub png_size: Option<(u32, u32)>,
    pub w: u16,
    pub h: u16,
    pub mode: ArtMode,
    pub kind: ArtKind,
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
        if request.kind == ArtKind::Thumb {
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
        png: None,
        png_size: None,
        w: request.w,
        h: request.h,
        mode: request.mode,
        kind: request.kind,
        error: None,
    };

    let local_image = request
        .local
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| decode_image(&bytes).ok());

    let mut image = if request.prefer_url {
        None
    } else {
        local_image.clone()
    };

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
                            let _ =
                                std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/tmp")));
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
    // Red caída o URL rota: la miniatura local sigue sirviendo.
    if image.is_none() {
        image = local_image;
        if image.is_some() {
            response.art_path = request.local.clone();
            response.error = None;
        }
    }

    let Some(image) = image else {
        if response.error.is_none() {
            response.error = Some("sin portada".into());
        }
        return response;
    };

    // La paleta del tema solo la necesita la portada grande; calcularla por
    // cada miniatura era trabajo tirado en el hilo de arte.
    if request.kind != ArtKind::Thumb {
        response.palette = Some(Palette::from_image(&image));
    }
    let palette = match response.palette {
        Some(palette) => palette,
        // Las miniaturas se piden en `Plain`: no necesitan la paleta.
        None if request.mode == ArtMode::Plain => Palette::default(),
        None => Palette::from_image(&image),
    };
    if request.mode != ArtMode::Off && request.w > 2 && request.h > 1 {
        response.art = Some(render_cover(
            &image,
            request.w,
            request.h,
            request.mode,
            &palette,
            0.0,
        ));
    }
    // La imagen original solo interesa para la portada grande en Kitty. Se
    // convierte a PNG una vez por cambio, no por frame.
    // Miniaturas para Kitty: se manda una versión normalizada (la terminal la
    // escala al hueco) y así una tarjeta de 6x3 se ve como la portada real,
    // no como un bloque de 6x3.
    if request.kind == ArtKind::Thumb && request.mode == ArtMode::Plain {
        let thumb = image.resize_to_fill(192, 192, image::imageops::FilterType::Lanczos3);
        let mut png = Vec::new();
        if thumb
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
        {
            response.png_size = Some((192, 192));
            response.png = Some(png);
        }
    }
    if request.kind == ArtKind::Main
        && request.mode == ArtMode::Plain
        && request.w >= 24
        && request.h >= 12
    {
        let mut png = Vec::new();
        if image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
        {
            response.png_size = Some((image.width(), image.height()));
            response.png = Some(png);
        }
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

#[derive(Debug, Clone)]
pub struct LyricsRequest {
    pub track: TrackRef,
    pub audio: Option<PathBuf>,
    /// La letra cacheada venía de un alineador viejo: re-alinear aunque el
    /// proveedor ya devuelva un LRC sincronizado.
    pub force_align: bool,
}

pub struct LyricsWorker {
    requests: Sender<(u64, LyricsRequest)>,
    /// Generación global: cada petición la incrementa. Un trabajo en curso
    /// compara su generación con esta para cancelar el ASR si el usuario ya
    /// cambió de pista.
    generation: Arc<AtomicU64>,
    responses: Receiver<LyricsMsg>,
}

impl LyricsWorker {
    pub fn start(cfg: spote_core::config::LyricsCfg) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(u64, LyricsRequest)>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
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
                // Una pista solo intenta el ASR una vez por sesión: si falló o
                // tardó, no se repite cada vez que suena. La letra cacheada se
                // sigue mostrando.
                let mut asr_attempted: HashSet<String> = HashSet::new();
                while let Ok((mut my_generation, mut request)) = rx.recv() {
                    // Si hay varias peticiones en cola (el usuario saltó de
                    // pista), solo interesa la última. Se conserva su `id` como
                    // generación propia: cargarla después del drenado podía
                    // hacer que el trabajo viejo no se cancelara.
                    while let Ok((newer_id, newer)) = rx.try_recv() {
                        my_generation = newer_id;
                        request = newer;
                    }
                    let key = request.track.key.clone();
                    let audio = match request.audio.take() {
                        Some(audio) if asr_attempted.insert(key.clone()) => Some(audio),
                        _ => None,
                    };
                    let cancel = Arc::new(AtomicBool::new(false));
                    let done = Arc::new(AtomicBool::new(false));
                    // Vigilante: al llegar una petición nueva, activa `cancel`.
                    // El ASR lo consulta entre lecturas y mata el proceso.
                    let watcher = {
                        let generation = Arc::clone(&worker_generation);
                        let watcher_cancel = Arc::clone(&cancel);
                        let watcher_done = Arc::clone(&done);
                        std::thread::Builder::new()
                            .name("spote-lyrics-cancel".into())
                            .spawn(move || {
                                while !watcher_done.load(Ordering::Relaxed) {
                                    if generation.load(Ordering::Relaxed) != my_generation {
                                        watcher_cancel.store(true, Ordering::Relaxed);
                                        return;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(80));
                                }
                            })
                            .ok()
                    };
                    let result = runtime.block_on(service.fetch_with_audio_cancellable(
                        &request.track,
                        audio.as_deref(),
                        request.force_align,
                        Some(cancel.as_ref()),
                    ));
                    done.store(true, Ordering::Relaxed);
                    // Un intento cancelado no cuenta: la pista volverá a tener
                    // su oportunidad cuando se reproduzca de nuevo.
                    if cancel.load(Ordering::Relaxed) {
                        asr_attempted.remove(&key);
                    }
                    drop(watcher);
                    let message = match result {
                        Ok(Some(doc)) => LyricsMsg::Loaded {
                            key: request.track.key.clone(),
                            lrc: Some(doc_to_lrc(&doc)),
                            doc: Some(doc),
                            error: None,
                        },
                        Ok(None) => LyricsMsg::Loaded {
                            key: request.track.key.clone(),
                            doc: None,
                            lrc: None,
                            error: None,
                        },
                        Err(err) => LyricsMsg::Loaded {
                            key: request.track.key.clone(),
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
            generation,
            responses: response_rx,
        }
    }

    pub fn request(&self, track: TrackRef, audio: Option<PathBuf>, force_align: bool) {
        // La generación se incrementa en el hilo llamante para que el trabajo
        // en curso vea la cancelación aunque este hilo aún no haya recibido la
        // petición.
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.requests.send((
            id,
            LyricsRequest {
                track,
                audio,
                force_align,
            },
        ));
    }

    pub fn try_recv(&self) -> Option<LyricsMsg> {
        self.responses.try_recv().ok()
    }
}

/// Búsqueda manual de letras: devuelve todos los candidatos de LRCLIB para que
/// el usuario pueda previsualizarlos y aceptar el correcto.
pub struct LyricsSearchWorker {
    requests: Sender<(u64, String)>,
    responses: Receiver<LyricsSearchMsg>,
    generation: Arc<AtomicU64>,
}

pub enum LyricsSearchMsg {
    Results {
        query_id: u64,
        query: String,
        result: Result<Vec<LyricsCandidate>, String>,
    },
}

impl LyricsSearchWorker {
    pub fn start(cfg: spote_core::config::LyricsCfg) -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::channel::<(u64, String)>();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        std::thread::Builder::new()
            .name("spote-lyrics-search".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let service = LyricsService::from_config(&cfg);
                while let Ok((mut query_id, mut query)) = request_rx.recv() {
                    while let Ok((newer, newer_query)) = request_rx.try_recv() {
                        query_id = newer;
                        query = newer_query;
                    }
                    if worker_generation.load(Ordering::Relaxed) != query_id {
                        continue;
                    }
                    let result = runtime
                        .block_on(service.search_candidates(&query))
                        .map_err(|err| err.to_string());
                    if response_tx
                        .send(LyricsSearchMsg::Results {
                            query_id,
                            query,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .ok();
        Self {
            requests: request_tx,
            responses: response_rx,
            generation,
        }
    }

    pub fn request(&self, query: String) -> u64 {
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.requests.send((id, query));
        id
    }

    pub fn try_recv(&self) -> Option<LyricsSearchMsg> {
        self.responses.try_recv().ok()
    }
}

/// Serializa de vuelta a LRC para cachear y para el `.lrc` junto al audio.
///
/// El `[offset:]` del proveedor ya viene horneado en los tiempos al parsear, y
/// el ajuste manual vive en la tabla de la librería; escribir un `[offset:]`
/// aquí haría que al recargar se aplicara dos veces.
pub fn doc_to_lrc(doc: &LyricsDoc) -> String {
    let mut out = String::new();
    for line in &doc.lines {
        let minutes = line.start_ms / 60_000;
        let seconds = (line.start_ms % 60_000) as f32 / 1000.0;
        // Solo se escriben tags de palabra si son reales (LRC mejorado o ASR).
        // Serializar las palabras sintetizadas las convertía en "exactas" al
        // recargar y la UI dejaba de refinarlas con el mapa de audio.
        if line.words.is_empty() || !line.words_exact {
            out.push_str(&format!("[{minutes:02}:{seconds:05.2}] {}\n", line.text));
        } else {
            let mut body = String::new();
            for word in &line.words {
                let wm = word.start_ms / 60_000;
                let ws = (word.start_ms % 60_000) as f32 / 1000.0;
                body.push_str(&format!("<{wm:02}:{ws:05.2}>{}\n", word.text));
            }
            out.push_str(&format!(
                "[{minutes:02}:{seconds:05.2}] {}\n",
                body.replace('\n', " ")
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Spotify (MPRIS)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_roundtrip_keeps_times_and_drops_offsets() {
        // El proveedor hornea su offset en las líneas; el ajuste de usuario se
        // guarda aparte. Serializar un `[offset:]` lo aplicaría otra vez al
        // recargar la caché.
        let doc = spote_core::lyrics::parse_lrc("[offset:+500]\n[00:10.00]hola\n[00:12.50]adios");
        assert_eq!(doc.lines[0].start_ms, 9_500);
        let text = doc_to_lrc(&doc);
        assert!(!text.contains("[offset:"), "{text}");
        let reloaded = spote_core::lyrics::parse_lrc(&text);
        let starts: Vec<u64> = reloaded.lines.iter().map(|line| line.start_ms).collect();
        assert_eq!(starts, vec![9_500, 12_000]);
        assert_eq!(reloaded.offset_ms, 0);
        assert!(
            reloaded.lines.iter().all(|line| !line.words_exact),
            "las palabras sintetizadas no deben volverse exactas al cachear"
        );
    }

    #[test]
    fn lrc_roundtrip_keeps_real_word_timings() {
        let doc = spote_core::lyrics::parse_lrc(
            "[00:10.00]<00:10.00>hola <00:10.50>mundo\n[00:15.00]fin",
        );
        assert!(doc.lines[0].words_exact);
        let text = doc_to_lrc(&doc);
        assert!(text.contains("<00:10.00>"), "{text}");
        let reloaded = spote_core::lyrics::parse_lrc(&text);
        assert!(reloaded.lines[0].words_exact);
        assert_eq!(reloaded.lines[0].words[0].start_ms, 10_000);
    }

    #[test]
    fn art_worker_outputs_png_for_big_plain_cover() {
        let path = std::path::Path::new("/tmp/cover.img");
        if !path.exists() {
            return;
        }
        let worker = ArtWorker::start();
        worker.request(ArtRequest {
            key: "test-png".into(),
            url: None,
            local: Some(path.to_path_buf()),
            prefer_url: false,
            w: 60,
            h: 30,
            mode: ArtMode::Plain,
            kind: ArtKind::Main,
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
        assert!(
            response.png.is_some(),
            "no generó PNG: {:?}",
            response.error
        );
    }

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
            prefer_url: false,
            w: 60,
            h: 20,
            mode: ArtMode::Dither,
            kind: ArtKind::Main,
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

// ---------------------------------------------------------------------------
// Perfiles de artista y metadatos de Spotify/Deezer
// ---------------------------------------------------------------------------

pub enum MetaJob {
    Artist(String),
    ArtistId {
        provider: String,
        id: String,
        name: String,
    },
    Album {
        provider: String,
        id: String,
    },
    AlbumSearch {
        album: String,
        artist: String,
    },
    Metadata {
        key: String,
        artist: String,
        title: String,
    },
}

pub enum MetaEvent {
    Artist {
        name: String,
        result: Result<spote_core::spotify_api::ArtistProfile, String>,
    },
    Album {
        provider: String,
        id: String,
        result: Result<spote_core::spotify_api::AlbumInfo, String>,
    },
    AlbumByName {
        album: String,
        artist: String,
        result: Result<spote_core::spotify_api::AlbumInfo, String>,
    },
    Metadata {
        key: String,
        result: Option<spote_core::spotify_api::SpotifyTrack>,
    },
}

pub struct MetaWorker {
    jobs: Sender<MetaJob>,
    events: Receiver<MetaEvent>,
}

impl MetaWorker {
    pub fn start() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<MetaJob>();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-meta".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let event = match job {
                        MetaJob::Artist(name) => {
                            let result = spotify_api::artist(&name).map_err(|err| err.to_string());
                            MetaEvent::Artist { name, result }
                        }
                        MetaJob::ArtistId { provider, id, name } => {
                            let result = spotify_api::artist_by_id(&provider, &id)
                                .map_err(|err| err.to_string());
                            MetaEvent::Artist { name, result }
                        }
                        MetaJob::Album { provider, id } => {
                            let result =
                                spotify_api::album(&provider, &id).map_err(|err| err.to_string());
                            MetaEvent::Album {
                                provider,
                                id,
                                result,
                            }
                        }
                        MetaJob::AlbumSearch { album, artist } => {
                            let result = search_album(&artist, &album)
                                .ok_or_else(|| "sin resultados".to_string());
                            MetaEvent::AlbumByName {
                                album,
                                artist,
                                result,
                            }
                        }
                        MetaJob::Metadata { key, artist, title } => {
                            let result = search_metadata(&artist, &title);
                            MetaEvent::Metadata { key, result }
                        }
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

    pub fn artist(&self, name: String) {
        let _ = self.jobs.send(MetaJob::Artist(name));
    }

    pub fn artist_by_id(&self, provider: String, id: String, name: String) {
        let _ = self.jobs.send(MetaJob::ArtistId { provider, id, name });
    }

    pub fn album(&self, provider: String, id: String) {
        let _ = self.jobs.send(MetaJob::Album { provider, id });
    }

    pub fn album_search(&self, album: String, artist: String) {
        let _ = self.jobs.send(MetaJob::AlbumSearch { album, artist });
    }

    pub fn metadata(&self, key: String, artist: String, title: String) {
        let _ = self.jobs.send(MetaJob::Metadata { key, artist, title });
    }

    pub fn try_recv(&self) -> Option<MetaEvent> {
        self.events.try_recv().ok()
    }
}

/// Busca un álbum por nombre y artista y devuelve su ficha completa.
fn search_album(artist: &str, album: &str) -> Option<spote_core::spotify_api::AlbumInfo> {
    let query = format!("{artist} {album}");
    let results = spotify_api::search_all(&query, 8).ok()?;
    let wanted = spote_core::util::normalize(album);
    let candidate = results
        .albums
        .into_iter()
        .filter(|item| {
            let got = spote_core::util::normalize(&item.title);
            got == wanted || got.contains(&wanted) || wanted.contains(&got)
        })
        .max_by_key(|item| {
            let artist_match = spote_core::util::normalize(&item.artist)
                .contains(&spote_core::util::normalize(artist));
            (artist_match, item.track_count)
        })?;
    spotify_api::album(&candidate.provider, &candidate.id).ok()
}

/// Busca metadatos y se queda con la pista que mejor coincide. Evita coger una
/// versión en directo o un remix solo porque salió primero.
fn search_metadata(artist: &str, title: &str) -> Option<spote_core::spotify_api::SpotifyTrack> {
    let query = format!("{artist} {title}");
    let tracks = spotify_api::search(&query, 8).ok()?;
    let wanted_title = util::normalize(title);
    let wanted_artist = util::normalize(artist);
    tracks
        .into_iter()
        .map(|track| {
            let t = util::normalize(&track.title);
            let a = util::normalize(&track.artists);
            let mut score = 0i32;
            if t == wanted_title {
                score += 60;
            } else if t.contains(&wanted_title) || wanted_title.contains(&t) {
                score += 30;
            }
            if a == wanted_artist {
                score += 40;
            } else if a.contains(&wanted_artist) || wanted_artist.contains(&a) {
                score += 20;
            }
            if track.duration_ms > 0 {
                score += 5;
            }
            (score, track)
        })
        // Por debajo de esto es otra canción con un nombre parecido: mejor no
        // pisar los metadatos del usuario.
        .filter(|(score, _)| *score >= 30)
        .max_by_key(|(score, _)| *score)
        .map(|(_, track)| track)
}

// ---------------------------------------------------------------------------
// Mapa de energía de la canción completa ("feeling")
// ---------------------------------------------------------------------------

pub struct SongMapWorker {
    jobs: Sender<(String, PathBuf)>,
    events: Receiver<SongMapEvent>,
}

pub enum SongMapEvent {
    Loaded {
        key: String,
        map: spote_core::audio_map::SongMap,
    },
    Failed {
        key: String,
        error: String,
    },
}

impl SongMapWorker {
    pub fn start() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<(String, PathBuf)>();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-song-map".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Si el usuario salta de pista, solo interesa la última.
                    while let Ok(next) = job_rx.try_recv() {
                        job = next;
                    }
                    let (key, path) = job;
                    let event = match spote_core::audio_map::analyze_file(&path) {
                        Ok(map) => SongMapEvent::Loaded { key, map },
                        Err(error) => SongMapEvent::Failed {
                            key,
                            error: error.to_string(),
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

    pub fn request(&self, key: String, path: PathBuf) {
        let _ = self.jobs.send((key, path));
    }

    pub fn try_recv(&self) -> Option<SongMapEvent> {
        self.events.try_recv().ok()
    }
}
