//! Resolución de URLs de reproducción de YouTube con `yt-dlp`.
//!
//! Un único worker consume trabajos de una cola `mpsc` y publica eventos por
//! otra. La caché es compartida con el worker y evita volver a resolver la misma
//! pista mientras la URL siga siendo válida. El módulo no abre conexiones por su
//! cuenta: todo el trabajo de red lo hace el binario externo.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use spote_protocol::SourceHint;

/// Tiempo máximo para una búsqueda o una resolución.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
/// Caducidad normal de una URL de reproducción.
const CACHE_TTL: Duration = Duration::from_secs(25 * 60);
/// Margen que se descuenta a `expire=<epoch>` antes de reutilizar una URL.
const EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Cadencia del sondeo de `Child::try_wait`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Resultados que se piden a la búsqueda de YouTube.
const SEARCH_RESULTS: usize = 8;
/// Diferencia de duración que se considera aceptable al puntuar candidatos.
const DURATION_TOLERANCE_MS: u64 = 8_000;
/// Líneas de stderr que se conservan en un mensaje de error.
const ERROR_LINES: usize = 4;

/// Hosts desde los que se acepta una URL de medios.
const ALLOWED_HOSTS: &[&str] = &["googlevideo.com", "youtube.com", "ytimg.com"];

/// Fuente reproducible ya resuelta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSource {
    pub url: String,
    pub provider: String,
    pub id: String,
}

/// Eventos que el worker envía a la UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Resolved { key: String, source: StreamSource },
    Failed { key: String, message: String },
}

/// Trabajo pendiente en la cola del worker.
#[derive(Debug, Clone)]
struct Job {
    key: String,
    artist: String,
    title: String,
    duration_ms: u64,
    hint: Option<SourceHint>,
}

/// Caché compartida: clave lógica -> fuente y momento en que se guardó.
type SharedCache = Arc<Mutex<HashMap<String, (StreamSource, Instant)>>>;

/// Cola de resolución en un hilo propio.
pub struct StreamResolver {
    tx: Sender<Job>,
    rx: Receiver<StreamEvent>,
    /// Se conserva para que el `Arc` viva mientras exista el resolver; el
    /// worker tiene su propia copia.
    _cache: SharedCache,
}

impl StreamResolver {
    /// Arranca el worker. Si el hilo no se puede crear, el error es fatal:
    /// sin worker la API no tendría sentido.
    pub fn start() -> Self {
        let (job_tx, job_rx): (Sender<Job>, Receiver<Job>) = mpsc::channel();
        let (event_tx, event_rx): (Sender<StreamEvent>, Receiver<StreamEvent>) = mpsc::channel();
        let cache: SharedCache = Arc::new(Mutex::new(HashMap::new()));
        let worker_cache = Arc::clone(&cache);

        thread::Builder::new()
            .name("spote-stream".into())
            .spawn(move || worker(job_rx, event_tx, worker_cache))
            .expect("no se pudo arrancar el resolver de streams");

        Self {
            tx: job_tx,
            rx: event_rx,
            _cache: cache,
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
        let _ = self.tx.send(Job {
            key: key.into(),
            artist: artist.to_string(),
            title: title.to_string(),
            duration_ms,
            hint,
        });
    }

    /// Lee un evento sin bloquear.
    pub fn try_recv(&self) -> Option<StreamEvent> {
        self.rx.try_recv().ok()
    }
}

/// Bucle del worker: cada trabajo produce exactamente un evento. Los errores
/// se convierten en `Failed` y nunca terminan el hilo.
fn worker(jobs: Receiver<Job>, events: Sender<StreamEvent>, cache: SharedCache) {
    while let Ok(job) = jobs.recv() {
        if let Some(source) = cache_lookup(&cache, &job.key) {
            let _ = events.send(StreamEvent::Resolved {
                key: job.key,
                source,
            });
            continue;
        }

        match resolve_job(&job) {
            Ok(source) => {
                cache_store(&cache, job.key.clone(), source.clone());
                let _ = events.send(StreamEvent::Resolved {
                    key: job.key,
                    source,
                });
            }
            Err(err) => {
                let _ = events.send(StreamEvent::Failed {
                    key: job.key,
                    message: format!("{err:#}"),
                });
            }
        }
    }
}

/// Resuelve un trabajo. El `hint` sólo se usa si apunta a YouTube y trae un ID
/// con forma de ID de vídeo; en cualquier otro caso se busca por texto.
fn resolve_job(job: &Job) -> Result<StreamSource> {
    let (id, provider) = match hinted_video(job.hint.as_ref()) {
        Some((id, provider)) => (id, provider),
        None => {
            let candidate = search(&job.artist, &job.title, job.duration_ms)?;
            (candidate.id, "youtube".to_string())
        }
    };

    // La URL de watch se construye siempre desde un ID ya validado.
    let url = resolve_media_url(&watch_url(&id))?;
    Ok(StreamSource { url, provider, id })
}

/// Devuelve `(id, provider)` sólo cuando el hint es utilizable.
fn hinted_video(hint: Option<&SourceHint>) -> Option<(String, String)> {
    let hint = hint?;
    if !is_youtube_provider(&hint.provider) || !is_valid_video_id(&hint.id) {
        return None;
    }
    Some((hint.id.clone(), hint.provider.clone()))
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
    title: String,
    artist: String,
    duration_ms: u64,
}

/// Busca en YouTube y elige el mejor candidato por título, artista y duración.
fn search(artist: &str, title: &str, duration_ms: u64) -> Result<SearchCandidate> {
    let query = format!("{} {}", artist.trim(), title.trim());
    let query = query.trim();
    if query.is_empty() {
        return Err(anyhow!("no hay texto para buscar en YouTube"));
    }

    let bin = ytdlp_bin();
    let args = vec![
        "--flat-playlist".to_string(),
        "--dump-single-json".to_string(),
        "--no-warnings".to_string(),
        format!("ytsearch{SEARCH_RESULTS}:{query}"),
    ];
    let output = run_with_timeout(&bin, &args, COMMAND_TIMEOUT)
        .with_context(|| format!("buscando «{query}»"))?;

    if output.timed_out {
        return Err(anyhow!(
            "la búsqueda en YouTube superó los {} s: {}",
            COMMAND_TIMEOUT.as_secs(),
            stderr_tail(&output.stderr)
        ));
    }
    if !output.status.success() {
        return Err(anyhow!("yt-dlp: {}", stderr_tail(&output.stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let candidates =
        parse_search_json(&stdout).context("respuesta de búsqueda de YouTube ilegible")?;
    best_candidate(&candidates, artist, title, duration_ms)
        .ok_or_else(|| anyhow!("YouTube no devolvió resultados para «{query}»"))
}

/// Extrae los candidatos válidos de la salida de `--dump-single-json`.
fn parse_search_json(json: &str) -> Result<Vec<SearchCandidate>> {
    let value: Value = serde_json::from_str(json)?;
    let Some(entries) = value.get("entries").and_then(|entries| entries.as_array()) else {
        return Ok(Vec::new());
    };

    let mut candidates = Vec::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !is_valid_video_id(id) {
            continue;
        }

        let title = entry
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim();
        let artist = entry
            .get("channel")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| entry.get("uploader").and_then(|value| value.as_str()))
            .unwrap_or_default()
            .trim();
        let duration_ms = entry
            .get("duration")
            .and_then(|value| value.as_f64())
            .map(|seconds| (seconds.max(0.0) * 1000.0).round() as u64)
            .unwrap_or(0);

        candidates.push(SearchCandidate {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            duration_ms,
        });
    }
    Ok(candidates)
}

/// Elige el candidato con más puntos. Si hay alguno con duración conocida, los
/// de duración 0 se descartan: suelen ser directos o resultados incompletos.
fn best_candidate(
    candidates: &[SearchCandidate],
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> Option<SearchCandidate> {
    if candidates.is_empty() {
        return None;
    }

    let has_duration = candidates.iter().any(|candidate| candidate.duration_ms > 0);
    let mut best: Option<(i64, &SearchCandidate)> = None;

    for candidate in candidates {
        if has_duration && candidate.duration_ms == 0 {
            continue;
        }
        let score = score_candidate(candidate, artist, title, duration_ms);
        let better = best
            .as_ref()
            .map_or(true, |(best_score, _)| score > *best_score);
        if better {
            best = Some((score, candidate));
        }
    }

    best.map(|(_, candidate)| candidate.clone())
}

/// Puntuación sencilla y determinista. No pretende ser un matcher semántico:
/// sólo ordenar resultados que ya vienen de una búsqueda acotada.
fn score_candidate(
    candidate: &SearchCandidate,
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> i64 {
    let wanted_title = normalize_text(title);
    let wanted_artist = normalize_text(artist);
    let candidate_title = normalize_text(&candidate.title);
    let candidate_artist = normalize_text(&candidate.artist);
    let combined = normalize_text(&format!("{} {}", candidate.title, candidate.artist));

    let mut score = 0i64;

    if !wanted_title.is_empty() {
        if candidate_title == wanted_title {
            score += 120;
        } else if candidate_title.contains(&wanted_title)
            || (!candidate_title.is_empty() && wanted_title.contains(&candidate_title))
        {
            score += 80;
        } else {
            score += token_overlap(&wanted_title, &candidate_title) as i64 * 50 / 100;
        }
        if combined.contains(&wanted_title) {
            score += 30;
        }
    }

    if !wanted_artist.is_empty() {
        if candidate_artist == wanted_artist {
            score += 60;
        } else if candidate_artist.contains(&wanted_artist)
            || (!candidate_artist.is_empty() && wanted_artist.contains(&candidate_artist))
        {
            score += 45;
        } else if combined.contains(&wanted_artist) {
            score += 35;
        } else {
            score += token_overlap(&wanted_artist, &candidate_artist) as i64 * 30 / 100;
        }
    }

    if duration_ms > 0 && candidate.duration_ms > 0 {
        let diff = duration_ms.abs_diff(candidate.duration_ms);
        if diff <= DURATION_TOLERANCE_MS {
            score += 50;
        } else {
            let seconds_over = (diff - DURATION_TOLERANCE_MS) / 1000;
            score -= seconds_over.min(120) as i64;
        }
    }

    score
}

/// Porcentaje de tokens de `wanted` presentes en `candidate`.
fn token_overlap(wanted: &str, candidate: &str) -> usize {
    let wanted_tokens: Vec<&str> = wanted.split_whitespace().collect();
    if wanted_tokens.is_empty() {
        return 0;
    }
    let candidate_tokens: Vec<&str> = candidate.split_whitespace().collect();
    let hits = wanted_tokens
        .iter()
        .filter(|token| candidate_tokens.contains(token))
        .count();
    hits * 100 / wanted_tokens.len()
}

/// Minúsculas, sin acentos básicos, sin paréntesis y con espacios colapsados.
fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;

    for raw in text.chars() {
        match raw {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            _ => {
                let lower = raw.to_lowercase().next().unwrap_or(raw);
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

    out.split_whitespace().collect::<Vec<_>>().join(" ")
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
fn resolve_media_url(watch_url: &str) -> Result<String> {
    let bin = ytdlp_bin();
    let clients = [
        "youtube:player_client=web_embedded,tv_embedded",
        "youtube:player_client=default,web_safari",
    ];
    let mut last_error: Option<anyhow::Error> = None;

    for client in clients {
        let args = vec![
            "-g".to_string(),
            "-f".to_string(),
            "bestaudio/best".to_string(),
            "--no-playlist".to_string(),
            "--no-warnings".to_string(),
            "--socket-timeout".to_string(),
            "15".to_string(),
            "--extractor-args".to_string(),
            client.to_string(),
            watch_url.to_string(),
        ];

        let result = run_with_timeout(&bin, &args, COMMAND_TIMEOUT).and_then(|output| {
            if output.timed_out {
                return Err(anyhow!(
                    "yt-dlp agotó los {} s resolviendo la URL: {}",
                    COMMAND_TIMEOUT.as_secs(),
                    stderr_tail(&output.stderr)
                ));
            }
            if !output.status.success() {
                return Err(anyhow!("yt-dlp: {}", stderr_tail(&output.stderr)));
            }
            parse_media_url(&String::from_utf8_lossy(&output.stdout))
        });

        match result {
            Ok(url) => return Ok(url),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("yt-dlp no devolvió una URL reproducible")))
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
            return Err(anyhow!(
                "yt-dlp devolvió una URL fuera de los hosts permitidos"
            ));
        }
    }
    Err(anyhow!("yt-dlp no devolvió ninguna URL https"))
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

    ALLOWED_HOSTS
        .iter()
        .any(|allowed| host.as_str() == *allowed || host.ends_with(&format!(".{allowed}")))
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

/// Ejecuta un proceso con timeout y captura stdout/stderr sin bloquear los
/// pipes. Si expira se mata y se espera: no quedan hijos zombie.
fn run_with_timeout(bin: &str, args: &[String], timeout: Duration) -> Result<CommandOutput> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("no se pudo ejecutar {bin}"))?;

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
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
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
    })
}

struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
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

/// Guarda una fuente y, de paso, poda las entradas que ya caducaron.
fn cache_store(cache: &SharedCache, key: String, source: StreamSource) {
    let now = Instant::now();
    let epoch = now_epoch_secs();
    if let Ok(mut map) = cache.lock() {
        map.retain(|_, entry| cache_is_live(&entry.0, entry.1, now, epoch));
        map.insert(key, (source, now));
    }
}

/// Busca una entrada viva en la caché y elimina las caducadas.
fn cache_lookup(cache: &SharedCache, key: &str) -> Option<StreamSource> {
    let now = Instant::now();
    let epoch = now_epoch_secs();
    let mut map = cache.lock().ok()?;

    let live = match map.get(key) {
        Some((source, stored_at)) => cache_is_live(source, *stored_at, now, epoch),
        None => return None,
    };

    if live {
        map.get(key).map(|(source, _)| source.clone())
    } else {
        map.remove(key);
        None
    }
}

/// Una entrada vive como máximo 25 minutos y nunca se usa en los últimos cinco
/// minutos de vida de la URL. La condición de `expire` es absoluta: así el TTL
/// no se encoge artificialmente cada vez que se consulta la caché.
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
    }

    #[test]
    fn parses_search_json_and_picks_the_matching_entry() {
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
                }
            ]
        }"#;

        let candidates = parse_search_json(json).expect("JSON válido");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[1].id, "dQw4w9WgXcQ");
        assert_eq!(candidates[1].duration_ms, 213_000);

        let chosen = best_candidate(
            &candidates,
            "Rick Astley",
            "Never Gonna Give You Up",
            212_000,
        )
        .expect("hay candidatos");
        assert_eq!(chosen.id, "dQw4w9WgXcQ");
    }

    #[test]
    fn skips_zero_duration_entries_when_there_are_alternatives() {
        let candidates = vec![
            SearchCandidate {
                id: "dQw4w9WgXcQ".to_string(),
                title: "Never Gonna Give You Up".to_string(),
                artist: "Rick Astley".to_string(),
                duration_ms: 0,
            },
            SearchCandidate {
                id: "aaaaaaaaaaa".to_string(),
                title: "Never Gonna Give You Up (cover)".to_string(),
                artist: "Rick Astley".to_string(),
                duration_ms: 212_000,
            },
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
    fn normalizes_case_accents_and_parentheses() {
        assert_eq!(normalize_text("Canción (Remastered)"), "cancion");
        assert_eq!(normalize_text("  Ñandú   &  Tú  "), "nandu and tu");
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
}
