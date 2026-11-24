//! Adquisición: búsqueda y descarga con yt-dlp, e ingest de ficheros locales.
//!
//! yt-dlp es un binario externo a propósito: rompe cada pocas semanas y se
//! actualiza sin recompilar spote. Un solo worker evita los rate limits.

use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use spote_protocol::TrackRef;

use crate::library::{Library, SourceStatus};
use crate::stream::ytdlp_bin;
use crate::util;

#[derive(Debug, Clone)]
pub struct SearchItem {
    pub provider_id: String,
    pub provider: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub channel: String,
    pub url: String,
    pub art_url: Option<String>,
    pub isrc: Option<String>,
}

impl SearchItem {
    pub fn to_track(&self) -> TrackRef {
        let key = self
            .isrc
            .as_ref()
            .map(|isrc| format!("isrc:{}", isrc.to_lowercase()))
            .unwrap_or_else(|| util::fallback_key(&self.artist, &self.title, self.duration_ms));
        TrackRef {
            key,
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            art_url: self.art_url.clone(),
            source_hint: Some(spote_protocol::SourceHint {
                provider: self.provider.clone(),
                id: self.provider_id.clone(),
                duration_ms: self.duration_ms,
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Found { query_id: u64, items: Vec<SearchItem> },
    SearchFailed { query_id: u64, message: String },
    /// Progreso "clásico": el paso ya viene formateado para pintarlo tal cual.
    Progress { key: String, percent: f32, stage: String },
    /// El mismo progreso pero con el paso tipado: la UI puede elegir icono o color.
    StageProgress {
        key: String,
        stage: DownloadStage,
        percent: f32,
    },
    Done { key: String, path: PathBuf },
    Failed { key: String, message: String },
    Message(String),
}

/// Pasos del pipeline de `yt-dlp`. `label()` es el texto que ve el usuario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Resolving,
    Downloading,
    Converting,
    Cover,
    Lyrics,
    Done,
}

impl DownloadStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolving => "buscando",
            Self::Downloading => "descargando",
            Self::Converting => "convirtiendo",
            Self::Cover => "portada",
            Self::Lyrics => "letras",
            Self::Done => "lista",
        }
    }
}

impl std::fmt::Display for DownloadStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Manda el par de eventos (formateado + tipado) que la TUI ya sabe consumir.
fn emit_progress(events: &Sender<DownloadEvent>, key: &str, stage: DownloadStage, percent: f32) {
    let percent = percent.clamp(0.0, 100.0);
    let _ = events.send(DownloadEvent::Progress {
        key: key.to_string(),
        percent,
        stage: stage.label().to_string(),
    });
    let _ = events.send(DownloadEvent::StageProgress {
        key: key.to_string(),
        stage,
        percent,
    });
}

/// Filtra el ruido del pipe: sólo deja pasar cambios de paso o de 0.5% reales.
struct ProgressReporter {
    events: Sender<DownloadEvent>,
    key: String,
    stage: Option<DownloadStage>,
    percent: f32,
}

impl ProgressReporter {
    fn new(events: &Sender<DownloadEvent>, key: &str) -> Self {
        Self {
            events: events.clone(),
            key: key.to_string(),
            stage: None,
            percent: 0.0,
        }
    }

    /// Porcentaje del último evento: sirve para pasos que no tienen progreso propio.
    fn percent(&self) -> f32 {
        self.percent
    }

    fn report(&mut self, stage: DownloadStage, percent: f32) {
        let percent = percent.clamp(0.0, 100.0);
        let stepped = (percent - self.percent).abs() >= 0.5;
        if self.stage == Some(stage) && !stepped {
            return;
        }
        self.stage = Some(stage);
        self.percent = percent;
        emit_progress(&self.events, &self.key, stage, percent);
    }

    /// Progreso de descarga normal. Si llega al 100%, el siguiente paso del
    /// pipeline es ffmpeg (`-x --audio-format opus`).
    fn report_download(&mut self, percent: f32) {
        // yt-dlp repite la última línea de descarga al cerrar: no volver atrás.
        if self.stage == Some(DownloadStage::Converting) {
            return;
        }
        self.report(DownloadStage::Downloading, percent);
        if percent >= 99.5 {
            self.report(DownloadStage::Converting, percent);
        }
    }
}

#[derive(Debug)]
enum Job {
    Search { id: u64, query: String },
    Download { key: String, source: DownloadSource, out: PathBuf },
    /// Busca la mejor fuente para una pista del catálogo y la descarga.
    Resolve {
        key: String,
        artist: String,
        title: String,
        duration_ms: u64,
        out: PathBuf,
    },
    Update,
    Quit,
}

#[derive(Debug, Clone)]
pub enum DownloadSource {
    /// URL directa o `ytsearch1:consulta`.
    Url(String),
}

/// Cola de descargas en un hilo propio. La UI manda trabajos y lee eventos.
pub struct Downloader {
    tx: Sender<Job>,
    events: Receiver<DownloadEvent>,
    children: Arc<Mutex<Vec<u32>>>,
    /// Claves con una descarga directa aceptada y todavía sin terminar. Evita
    /// descargar dos veces la misma pista (stream cache + descarga explícita).
    /// Los `Resolve` no se deduplican: son el fallback si la fuente falló.
    inflight: Arc<Mutex<HashSet<String>>>,
}

impl Downloader {
    pub fn start() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let children: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&children);
        let inflight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let worker_inflight = Arc::clone(&inflight);
        std::thread::Builder::new()
            .name("spote-downloader".into())
            .spawn(move || worker(job_rx, event_tx, shared, worker_inflight))
            .expect("no se pudo arrancar el downloader");
        Self {
            tx: job_tx,
            events: event_rx,
            children,
            inflight,
        }
    }

    pub fn search(&self, id: u64, query: impl Into<String>) {
        let _ = self.tx.send(Job::Search {
            id,
            query: query.into(),
        });
    }

    pub fn download(&self, key: &str, source: DownloadSource, out: PathBuf) {
        let key = key.to_string();
        if !begin_job(&self.inflight, &key) {
            return;
        }
        if self
            .tx
            .send(Job::Download {
                key: key.clone(),
                source,
                out,
            })
            .is_err()
        {
            finish_job(&self.inflight, &key);
        }
    }

    /// Resuelve contra yt-dlp con matching estricto y descarga la mejor fuente.
    /// Es lo que usa la importación de catálogo: una pista puede no tener URL
    /// todavía, sólo artista/título/duración.
    pub fn resolve_and_download(
        &self,
        key: &str,
        artist: &str,
        title: &str,
        duration_ms: u64,
        out: PathBuf,
    ) {
        // Un `Resolve` busca una fuente nueva y no se deduplica: si la copia
        // de caché de un stream está en vuelo y mpv falla, este trabajo es el
        // fallback que debe ejecutarse igualmente.
        let _ = self.tx.send(Job::Resolve {
            key: key.to_string(),
            artist: artist.to_string(),
            title: title.to_string(),
            duration_ms,
            out,
        });
    }

    pub fn update_ytdlp(&self) {
        let _ = self.tx.send(Job::Update);
    }

    pub fn try_recv(&self) -> Option<DownloadEvent> {
        self.events.try_recv().ok()
    }

    pub fn drain(&self) -> Vec<DownloadEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    /// Mata todos los yt-dlp vivos (salida limpia o cancelación).
    pub fn kill_all(&self) {
        if let Ok(children) = self.children.lock() {
            for pid in children.iter() {
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status();
            }
        }
    }
}

impl Drop for Downloader {
    fn drop(&mut self) {
        self.kill_all();
        let _ = self.tx.send(Job::Quit);
    }
}

/// Marca una clave como descargándose. `false` = ya hay un trabajo para ella.
fn begin_job(inflight: &Mutex<HashSet<String>>, key: &str) -> bool {
    inflight
        .lock()
        .map(|mut keys| keys.insert(key.to_string()))
        .unwrap_or(true)
}

fn finish_job(inflight: &Mutex<HashSet<String>>, key: &str) {
    if let Ok(mut keys) = inflight.lock() {
        keys.remove(key);
    }
}

fn worker(
    jobs: Receiver<Job>,
    events: Sender<DownloadEvent>,
    children: Arc<Mutex<Vec<u32>>>,
    inflight: Arc<Mutex<HashSet<String>>>,
) {
    let mut queue: VecDeque<Job> = VecDeque::new();
    loop {
        // Prioriza el trabajo en curso; recoge todo lo que haya. `Quit` corta
        // de inmediato: al cerrar no se procesan descargas pendientes ni se
        // spawnean yt-dlp nuevos después de matar los hijos.
        while let Ok(job) = jobs.try_recv() {
            if matches!(job, Job::Quit) {
                return;
            }
            queue.push_back(job);
        }
        let Some(job) = queue.pop_front() else {
            match jobs.recv_timeout(std::time::Duration::from_millis(80)) {
                Ok(Job::Quit) => return,
                Ok(job) => queue.push_back(job),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return,
            }
            continue;
        };
        match job {
            Job::Quit => return,
            Job::Search { id, query } => {
                let result = search(&query);
                match result {
                    Ok(items) => {
                        let _ = events.send(DownloadEvent::Found { query_id: id, items });
                    }
                    Err(err) => {
                        let _ = events.send(DownloadEvent::SearchFailed {
                            query_id: id,
                            message: err.to_string(),
                        });
                    }
                }
            }
            Job::Download { key, source, out } => {
                emit_progress(&events, &key, DownloadStage::Resolving, 0.0);
                let done_key = key.clone();
                match download(&key, &source, &out, &events, &children) {
                    Ok(path) => {
                        emit_progress(&events, &key, DownloadStage::Done, 100.0);
                        let _ = events.send(DownloadEvent::Done { key, path });
                    }
                    Err(err) => {
                        let _ = events.send(DownloadEvent::Failed {
                            key,
                            message: err.to_string(),
                        });
                    }
                }
                finish_job(&inflight, &done_key);
            }
            Job::Resolve {
                key,
                artist,
                title,
                duration_ms,
                out,
            } => {
                emit_progress(&events, &key, DownloadStage::Resolving, 0.0);
                // Mismo matcher estricto que la reproducción: variantes de
                // consulta, rechazo de covers/directos y duración. Si el
                // primer upload está bloqueado o no deja descargar, se prueba
                // el siguiente candidato antes de rendirse.
                let result = crate::stream::find_video_ids(&artist, &title, duration_ms, 4)
                    .map_err(|err| anyhow!("{err:#}"))
                    .and_then(|ids| {
                        let mut last_error: Option<anyhow::Error> = None;
                        for id in ids {
                            let source = DownloadSource::Url(format!(
                                "https://www.youtube.com/watch?v={id}"
                            ));
                            match download(&key, &source, &out, &events, &children) {
                                Ok(path) => return Ok(path),
                                Err(err) => last_error = Some(err),
                            }
                        }
                        Err(last_error.unwrap_or_else(|| {
                            anyhow!("ningún candidato de YouTube se pudo descargar")
                        }))
                    });
                match result {
                    Ok(path) => {
                        emit_progress(&events, &key, DownloadStage::Done, 100.0);
                        let _ = events.send(DownloadEvent::Done { key, path });
                    }
                    Err(err) => {
                        let _ = events.send(DownloadEvent::Failed {
                            key,
                            message: err.to_string(),
                        });
                    }
                }
            }
            Job::Update => {
                let output = Command::new(ytdlp_bin()).arg("-U").output();
                let message = match output {
                    Ok(output) => {
                        let text = String::from_utf8_lossy(&output.stdout).to_string()
                            + &String::from_utf8_lossy(&output.stderr);
                        text.lines().last().unwrap_or("yt-dlp actualizado").to_string()
                    }
                    Err(err) => err.to_string(),
                };
                let _ = events.send(DownloadEvent::Message(message));
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct SearchJson {
    #[serde(default)]
    entries: Vec<SearchEntry>,
}

#[derive(Debug, Deserialize)]
struct SearchEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    // yt-dlp manda `null` cuando el vídeo no expone canal: no puede ser String.
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    webpage_url: Option<String>,
}

fn search(query: &str) -> Result<Vec<SearchItem>> {
    let output = Command::new(ytdlp_bin())
        .args([
            "--flat-playlist",
            "--dump-single-json",
            "--no-warnings",
            "--no-playlist",
            "--socket-timeout",
            "15",
            "--extractor-args",
            "youtube:player_client=default,web_safari",
            "--",
            &format!("ytsearch12:{query}"),
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("no se pudo ejecutar yt-dlp")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("yt-dlp: {}", last_lines(&err, 3)));
    }
    let json: SearchJson = serde_json::from_slice(&output.stdout)
        .context("respuesta de búsqueda ilegible")?;
    let mut items: Vec<SearchItem> = json
        .entries
        .into_iter()
        .filter(|entry| !entry.id.is_empty() && entry.duration.unwrap_or(0.0) > 30.0)
        .map(|entry| {
            let channel = entry
                .channel
                .filter(|value| !value.is_empty())
                .or_else(|| entry.uploader.filter(|value| !value.is_empty()))
                .unwrap_or_default();
            let raw_title = entry.title.unwrap_or_default();
            let (artist, title) = split_artist_title(&raw_title, &channel);
            SearchItem {
                provider_id: entry.id.clone(),
                provider: "ytmusic".into(),
                title,
                artist,
                album: None,
                art_url: None,
                isrc: None,
                duration_ms: (entry.duration.unwrap_or(0.0) * 1000.0) as u64,
                channel,
                url: entry
                    .webpage_url
                    .filter(|value| !value.is_empty())
                    .unwrap_or(entry.url),
            }
        })
        .collect();

    // Topic primero (audio de álbum), luego el resto por duración.
    items.sort_by_key(|item| {
        (
            !item.channel.ends_with(" - Topic"),
            item.title.len(),
            item.provider_id.clone(),
        )
    });
    Ok(items)
}

/// Porcentaje de una línea de progreso de yt-dlp:
/// `[download]  42.3% of 3.2MiB at ...` → `Some(42.3)`.
/// Las líneas de `[download]` sin porcentaje (Destination, etc.) dan `None`.
fn parse_percent(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let (value, _) = rest.split_once('%')?;
    let percent: f32 = value.trim().parse().ok()?;
    Some(percent.clamp(0.0, 100.0))
}

/// Línea del `--progress-template`: `descargado total_estimado total`.
/// yt-dlp escribe `NA` cuando no conoce el tamaño; el total del servidor vale
/// como respaldo.
fn parse_byte_progress(rest: &str) -> Option<f32> {
    let mut parts = rest.split_whitespace();
    let downloaded: f64 = parts.next().and_then(|value| value.parse().ok())?;
    let estimate: f64 = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0.0);
    let total: f64 = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0.0);
    let total = if estimate > 0.0 { estimate } else { total };
    if total <= 0.0 {
        return None;
    }
    Some(((downloaded / total * 100.0) as f32).clamp(0.0, 100.0))
}

/// "Artist - Title" con canal "- Topic" → (artist, title). Si no, el canal es
/// el artista y el título queda limpio de sufijos.
fn split_artist_title(title: &str, channel: &str) -> (String, String) {
    let topic = channel
        .strip_suffix(" - Topic")
        .or_else(|| channel.strip_suffix(" - Topic / Topic"));
    if let Some(artist) = topic {
        // En el canal Topic el título suele ser exactamente el nombre canción.
        return (artist.trim().to_string(), clean_title(title));
    }
    if let Some((left, right)) = title.split_once(" - ") {
        // "Artist - Title" solo si el canal no coincide con el artista.
        if !channel.is_empty() && !channel.eq_ignore_ascii_case(left.trim()) {
            return (clean_title(channel), clean_title(right));
        }
        return (clean_title(left), clean_title(right));
    }
    let artist = if channel.is_empty() {
        String::new()
    } else {
        clean_title(&channel.replace(" - Topic", ""))
    };
    (artist, clean_title(title))
}

fn clean_title(title: &str) -> String {
    let mut out = title.to_string();
    for noise in [
        "(Official Video)",
        "[Official Video]",
        "(Official Music Video)",
        "(Official Audio)",
        "(Lyric Video)",
        "[Lyric Video]",
        "(Lyrics)",
        "[Lyrics]",
        "(Audio)",
        "[Audio]",
        "(Visualizer)",
        "| Official Video",
    ] {
        out = out.replace(noise, "");
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn download(
    key: &str,
    source: &DownloadSource,
    out_dir: &Path,
    events: &Sender<DownloadEvent>,
    children: &Arc<Mutex<Vec<u32>>>,
) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir).ok();
    let DownloadSource::Url(url) = source;
    // Nombre estable por pista. Ojo: `slug(...).max(...)` NO es un fallback,
    // es comparación lexicográfica y colapsaba todo a "track".
    let name = util::slug(key, 80);
    let name = if name.is_empty() { "track".to_string() } else { name };
    let target = out_dir.join(name);
    // El `media_dir` del usuario puede contener `%`: yt-dlp lo interpreta
    // como plantilla. Se escapa el prefijo y se deja solo el `%(ext)s` real.
    let template_base = target.display().to_string().replace('%', "%%");

    let mut command = Command::new(ytdlp_bin());
    command
        .arg("-f")
        .arg("bestaudio/best")
        .arg("-x")
        .arg("--audio-format")
        .arg("opus")
        .arg("--audio-quality")
        .arg("0")
        .arg("--no-playlist")
        .arg("--newline")
        .arg("--no-warnings")
        .arg("--socket-timeout")
        .arg("15")
        .arg("--embed-metadata")
        .arg("--write-thumbnail")
        .arg("--convert-thumbnails")
        .arg("jpg")
        .arg("--extractor-args")
        .arg("youtube:player_client=default,web_safari")
        .arg("--progress-template")
        .arg(
            "download:SPOTE_PROGRESS %(progress.downloaded_bytes)s \
             %(progress.total_bytes_estimate)s %(progress.total_bytes)s",
        )
        .arg("-o")
        .arg(format!("{template_base}.%(ext)s"))
        .arg("-o")
        .arg(format!("thumbnail:{template_base}.%(ext)s"))
        // `--` corta las opciones: una URL que empiece por `-` no puede
        // convertirse en un flag de yt-dlp.
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child: Child = command.spawn().context("no se pudo ejecutar yt-dlp")?;
    if let Ok(mut list) = children.lock() {
        list.push(child.id());
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // stderr en otro hilo para no bloquear el pipe.
    let error_text = Arc::new(Mutex::new(String::new()));
    let err_shared = Arc::clone(&error_text);
    let err_handle = stderr.map(|stderr| {
        let err_shared = Arc::clone(&err_shared);
        let key_owned = key.to_string();
        let events_owned = events.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut text) = err_shared.lock() {
                    text.push_str(&line);
                    text.push('\n');
                }
                // ffmpeg solo entra cuando la descarga ya terminó: es el único
                // paso que se puede anunciar desde stderr sin mentir en el %.
                if line.contains("[ExtractAudio]")
                    || line.contains("[Merger]")
                    || line.contains("[Fixup")
                {
                    emit_progress(&events_owned, &key_owned, DownloadStage::Converting, 100.0);
                }
            }
        })
    });

    let mut reporter = ProgressReporter::new(events, key);
    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("SPOTE_PROGRESS ") {
                // Formato del --progress-template: bytes bajados / total.
                match parse_byte_progress(rest) {
                    Some(percent) => reporter.report_download(percent),
                    // Sin total no hay porcentaje: al menos se anuncia el paso.
                    None => {
                        let percent = reporter.percent();
                        reporter.report(DownloadStage::Downloading, percent);
                    }
                }
                continue;
            }
            if let Some(percent) = parse_percent(&line) {
                reporter.report_download(percent);
                continue;
            }
            if line.contains("[ExtractAudio]")
                || line.contains("[Merger]")
                || line.contains("[Fixup")
            {
                let percent = reporter.percent();
                reporter.report(DownloadStage::Converting, percent);
            }
        }
    }

    let status = child.wait()?;
    if let Ok(mut list) = children.lock() {
        list.retain(|pid| *pid != child.id());
    }
    if let Some(handle) = err_handle {
        let _ = handle.join();
    }
    if !status.success() {
        let text = error_text.lock().map(|t| t.clone()).unwrap_or_default();
        return Err(anyhow!("yt-dlp falló: {}", last_lines(&text, 4)));
    }

    // yt-dlp puede dejar .opus, .webm o .m4a si la conversión no aplica.
    for ext in ["opus", "ogg", "webm", "m4a", "mp3"] {
        let candidate = target.with_extension(ext);
        if candidate.exists() {
            // `--write-thumbnail` puede dejar jpg/png/webp según el origen.
            if ["jpg", "png", "webp"]
                .iter()
                .any(|ext| target.with_extension(ext).exists())
            {
                reporter.report(DownloadStage::Cover, 100.0);
            }
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "yt-dlp terminó pero no encuentro el fichero en {}",
        target.display()
    ))
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join(" / ")
}

// ---------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct IngestReport {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Escanea un directorio de audio y lo indexa en la librería. Respeta tags
/// (ffprobe) y el `manifest.jsonl` opcional que produzca el extractor.
pub fn ingest_dir(lib: &mut Library, dir: &Path, copy_to: &Path) -> Result<IngestReport> {
    let mut report = IngestReport::default();
    std::fs::create_dir_all(copy_to).ok();
    let manifest = read_manifest(dir);
    let mut files: Vec<PathBuf> = Vec::new();
    collect_audio(dir, &mut files, 0)?;

    for file in files {
        let tags = ffprobe_tags(&file).unwrap_or_default();
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track")
            .to_string();
        let manifest_entry = manifest.get(&file).cloned();
        let manifest_duration = manifest_entry.as_ref().map(|entry| entry.duration_ms).unwrap_or(0);
        let (title, artist, album) = match (tags.title.clone(), tags.artist.clone()) {
            (Some(title), Some(artist)) => (title, artist, tags.album.clone()),
            _ => match manifest_entry {
                Some(entry) => (entry.title, entry.artist, entry.album),
                None => (stem.clone(), "Unknown".to_string(), None),
            },
        };
        let duration_ms = tags.duration_ms.filter(|value| *value > 0).unwrap_or(manifest_duration);
        let key = util::fallback_key(&artist, &title, duration_ms);
        let track = TrackRef {
            key,
            title,
            artist,
            album,
            duration_ms,
            art_url: None,
            source_hint: None,
        };
        let id = lib.upsert_track(&track)?;
        let destination = copy_to.join(format!(
            "{}.{}",
            util::slug(&track.key, 90),
            file.extension().and_then(|e| e.to_str()).unwrap_or("opus")
        ));
        if !destination.exists() {
            if std::fs::copy(&file, &destination).is_err() {
                // Si no se puede copiar (otro filesystem lleno), se indexa in situ.
                lib.set_source(
                    id,
                    "local",
                    None,
                    Some(&file),
                    SourceStatus::Ready,
                    file_size(&file),
                )?;
                report.imported += 1;
                continue;
            }
        }
        lib.set_source(
            id,
            "local",
            None,
            Some(&destination),
            SourceStatus::Ready,
            file_size(&destination),
        )?;
        if let Some(cover) = find_cover(&file) {
            // La portada se copia al directorio de medios: la ruta original es
            // temporal (carpeta de importación) y las miniaturas la necesitan.
            let extension = cover
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("jpg");
            let cover_dest = copy_to.join(format!(
                "{}.{}",
                util::slug(&track.key, 90),
                extension
            ));
            if !cover_dest.exists() {
                std::fs::copy(&cover, &cover_dest).ok();
            }
            let source = if cover_dest.exists() { cover_dest } else { cover };
            lib.set_art(&track.key, Some(&source), None)?;
        }
        report.imported += 1;
    }
    Ok(report)
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    duration_ms: u64,
}

fn read_manifest(dir: &Path) -> std::collections::HashMap<PathBuf, ManifestEntry> {
    let mut out = std::collections::HashMap::new();
    let path = dir.join("manifest.jsonl");
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(file) = value.get("file").and_then(|v| v.as_str()) else {
            continue;
        };
        let entry = ManifestEntry {
            title: value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            artist: value
                .get("artist")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            album: value.get("album").and_then(|v| v.as_str()).map(str::to_string),
            duration_ms: value
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        };
        let resolved = if Path::new(file).is_absolute() {
            PathBuf::from(file)
        } else {
            dir.join(file)
        };
        out.insert(resolved, entry);
    }
    out
}

fn collect_audio(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    if depth > 6 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_audio(&path, out, depth + 1)?;
        } else if crate::util::audio_ext(&path).is_some() {
            out.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
}

fn ffprobe_tags(path: &Path) -> Option<Tags> {
    let output = Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let format = value.get("format")?;
    // En Ogg/Opus los tags viven en el stream, no en el formato: se combinan
    // ambas fuentes quedándose con lo primero que aparezca.
    let mut maps: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
    if let Some(tags) = format.get("tags").and_then(|tags| tags.as_object()) {
        maps.push(tags.clone());
    }
    if let Some(streams) = value.get("streams").and_then(|streams| streams.as_array()) {
        for stream in streams {
            if let Some(tags) = stream.get("tags").and_then(|tags| tags.as_object()) {
                maps.push(tags.clone());
            }
        }
    }
    let get = |key: &str| {
        for map in &maps {
            let found = map
                .get(key)
                .or_else(|| map.get(&key.to_uppercase()))
                .and_then(|value| value.as_str());
            if let Some(value) = found {
                if !value.trim().is_empty() {
                    return Some(value.to_string());
                }
            }
        }
        None
    };
    Some(Tags {
        title: get("title"),
        artist: get("artist").or_else(|| get("album_artist")),
        album: get("album"),
        duration_ms: format
            .get("duration")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .map(|secs| (secs * 1000.0) as u64),
    })
}

fn find_cover(audio: &Path) -> Option<PathBuf> {
    let dir = audio.parent()?;
    for name in ["cover.jpg", "cover.png", "folder.jpg", "front.jpg"] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflight_keys_are_deduplicated_until_finished() {
        let inflight = Mutex::new(HashSet::new());
        assert!(begin_job(&inflight, "sha1:a"));
        assert!(!begin_job(&inflight, "sha1:a"));
        assert!(begin_job(&inflight, "sha1:b"));
        finish_job(&inflight, "sha1:a");
        assert!(begin_job(&inflight, "sha1:a"));
    }

    #[test]
    fn splits_topic_channels() {
        let (artist, title) = split_artist_title("Do I Wanna Know?", "Arctic Monkeys - Topic");
        assert_eq!(artist, "Arctic Monkeys");
        assert_eq!(title, "Do I Wanna Know?");
    }

    #[test]
    fn splits_dash_titles_and_strips_noise() {
        let (artist, title) =
            split_artist_title("Tame Impala - The Less I Know (Official Video)", "Tame Impala");
        assert_eq!(artist, "Tame Impala");
        assert_eq!(title, "The Less I Know");
    }

    #[test]
    fn falls_back_to_channel() {
        let (artist, title) = split_artist_title("Some Song", "Random Uploads");
        assert_eq!(artist, "Random Uploads");
        assert_eq!(title, "Some Song");
    }

    #[test]
    fn clean_title_collapses_whitespace() {
        assert_eq!(clean_title("  A    B (Lyrics) "), "A B");
    }

    #[test]
    fn parses_ytdlp_percent_lines() {
        assert_eq!(
            parse_percent("[download]  42.3% of 3.2MiB at 1.20MiB/s ETA 00:02"),
            Some(42.3)
        );
        assert_eq!(parse_percent("[download] 100% of 3.2MiB in 00:02"), Some(100.0));
        assert_eq!(parse_percent("  [download]    7.5% of ~12.0MiB"), Some(7.5));
        // Sin porcentaje no hay nada que inventar.
        assert_eq!(parse_percent("[download] Destination: /tmp/foo.opus"), None);
        assert_eq!(parse_percent("[ExtractAudio] Destination: /tmp/foo.opus"), None);
        assert_eq!(parse_percent("SPOTE_PROGRESS 1024 4096"), None);
        assert_eq!(parse_percent(""), None);
    }

    #[test]
    fn parses_byte_progress_with_na_totals() {
        // Formato real de yt-dlp: "NA" cuando no conoce el tamaño.
        assert_eq!(parse_byte_progress("1024 NA 4096"), Some(25.0));
        assert_eq!(parse_byte_progress("1048576 2097152 NA"), Some(50.0));
        assert_eq!(parse_byte_progress("1024 NA"), None);
        assert_eq!(parse_byte_progress("NA NA NA"), None);
        assert_eq!(parse_byte_progress(""), None);
    }

    #[test]
    fn stage_labels_are_spanish() {
        assert_eq!(DownloadStage::Resolving.label(), "buscando");
        assert_eq!(DownloadStage::Downloading.label(), "descargando");
        assert_eq!(DownloadStage::Converting.label(), "convirtiendo");
        assert_eq!(DownloadStage::Cover.label(), "portada");
        assert_eq!(DownloadStage::Lyrics.label(), "letras");
        assert_eq!(DownloadStage::Done.label(), "lista");
        assert_eq!(DownloadStage::Done.to_string(), "lista");
    }

    #[test]
    fn reporter_emits_stage_changes_and_dedupes_percent() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut reporter = ProgressReporter::new(&tx, "k1");
        reporter.report(DownloadStage::Downloading, 10.0);
        // Mismo paso y menos de medio punto: no se emite.
        reporter.report(DownloadStage::Downloading, 10.2);
        reporter.report(DownloadStage::Downloading, 11.0);
        // Cambio de paso aunque el porcentaje no se mueva.
        reporter.report(DownloadStage::Converting, 11.0);
        drop(reporter);

        let events: Vec<DownloadEvent> = rx.try_iter().collect();
        let stages: Vec<(DownloadStage, f32)> = events
            .iter()
            .filter_map(|event| match event {
                DownloadEvent::StageProgress { stage, percent, .. } => Some((*stage, *percent)),
                _ => None,
            })
            .collect();
        assert_eq!(
            stages,
            vec![
                (DownloadStage::Downloading, 10.0),
                (DownloadStage::Downloading, 11.0),
                (DownloadStage::Converting, 11.0),
            ]
        );
        // Y por cada uno, el evento clásico con la etiqueta ya formateada.
        let labels: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                DownloadEvent::Progress { stage, .. } => Some(stage.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["descargando", "descargando", "convirtiendo"]);
    }

    #[test]
    fn download_percent_switches_to_converting_at_the_end() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut reporter = ProgressReporter::new(&tx, "k2");
        reporter.report_download(99.9);
        // yt-dlp repite la línea final: no debe rebobinar a "descargando".
        reporter.report_download(100.0);
        drop(reporter);
        let stages: Vec<DownloadStage> = rx
            .try_iter()
            .filter_map(|event| match event {
                DownloadEvent::StageProgress { stage, .. } => Some(stage),
                _ => None,
            })
            .collect();
        assert_eq!(
            stages,
            vec![DownloadStage::Downloading, DownloadStage::Converting]
        );
    }

    #[test]
    fn ingest_missing_dir_errors() {
        let mut lib = Library::open_in_memory().unwrap();
        let report = ingest_dir(&mut lib, Path::new("/nonexistent-spote"), Path::new("/tmp"));
        assert!(report.is_err());
    }
}
