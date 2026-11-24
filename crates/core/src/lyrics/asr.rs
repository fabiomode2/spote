//! Lectura del audio con faster-whisper y orquestación de varias pasadas.
//!
//! El transcriptor vive en `scripts/asr_words.py` y es deliberadamente tonto:
//! recibe una lista de ventanas y devuelve palabras con tiempos. Aquí está lo
//! que importa: leer la canción entera, alinear, detectar las zonas donde la
//! letra se quedó sin ancla (outro, instrumental largo) y volver a leer solo
//! esas zonas con 15 s de contexto a cada lado. De las lecturas se queda la
//! que mejor encaja.
//!
//! La alineación y la decisión de qué releer son de spote; el script de Python
//! solo ejecuta el modelo.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::align::{align_lines, AlignStats, AsrWord};
use super::{LyricLine, Word};

/// Tope de una lectura. Si el modelo se atasca, el worker no se queda esperando
/// minutos: se mata el proceso y se conserva la letra anterior.
const READ_TIMEOUT: Duration = Duration::from_secs(70);
/// Presupuesto total de la alineación (lectura completa + relecturas).
const TOTAL_BUDGET: Duration = Duration::from_secs(95);
/// Confianza a partir de la cual no merece la pena releer zonas: cada relectura
/// decodifica y transcribe otra vez y era la causa de que el ASR tardara.
const GOOD_CONFIDENCE: f64 = 0.55;
const MIN_TAIL_GAP_S: f64 = 8.0;
const CONTEXT_S: f64 = 12.0;
const WINDOW_S: f64 = 30.0;
/// Una lectura completa y como mucho dos relecturas de zonas cojas.
const MAX_EXTRA_READS: usize = 2;
/// Cada cuánto se comprueba la cancelación mientras el modelo trabaja.
const CANCEL_POLL: Duration = Duration::from_millis(120);

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
        let script = super::resolve_script(&cfg.asr_script)?;
        let mut python = if cfg.python.trim().is_empty() {
            "python3".to_string()
        } else {
            cfg.python.clone()
        };
        // El instalador crea un venv propio con faster-whisper; si no,
        // se puede forzar por entorno.
        if python == "python3" {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(path) = std::env::var_os("SPOTE_ASR_PYTHON") {
                candidates.push(PathBuf::from(path));
            }
            candidates.push(crate::config::data_dir().join("venv/bin/python"));
            if let Some(venv) = candidates.into_iter().find(|path| path.is_file()) {
                python = venv.to_string_lossy().into_owned();
            }
        }
        Some(Self {
            python,
            script,
            // Permite probar un modelo pequeño/grande sin recompilar ni tocar
            // la configuración guardada.
            model: std::env::var("SPOTE_ASR_MODEL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| cfg.asr_model.clone()),
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
        cancel: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Option<(Vec<Vec<AsrWord>>, Option<f64>)> {
        if is_cancelled(cancel) || Instant::now() >= deadline {
            return None;
        }
        let audio = audio.to_string_lossy().into_owned();
        let request = ReadRequest {
            audio: &audio,
            windows: windows.iter().map(|(start, end)| [*start, *end]).collect(),
            model: &self.model,
            device: &self.device,
            language: self.language.as_deref(),
        };
        let line = serde_json::to_string(&request).ok()?;
        if writeln!(self.stdin, "{line}").is_err() || self.stdin.flush().is_err() {
            let _ = self.child.kill();
            return None;
        }
        let started = Instant::now();
        let response = loop {
            if is_cancelled(cancel) || Instant::now() >= deadline {
                // Matar el hijo corta la transcripción en curso; el worker puede
                // atender la pista nueva sin esperar al modelo.
                let _ = self.child.kill();
                return None;
            }
            match self.lines.recv_timeout(CANCEL_POLL) {
                Ok(response) => break response,
                Err(RecvTimeoutError::Timeout) => {
                    if started.elapsed() >= READ_TIMEOUT {
                        let _ = self.child.kill();
                        return None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = self.child.kill();
                    return None;
                }
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

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

impl Drop for AsrSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Igual que [`align_track`], con cancelación y plazo opcionales. El worker
/// pasa una bandera que se activa al cambiar de pista y un deadline; así una
/// transcripción lenta no bloquea el resto de la sesión.
pub fn align_track_with(
    cfg: &AsrConfig,
    audio: &Path,
    duration_s: f64,
    lines: &[LyricLine],
    cancel: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> Option<(Vec<LyricLine>, f64)> {
    let deadline = deadline.unwrap_or_else(|| Instant::now() + TOTAL_BUDGET);
    if is_cancelled(cancel) || Instant::now() >= deadline {
        return None;
    }
    let mut session = AsrSession::start(cfg)?;
    // La duración puede venir del reproductor o del propio fichero. Si no se
    // conoce, el script lee hasta el final.
    let (reads, helper_duration) = session.read(audio, &[(0.0, duration_s)], cancel, deadline)?;
    let words = reads.into_iter().next()?;
    let duration = if duration_s > 0.0 {
        duration_s
    } else {
        helper_duration.unwrap_or(0.0)
    };
    let (mut best_lines, mut best_confidence, stats) = align_lines(lines, &words, duration)?;
    tracing::debug!(
        target: "spote::lyrics",
        "ASR: {} líneas ancladas de {}, shift {:+.2}s, confianza {:.2}",
        stats.matched, lines.len(), stats.median_shift, best_confidence
    );

    // Con una alineación ya buena no se relanza el modelo: las relecturas eran
    // el 80 % del tiempo del ASR y apenas cambiaban el resultado.
    if best_confidence >= GOOD_CONFIDENCE {
        return Some((best_lines, best_confidence));
    }

    let mut all_words = words;
    let windows = candidate_windows(&best_lines, &stats, &all_words, duration);
    let mut reads_done = 0usize;
    for (begin, end) in windows {
        if reads_done >= MAX_EXTRA_READS || is_cancelled(cancel) || Instant::now() >= deadline {
            break;
        }
        let Some((extra_reads, _)) = session.read(audio, &[(begin, end)], cancel, deadline) else {
            // El proceso se mató (timeout/cancelación): no se insiste.
            break;
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

/// Transcribe el audio completo y agrupa las palabras en líneas. Es la única
/// ruta que inventa texto: solo se llama si el usuario activó el modo de
/// letras generadas y no hay una fuente fiable.
pub fn transcribe_track_with(
    cfg: &AsrConfig,
    audio: &Path,
    duration_s: f64,
    cancel: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> Option<(Vec<LyricLine>, f64)> {
    let deadline = deadline.unwrap_or_else(|| Instant::now() + TOTAL_BUDGET);
    if is_cancelled(cancel) || Instant::now() >= deadline {
        return None;
    }
    let mut session = AsrSession::start(cfg)?;
    let (reads, helper_duration) = session.read(audio, &[(0.0, duration_s)], cancel, deadline)?;
    let words = reads.into_iter().next()?;
    let duration = if duration_s > 0.0 {
        duration_s
    } else {
        helper_duration.unwrap_or(0.0)
    };
    lines_from_words(&words, duration)
}

/// Agrupa palabras ASR en versos: corta por silencios, puntuación y longitud.
/// La confianza es la probabilidad media que devolvió el modelo.
fn lines_from_words(words: &[AsrWord], duration_s: f64) -> Option<(Vec<LyricLine>, f64)> {
    let mut words: Vec<&AsrWord> = words
        .iter()
        .filter(|word| {
            word.end_s >= word.start_s
                && word.start_s.is_finite()
                && word.end_s.is_finite()
                && word.probability >= 0.20
                && !is_asr_noise(&word.text)
                && !super::align::normalize_word(&word.text).is_empty()
        })
        .collect();
    words.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    if words.len() < 2 {
        return None;
    }

    let mut groups: Vec<Vec<&AsrWord>> = Vec::new();
    let mut current: Vec<&AsrWord> = Vec::new();
    for word in words {
        if let Some(previous) = current.last() {
            let gap = word.start_s - previous.end_s;
            let chars: usize = current
                .iter()
                .map(|word| word.text.chars().count() + 1)
                .sum();
            if gap > 0.65 || current.len() >= 8 || chars >= 48 || ends_sentence(&previous.text) {
                groups.push(std::mem::take(&mut current));
            }
        }
        current.push(word);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    if groups.len() < 2 {
        return None;
    }

    let mut total_probability = 0.0f32;
    let mut count = 0usize;
    for group in &groups {
        for word in group {
            total_probability += word.probability.clamp(0.0, 1.0);
            count += 1;
        }
    }
    let confidence = if count > 0 {
        (total_probability / count as f32) as f64
    } else {
        0.0
    };

    let mut lines = Vec::with_capacity(groups.len());
    let mut previous_line = String::new();
    for group in groups {
        let Some(first) = group.first() else { continue };
        let Some(last) = group.last() else { continue };
        let start_ms = (first.start_s.max(0.0) * 1000.0) as u64;
        let end_ms =
            ((last.end_s.max(first.start_s) * 1000.0) as u64).max(start_ms.saturating_add(250));
        let text = group
            .iter()
            .map(|word| word.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() || text == previous_line {
            continue;
        }
        previous_line = text.clone();
        let words = group
            .iter()
            .map(|word| Word {
                start_ms: (word.start_s.max(0.0) * 1000.0) as u64,
                end_ms: (word.end_s.max(word.start_s) * 1000.0) as u64,
                text: word.text.trim().to_string(),
                optional: false,
            })
            .collect();
        lines.push(LyricLine {
            start_ms,
            end_ms,
            text,
            words,
            words_exact: true,
        });
    }
    if lines.len() < 2 {
        return None;
    }
    for index in 0..lines.len().saturating_sub(1) {
        let next = lines[index + 1].start_ms;
        let start = lines[index].start_ms;
        if next > start {
            let cap = next.saturating_sub(1);
            lines[index].end_ms = lines[index]
                .end_ms
                .min(cap)
                .max(start.saturating_add(1).min(cap));
        }
    }
    if duration_s > 0.0 {
        if let Some(last) = lines.last_mut() {
            let max = (duration_s * 1000.0) as u64;
            last.end_ms = last.end_ms.min(max.max(last.start_ms.saturating_add(250)));
        }
    }
    Some((lines, confidence.clamp(0.0, 1.0)))
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | '…' | ';' | ':'))
}

fn is_asr_noise(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    if text.chars().all(|ch| ch == '♪' || ch.is_whitespace()) {
        return true;
    }
    text.starts_with('[') && text.ends_with(']') && !text[1..text.len() - 1].contains('[')
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
    use std::sync::atomic::{AtomicBool, Ordering};

    fn line(start_ms: u64) -> LyricLine {
        LyricLine {
            start_ms,
            text: "x".into(),
            ..LyricLine::default()
        }
    }

    #[test]
    fn cancellation_flag_is_honoured() {
        let flag = AtomicBool::new(false);
        assert!(!is_cancelled(Some(&flag)));
        flag.store(true, Ordering::Relaxed);
        assert!(is_cancelled(Some(&flag)));
        assert!(!is_cancelled(None));
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
    fn generated_lines_group_by_silence_and_skip_noise() {
        let word = |start: f64, text: &str, probability: f32| AsrWord {
            start_s: start,
            end_s: start + 0.4,
            text: text.to_string(),
            probability,
        };
        let words = vec![
            word(1.0, "hola", 0.9),
            word(1.5, "mundo", 0.8),
            word(2.0, "hermoso", 0.7),
            word(2.5, "[Música]", 0.99),
            word(5.0, "la", 0.9),
            word(5.5, "noche", 0.8),
            word(6.0, "es", 0.1),
            word(6.5, "larga", 0.8),
        ];
        let (lines, confidence) = lines_from_words(&words, 10.0).expect("hay dos versos");
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["hola mundo hermoso", "la noche larga"]);
        assert!(confidence > 0.7);
        assert!(lines
            .windows(2)
            .all(|pair| pair[0].end_ms <= pair[1].start_ms));
        assert!(lines.iter().all(|line| line.end_ms > line.start_ms));
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
