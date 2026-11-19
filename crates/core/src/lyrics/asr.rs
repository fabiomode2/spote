//! Lectura del audio con faster-whisper y orquestación de varias pasadas.
//!
//! El transcriptor vive en `scripts/asr_words.py` y es deliberadamente tonto:
//! recibe una lista de ventanas y devuelve palabras con tiempos. Aquí está lo
//! que importa: leer la canción entera, alinear, detectar las zonas donde la
//! letra se quedó sin ancla (outro, instrumental largo) y volver a leer solo
//! esas zonas con 15 s de contexto a cada lado. De las lecturas se queda la
//! que mejor encaja.
//!
//! Es el equivalente Rust de la técnica de lyricbar; spote no importa su
//! código para esto.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::align::{align_lines, AlignStats, AsrWord};
use super::LyricLine;

const READ_TIMEOUT: Duration = Duration::from_secs(180);
const MIN_TAIL_GAP_S: f64 = 8.0;
const CONTEXT_S: f64 = 15.0;
const WINDOW_S: f64 = 45.0;
const MAX_READS: usize = 4;

#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub python: String,
    pub script: PathBuf,
    pub model: String,
    pub device: String,
    pub language: String,
}

impl AsrConfig {
    /// Resuelve el intérprete y el script. Si no se encuentra el transcriptor,
    /// el ASR queda apagado y la letra estimada se muestra tal cual.
    pub fn resolve(cfg: &crate::config::LyricsCfg) -> Option<Self> {
        if !cfg.asr {
            return None;
        }
        let script = super::bridge::resolve_script(&cfg.asr_script)?;
        let mut python = if cfg.python.trim().is_empty() {
            "python3".to_string()
        } else {
            cfg.python.clone()
        };
        // El instalador crea un venv propio con faster-whisper; si no,
        // reutiliza el de lyricbar. También se puede forzar por entorno.
        if python == "python3" {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(path) = std::env::var_os("SPOTE_ASR_PYTHON") {
                candidates.push(PathBuf::from(path));
            }
            candidates.push(crate::config::data_dir().join("venv/bin/python"));
            if !cfg.root.is_empty() {
                candidates.push(Path::new(&cfg.root).join(".venv/bin/python"));
            }
            if let Some(venv) = candidates.into_iter().find(|path| path.is_file()) {
                python = venv.to_string_lossy().into_owned();
            }
        }
        Some(Self {
            python,
            script,
            model: cfg.asr_model.clone(),
            device: cfg.asr_device.clone(),
            language: cfg.asr_language.clone(),
        })
    }
}

#[derive(Serialize)]
struct ReadRequest<'a> {
    audio: &'a str,
    windows: Vec<[f64; 2]>,
    model: &'a str,
    device: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
}

#[derive(Deserialize)]
struct HelperWord(f64, f64, String, #[serde(default)] f32);

#[derive(Deserialize)]
struct HelperResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    duration_s: Option<f64>,
    #[serde(default)]
    reads: Vec<Vec<HelperWord>>,
}

/// Proceso persistente del transcriptor: carga el modelo una vez y responde a
/// varias peticiones de ventanas por la misma tubería.
struct AsrSession {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    model: String,
    device: String,
    language: Option<String>,
}

impl AsrSession {
    fn start(cfg: &AsrConfig) -> Option<Self> {
        let mut child = Command::new(&cfg.python)
            .arg(&cfg.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = mpsc::channel();
        if std::thread::Builder::new()
            .name("spote-asr-read".into())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .is_err()
        {
            let _ = child.kill();
            return None;
        }
        Some(Self {
            child,
            stdin,
            lines: rx,
            model: cfg.model.clone(),
            device: cfg.device.clone(),
            language: if cfg.language.trim().is_empty() || cfg.language == "auto" {
                None
            } else {
                Some(cfg.language.clone())
            },
        })
    }

    fn read(
        &mut self,
        audio: &Path,
        windows: &[(f64, f64)],
    ) -> Option<(Vec<Vec<AsrWord>>, Option<f64>)> {
        let audio = audio.to_string_lossy().into_owned();
        let request = ReadRequest {
            audio: &audio,
            windows: windows.iter().map(|(start, end)| [*start, *end]).collect(),
            model: &self.model,
            device: &self.device,
            language: self.language.as_deref(),
        };
        let line = serde_json::to_string(&request).ok()?;
        writeln!(self.stdin, "{line}").ok()?;
        self.stdin.flush().ok()?;
        let response = match self.lines.recv_timeout(READ_TIMEOUT) {
            Ok(response) => response,
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                let _ = self.child.kill();
                return None;
            }
        };
        let parsed: HelperResponse = serde_json::from_str(&response).ok()?;
        if !parsed.ok {
            if std::env::var_os("SPOTE_ASR_DEBUG").is_some() {
                eprintln!("[asr] {}", parsed.error.unwrap_or_default());
            }
            return None;
        }
        let reads = parsed
            .reads
            .into_iter()
            .map(|words| {
                words
                    .into_iter()
                    .map(|HelperWord(start, end, text, probability)| AsrWord {
                        start_s: start,
                        end_s: end,
                        text,
                        probability,
                    })
                    .collect()
            })
            .collect();
        Some((reads, parsed.duration_s))
    }
}

impl Drop for AsrSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Alinea una letra contra el audio local con una lectura completa y
/// relecturas de las zonas cojas.
pub fn align_track(
    cfg: &AsrConfig,
    audio: &Path,
    duration_s: f64,
    lines: &[LyricLine],
) -> Option<(Vec<LyricLine>, f64)> {
    let mut session = AsrSession::start(cfg)?;
    // La duración puede venir del reproductor o del propio fichero. Si no se
    // conoce, el script lee hasta el final.
    let (reads, helper_duration) = session.read(audio, &[(0.0, duration_s)])?;
    let words = reads.into_iter().next()?;
    let duration = if duration_s > 0.0 {
        duration_s
    } else {
        helper_duration.unwrap_or(0.0)
    };
    let (mut best_lines, mut best_confidence, stats) = align_lines(lines, &words, duration)?;
    tracing::debug!(
        target: "spote::lyrics",
        "ASR: {} líneas ancladas de {}, shift {:+.2}s",
        stats.matched, lines.len(), stats.median_shift
    );

    let mut all_words = words;
    let windows = candidate_windows(&best_lines, &stats, &all_words, duration);
    let mut reads_done = 1usize;
    for (begin, end) in windows {
        if reads_done >= MAX_READS {
            break;
        }
        let Some((extra_reads, _)) = session.read(audio, &[(begin, end)]) else {
            continue;
        };
        let Some(extra) = extra_reads.into_iter().next() else {
            continue;
        };
        if extra.is_empty() {
            continue;
        }
        reads_done += 1;
        let merged = merge_words(&all_words, &extra, begin, end);
        let Some((candidate_lines, candidate_confidence, _)) =
            align_lines(lines, &merged, duration)
        else {
            continue;
        };
        if better(
            candidate_confidence,
            &candidate_lines,
            best_confidence,
            &best_lines,
        ) {
            all_words = merged;
            best_lines = candidate_lines;
            best_confidence = candidate_confidence;
        }
    }
    Some((best_lines, best_confidence))
}

fn crammed(lines: &[LyricLine]) -> usize {
    lines
        .windows(2)
        .filter(|pair| pair[1].start_ms.saturating_sub(pair[0].start_ms) < 250)
        .count()
}

fn better(
    confidence: f64,
    lines: &[LyricLine],
    best_confidence: f64,
    best_lines: &[LyricLine],
) -> bool {
    if (confidence - best_confidence).abs() > 1e-9 {
        return confidence > best_confidence;
    }
    crammed(lines) < crammed(best_lines)
}

fn merge_words(words: &[AsrWord], extra: &[AsrWord], begin: f64, end: f64) -> Vec<AsrWord> {
    let border = 1.0;
    let inside = |word: &AsrWord| word.start_s >= begin - border && word.start_s <= end + border;
    let mut merged: Vec<AsrWord> = words
        .iter()
        .filter(|word| !inside(word))
        .cloned()
        .chain(extra.iter().filter(|word| inside(word)).cloned())
        .collect();
    merged.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    merged
}

/// Zonas que merecen otra lectura: la cola que el modelo no transcribió y las
/// tiradas de líneas que quedaron sin ancla o pegadas.
fn candidate_windows(
    lines: &[LyricLine],
    stats: &AlignStats,
    words: &[AsrWord],
    duration_s: f64,
) -> Vec<(f64, f64)> {
    let mut windows: Vec<(f64, f64)> = Vec::new();
    let last_word = words.iter().map(|word| word.end_s).fold(0.0, f64::max);
    if duration_s - last_word > MIN_TAIL_GAP_S {
        windows.push(((last_word - CONTEXT_S).max(0.0), duration_s));
    }

    let mut anchored = stats.anchored.clone();
    anchored.resize(lines.len(), false);
    let times: Vec<f64> = lines
        .iter()
        .map(|line| line.start_ms as f64 / 1000.0)
        .collect();
    let mut index = 0;
    while index < times.len() {
        if anchored[index] {
            index += 1;
            continue;
        }
        let mut run_end = index;
        while run_end + 1 < times.len() && !anchored[run_end + 1] {
            run_end += 1;
        }
        let tail = times[index] >= 15.0f64.max(duration_s * 0.72);
        if run_end > index || tail {
            let begin = (times[index] - CONTEXT_S).max(0.0);
            let end = (times[run_end] + WINDOW_S).min(duration_s);
            if end - begin >= 4.0 {
                windows.push((begin, end));
            }
        }
        index = run_end + 1;
    }

    let mut index = 0;
    while index + 1 < times.len() {
        if times[index + 1] - times[index] >= 0.45 {
            index += 1;
            continue;
        }
        let mut run_end = index + 1;
        while run_end + 1 < times.len() && times[run_end + 1] - times[run_end] < 0.45 {
            run_end += 1;
        }
        if run_end - index + 1 >= 3 {
            let begin = (times[index] - CONTEXT_S).max(0.0);
            let end = (times[run_end] + WINDOW_S).min(duration_s);
            if end - begin >= 4.0 {
                windows.push((begin, end));
            }
        }
        index = run_end + 1;
    }

    // Ventanas solapadas se leen una sola vez.
    windows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (begin, end) in windows {
        if let Some(last) = merged.last_mut() {
            if begin <= last.1 + 5.0 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((begin, end));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start_ms: u64) -> LyricLine {
        LyricLine {
            start_ms,
            text: "x".into(),
            ..LyricLine::default()
        }
    }

    #[test]
    fn loose_tail_gets_a_window() {
        let lines: Vec<LyricLine> = (0..6).map(|index| line(index * 10_000)).collect();
        let stats = AlignStats {
            anchored: vec![true, true, true, false, false, false],
            ..AlignStats::default()
        };
        let windows = candidate_windows(&lines, &stats, &[], 60.0);
        assert!(windows
            .iter()
            .any(|(begin, end)| *begin <= 30.0 && *end >= 59.0));
    }

    #[test]
    fn merge_words_replaces_the_window() {
        let words = vec![
            AsrWord {
                start_s: 1.0,
                end_s: 1.2,
                text: "antes".into(),
                probability: 0.8,
            },
            AsrWord {
                start_s: 10.0,
                end_s: 10.2,
                text: "viejo".into(),
                probability: 0.8,
            },
            AsrWord {
                start_s: 30.0,
                end_s: 30.2,
                text: "despues".into(),
                probability: 0.8,
            },
        ];
        let extra = vec![AsrWord {
            start_s: 10.1,
            end_s: 10.3,
            text: "nuevo".into(),
            probability: 0.9,
        }];
        let merged = merge_words(&words, &extra, 10.0, 11.0);
        let texts: Vec<&str> = merged.iter().map(|word| word.text.as_str()).collect();
        assert_eq!(texts, ["antes", "nuevo", "despues"]);
    }
}
