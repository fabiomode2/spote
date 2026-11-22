//! Resolución de reproducción de YouTube con `yt-dlp`.
//!
//! Un único worker consume trabajos de una cola `mpsc` y publica eventos por
//! otra. Para cada pista intenta primero una URL directa (`yt-dlp -g`); si el
//! vídeo no se puede resolver por esa vía, descarga el audio **sin
//! transcodificar** a una caché propia y publica la ruta. Así la UI no tiene
//! que caer siempre al pipeline completo de `Downloader` (que sí convierte a
//! opus) cuando YouTube cambia los clientes o exige PO token.
//!
//! El módulo no abre conexiones por su cuenta: todo el trabajo de red lo hace
//! el binario externo. La caché de URLs caduca con la propia URL; la de
//! ficheros se conserva mientras el fichero exista.
//!
//! ## Integración en la TUI
//!
//! `StreamResolver::resolve` conserva su firma. El worker puede responder de
//! tres formas:
//!
//! - `StreamEvent::Resolved { key, source }` → `player.load_url(source.url, start_secs)`.
//! - `StreamEvent::Downloaded { key, path, provider, id }` → `player.load(path, start_secs)`.
//!   El fichero ya está descargado y no necesita pasar por `Downloader`.
//! - `StreamEvent::Failed { key, message }` → mostrar el mensaje; ya incluye
//!   una etiqueta accionable (`AgeGated`, `GeoBlocked`, `Unavailable`,
//!   `NoMatch`, `Timeout`).
//!
//! En `App::poll_stream` la rama de `Downloaded` usa la misma comprobación de
//! `key` actual que las otras dos, limpia `pending_stream`, marca
//! `now.loading = false` / `now.playing = true`, guarda `now.path`, registra
//! `provider`/`id` si se quiere conservar la fuente y llama a `Player::load`
//! (o a `play_local`, que acaba ahí). Si se quiere prefetch explícito, existe
//! `StreamResolver::resolve_download`, que salta `-g` y emite `Downloaded`.
//! Para usar otra carpeta de caché (por ejemplo `cfg.media_dir()`), se puede
//! arrancar con `StreamResolver::start_in(dir)`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use spote_protocol::SourceHint;

use crate::util;

/// Tiempo máximo para una búsqueda o una resolución con `-g`.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
/// Tiempo máximo para descargar el audio de respaldo.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// Caducidad normal de una URL de reproducción.
const CACHE_TTL: Duration = Duration::from_secs(25 * 60);
/// Margen que se descuenta a `expire=<epoch>` antes de reutilizar una URL.
const EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Cadencia del sondeo de `Child::try_wait`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Resultados que se piden a cada variante de búsqueda.
const SEARCH_RESULTS: usize = 8;
/// Diferencia de duración que se considera un match estricto.
const STRICT_DURATION_TOLERANCE_MS: u64 = 2_000;
/// Diferencia de duración que se acepta como match laxo.
const LOOSE_DURATION_TOLERANCE_MS: u64 = 8_000;
/// Puntuación mínima para aceptar un candidato.
const MIN_MATCH_SCORE: i32 = 70;
/// Puntuación a partir de la cual no hace falta probar más variantes.
const STRONG_MATCH_SCORE: i32 = 130;
/// Líneas de stderr que se conservan en un mensaje de error.
const ERROR_LINES: usize = 4;
/// Timeout de socket que se pasa a yt-dlp.
const SOCKET_TIMEOUT_SECS: &str = "15";

/// Orden de reintento de clientes de YouTube. Los embebidos suelen servir
/// audio sin PO token para vídeos de discográficas; los web normales son el
/// respaldo cuando el vídeo no es embebible.
const PLAYER_CLIENTS: [&str; 2] = [
    "youtube:player_client=web_embedded,tv_embedded",
    "youtube:player_client=default,web_safari",
];

/// Hosts desde los que se acepta una URL de medios.
const ALLOWED_HOSTS: &[&str] = &["googlevideo.com", "youtube.com", "ytimg.com"];

/// Extensiones que puede dejar `yt-dlp` con `bestaudio/best` sin convertir.
/// Incluye `mp4`/`mkv` porque el selector `best` puede caer a un contenedor con
/// vídeo cuando no hay pista de audio sola.
const DOWNLOAD_EXTENSIONS: &[&str] = &[
    "m4a", "webm", "opus", "ogg", "mp3", "aac", "flac", "wav", "mp4", "mkv",
];

/// Fuente reproducible ya resuelta como URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSource {
    pub url: String,
    pub provider: String,
    pub id: String,
}

/// Eventos que el worker envía a la UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// Hay una URL directa reproducible.
    Resolved { key: String, source: StreamSource },
    /// El audio se descargó a la caché de streams (fallback de `-g` o
    /// `resolve_download`). Se reproduce con `Player::load(path, start_secs)`,
    /// no con `load_url`.
    ///
    /// `provider`/`id` identifican el vídeo cuando se conocen; en un acierto de
    /// la caché en disco tras reiniciar pueden venir vacíos y la UI debe caer
    /// al `source_hint` de la biblioteca.
    Downloaded {
        key: String,
        path: PathBuf,
        provider: String,
        id: String,
    },
    /// No se pudo resolver ni descargar. `message` empieza por una etiqueta
    /// accionable (`AgeGated`, `GeoBlocked`, `Unavailable`, `NoMatch`, `Timeout`).
    Failed { key: String, message: String },
}

/// Estrategia de un trabajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strategy {
    /// Intenta URL y, si falla, descarga el audio.
    StreamFirst,
    /// Salta `-g` y descarga directamente (prefetch explícito).
    DownloadOnly,
}

/// Trabajo pendiente en la cola del worker.
#[derive(Debug, Clone)]
struct Job {
    key: String,
    artist: String,
    title: String,
    duration_ms: u64,
    hint: Option<SourceHint>,
    strategy: Strategy,
    /// Generación del `resolve` que lo creó (solo `StreamFirst`). Permite
    /// cancelar un trabajo que ya nació obsoleto por una pista más nueva.
    generation: Option<u64>,
}

/// Caché de URLs: clave lógica -> fuente y momento en que se guardó.
type UrlCache = Arc<Mutex<HashMap<String, (StreamSource, Instant)>>>;

/// Fichero de respaldo ya descargado, con la identidad del vídeo que lo originó.
#[derive(Debug, Clone)]
struct CachedFile {
    path: PathBuf,
    provider: String,
    id: String,
}

/// Caché de ficheros descargados: clave lógica -> fichero y su identidad.
type FileCache = Arc<Mutex<HashMap<String, CachedFile>>>;

/// Token de cancelación de una resolución en curso.
///
/// La UI incrementa `generation` al pedir una pista nueva. El worker captura
/// el valor al empezar y mata el `yt-dlp` en vuelo en cuanto ve otro valor:
/// saltar de pista no espera a que expire un timeout de 45 s o de 3 min.
#[derive(Clone)]
struct Cancel {
    generation: Arc<AtomicU64>,
    mine: u64,
}

impl Cancel {
    /// `mine` es la generación con la que nació el trabajo; `None` para los
    /// prefetch, que se cancelan solo si llega una reproducción después.
    fn new(generation: &Arc<AtomicU64>, mine: Option<u64>) -> Self {
        Self {
            generation: Arc::clone(generation),
            mine: mine.unwrap_or_else(|| generation.load(Ordering::Relaxed)),
        }
    }

    fn requested(&self) -> bool {
        self.generation.load(Ordering::Relaxed) != self.mine
    }
}

/// Error interno para no publicar `Failed` cuando el trabajo simplemente se
/// sustituyó por otro más reciente.
#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("resolución cancelada por una pista más reciente")
    }
}

impl std::error::Error for Cancelled {}

fn cancelled_error() -> anyhow::Error {
    anyhow::Error::new(Cancelled)
}

fn is_cancelled(err: &anyhow::Error) -> bool {
    err.is::<Cancelled>()
}

/// Cola de resolución en un hilo propio.
pub struct StreamResolver {
    tx: Sender<Job>,
    rx: Receiver<StreamEvent>,
    /// Se incrementa con cada `resolve` (reproducción). Los prefetch no la
    /// tocan para no cancelar la pista que está sonando.
    generation: Arc<AtomicU64>,
    /// Se conservan para que los `Arc` vivan mientras exista el resolver; el
    /// worker tiene sus propias copias.
    _urls: UrlCache,
    _files: FileCache,
    _cache_dir: PathBuf,
}

impl StreamResolver {
    /// Arranca el worker con la caché de ficheros por defecto
    /// (`<cache>/spote/stream`, o el directorio temporal si no hay `cache_dir`).
    pub fn start() -> Self {
        Self::start_in(default_cache_dir())
    }

    /// Igual que [`start`](Self::start), pero con una carpeta de caché
    /// concreta. Útil para la TUI (p. ej. `cfg.media_dir()`) y para tests.
    pub fn start_in(cache_dir: impl AsRef<Path>) -> Self {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        let (job_tx, job_rx): (Sender<Job>, Receiver<Job>) = mpsc::channel();
        let (event_tx, event_rx): (Sender<StreamEvent>, Receiver<StreamEvent>) = mpsc::channel();
        let urls: UrlCache = Arc::new(Mutex::new(HashMap::new()));
        let files: FileCache = Arc::new(Mutex::new(HashMap::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let worker_urls = Arc::clone(&urls);
        let worker_files = Arc::clone(&files);
        let worker_generation = Arc::clone(&generation);
        let worker_dir = cache_dir.clone();

        thread::Builder::new()
            .name("spote-stream".into())
            .spawn(move || {
                worker(
                    job_rx,
                    event_tx,
                    worker_urls,
                    worker_files,
                    worker_generation,
                    worker_dir,
                )
            })
            .expect("no se pudo arrancar el resolver de streams");

        Self {
            tx: job_tx,
            rx: event_rx,
            generation,
            _urls: urls,
            _files: files,
            _cache_dir: cache_dir,
        }
    }

    /// Encola una resolución. Si la cola está cerrada el trabajo se descarta:
    /// la UI ya no está escuchando.
    pub fn resolve(
        &self,
        key: impl Into<String>,
        artist: &str,
        title: &str,
        duration_ms: u64,
        hint: Option<SourceHint>,
    ) {
        // Invalida el trabajo en vuelo (si lo hay) antes de encolar: el worker
        // lo mata en el siguiente sondeo y atiende esta pista.
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.enqueue(Job {
            key: key.into(),
            artist: artist.to_string(),
            title: title.to_string(),
            duration_ms,
            hint,
            strategy: Strategy::StreamFirst,
            generation: Some(generation),
        });
    }

    /// Fuerza la vía de fichero: no intenta `-g`, descarga a la caché y emite
    /// `Downloaded`. Sirve para prefetch o para pistas que ya sabemos que
    /// fallan como stream.
    pub fn resolve_download(
        &self,
        key: impl Into<String>,
        artist: &str,
        title: &str,
        duration_ms: u64,
        hint: Option<SourceHint>,
    ) {
        self.enqueue(Job {
            key: key.into(),
            artist: artist.to_string(),
            title: title.to_string(),
            duration_ms,
            hint,
            strategy: Strategy::DownloadOnly,
            generation: None,
        });
    }

    fn enqueue(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// Lee un evento sin bloquear.
    pub fn try_recv(&self) -> Option<StreamEvent> {
        self.rx.try_recv().ok()
    }
}

/// Bucle del worker. Antes de un prefetch diferido se mira la cola: si hay una
/// reproducción pendiente, manda ella. Un trabajo en vuelo se cancela cuando
/// llega otra reproducción (`resolve` incrementa la generación), así que saltar
/// de pista no espera a que termine un `yt-dlp` colgado.
fn worker(
    jobs: Receiver<Job>,
    events: Sender<StreamEvent>,
    urls: UrlCache,
    files: FileCache,
    generation: Arc<AtomicU64>,
    cache_dir: PathBuf,
) {
    let mut deferred: VecDeque<Job> = VecDeque::new();

    loop {
        // Vacía la cola: el trabajo de reproducción más reciente gana y los
        // prefetch quedan aparcados, pero nunca por delante de una pista.
        let mut newest_play: Option<Job> = None;
        while let Ok(job) = jobs.try_recv() {
            if job.strategy == Strategy::StreamFirst {
                newest_play = Some(job);
            } else {
                deferred.push_back(job);
            }
        }

        let job = if let Some(job) = newest_play {
            job
        } else if let Some(job) = deferred.pop_front() {
            job
        } else {
            match jobs.recv() {
                // Un prefetch nunca debe adelantar a una reproducción que ya
                // esté en la cola: se aparca y se vuelve a mirar arriba.
                Ok(job) if job.strategy == Strategy::DownloadOnly => {
                    deferred.push_back(job);
                    continue;
                }
                Ok(job) => job,
                Err(_) => {
                    // Sin emisor no queda nadie escuchando: no se procesan
                    // prefetch que ya no sirven a ninguna UI.
                    return;
                }
            }
        };

        let cancel = Cancel::new(&generation, job.generation);
        handle_job(job, &events, &urls, &files, &cache_dir, &cancel);
    }
}

/// Ejecuta un trabajo ya seleccionado y publica como mucho un evento. Un
/// trabajo cancelado no publica `Failed`: lo sustituyó otra pista.
fn handle_job(
    job: Job,
    events: &Sender<StreamEvent>,
    urls: &UrlCache,
    files: &FileCache,
    cache_dir: &Path,
    cancel: &Cancel,
) {
    // Para un stream normal, una URL viva es más barata que ir al disco.
    // Para un prefetch explícito, el fichero manda.
    if job.strategy == Strategy::StreamFirst {
        if let Some(source) = url_cache_lookup(urls, &job.key) {
            let _ = events.send(StreamEvent::Resolved {
                key: job.key,
                source,
            });
            return;
        }
    }

    if let Some(cached) = file_cache_lookup(files, cache_dir, &job.key) {
        let _ = events.send(StreamEvent::Downloaded {
            key: job.key,
            path: cached.path,
            provider: cached.provider,
            id: cached.id,
        });
        return;
    }

    if cancel.requested() {
        return;
    }

    match resolve_job(&job, cache_dir, Some(cancel)) {
        Ok(Resolution::Url(source)) => {
            url_cache_store(urls, job.key.clone(), source.clone());
            let _ = events.send(StreamEvent::Resolved {
                key: job.key,
                source,
            });
        }
        Ok(Resolution::File { path, provider, id }) => {
            file_cache_store(
                files,
                job.key.clone(),
                path.clone(),
                provider.clone(),
                id.clone(),
            );
            let _ = events.send(StreamEvent::Downloaded {
                key: job.key,
                path,
                provider,
                id,
            });
        }
        Err(err) if is_cancelled(&err) => {}
        Err(err) => {
            let _ = events.send(StreamEvent::Failed {
                key: job.key,
                message: format!("{err:#}"),
            });
        }
    }
}

/// Resultado interno de resolver un trabajo.
enum Resolution {
    Url(StreamSource),
    File {
        path: PathBuf,
        provider: String,
        id: String,
    },
}

/// Resuelve un trabajo. El `hint` sólo se usa si apunta a YouTube y trae un ID
/// con forma de ID de vídeo; en cualquier otro caso se busca por texto.
///
/// Si `-g` falla por el motivo que sea (PO token, edad, formato, host raro),
/// se intenta la descarga antes de devolver el error. Un hint válido nunca
/// termina en `Failed` sin haber probado también la descarga. Si el trabajo se
/// cancela por una pista nueva, la cancelación gana a cualquier fallback.
fn resolve_job(job: &Job, cache_dir: &Path, cancel: Option<&Cancel>) -> Result<Resolution> {
    // El `TrackRef` puede no traer duración, pero el hint suele conservarla.
    let wanted_duration = if job.duration_ms > 0 {
        job.duration_ms
    } else {
        job.hint.as_ref().map(|hint| hint.duration_ms).unwrap_or(0)
    };

    let (id, provider) = match hinted_video(job.hint.as_ref()) {
        Some((id, provider)) => (id, provider),
        None => {
            let candidate = search(&job.artist, &job.title, wanted_duration, cancel)?;
            (candidate.id, "youtube".to_string())
        }
    };

    // Defensa en profundidad: todo ID que llega aquí debe tener ya la forma
    // exacta de YouTube. La URL de watch se construye siempre desde el ID.
    if !is_valid_video_id(&id) {
        return Err(failure(
            FailureKind::Unavailable,
            format!("ID de vídeo inválido: {id:?}"),
        ));
    }

    if job.strategy == Strategy::DownloadOnly {
        let path = download_audio(&id, &job.key, cache_dir, cancel)?;
        return Ok(Resolution::File { path, provider, id });
    }

    match resolve_media_url(&watch_url(&id), cancel) {
        Ok(url) => Ok(Resolution::Url(StreamSource { url, provider, id })),
        Err(stream_error) if is_cancelled(&stream_error) => Err(stream_error),
        Err(stream_error) => match download_audio(&id, &job.key, cache_dir, cancel) {
            Ok(path) => Ok(Resolution::File { path, provider, id }),
            Err(download_error) if is_cancelled(&download_error) => Err(download_error),
            Err(download_error) => Err(combined_failure(stream_error, download_error)),
        },
    }
}

/// Devuelve `(id, provider)` sólo cuando el hint es utilizable.
fn hinted_video(hint: Option<&SourceHint>) -> Option<(String, String)> {
    let hint = hint?;
    let provider = hint.provider.trim();
    let id = hint.id.trim();
    if !is_youtube_provider(provider) || !is_valid_video_id(id) {
        return None;
    }
    Some((id.to_string(), provider.to_string()))
}

fn is_youtube_provider(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    provider.contains("youtube") || provider.contains("ytmusic")
}

/// `^[A-Za-z0-9_-]{11}$`, sin regex para no añadir dependencias.
fn is_valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Candidato tal como sale del JSON de `ytsearch`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchCandidate {
    id: String,
    /// Título crudo del vídeo (puede incluir "Artista - Tema" y sufijos).
    title: String,
    /// Canal que publicó el vídeo, ya sea `uploader` o `channel`.
    channel: String,
    duration_ms: u64,
    /// yt-dlp marca directos y directos ya terminados con `live_status`.
    is_live: bool,
}

/// Genera las consultas de búsqueda en orden de especificidad. Se deduplican
/// sin perder el orden para no gastar una llamada a yt-dlp en la misma cadena.
fn query_variants(artist: &str, title: &str) -> Vec<String> {
    let artist = artist.trim();
    let title = title.trim();
    let mut out = Vec::new();
    if !artist.is_empty() && !title.is_empty() {
        out.push(format!("{artist} {title}"));
        out.push(format!("{title} {artist}"));
    }
    if !title.is_empty() {
        out.push(title.to_string());
    }
    if out.is_empty() && !artist.is_empty() {
        out.push(artist.to_string());
    }

    let mut seen = HashSet::new();
    out.retain(|query| seen.insert(query.to_lowercase()));
    out
}

/// Busca un vídeo para una pista y devuelve su ID. La usa el `Downloader`
/// para no mantener un segundo matcher más laxo que el de reproducción.
pub fn find_video_id(artist: &str, title: &str, duration_ms: u64) -> Result<String> {
    search(artist, title, duration_ms, None).map(|candidate| candidate.id)
}

/// Busca en YouTube probando las variantes y elige el mejor candidato. Si una
/// variante da un match fuerte se corta ahí; si no, se acumulan resultados de
/// todas y se decide al final. Un resultado dudoso no se acepta. `cancel` sólo
/// lo pasa la reproducción: el `Downloader` no cancela sus búsquedas.
fn search(
    artist: &str,
    title: &str,
    duration_ms: u64,
    cancel: Option<&Cancel>,
) -> Result<SearchCandidate> {
    let queries = query_variants(artist, title);
    let Some(first_query) = queries.first() else {
        return Err(failure(
            FailureKind::NoMatch,
            "no hay artista ni título para buscar",
        ));
    };

    let mut seen = HashSet::new();
    let mut candidates: Vec<SearchCandidate> = Vec::new();
    let mut best: Option<(i32, SearchCandidate)> = None;
    let mut last_error: Option<anyhow::Error> = None;
    let mut any_search_succeeded = false;

    for query in &queries {
        if cancel.is_some_and(Cancel::requested) {
            return Err(cancelled_error());
        }
        match search_once(query, cancel) {
            Ok(items) => {
                any_search_succeeded = true;
                for item in items {
                    if seen.insert(item.id.clone()) {
                        candidates.push(item);
                    }
                }
                if let Some((score, candidate)) =
                    rank_candidates(&candidates, artist, title, duration_ms)
                {
                    if score >= STRONG_MATCH_SCORE {
                        return Ok(candidate);
                    }
                    let better = best.as_ref().map_or(true, |(best_score, known)| {
                        score > *best_score
                            || (score == *best_score && tie_break(&candidate, known))
                    });
                    if better {
                        best = Some((score, candidate));
                    }
                }
            }
            Err(err) if is_cancelled(&err) => return Err(err),
            Err(err) => {
                // Se conserva la etiqueta al principio del mensaje; la
                // consulta se añade detrás para no romper ese prefijo.
                last_error = Some(anyhow!("{err:#} (consulta: «{query}»)"));
            }
        }
    }

    if let Some((_, candidate)) = best {
        return Ok(candidate);
    }
    if any_search_succeeded {
        Err(failure(FailureKind::NoMatch, format!("«{first_query}»")))
    } else {
        Err(last_error.unwrap_or_else(|| failure(FailureKind::NoMatch, format!("«{first_query}»"))))
    }
}

/// Una llamada a `ytsearchN:` con una única consulta.
fn search_once(query: &str, cancel: Option<&Cancel>) -> Result<Vec<SearchCandidate>> {
    let args = vec![
        "--flat-playlist".to_string(),
        "--dump-single-json".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--socket-timeout".to_string(),
        SOCKET_TIMEOUT_SECS.to_string(),
        // `--` corta las opciones: una consulta que empiece por `-` no puede
        // convertirse en un flag de yt-dlp.
        "--".to_string(),
        format!("ytsearch{SEARCH_RESULTS}:{query}"),
    ];
    let output = run_ytdlp(&args, COMMAND_TIMEOUT, cancel)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_search_json(&stdout)
}

/// Extrae candidatos válidos de la salida de `--dump-single-json` o de un
/// stream JSONL (`--dump-json`). Las líneas o entradas ilegibles se saltan.
fn parse_search_json(json: &str) -> Result<Vec<SearchCandidate>> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // Caso normal: un objeto de búsqueda con `entries`, una lista o un único
    // vídeo. `entries` se mira primero para no confundir un objeto de
    // búsqueda con un candidato suelto.
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let entries = match &value {
            Value::Object(map) => map.get("entries").and_then(Value::as_array),
            Value::Array(items) => Some(items),
            _ => None,
        };
        if let Some(entries) = entries {
            return Ok(entries.iter().filter_map(parse_candidate).collect());
        }
        if let Some(candidate) = parse_candidate(&value) {
            return Ok(vec![candidate]);
        }
        return Ok(Vec::new());
    }

    // Respaldo: una línea JSON por resultado, con avisos intercalados.
    let mut out = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(candidate) = parse_candidate(&value) {
                out.push(candidate);
            }
        }
    }
    Ok(out)
}

/// Convierte una entrada de yt-dlp en candidato. Devuelve `None` si el ID no
/// es válido o no hay título con el que comparar.
fn parse_candidate(value: &Value) -> Option<SearchCandidate> {
    let id = value.get("id").and_then(Value::as_str)?.trim();
    if !is_valid_video_id(id) {
        return None;
    }
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if title.is_empty() {
        return None;
    }
    let channel = ["channel", "uploader", "creator", "artist"]
        .iter()
        .find_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_default();

    let is_live = value
        .get("live_status")
        .and_then(Value::as_str)
        .map(|status| matches!(status, "is_live" | "was_live" | "post_live" | "is_upcoming"))
        .unwrap_or(false)
        || value
            .get("is_live")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    Some(SearchCandidate {
        id: id.to_string(),
        title: title.to_string(),
        channel: channel.to_string(),
        duration_ms: parse_duration_ms(value.get("duration")),
        is_live,
    })
}

/// `duration` puede venir como número, como cadena o como `null`.
fn parse_duration_ms(value: Option<&Value>) -> u64 {
    let seconds = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    seconds
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| (value * 1000.0).round() as u64)
        .unwrap_or(0)
}

/// Elige el candidato con más puntos. Si hay alguno con duración conocida, los
/// de duración 0 se descartan: suelen ser directos o resultados incompletos.
#[cfg(test)]
fn best_candidate(
    candidates: &[SearchCandidate],
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> Option<SearchCandidate> {
    rank_candidates(candidates, artist, title, duration_ms).map(|(_, candidate)| candidate)
}

/// Devuelve el candidato y su puntuación sólo si supera el mínimo. La
/// puntuación también sirve para cortar antes de probar más variantes.
fn rank_candidates(
    candidates: &[SearchCandidate],
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> Option<(i32, SearchCandidate)> {
    if candidates.is_empty() {
        return None;
    }

    // Sólo se puntúan los candidatos que pasan los filtros (título, artista,
    // variante y duración). Así un cover descartado no impide que se tenga en
    // cuenta un resultado sin duración.
    let mut scored: Vec<(i32, &SearchCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            let score = score_candidate(candidate, artist, title, duration_ms)?;
            (score >= MIN_MATCH_SCORE).then_some((score, candidate))
        })
        .collect();

    if scored.is_empty() {
        return None;
    }

    // Si hay un candidato válido con duración, los de duración desconocida
    // quedan como última opción: suelen ser directos o resultados incompletos.
    if scored
        .iter()
        .any(|(_, candidate)| candidate.duration_ms > 0)
    {
        scored.retain(|(_, candidate)| candidate.duration_ms > 0);
    }

    let mut best: Option<(i32, &SearchCandidate)> = None;
    for (score, candidate) in scored {
        let better = match best {
            None => true,
            Some((best_score, best_candidate)) => {
                score > best_score || (score == best_score && tie_break(candidate, best_candidate))
            }
        };
        if better {
            best = Some((score, candidate));
        }
    }

    best.map(|(score, candidate)| (score, (*candidate).clone()))
}

/// Heurística determinista inspirada en `catalog::pick_best`: el título manda,
/// el artista no puede ser otro, la duración sólo tolera ±2 s estricta o ±8 s
/// laxa y las variantes no pedidas (cover, remix, live, karaoke…) se rechazan.
fn score_candidate(
    candidate: &SearchCandidate,
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> Option<i32> {
    let wanted_title = normalize_title(title);
    let wanted_artist = normalize_text(artist);
    if wanted_title.is_empty() {
        return None;
    }
    // Se pasa el texto crudo: así "(Live)" o "(Remix)" del título pedido
    // cuentan como variante deseada y no se descartan.
    if has_unwanted_noise(candidate, title, artist) {
        return None;
    }

    let primary = primary_artist(artist);
    let (got_title, left_artist) =
        candidate_title_parts(candidate, &wanted_artist, primary.as_deref());
    let (mut score, exact_title) = title_match_score(&wanted_title, &got_title)?;

    let got_artist = normalize_text(&format!("{} {}", candidate.channel, left_artist));
    if !wanted_artist.is_empty() {
        let artist_ok = artist_matches(&wanted_artist, &got_artist)
            || primary
                .as_deref()
                .is_some_and(|name| artist_matches(name, &got_artist));
        if artist_ok {
            score += 32;
        } else if exact_title {
            // Mismo título pero otro artista: casi seguro un cover, una
            // versión de otro canal o una canción homónima. No se baja.
            if got_artist.trim().is_empty() {
                score -= 5;
            } else {
                return None;
            }
        } else {
            score -= 12;
        }
    }

    score += channel_bonus(&candidate.channel);
    score += duration_score(duration_ms, candidate.duration_ms)?;
    Some(score)
}

/// Puntos por título. `None` significa que no hay match suficiente: un título
/// parecido por tokens no basta para arriesgar una descarga equivocada.
fn title_match_score(wanted: &str, got: &str) -> Option<(i32, bool)> {
    if wanted.is_empty() || got.is_empty() {
        return None;
    }
    if got == wanted {
        return Some((60, true));
    }
    // Las contenciones exigen un mínimo de tres caracteres para que un
    // resultado titulado "A" o "The" no cuele como el tema buscado. Una
    // contención larga ("alma en pena" dentro de "como alma en pena") es
    // casi un match exacto: los visualizers suelen añadir una o dos palabras.
    let wanted_len = wanted.chars().count();
    let got_len = got.chars().count();
    if wanted_len >= 3 && got.contains(wanted) {
        Some((if wanted_len >= 6 { 45 } else { 28 }, false))
    } else if got_len >= 3 && wanted.contains(got) {
        Some((if got_len >= 6 { 38 } else { 28 }, false))
    } else {
        None
    }
}

/// Puntos por duración. Las diferencias cortas puntúan; las de videoclip o
/// visualizer no descartan si título y artista ya son sólidos. Solo se rechaza
/// una diferencia mayor de dos minutos. Sin duración se penaliza, no se descarta.
fn duration_score(wanted_ms: u64, got_ms: u64) -> Option<i32> {
    if wanted_ms == 0 || got_ms == 0 {
        return Some(-8);
    }
    let diff = wanted_ms.abs_diff(got_ms);
    if diff <= STRICT_DURATION_TOLERANCE_MS {
        Some(45)
    } else if diff <= LOOSE_DURATION_TOLERANCE_MS {
        Some(28)
    } else if diff <= 30_000 {
        // Un videoclip puede añadir una intro o un outro corto.
        Some(12)
    } else if diff <= 120_000 {
        // Visualizers y vídeos con intro larga: el título y el artista ya
        // han pasado el filtro, la duración solo desempata.
        Some(0)
    } else {
        None
    }
}

/// Separa "Artista - Tema" sólo cuando la parte izquierda coincide con el
/// artista buscado o con el artista de un canal `- Topic`. Si no, se compara
/// el título completo para no perder coincidencias legítimas.
fn candidate_title_parts(
    candidate: &SearchCandidate,
    wanted_artist: &str,
    alternate_artist: Option<&str>,
) -> (String, String) {
    let full = normalize_title(&candidate.title);
    let Some((left, right)) = candidate.title.split_once(" - ") else {
        return (full, String::new());
    };

    let left_title = normalize_title(left);
    let right_title = normalize_title(right);
    if right_title.is_empty() {
        return (full, left_title);
    }

    let topic_artist = topic_channel_artist(&candidate.channel)
        .map(|artist| normalize_text(&artist))
        .unwrap_or_default();
    let left_matches_wanted = !wanted_artist.is_empty()
        && (artist_matches(wanted_artist, &left_title)
            || alternate_artist.is_some_and(|alternate| {
                !alternate.is_empty() && artist_matches(alternate, &left_title)
            }));
    let left_matches_topic = !topic_artist.is_empty() && artist_matches(&topic_artist, &left_title);

    if left_matches_wanted || left_matches_topic {
        (right_title, left_title)
    } else {
        (full, left_title)
    }
}

/// Primer artista de un crédito múltiple: "Daft Punk, Pharrell Williams" o
/// "Daft Punk feat. Pharrell" -> "Daft Punk". Devuelve `None` si no hay una
/// variante distinta que probar.
fn primary_artist(artist: &str) -> Option<String> {
    let artist = artist.trim();
    if artist.is_empty() {
        return None;
    }

    let mut primary = artist;
    // Los separadores con espacios no rompen nombres como "R&B" o "AC/DC".
    for separator in [",", ";", " & ", " / "] {
        if let Some((left, _)) = primary.split_once(separator) {
            primary = left.trim();
            break;
        }
    }
    let lower = primary.to_ascii_lowercase();
    for marker in [
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " with ",
        " x ",
    ] {
        if let Some(index) = lower.find(marker) {
            primary = primary[..index].trim();
            break;
        }
    }

    if primary.is_empty() || primary.eq_ignore_ascii_case(artist) {
        None
    } else {
        Some(normalize_text(primary))
    }
}

/// Compara artistas por tokens completos y por versión compacta (sin espacios),
/// para que `Daft Punk` case con `DaftPunkVEVO` y con `Daft Punk - Topic`.
fn artist_matches(wanted: &str, got: &str) -> bool {
    if wanted.is_empty() || got.is_empty() {
        return false;
    }
    if wanted == got {
        return true;
    }

    // Comparación por tokens completos: "Queen" no debe casar con "Queens of
    // the Stone Age" sólo porque comparta prefijo.
    let wanted_tokens: Vec<&str> = wanted.split_whitespace().collect();
    let got_tokens: Vec<&str> = got.split_whitespace().collect();
    if !wanted_tokens.is_empty() && wanted_tokens.iter().all(|token| got_tokens.contains(token)) {
        return true;
    }

    // VEVO y algunos canales oficiales concatenan el nombre sin espacios:
    // "DaftPunkVEVO", "DaftPunkOfficial". Se acepta ese sufijo exacto.
    let wanted_compact: String = wanted_tokens.join("");
    let got_compact: String = got_tokens.join("");
    if wanted_compact.is_empty() || got_compact.is_empty() {
        return false;
    }
    if got_compact == wanted_compact {
        return true;
    }
    if let Some(base) = got_compact
        .strip_suffix("vevo")
        .or_else(|| got_compact.strip_suffix("official"))
        .or_else(|| got_compact.strip_suffix("topic"))
    {
        // "TheBeatlesVEVO" también vale para el artista "Beatles".
        if base.ends_with(wanted_compact.as_str()) {
            return true;
        }
    }
    false
}

/// Canales `- Topic` son audio de álbum; VEVO y canales oficiales suman algo
/// menos. El bonus sólo desempata entre candidatos que ya han pasado el filtro.
fn channel_bonus(channel: &str) -> i32 {
    if channel_is_topic(channel) {
        18
    } else if channel_is_vevo(channel) {
        12
    } else if contains_phrase(&keyword_text(channel), "official") {
        6
    } else {
        0
    }
}

fn channel_is_topic(channel: &str) -> bool {
    let lower = channel.trim().to_ascii_lowercase();
    lower.ends_with(" - topic") || lower.ends_with(" - topic / topic")
}

fn channel_is_vevo(channel: &str) -> bool {
    channel.trim().to_ascii_lowercase().ends_with("vevo")
}

/// `"Daft Punk - Topic"` -> `"Daft Punk"`; `None` si no es un canal Topic.
fn topic_channel_artist(channel: &str) -> Option<String> {
    let channel = channel.trim();
    let lower = channel.to_ascii_lowercase();
    for suffix in [" - topic / topic", " - topic"] {
        if lower.ends_with(suffix) {
            let artist = channel[..channel.len() - suffix.len()].trim();
            if !artist.is_empty() {
                return Some(artist.to_string());
            }
        }
    }
    None
}

/// Descarta variantes que el usuario no pidió. Un directo, un remix o un
/// karaoke con la duración correcta es peor que no encontrar nada.
fn has_unwanted_noise(
    candidate: &SearchCandidate,
    wanted_title: &str,
    wanted_artist: &str,
) -> bool {
    let title = keyword_text(&candidate.title);
    let channel = keyword_text(&candidate.channel);
    let wanted = format!(
        "{} {}",
        keyword_text(wanted_title),
        keyword_text(wanted_artist)
    );

    // `live_status` es más fiable que el título: un directo puede llamarse
    // simplemente "Song" y seguir siendo un directo.
    if candidate.is_live
        && ![
            "live",
            "en vivo",
            "en directo",
            "directo",
            "concert",
            "concierto",
            "performance",
            "actuacion",
        ]
        .iter()
        .any(|term| contains_phrase(&wanted, *term))
    {
        return true;
    }

    const TITLE_NOISE: &[&str] = &[
        "cover",
        "covers",
        "karaoke",
        "remix",
        "remixed",
        "tribute",
        "tributo",
        "parody",
        "parodia",
        "nightcore",
        "sped up",
        "slowed",
        "reverb",
        "8d",
        "8d audio",
        "reaction",
        "tutorial",
        "instrumental",
        "acoustic",
        "acustico",
        "version acustica",
        "live",
        "en vivo",
        "en directo",
        "directo",
        "concert",
        "concierto",
        "performance",
        "actuacion",
        "full album",
        "album stream",
        "playlist",
        "1 hour",
        "one hour",
        "1 hora",
        "extended",
    ];
    for noise in TITLE_NOISE {
        if contains_phrase(&title, *noise) && !contains_phrase(&wanted, *noise) {
            return true;
        }
    }

    const CHANNEL_NOISE: &[&str] = &[
        "cover",
        "covers",
        "karaoke",
        "remix",
        "tribute",
        "tributo",
        "parody",
        "parodia",
        "nightcore",
        "live",
        "reaction",
        "tutorial",
        "lyrics",
    ];
    for noise in CHANNEL_NOISE {
        if contains_phrase(&channel, *noise) && !contains_phrase(&wanted, *noise) {
            return true;
        }
    }

    false
}

/// Desempate estable entre candidatos con la misma puntuación: Topic primero,
/// luego el título más corto y, por último, el ID para que sea determinista.
fn tie_break(candidate: &SearchCandidate, best: &SearchCandidate) -> bool {
    let candidate_topic = channel_is_topic(&candidate.channel);
    let best_topic = channel_is_topic(&best.channel);
    if candidate_topic != best_topic {
        return candidate_topic;
    }
    let candidate_len = candidate.title.chars().count();
    let best_len = best.title.chars().count();
    if candidate_len != best_len {
        return candidate_len < best_len;
    }
    candidate.id.as_str() < best.id.as_str()
}

/// Minúsculas, sin acentos básicos, sin paréntesis y con espacios colapsados.
/// También corta colaboraciones (`feat.`, `ft.`, `featuring`).
fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;

    for raw in text.chars() {
        if ('\u{0300}'..='\u{036f}').contains(&raw) {
            continue;
        }
        match raw {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            _ => {
                for lower in raw.to_lowercase() {
                    let plain = deaccent(lower);
                    if plain.is_alphanumeric() {
                        out.push(plain);
                    } else if plain == '&' {
                        out.push_str(" and ");
                    } else {
                        out.push(' ');
                    }
                }
            }
        }
    }

    let tokens: Vec<&str> = out.split_whitespace().collect();
    let cut = tokens
        .iter()
        .position(|token| matches!(*token, "feat" | "ft" | "featuring"))
        .filter(|index| *index > 0)
        .unwrap_or(tokens.len());
    tokens[..cut].join(" ")
}

/// Como [`normalize_text`], pero además quita los sufijos de vídeo que a veces
/// llegan sin paréntesis: "Official Video", "Lyric Video", "Audio", "HD"…
fn normalize_title(text: &str) -> String {
    let mut out = normalize_text(text);
    const NOISE_SUFFIXES: &[&str] = &[
        " official music video",
        " official video",
        " official audio",
        " official lyric video",
        " lyric video",
        " video oficial",
        " audio oficial",
        " visualizer",
        " remastered",
        " remaster",
        " lyrics",
        " letra",
        " audio",
        " video",
        " hd",
        " hq",
        " 4k",
        " 8k",
        " 1080p",
        " m v",
        " mv",
    ];

    // Dos pasadas: "Song Official Audio HD" necesita quitar " HD" y después
    // " Official Audio".
    for _ in 0..2 {
        for noise in NOISE_SUFFIXES {
            if out.ends_with(*noise) && out.len() > noise.len() {
                out.truncate(out.len() - noise.len());
                out = out.trim_end().to_string();
            }
        }
    }
    out
}

/// Texto para detectar palabras de variante sin perder el contenido de los
/// paréntesis: `normalize_text` los borra y "Get Lucky (Cover)" quedaría como
/// el tema original.
fn keyword_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.chars() {
        for lower in raw.to_lowercase() {
            let plain = deaccent(lower);
            if plain.is_alphanumeric() {
                out.push(plain);
            } else {
                out.push(' ');
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Busca una palabra o frase completa, no una subcadena: `cover` no debe
/// activarse dentro de `discover`.
fn contains_phrase(text: &str, phrase: &str) -> bool {
    if text.is_empty() || phrase.is_empty() {
        return false;
    }
    let haystack = format!(" {text} ");
    let needle = format!(" {phrase} ");
    haystack.contains(needle.as_str())
}

fn deaccent(ch: char) -> char {
    match ch {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

/// Resuelve la URL de medios. Se prueba primero con los clientes embebidos y,
/// si falla, una sola vez con los clientes web habituales.
fn resolve_media_url(watch_url: &str, cancel: Option<&Cancel>) -> Result<String> {
    let mut last_error: Option<anyhow::Error> = None;

    for client in PLAYER_CLIENTS {
        if cancel.is_some_and(Cancel::requested) {
            return Err(cancelled_error());
        }
        let args = stream_args(watch_url, client);
        let result = run_ytdlp(&args, COMMAND_TIMEOUT, cancel).and_then(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_media_url(&stdout)
        });

        match result {
            Ok(url) => return Ok(url),
            Err(err) if is_cancelled(&err) => return Err(err),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        failure(
            FailureKind::Unavailable,
            "yt-dlp no devolvió una URL reproducible",
        )
    }))
}

/// Argumentos de una resolución `-g`. `--` separa la URL de las opciones.
fn stream_args(watch_url: &str, client: &str) -> Vec<String> {
    vec![
        "-g".to_string(),
        "-f".to_string(),
        "bestaudio/best".to_string(),
        "--no-playlist".to_string(),
        "--no-warnings".to_string(),
        "--socket-timeout".to_string(),
        SOCKET_TIMEOUT_SECS.to_string(),
        "--extractor-args".to_string(),
        client.to_string(),
        "--".to_string(),
        watch_url.to_string(),
    ]
}

/// Toma la primera línea `https://` de la salida de `-g` y valida su host.
/// Es estricto con la primera: si apunta a un host no permitido no se sigue
/// buscando, porque yt-dlp debería devolver una única fuente de audio.
fn parse_media_url(stdout: &str) -> Result<String> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("https://") {
            if is_allowed_media_url(line) {
                return Ok(line.to_string());
            }
            return Err(failure(
                FailureKind::Unavailable,
                "yt-dlp devolvió una URL fuera de los hosts permitidos",
            ));
        }
    }
    Err(failure(
        FailureKind::Unavailable,
        "yt-dlp no devolvió ninguna URL https",
    ))
}

/// Acepta el host exacto o cualquier subdominio de los hosts permitidos.
fn is_allowed_media_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };

    // Autoridad = hasta la primera barra, interrogación o almohadilla.
    let authority = rest
        .split(|ch| matches!(ch, '/' | '?' | '#'))
        .next()
        .unwrap_or_default();
    // Una autoridad real no lleva espacios ni barras invertidas; rechazarlas
    // evita que un parser laxo interprete el host de forma distinta.
    if authority.is_empty() || authority.chars().any(|ch| ch.is_whitespace() || ch == '\\') {
        return false;
    }
    // Quita userinfo (`usuario@host`) y puerto.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }

    ALLOWED_HOSTS.iter().any(|allowed| {
        host.as_str() == *allowed || {
            let suffix = format!(".{allowed}");
            host.ends_with(suffix.as_str())
        }
    })
}

/// Descarga el audio sin transcodificar a la caché. Reintenta con los clientes
/// embebidos y luego con los web, igual que `-g`. No usa `-x` ni ffmpeg: el
/// fichero queda en el formato nativo que sirva YouTube (`m4a`, `webm`, ...).
fn download_audio(
    id: &str,
    key: &str,
    cache_dir: &Path,
    cancel: Option<&Cancel>,
) -> Result<PathBuf> {
    if !is_valid_video_id(id) {
        return Err(failure(
            FailureKind::Unavailable,
            format!("ID de vídeo inválido: {id:?}"),
        ));
    }

    std::fs::create_dir_all(cache_dir).with_context(|| {
        format!(
            "no pude crear el directorio de caché de streams: {}",
            cache_dir.display()
        )
    })?;

    let base = cache_dir.join(cache_file_name(key));
    let watch = watch_url(id);
    let mut last_error: Option<anyhow::Error> = None;

    for client in PLAYER_CLIENTS {
        if cancel.is_some_and(Cancel::requested) {
            remove_download_artifacts(&base);
            return Err(cancelled_error());
        }
        remove_download_artifacts(&base);
        let args = download_args(&base, &watch, client);
        match run_ytdlp(&args, DOWNLOAD_TIMEOUT, cancel) {
            Ok(_) => match find_downloaded(&base) {
                Some(path) => return Ok(path),
                None => {
                    last_error = Some(failure(
                        FailureKind::Unavailable,
                        format!("yt-dlp terminó pero no dejó audio en {}", base.display()),
                    ));
                }
            },
            Err(err) if is_cancelled(&err) => {
                // Un `.part` a medias no debe quedarse en la caché: la
                // siguiente pista reutiliza el mismo nombre.
                remove_download_artifacts(&base);
                return Err(err);
            }
            Err(err) => last_error = Some(err),
        }
    }

    remove_download_artifacts(&base);
    Err(last_error
        .unwrap_or_else(|| failure(FailureKind::Unavailable, "yt-dlp no descargó ningún audio")))
}

/// Argumentos de la descarga de respaldo. Sin `-x`: nada de transcodificar.
fn download_args(base: &Path, watch_url: &str, client: &str) -> Vec<String> {
    vec![
        "--no-playlist".to_string(),
        "-f".to_string(),
        "bestaudio/best".to_string(),
        "--no-warnings".to_string(),
        "--newline".to_string(),
        "--no-progress".to_string(),
        "--force-overwrites".to_string(),
        // Sin `--no-part`: yt-dlp escribe `.part` y renombra al terminar. Un
        // corte deja el `.part`, nunca un audio final truncado en la caché.
        "--socket-timeout".to_string(),
        SOCKET_TIMEOUT_SECS.to_string(),
        "--extractor-args".to_string(),
        client.to_string(),
        "-o".to_string(),
        format!("{}.%(ext)s", base.display()),
        "--".to_string(),
        watch_url.to_string(),
    ]
}

/// Nombre estable y seguro para un fichero de caché. Incluye un hash corto
/// para que dos claves con el mismo slug no colisionen.
fn cache_file_name(key: &str) -> String {
    let slug = util::slug(key, 64);
    let hash = util::short_hash(key);
    if slug.is_empty() {
        format!("stream-{hash}")
    } else {
        format!("{slug}-{hash}")
    }
}

/// Primer fichero con extensión de audio que haya dejado yt-dlp.
fn find_downloaded(base: &Path) -> Option<PathBuf> {
    DOWNLOAD_EXTENSIONS
        .iter()
        .map(|ext| base.with_extension(*ext))
        .find(|path| file_is_usable(path))
}

fn file_is_usable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

/// Limpia restos de intentos anteriores para no devolver un fichero viejo.
fn remove_download_artifacts(base: &Path) {
    for ext in DOWNLOAD_EXTENSIONS {
        let _ = std::fs::remove_file(base.with_extension(*ext));
        let _ = std::fs::remove_file(base.with_extension(format!("{ext}.part")));
    }
    let _ = std::fs::remove_file(base.with_extension("ytdl"));
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("spote").join("stream"))
        .unwrap_or_else(|| std::env::temp_dir().join("spote-stream"))
}

/// Binario configurable por entorno. `SPOTE_YTDLP` gana sobre
/// `SPOTE_YTDLP_PATH`; si ninguno sirve se usa el `yt-dlp` del `PATH`.
fn ytdlp_bin() -> String {
    for var in ["SPOTE_YTDLP", "SPOTE_YTDLP_PATH"] {
        if let Ok(value) = std::env::var(var) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    "yt-dlp".to_string()
}

/// Ejecuta yt-dlp y convierte timeout, cancelación, salida no cero y stderr en
/// errores accionables.
fn run_ytdlp(
    args: &[String],
    timeout: Duration,
    cancel: Option<&Cancel>,
) -> Result<CommandOutput> {
    let bin = ytdlp_bin();
    let output = run_with_timeout(&bin, args, timeout, cancel)?;

    if output.cancelled {
        return Err(cancelled_error());
    }
    if output.timed_out {
        return Err(failure(
            FailureKind::Timeout,
            format!("yt-dlp agotó los {} s", timeout.as_secs()),
        ));
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let kind = classify_stderr(&stderr);
        return Err(failure(kind, stderr_tail(&output.stderr)));
    }
    Ok(output)
}

/// Ejecuta un proceso con timeout/cancelación y captura stdout/stderr sin
/// bloquear los pipes. Si expira o lo sustituye otra pista se mata y se espera:
/// no quedan hijos zombie.
fn run_with_timeout(
    bin: &str,
    args: &[String],
    timeout: Duration,
    cancel: Option<&Cancel>,
) -> Result<CommandOutput> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("no se pudo ejecutar {bin}; instala yt-dlp o define SPOTE_YTDLP")
        })?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(anyhow!("{bin} no expone stdout"));
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(anyhow!("{bin} no expone stderr"));
    };
    let stdout_reader = thread::spawn(move || read_pipe(stdout));
    let stderr_reader = thread::spawn(move || read_pipe(stderr));

    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if cancel.is_some_and(Cancel::requested) {
                    cancelled = true;
                    let _ = child.kill();
                    break child
                        .wait()
                        .with_context(|| format!("esperando a {bin} tras cancelarlo"))?;
                }
                if started.elapsed() >= timeout {
                    timed_out = true;
                    let _ = child.kill();
                    break child
                        .wait()
                        .with_context(|| format!("esperando a {bin} tras matarlo"))?;
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("esperando a {bin}: {err}"));
            }
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    Ok(CommandOutput {
        status,
        stdout,
        stderr,
        timed_out,
        cancelled,
    })
}

struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    cancelled: bool,
}

fn read_pipe(mut pipe: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    bytes
}

/// Últimas líneas útiles de stderr, para no volcar un log entero al usuario.
fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return "sin salida de error".to_string();
    }
    lines[lines.len().saturating_sub(ERROR_LINES)..].join(" / ")
}

/// Tipos de fallo que la UI puede presentar tal cual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    AgeGated,
    GeoBlocked,
    Unavailable,
    NoMatch,
    Timeout,
    Other,
}

impl FailureKind {
    fn code(self) -> &'static str {
        match self {
            Self::AgeGated => "AgeGated",
            Self::GeoBlocked => "GeoBlocked",
            Self::Unavailable => "Unavailable",
            Self::NoMatch => "NoMatch",
            Self::Timeout => "Timeout",
            Self::Other => "Error",
        }
    }

    fn advice(self) -> &'static str {
        match self {
            Self::AgeGated => {
                "el vídeo pide verificación de edad; usa cookies de una cuenta verificada con --cookies-from-browser"
            }
            Self::GeoBlocked => {
                "el vídeo no está disponible en tu región; prueba una VPN o actualiza yt-dlp"
            }
            Self::Unavailable => {
                "el vídeo no está disponible (privado, borrado o sin formato de audio); actualiza yt-dlp"
            }
            Self::NoMatch => {
                "sin coincidencia fiable en YouTube; no se descarga un resultado dudoso"
            }
            Self::Timeout => "yt-dlp no terminó a tiempo",
            Self::Other => "yt-dlp falló; revisa el detalle y actualiza yt-dlp",
        }
    }
}

/// Clasifica el stderr de yt-dlp para que el mensaje diga qué hacer.
fn classify_stderr(stderr: &str) -> FailureKind {
    let lower = stderr.to_ascii_lowercase();

    const AGE: &[&str] = &[
        "sign in to confirm your age",
        "confirm your age",
        "age-restricted",
        "age restricted",
        "inappropriate for some users",
    ];
    if AGE.iter().any(|needle| lower.contains(*needle)) {
        return FailureKind::AgeGated;
    }

    const GEO: &[&str] = &[
        "not available in your country",
        "available in your country",
        "not made this video available",
        "not available in your region",
        "unavailable in your country",
        "unavailable in your region",
        "geo-restricted",
        "geo restricted",
        "blocked in your country",
        "blocked it in your country",
    ];
    if GEO.iter().any(|needle| lower.contains(*needle)) {
        return FailureKind::GeoBlocked;
    }

    if lower.contains("timed out") || lower.contains("timeout") {
        return FailureKind::Timeout;
    }

    const UNAVAILABLE: &[&str] = &[
        "video unavailable",
        "private video",
        "video is private",
        "members-only",
        "members only",
        "has been removed",
        "account associated with this video has been terminated",
        "requested format is not available",
        "no video formats found",
        "this video is not available",
        "confirm you're not a bot",
        "confirm you are not a bot",
        "unable to extract",
        "failed to extract",
    ];
    if UNAVAILABLE.iter().any(|needle| lower.contains(*needle)) {
        return FailureKind::Unavailable;
    }

    FailureKind::Other
}

/// Construye el mensaje final con etiqueta, consejo y detalle técnico.
fn failure_message(kind: FailureKind, detail: &str) -> String {
    let detail = detail.trim();
    let base = format!("{}: {}.", kind.code(), kind.advice());
    if detail.is_empty() {
        base
    } else {
        format!("{base} Detalle: {detail}")
    }
}

fn failure(kind: FailureKind, detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(failure_message(kind, &detail.to_string()))
}

/// Si fallan `-g` y la descarga, se conserva el error del stream (suele nombrar
/// el problema real: 403, edad, red) y se añade el de la descarga.
fn combined_failure(stream_error: anyhow::Error, download_error: anyhow::Error) -> anyhow::Error {
    anyhow!("{stream_error:#}; descarga de respaldo: {download_error:#}")
}

/// Guarda una URL y, de paso, poda las entradas que ya caducaron.
fn url_cache_store(cache: &UrlCache, key: String, source: StreamSource) {
    let now = Instant::now();
    let epoch = now_epoch_secs();
    if let Ok(mut map) = cache.lock() {
        map.retain(|_, entry| cache_is_live(&entry.0, entry.1, now, epoch));
        map.insert(key, (source, now));
    }
}

/// Busca una URL viva en la caché y elimina las caducadas.
fn url_cache_lookup(cache: &UrlCache, key: &str) -> Option<StreamSource> {
    let now = Instant::now();
    let epoch = now_epoch_secs();
    let mut map = cache.lock().ok()?;

    let live = match map.get(key) {
        Some((source, stored_at)) => cache_is_live(source, *stored_at, now, epoch),
        None => return None,
    };

    if live {
        map.get(key).map(|(source, _)| (*source).clone())
    } else {
        map.remove(key);
        None
    }
}

/// Devuelve un fichero de caché sólo si sigue existiendo y no está vacío.
///
/// Además del mapa en memoria se mira la ruta determinista del directorio de
/// caché: así un reinicio del proceso reaprovecha lo descargado antes.
fn file_cache_lookup(cache: &FileCache, cache_dir: &Path, key: &str) -> Option<CachedFile> {
    if let Ok(mut map) = cache.lock() {
        // `.cloned()` termina el préstamo antes de un posible `remove`.
        if let Some(cached) = map.get(key).cloned() {
            if file_is_usable(&cached.path) {
                return Some(cached);
            }
            map.remove(key);
        }
    }

    // Tras reiniciar no hay identidad en memoria; se deja vacía para que la UI
    // caiga al `source_hint` de la biblioteca.
    let path = find_downloaded(&cache_dir.join(cache_file_name(key)))?;
    let cached = CachedFile {
        path,
        provider: String::new(),
        id: String::new(),
    };
    if let Ok(mut map) = cache.lock() {
        map.insert(key.to_string(), cached.clone());
    }
    Some(cached)
}

/// Guarda un fichero descargado con su identidad y poda los que ya no existan.
fn file_cache_store(cache: &FileCache, key: String, path: PathBuf, provider: String, id: String) {
    if let Ok(mut map) = cache.lock() {
        map.retain(|_, cached| file_is_usable(cached.path.as_path()));
        map.insert(key, CachedFile { path, provider, id });
    }
}

/// Una entrada de URL vive como máximo 25 minutos y nunca se usa en los últimos
/// cinco minutos de vida de la URL. La condición de `expire` es absoluta: así
/// el TTL no se encoge artificialmente cada vez que se consulta la caché.
fn cache_is_live(source: &StreamSource, stored_at: Instant, now: Instant, now_epoch: u64) -> bool {
    if now.saturating_duration_since(stored_at) >= CACHE_TTL {
        return false;
    }
    match url_expire_epoch(&source.url) {
        Some(expire) => now_epoch.saturating_add(EXPIRY_MARGIN.as_secs()) < expire,
        None => true,
    }
}

/// Extrae el epoch de `expire=<epoch>` sin depender de un parser de URLs.
fn url_expire_epoch(url: &str) -> Option<u64> {
    let marker = "expire=";
    let start = url.find(marker)? + marker.len();
    let digits: String = url[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(url: &str) -> StreamSource {
        StreamSource {
            url: url.to_string(),
            provider: "youtube".to_string(),
            id: "dQw4w9WgXcQ".to_string(),
        }
    }

    fn candidate(id: &str, title: &str, channel: &str, duration_ms: u64) -> SearchCandidate {
        SearchCandidate {
            id: id.to_string(),
            title: title.to_string(),
            channel: channel.to_string(),
            duration_ms,
            is_live: false,
        }
    }

    #[test]
    fn accepts_only_eleven_character_video_ids() {
        assert!(is_valid_video_id("dQw4w9WgXcQ"));
        assert!(is_valid_video_id("_-aB3dE5gH1"));
        assert!(!is_valid_video_id(""));
        assert!(!is_valid_video_id("dQw4w9WgXc"));
        assert!(!is_valid_video_id("dQw4w9WgXcQx"));
        assert!(!is_valid_video_id("dQw4w9WgXc!"));
        assert!(!is_valid_video_id("dQw4w9WgXc€"));
        assert!(!is_valid_video_id("dQw4w9Wg cQ"));
    }

    #[test]
    fn builds_watch_url_from_valid_id() {
        assert_eq!(
            watch_url("dQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
    }

    #[test]
    fn is_youtube_provider_accepts_common_spellings() {
        assert!(is_youtube_provider("youtube"));
        assert!(is_youtube_provider("YouTube Music"));
        assert!(is_youtube_provider("ytmusic"));
        assert!(!is_youtube_provider("spotify"));
    }

    #[test]
    fn only_youtube_hints_with_valid_id_are_used() {
        let valid = SourceHint {
            provider: "ytmusic".to_string(),
            id: "dQw4w9WgXcQ".to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            hinted_video(Some(&valid)),
            Some(("dQw4w9WgXcQ".to_string(), "ytmusic".to_string()))
        );

        let wrong_provider = SourceHint {
            provider: "spotify".to_string(),
            id: "dQw4w9WgXcQ".to_string(),
            duration_ms: 0,
        };
        assert_eq!(hinted_video(Some(&wrong_provider)), None);

        let wrong_id = SourceHint {
            provider: "youtube".to_string(),
            id: "corto".to_string(),
            duration_ms: 0,
        };
        assert_eq!(hinted_video(Some(&wrong_id)), None);

        let padded = SourceHint {
            provider: " youtube ".to_string(),
            id: " dQw4w9WgXcQ ".to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            hinted_video(Some(&padded)),
            Some(("dQw4w9WgXcQ".to_string(), "youtube".to_string()))
        );
    }

    #[test]
    fn query_variants_are_ordered_and_deduplicated() {
        assert_eq!(
            query_variants("Daft Punk", "Get Lucky"),
            vec![
                "Daft Punk Get Lucky".to_string(),
                "Get Lucky Daft Punk".to_string(),
                "Get Lucky".to_string()
            ]
        );
        assert_eq!(
            query_variants("", "Get Lucky"),
            vec!["Get Lucky".to_string()]
        );
        assert_eq!(
            query_variants("Daft Punk", ""),
            vec!["Daft Punk".to_string()]
        );
        assert!(query_variants("", "").is_empty());
        // La variante repetida no se busca dos veces.
        assert_eq!(
            query_variants("Get Lucky", "Get Lucky"),
            vec!["Get Lucky Get Lucky".to_string(), "Get Lucky".to_string()]
        );
    }

    #[test]
    fn parses_search_json_and_skips_invalid_entries() {
        let json = r#"{
            "entries": [
                {
                    "id": "aaaaaaaaaaa",
                    "title": "Otra canción",
                    "channel": "Otro Artista",
                    "duration": 210
                },
                {
                    "id": "dQw4w9WgXcQ",
                    "title": "Rick Astley - Never Gonna Give You Up (Official Video)",
                    "channel": "Rick Astley",
                    "duration": 213
                },
                {
                    "id": "id inválido",
                    "title": "No debe entrar",
                    "duration": 10
                },
                {
                    "id": "bbbbbbbbbbb",
                    "title": "Get Lucky",
                    "channel": null,
                    "uploader": "Daft Punk",
                    "duration": "249.5"
                }
            ]
        }"#;

        let candidates = parse_search_json(json).expect("JSON válido");
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[1].id, "dQw4w9WgXcQ");
        assert_eq!(candidates[1].duration_ms, 213_000);
        assert_eq!(candidates[2].channel, "Daft Punk");
        assert_eq!(candidates[2].duration_ms, 249_500);

        let chosen = best_candidate(
            &candidates,
            "Rick Astley",
            "Never Gonna Give You Up",
            213_000,
        )
        .expect("hay candidatos");
        assert_eq!(chosen.id, "dQw4w9WgXcQ");
    }

    #[test]
    fn parses_live_status_from_json() {
        let json = r#"{"entries":[
            {"id":"aaaaaaaaaaa","title":"Get Lucky","channel":"Daft Punk","duration":249,"live_status":"is_live"},
            {"id":"bbbbbbbbbbb","title":"Get Lucky","channel":"Daft Punk","duration":249,"live_status":"not_live"}
        ]}"#;
        let candidates = parse_search_json(json).expect("JSON válido");
        assert!(candidates[0].is_live);
        assert!(!candidates[1].is_live);

        let json = r#"{"id":"ccccccccccc","title":"Get Lucky","is_live":true}"#;
        let candidates = parse_search_json(json).expect("JSON válido");
        assert!(candidates[0].is_live);
    }

    #[test]
    fn parses_json_lines_when_not_a_single_object() {
        let json = concat!(
            r#"{"id":"aaaaaaaaaaa","title":"Uno","channel":"Canal","duration":100}"#,
            "\n",
            "no es json\n",
            r#"{"id":"bbbbbbbbbbb","title":"Dos","uploader":"Otro","duration":null}"#,
            "\n",
        );
        let candidates = parse_search_json(json).expect("JSONL válido");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].channel, "Canal");
        assert_eq!(candidates[1].channel, "Otro");
        assert_eq!(candidates[1].duration_ms, 0);
    }

    #[test]
    fn title_matching_distinguishes_exact_and_loose() {
        assert_eq!(
            title_match_score("get lucky", "get lucky"),
            Some((60, true))
        );
        assert_eq!(
            title_match_score("get lucky", "daft punk get lucky"),
            Some((45, false))
        );
        assert_eq!(
            title_match_score("get lucky", "get lucky remix"),
            Some((45, false))
        );
        assert_eq!(title_match_score("get lucky", "one more time"), None);
        assert_eq!(title_match_score("go", "a"), None);
        assert_eq!(title_match_score("go", "go"), Some((60, true)));
        assert_eq!(title_match_score("", "get lucky"), None);
        assert_eq!(title_match_score("get lucky", ""), None);
    }

    #[test]
    fn duration_tolerances_are_strict_then_lax() {
        assert_eq!(duration_score(249_000, 250_000), Some(45));
        assert_eq!(duration_score(249_000, 251_000), Some(45));
        assert_eq!(duration_score(249_000, 257_000), Some(28));
        assert_eq!(duration_score(249_000, 258_000), Some(12));
        assert_eq!(duration_score(249_000, 279_000), Some(12));
        assert_eq!(duration_score(249_000, 296_000), Some(0));
        assert_eq!(duration_score(249_000, 370_000), None);
        assert_eq!(duration_score(0, 249_000), Some(-8));
        assert_eq!(duration_score(249_000, 0), Some(-8));
    }

    #[test]
    fn visualizer_with_long_intro_still_matches() {
        // Caso real: Deezer da 118 s y el visualizer de YouTube dura 165 s.
        let candidate = candidate(
            "jExWbhJqRU0",
            "superreservao - como alma en pena (visualizer)",
            "superreservao ",
            165_000,
        );
        let score = score_candidate(&candidate, "Superreservao", "Alma en pena", 118_000)
            .expect("el visualizer debe puntuar");
        assert!(score >= MIN_MATCH_SCORE, "score={score}");
    }

    #[test]
    #[ignore = "diagnóstico manual con red"]
    fn live_finds_superreservao_visualizer() {
        let id = find_video_id("Superreservao", "Alma en pena", 118_000)
            .expect("debe encontrar el visualizer");
        assert_eq!(id, "jExWbhJqRU0");
    }

    #[test]
    fn picks_topic_audio_over_a_cover() {
        let candidates = vec![
            candidate(
                "aaaaaaaaaaa",
                "Get Lucky (Cover)",
                "Some Cover Channel",
                249_000,
            ),
            candidate("bbbbbbbbbbb", "Get Lucky", "Daft Punk - Topic", 249_000),
        ];

        let chosen = best_candidate(&candidates, "Daft Punk", "Get Lucky", 249_000)
            .expect("hay un Topic válido");
        assert_eq!(chosen.id, "bbbbbbbbbbb");
    }

    #[test]
    fn matches_the_primary_artist_in_multi_artist_metadata() {
        let candidates = vec![candidate(
            "aaaaaaaaaaa",
            "Get Lucky",
            "Daft Punk - Topic",
            249_000,
        )];
        let chosen = best_candidate(
            &candidates,
            "Daft Punk, Pharrell Williams",
            "Get Lucky",
            249_000,
        )
        .expect("el artista primario debe bastar");
        assert_eq!(chosen.id, "aaaaaaaaaaa");
    }

    #[test]
    fn rejects_exact_title_from_a_different_artist() {
        let candidates = vec![candidate("aaaaaaaaaaa", "Get Lucky", "Random Guy", 249_000)];
        assert_eq!(
            best_candidate(&candidates, "Daft Punk", "Get Lucky", 249_000),
            None
        );
    }

    #[test]
    fn primary_artist_splits_credits_without_breaking_names() {
        assert_eq!(
            primary_artist("Daft Punk, Pharrell Williams"),
            Some("daft punk".to_string())
        );
        assert_eq!(
            primary_artist("Daft Punk feat. Pharrell"),
            Some("daft punk".to_string())
        );
        assert_eq!(
            primary_artist("Daft Punk & Pharrell"),
            Some("daft punk".to_string())
        );
        assert_eq!(primary_artist("Daft Punk"), None);
        assert_eq!(primary_artist("R&B"), None);
        assert_eq!(primary_artist("AC/DC"), None);
    }

    #[test]
    fn artist_matching_uses_tokens_and_channel_suffixes() {
        assert!(artist_matches("daft punk", "daft punk topic"));
        assert!(artist_matches("daft punk", "daftpunkvevo"));
        assert!(artist_matches("daft punk", "the daft punk official"));
        assert!(!artist_matches("queen", "queens of the stone age"));
        assert!(!artist_matches("", "daft punk"));
    }

    #[test]
    fn keeps_a_variant_when_the_requested_title_asks_for_it() {
        let live = candidate("aaaaaaaaaaa", "Get Lucky (Live)", "Daft Punk", 249_000);
        assert!(!has_unwanted_noise(&live, "Get Lucky (Live)", "Daft Punk"));
        assert!(has_unwanted_noise(&live, "Get Lucky", "Daft Punk"));

        let en_vivo = candidate("bbbbbbbbbbb", "Get Lucky (En Vivo)", "Daft Punk", 249_000);
        assert!(has_unwanted_noise(&en_vivo, "Get Lucky", "Daft Punk"));

        // Un directo marcado por yt-dlp se rechaza aunque el título no lo diga.
        let mut flagged = candidate("ccccccccccc", "Get Lucky", "Daft Punk - Topic", 249_000);
        flagged.is_live = true;
        assert!(has_unwanted_noise(&flagged, "Get Lucky", "Daft Punk"));
        assert!(!has_unwanted_noise(
            &flagged,
            "Get Lucky (Live)",
            "Daft Punk"
        ));
    }

    #[test]
    fn rejects_remixes_live_versions_and_karaoke() {
        let candidates = vec![
            candidate(
                "aaaaaaaaaaa",
                "Get Lucky (Daft Punk Remix)",
                "Daft Punk - Topic",
                249_000,
            ),
            candidate("bbbbbbbbbbb", "Get Lucky (Live)", "Daft Punk", 249_000),
            candidate(
                "ccccccccccc",
                "Get Lucky (Karaoke Version)",
                "Karaoke Hits",
                249_000,
            ),
        ];
        assert_eq!(
            best_candidate(&candidates, "Daft Punk", "Get Lucky", 249_000),
            None
        );
    }

    #[test]
    fn accepts_music_video_length_but_rejects_full_album() {
        // 11 s: intro/outro de videoclip, con título y artista exactos.
        let video = vec![candidate(
            "aaaaaaaaaaa",
            "Get Lucky",
            "Daft Punk - Topic",
            260_000,
        )];
        assert!(best_candidate(&video, "Daft Punk", "Get Lucky", 249_000).is_some());

        // 150 s: ya es otra cosa (álbum completo, directo largo...).
        let album = vec![candidate(
            "bbbbbbbbbbb",
            "Get Lucky",
            "Daft Punk - Topic",
            399_000,
        )];
        assert_eq!(
            best_candidate(&album, "Daft Punk", "Get Lucky", 249_000),
            None
        );
    }

    #[test]
    fn accepts_a_loose_duration_with_a_solid_title_and_artist() {
        // 5 s de diferencia: entra por la tolerancia laxa.
        let candidates = vec![candidate(
            "aaaaaaaaaaa",
            "Daft Punk - Get Lucky (Official Audio)",
            "Daft Punk",
            254_000,
        )];
        let chosen = best_candidate(&candidates, "Daft Punk", "Get Lucky", 249_000)
            .expect("el match laxo es aceptable");
        assert_eq!(chosen.id, "aaaaaaaaaaa");
    }

    #[test]
    fn skips_zero_duration_entries_when_there_are_alternatives() {
        let candidates = vec![
            candidate(
                "dQw4w9WgXcQ",
                "Never Gonna Give You Up",
                "Rick Astley - Topic",
                0,
            ),
            candidate(
                "aaaaaaaaaaa",
                "Never Gonna Give You Up (Radio Edit)",
                "Rick Astley",
                212_000,
            ),
        ];

        let chosen = best_candidate(
            &candidates,
            "Rick Astley",
            "Never Gonna Give You Up",
            212_000,
        )
        .expect("hay candidatos");
        assert_eq!(chosen.id, "aaaaaaaaaaa");
    }

    #[test]
    fn vevo_channel_beats_an_anonymous_reupload() {
        let candidates = vec![
            candidate("aaaaaaaaaaa", "Get Lucky", "Daft Punk", 249_000),
            candidate("bbbbbbbbbbb", "Get Lucky", "DaftPunkVEVO", 249_000),
        ];
        let chosen =
            best_candidate(&candidates, "Daft Punk", "Get Lucky", 249_000).expect("hay candidatos");
        assert_eq!(chosen.id, "bbbbbbbbbbb");
    }

    #[test]
    fn normalizes_case_accents_feat_and_noise_suffixes() {
        assert_eq!(normalize_text("Canción (Remastered)"), "cancion");
        assert_eq!(normalize_text("  Ñandú   &  Tú  "), "nandu and tu");
        assert_eq!(normalize_text("Get Lucky feat. Pharrell"), "get lucky");
        assert_eq!(normalize_text("Get Lucky ft. Pharrell"), "get lucky");
        assert_eq!(normalize_title("Get Lucky (Official Audio)"), "get lucky");
        assert_eq!(normalize_title("Get Lucky Official Audio HD"), "get lucky");
        assert_eq!(normalize_title("Canción (Official Video)"), "cancion");
    }

    #[test]
    fn keyword_text_keeps_parenthesised_variants() {
        assert_eq!(keyword_text("Get Lucky (Cover)"), "get lucky cover");
        assert!(contains_phrase(&keyword_text("Get Lucky (Cover)"), "cover"));
        // "discover" no debe confundirse con "cover".
        assert!(!contains_phrase(&keyword_text("Discover"), "cover"));
        assert!(contains_phrase(&keyword_text("Sped Up Version"), "sped up"));
    }

    #[test]
    fn classifies_actionable_ytdlp_errors() {
        assert_eq!(
            classify_stderr("ERROR: Sign in to confirm your age"),
            FailureKind::AgeGated
        );
        assert_eq!(
            classify_stderr(
                "ERROR: The uploader has not made this video available in your country"
            ),
            FailureKind::GeoBlocked
        );
        assert_eq!(
            classify_stderr("ERROR: Video unavailable"),
            FailureKind::Unavailable
        );
        assert_eq!(
            classify_stderr("ERROR: This video is private"),
            FailureKind::Unavailable
        );
        assert_eq!(
            classify_stderr("ERROR: The read operation timed out"),
            FailureKind::Timeout
        );
        assert_eq!(classify_stderr("ERROR: something else"), FailureKind::Other);
    }

    #[test]
    fn failure_messages_name_the_error_kind_and_advice() {
        let message = failure_message(FailureKind::NoMatch, "«get lucky»");
        assert!(message.starts_with("NoMatch:"), "{message}");
        assert!(message.contains("«get lucky»"), "{message}");

        let message = failure_message(FailureKind::AgeGated, "");
        assert!(message.starts_with("AgeGated:"), "{message}");
        assert!(message.contains("cookies"), "{message}");
        assert!(!message.contains("Detalle:"), "{message}");

        for kind in [
            FailureKind::AgeGated,
            FailureKind::GeoBlocked,
            FailureKind::Unavailable,
            FailureKind::NoMatch,
            FailureKind::Timeout,
            FailureKind::Other,
        ] {
            let message = failure_message(kind, "detalle");
            assert!(message.starts_with(kind.code()), "{message}");
            assert!(message.contains("detalle"), "{message}");
        }
    }

    #[test]
    fn stream_args_validate_ids_and_terminate_options() {
        let watch = watch_url("dQw4w9WgXcQ");
        let args = stream_args(&watch, PLAYER_CLIENTS[0]);
        assert!(args.contains(&"-g".to_string()));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0].as_str() == "-f" && pair[1].as_str() == "bestaudio/best" }));
        assert_eq!(args.last().map(String::as_str), Some(watch.as_str()));
        assert_eq!(args[args.len() - 2].as_str(), "--");
        assert!(args.windows(2).any(|pair| {
            pair[0].as_str() == "--extractor-args" && pair[1].as_str() == PLAYER_CLIENTS[0]
        }));
    }

    #[test]
    fn download_args_never_transcode_and_terminate_options() {
        let watch = watch_url("dQw4w9WgXcQ");
        let base = Path::new("/tmp/spote-stream/abc");
        let args = download_args(base, &watch, PLAYER_CLIENTS[1]);

        assert!(args.contains(&"bestaudio/best".to_string()));
        assert!(args.contains(&"--no-playlist".to_string()));
        assert!(!args.iter().any(|arg| arg.as_str() == "-x"));
        assert!(!args.iter().any(|arg| arg.as_str() == "--audio-format"));
        assert!(!args.iter().any(|arg| arg.as_str() == "--no-part"));
        assert_eq!(args.last().map(String::as_str), Some(watch.as_str()));
        assert_eq!(args[args.len() - 2].as_str(), "--");
        assert!(args.windows(2).any(|pair| {
            pair[0].as_str() == "--extractor-args" && pair[1].as_str() == PLAYER_CLIENTS[1]
        }));
        assert!(args.iter().any(|arg| arg.as_str().ends_with(".%(ext)s")));
    }

    #[test]
    fn cache_file_names_are_stable_and_path_safe() {
        let first = cache_file_name("spotify:track:3zOYmiNOuyDL0ClxJcEJ0T");
        let second = cache_file_name("spotify:track:3zOYmiNOuyDL0ClxJcEJ0T");
        assert_eq!(first, second);
        assert!(first.starts_with("spotify-track-3zoyminouydl0clxjcej0t-"));
        assert!(!first.contains('/'));
        assert!(!first.contains('\\'));
        assert!(!first.contains(".."));
        assert_ne!(
            first,
            cache_file_name("spotify:track:2yg9UN4eo5eMVJ7OB4RWj3")
        );
        assert!(cache_file_name("").starts_with("stream-"));
    }

    #[test]
    fn parses_first_https_line_after_garbage() {
        let stdout = "WARNING: [youtube] fallback\n\
                      https://rr3---sn-abcd.googlevideo.com/videoplayback?expire=123&id=xyz\n";
        let url = parse_media_url(stdout).expect("URL permitida");
        assert!(url.starts_with("https://rr3---sn-abcd.googlevideo.com/videoplayback"));
    }

    #[test]
    fn rejects_hosts_that_are_not_allowlisted() {
        assert!(parse_media_url("https://example.com/audio\n").is_err());
        assert!(parse_media_url("https://googlevideo.com.evil.com/audio\n").is_err());
        assert!(parse_media_url("https://notyoutube.com/audio\n").is_err());
        assert!(parse_media_url("http://rr1---sn.googlevideo.com/audio\n").is_err());
        // La primera línea https manda: no se salta a una segunda URL válida.
        assert!(parse_media_url(
            "https://example.com/audio\nhttps://rr1---sn.googlevideo.com/audio\n"
        )
        .is_err());
    }

    #[test]
    fn accepts_allowlisted_hosts_and_subdomains() {
        assert!(parse_media_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ\n").is_ok());
        assert!(parse_media_url("https://i.ytimg.com/vi/x/hq.jpg\n").is_ok());
        assert!(parse_media_url("https://rr1---sn-abcd.googlevideo.com/videoplayback\n").is_ok());
    }

    #[test]
    fn default_cache_ttl_is_twenty_five_minutes() {
        let source = source("https://rr1---sn-abcd.googlevideo.com/videoplayback");
        let now = Instant::now();
        let epoch = 1_000;

        assert!(cache_is_live(
            &source,
            now.checked_sub(Duration::from_secs(24 * 60)).unwrap(),
            now,
            epoch
        ));
        assert!(!cache_is_live(
            &source,
            now.checked_sub(Duration::from_secs(26 * 60)).unwrap(),
            now,
            epoch
        ));
    }

    #[test]
    fn expire_deadline_is_respected_with_margin() {
        let source = source("https://rr1---sn-abcd.googlevideo.com/videoplayback?expire=2000&id=x");
        let now = Instant::now();

        // 1600 + 300 < 2000: aún fuera del margen de cinco minutos.
        assert!(cache_is_live(&source, now, now, 1_600));
        // 1701 + 300 = 2001: la URL entra en el margen y deja de usarse.
        assert!(!cache_is_live(&source, now, now, 1_701));
    }

    #[test]
    fn url_with_less_than_margin_left_is_not_cached() {
        let source = source("https://rr1---sn-abcd.googlevideo.com/videoplayback?expire=1800&id=x");
        let now = Instant::now();

        assert!(!cache_is_live(&source, now, now, 1_600));
    }

    #[test]
    fn parses_expire_from_url() {
        assert_eq!(
            url_expire_epoch("https://x.googlevideo.com/a?expire=1700000000&id=1"),
            Some(1_700_000_000)
        );
        assert_eq!(url_expire_epoch("https://x.googlevideo.com/a?id=1"), None);
        assert_eq!(
            url_expire_epoch("https://x.googlevideo.com/a?expire=abc"),
            None
        );
    }

    #[test]
    fn cancel_token_only_fires_when_generation_changes() {
        let generation = Arc::new(AtomicU64::new(0));
        let token = Cancel::new(&generation, None);
        assert!(!token.requested());
        generation.fetch_add(1, Ordering::Relaxed);
        assert!(token.requested());
    }

    #[test]
    fn cancel_kills_the_child_promptly() {
        let generation = Arc::new(AtomicU64::new(0));
        let cancel = Cancel::new(&generation, None);
        let bump = Arc::clone(&generation);
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            bump.fetch_add(1, Ordering::Relaxed);
        });

        let started = Instant::now();
        let output = run_with_timeout(
            "sleep",
            &["5".to_string()],
            Duration::from_secs(10),
            Some(&cancel),
        )
        .expect("debe devolver la salida del hijo cancelado");
        trigger.join().unwrap();

        assert!(output.cancelled);
        assert!(!output.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "la cancelación debe ser inmediata, no esperar al timeout"
        );
    }

    #[test]
    fn downloaded_events_are_comparable_and_carry_the_cache_path() {
        let event = StreamEvent::Downloaded {
            key: "sha1:abc".to_string(),
            path: PathBuf::from("/tmp/spote-stream/sha1-abc-1234.m4a"),
            provider: "ytmusic".to_string(),
            id: "dQw4w9WgXcQ".to_string(),
        };
        assert_eq!(event.clone(), event);
        match event {
            StreamEvent::Downloaded {
                key,
                path,
                provider,
                id,
            } => {
                assert_eq!(key, "sha1:abc");
                assert_eq!(path, PathBuf::from("/tmp/spote-stream/sha1-abc-1234.m4a"));
                assert_eq!(provider, "ytmusic");
                assert_eq!(id, "dQw4w9WgXcQ");
            }
            _ => panic!("debe ser Downloaded"),
        }
    }
}
