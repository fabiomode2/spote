//! Captions de YouTube: tiempos sobre una letra conocida y, si el usuario
//! activa las letras generadas, texto de emergencia cuando no hay nada.
//!
//! Las auto-captions pueden inventar líneas, cambiar el idioma o repetir
//! fragmentos, por eso por defecto solo se usan para alinear contra una letra
//! que ya existe (Genius, LRCLIB o lrcmux). Con `allow_text` se permite la
//! ruta antigua: construir líneas con el texto del VTT.
//!
//! Se buscan vídeos candidatos con `yt-dlp`, se baja su VTT y se alinean las
//! palabras del VTT contra las líneas conocidas. Si `yt-dlp` no está, el vídeo
//! no tiene subtítulos o la alineación no es fiable, se devuelve `None` y el
//! llamante conserva la letra que ya tenía.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;
use spote_protocol::TrackRef;
use tokio::process::Command;

use super::align::{align_lines, AsrWord};
use super::text::{artist_similarity, is_youtube_provider, similarity, version_kind};
use super::{LyricLine, LyricsDoc, LyricsSource};
use crate::util;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(8);
const SUBS_TIMEOUT: Duration = Duration::from_secs(14);
const MAX_CANDIDATES: usize = 2;

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
}

/// Busca captions para `track`. Si `known` existe, solo se usan para poner
/// tiempos. Si no existe y `allow_text` es `true`, se construye un documento
/// con el texto del VTT (letra inventada); si no, se devuelve `None`.
///
/// Si la pista viene de YouTube con `videoId`, solo se usan las captions de ese
/// vídeo: son las del audio descargado y no hay que adivinar por búsqueda. Si
/// el vídeo no tiene subtítulos se devuelve `None` y el llamante conserva la
/// letra conocida; mezclar captions de otro upload desplaza los tiempos.
pub async fn fetch(
    track: &TrackRef,
    known: Option<&LyricsDoc>,
    deadline: Instant,
    allow_text: bool,
) -> Option<LyricsDoc> {
    let known = known.filter(|doc| doc.lines.len() >= 2);
    if known.is_none() && !allow_text {
        tracing::debug!(
            target: "spote::lyrics",
            "captions: sin letra conocida no se consultan (no son fuente de texto)"
        );
        return None;
    }
    let ytdlp = util::which("yt-dlp")?;
    if deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(1) {
        return None;
    }
    let candidates = match hint_video_id(track) {
        Some(id) => vec![Candidate { id: id.to_string() }],
        None => candidates(&ytdlp, track, deadline).await,
    };
    for candidate in candidates.iter().take(MAX_CANDIDATES) {
        let Some(texts) = subtitles(&ytdlp, &candidate.id, deadline).await else {
            continue;
        };
        let Some(vtt) = pick_subtitle(&texts, known, allow_text) else {
            continue;
        };
        let cues = parse_vtt(&vtt);
        if let Some(doc) = build_doc(track, known, &cues, allow_text) {
            tracing::info!(
                target: "spote::lyrics",
                "captions: {} líneas {} (vídeo {})",
                doc.lines.len(),
                if known.is_some() {
                    "alineadas sobre texto conocido"
                } else {
                    "de texto generado"
                },
                candidate.id
            );
            return Some(doc);
        }
    }
    None
}

/// Id exacto del vídeo cuando la pista se descargó de YouTube/YT Music.
fn hint_video_id(track: &TrackRef) -> Option<&str> {
    let hint = track.source_hint.as_ref()?;
    if !is_youtube_provider(&hint.provider) {
        return None;
    }
    let id = hint.id.trim();
    let valid = (6..=20).contains(&id.len())
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    valid.then_some(id)
}

async fn candidates(ytdlp: &PathBuf, track: &TrackRef, deadline: Instant) -> Vec<Candidate> {
    let mut out: Vec<(f64, Candidate)> = Vec::new();
    // Solo se busca cuando no hay un id exacto: la pista llegó de Spotify/Deezer
    // y el audio puede ser de cualquier upload. Se limpian los metadatos para
    // que un título de YouTube no arrastre "Official Video" a la búsqueda.
    let target = super::remote::MatchTarget::from_track(track);
    let query = format!(
        "ytsearch6:{} {} lyrics",
        target.artist.trim(),
        target.title.trim()
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining < Duration::from_secs(2) {
        return out.into_iter().map(|(_, candidate)| candidate).collect();
    }
    let output = match tokio::time::timeout(
        remaining.min(SEARCH_TIMEOUT),
        Command::new(ytdlp)
            .kill_on_drop(true)
            .args([
                "--flat-playlist",
                "--dump-json",
                "--skip-download",
                "--no-playlist",
                "--no-warnings",
                &query,
            ])
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => Some(output),
        _ => None,
    };
    if let Some(output) = output {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                continue;
            };
            let title = value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let channel = value
                .get("channel")
                .or_else(|| value.get("uploader"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let duration_s = value.get("duration").and_then(Value::as_f64);
            // Una versión distinta (directo, remix, cover) no tiene por qué
            // compartir tiempos ni texto con la pista pedida.
            if version_kind(&title, None) != target.version {
                continue;
            }
            let title_score = similarity(&title, &target.title);
            let minimum = if target.artist.trim().is_empty() {
                0.65
            } else {
                0.45
            };
            if title_score < minimum {
                continue;
            }
            let artist_score = artist_similarity(&channel, &target.artist);
            let duration_score = match (duration_s, target.duration_ms) {
                (Some(duration), duration_ms) if duration_ms > 0 => {
                    let diff = (duration * 1000.0 - duration_ms as f64).abs();
                    if diff <= 3_000.0 {
                        1.0
                    } else if diff <= 10_000.0 {
                        0.8
                    } else if diff <= 25_000.0 {
                        0.35
                    } else {
                        0.0
                    }
                }
                _ => 0.5,
            };
            let score = title_score * 0.55 + artist_score * 0.20 + duration_score * 0.25;
            if duration_score <= 0.0 {
                continue;
            }
            out.push((score, Candidate { id: id.to_string() }));
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut seen = HashSet::new();
    out.into_iter()
        .filter(|(_, candidate)| seen.insert(candidate.id.clone()))
        .map(|(_, candidate)| candidate)
        .collect()
}

async fn subtitles(ytdlp: &PathBuf, video_id: &str, deadline: Instant) -> Option<Vec<String>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining < Duration::from_secs(1) {
        return None;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("spote-subs-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let pattern = dir.join("%(id)s.%(ext)s");
    let url = format!("https://www.youtube.com/watch?v={video_id}");
    let _ = tokio::time::timeout(
        remaining.min(SUBS_TIMEOUT),
        Command::new(ytdlp)
            .kill_on_drop(true)
            .args([
                "--skip-download",
                "--write-subs",
                "--write-auto-subs",
                "--sub-langs",
                "es,es-orig,en,en-orig,es.*,en.*",
                "--sub-format",
                "vtt",
                "--convert-subs",
                "vtt",
                "--no-playlist",
                "--no-warnings",
                "--force-overwrites",
                "--extractor-args",
                "youtube:player_client=default,web_safari",
                "-o",
                &pattern.to_string_lossy(),
                &url,
            ])
            .output(),
    )
    .await;

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "vtt"))
        .collect();
    files.sort();
    let texts: Vec<String> = files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (!texts.is_empty()).then_some(texts)
}

/// Elige el VTT que mejor casa con la letra conocida. yt-dlp puede bajar la
/// pista original y su traducción; el archivo más grande no siempre es el
/// bueno. Sin letra conocida se queda el más largo, pero solo si se permite
/// generar texto.
fn pick_subtitle(texts: &[String], known: Option<&LyricsDoc>, allow_text: bool) -> Option<String> {
    if texts.is_empty() {
        return None;
    }
    let Some(known) = known else {
        return allow_text
            .then(|| texts.iter().max_by_key(|text| text.len()).cloned())
            .flatten();
    };
    let wanted: HashSet<String> = known
        .lines
        .iter()
        .flat_map(|line| {
            line.text
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|token| token.chars().count() >= 2)
                .map(|token| token.to_lowercase())
        })
        .collect();
    if wanted.is_empty() {
        return texts.iter().max_by_key(|text| text.len()).cloned();
    }
    let mut best: Option<(usize, &String)> = None;
    for text in texts {
        let plain = strip_tags(text);
        let overlap = plain
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|token| token.chars().count() >= 2)
            .map(|token| token.to_lowercase())
            .filter(|token| wanted.contains(token))
            .count();
        if best.as_ref().is_none_or(|(best, _)| overlap > *best) {
            best = Some((overlap, text));
        }
    }
    best.map(|(_, text)| text.clone())
}

#[derive(Debug, Clone, Default)]
struct Cue {
    words: Vec<AsrWord>,
}

/// Alinea las palabras del VTT contra `known`. Si no hay letra conocida y
/// `allow_text`, construye líneas con el texto de los cues (letra inventada).
/// En el modo alineado, las líneas devueltas son siempre las de `known`.
fn build_doc(
    track: &TrackRef,
    known: Option<&LyricsDoc>,
    cues: &[Cue],
    allow_text: bool,
) -> Option<LyricsDoc> {
    let duration_s = track.duration_ms as f64 / 1000.0;
    let words: Vec<AsrWord> = cues
        .iter()
        .flat_map(|cue| cue.words.iter().cloned())
        .collect();
    if words.len() < 6 {
        tracing::debug!(
            target: "spote::lyrics",
            "captions: {} palabras, insuficientes para alinear",
            words.len()
        );
        return None;
    }

    if let Some(known) = known.filter(|doc| doc.lines.len() >= 2) {
        // `align_lines` calcula contra tiempo de audio: hay que quitar el
        // offset del documento (hoy siempre 0, pero el parser lo hornea) y
        // las marcas instrumentales, que no se cantan.
        let offset = known.offset_ms;
        let known_lines: Vec<LyricLine> = known
            .lines
            .iter()
            .filter(|line| {
                let text = line.text.trim();
                !text.is_empty() && text != "♪" && text.chars().any(|ch| ch.is_alphanumeric())
            })
            .map(|line| {
                let mut line = line.clone();
                line.start_ms = (line.start_ms as i64 - offset).max(0) as u64;
                line.end_ms = (line.end_ms as i64 - offset).max(0) as u64;
                line.words.clear();
                line.words_exact = false;
                line
            })
            .collect();
        if known_lines.len() < 2 {
            return None;
        }
        let Some((lines, confidence, _)) = align_lines(&known_lines, &words, duration_s) else {
            tracing::debug!(
                target: "spote::lyrics",
                "captions: sin anclas suficientes sobre la letra conocida"
            );
            return None;
        };
        // Sustituir un LRC ya sincronizado exige más confianza que completar
        // una letra plana.
        let minimum = if known.synced { 0.45 } else { 0.35 };
        if confidence < minimum {
            tracing::debug!(
                target: "spote::lyrics",
                "captions: confianza {confidence:.2} < {minimum:.2}; se conserva la letra conocida"
            );
            return None;
        }
        return Some(LyricsDoc {
            lines,
            synced: true,
            instrumental: known.instrumental,
            source: known.source,
            offset_ms: 0,
            aligned: false,
            captions_timed: true,
        });
    }

    if !allow_text {
        return None;
    }

    // Ruta antigua de emergencia: el VTT se convierte en letra. Se eliminan
    // anotaciones ([Música], [Applause]), líneas vacías y repeticiones
    // consecutivas típicas de las auto-captions.
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut previous_text = String::new();
    for cue in cues {
        let text = cue
            .words
            .iter()
            .map(|word| word.text.trim())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let text = collapse(&text);
        if text.is_empty() || text == previous_text || is_caption_noise(&text) {
            continue;
        }
        previous_text = text.clone();
        let start_ms = cue
            .words
            .first()
            .map(|word| (word.start_s.max(0.0) * 1000.0) as u64)
            .unwrap_or(0);
        let end_ms = cue
            .words
            .last()
            .map(|word| (word.end_s.max(word.start_s) * 1000.0) as u64)
            .unwrap_or(start_ms.saturating_add(250))
            .max(start_ms.saturating_add(250));
        lines.push(LyricLine {
            start_ms,
            end_ms,
            text,
            ..LyricLine::default()
        });
    }
    if lines.len() < 2 {
        return None;
    }
    lines.sort_by_key(|line| line.start_ms);
    for index in 0..lines.len().saturating_sub(1) {
        lines[index].end_ms = lines[index]
            .end_ms
            .min(lines[index + 1].start_ms.saturating_sub(40))
            .max(lines[index].start_ms + 250);
    }
    Some(LyricsDoc {
        lines,
        synced: true,
        instrumental: false,
        source: LyricsSource::Captions,
        offset_ms: 0,
        aligned: false,
        captions_timed: false,
    })
}

/// Anotaciones de auto-captions que no son letra: `[Música]`, `[Applause]`,
/// `♪` suelto. Si se cuelan, el documento parece tener versos que no existen.
fn is_caption_noise(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    if text.chars().all(|ch| ch == '♪' || ch.is_whitespace()) {
        return true;
    }
    text.starts_with('[') && text.ends_with(']') && !text[1..text.len() - 1].contains('[')
}

fn parse_vtt(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut header: Option<(f64, f64)> = None;
    let mut body: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush_cue(&mut header, &mut body, &mut cues);
            continue;
        }
        if let Some(range) = parse_header(line) {
            flush_cue(&mut header, &mut body, &mut cues);
            header = Some(range);
        } else if header.is_some() {
            body.push(line.to_string());
        }
    }
    flush_cue(&mut header, &mut body, &mut cues);
    cues
}

fn flush_cue(header: &mut Option<(f64, f64)>, body: &mut Vec<String>, cues: &mut Vec<Cue>) {
    let Some((start, end)) = header.take() else {
        body.clear();
        return;
    };
    if body.is_empty() {
        return;
    }
    let raw = body.join(" ");
    body.clear();
    let plain = strip_tags(&raw);
    let text = collapse(&plain);
    if text.is_empty() {
        return;
    }
    let mut words = parse_words(&raw, start, end);
    if words.len() < 2 && text.split_whitespace().count() > words.len() {
        words = spread_words(&text, start, end);
    }
    cues.push(Cue { words });
}

fn parse_header(line: &str) -> Option<(f64, f64)> {
    let (left, right) = line.split_once("-->")?;
    let start = parse_time(left.trim())?;
    let end = right.split_whitespace().next().and_then(parse_time)?;
    Some((start, end))
}

fn parse_time(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".");
    let parts: Vec<&str> = text.split(':').collect();
    match parts.as_slice() {
        [hours, minutes, seconds] => Some(
            hours.parse::<f64>().ok()? * 3600.0
                + minutes.parse::<f64>().ok()? * 60.0
                + seconds.parse::<f64>().ok()?,
        ),
        [minutes, seconds] => {
            Some(minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?)
        }
        _ => None,
    }
}

fn strip_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            break;
        };
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Convierte el texto con tags `<00:00:01.000><c>palabra</c>` en palabras.
fn parse_words(raw: &str, cue_start: f64, cue_end: f64) -> Vec<AsrWord> {
    let mut timed: Vec<(f64, String)> = Vec::new();
    let mut pending: Option<f64> = None;
    let mut buffer = String::new();
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        buffer.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim();
        if let Some(time) = parse_time(tag) {
            if let Some(time) = pending.replace(time) {
                push_word(&mut timed, time, &mut buffer);
            } else {
                push_word(&mut timed, cue_start, &mut buffer);
            }
        }
        rest = &rest[lt + gt + 1..];
    }
    buffer.push_str(rest);
    push_word(&mut timed, pending.unwrap_or(cue_start), &mut buffer);

    if timed.is_empty() {
        return Vec::new();
    }
    timed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut words = Vec::new();
    let mut previous = f64::NEG_INFINITY;
    for (index, (start, text)) in timed.iter().enumerate() {
        let start = start.max(cue_start);
        if start < previous - 0.05 {
            continue;
        }
        let end = timed
            .get(index + 1)
            .map(|(next, _)| *next)
            .unwrap_or(cue_end)
            .max(start + 0.08);
        words.push(AsrWord {
            start_s: start,
            end_s: end,
            text: text.clone(),
            probability: 1.0,
        });
        previous = start;
    }
    words
}

fn push_word(words: &mut Vec<(f64, String)>, time: f64, buffer: &mut String) {
    let text = decode_entities(buffer.trim());
    buffer.clear();
    if text.is_empty() {
        return;
    }
    words.push((time, text));
}

fn spread_words(text: &str, start: f64, end: f64) -> Vec<AsrWord> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let span = (end - start).max(0.4);
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let begin = start + span * index as f64 / tokens.len() as f64;
            let finish = start + span * (index + 1) as f64 / tokens.len() as f64;
            AsrWord {
                start_s: begin,
                end_s: finish.max(begin + 0.05),
                text: (*token).to_string(),
                probability: 0.8,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_word_level_vtt() {
        let vtt = "WEBVTT\n\n\
00:00:01.000 --> 00:00:04.000\n\
<00:00:01.000><c>Hola</c> <00:00:02.000><c>mundo</c>\n";
        let cues = parse_vtt(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].words.len(), 2);
        assert_eq!(cues[0].words[0].text, "Hola");
        assert_eq!(cues[0].words[1].start_s, 2.0);
    }

    #[test]
    fn parses_plain_vtt_and_spreads_words() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHola mundo\n";
        let cues = parse_vtt(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].words.len(), 2);
    }

    #[test]
    fn captions_only_apply_times_to_the_known_text() {
        // El texto del VTT puede ser otro idioma o una alucinación del ASR de
        // YouTube. Solo sus tiempos deben llegar al documento final.
        let known = LyricsDoc::from_lrc("hola mundo hermoso\nla noche es larga", false);
        let words: Vec<AsrWord> = ["hola", "mundo", "hermoso", "la", "noche", "es", "larga"]
            .iter()
            .enumerate()
            .map(|(index, word)| AsrWord {
                start_s: 1.0 + index as f64 * 0.5,
                end_s: 1.5 + index as f64 * 0.5,
                text: (*word).to_string(),
                probability: 1.0,
            })
            .collect();
        let cues = [Cue { words }];
        let doc = build_doc(
            &TrackRef {
                duration_ms: 60_000,
                ..TrackRef::default()
            },
            Some(&known),
            &cues,
            false,
        )
        .expect("la letra conocida se alinea");
        let texts: Vec<&str> = doc.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, vec!["hola mundo hermoso", "la noche es larga"]);
        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
        assert!(doc.captions_timed);
    }

    #[test]
    fn captions_without_anchors_do_not_create_text() {
        let known = LyricsDoc::from_lrc("hola mundo hermoso\nla noche es larga", false);
        let words: Vec<AsrWord> = [
            "texto",
            "inventado",
            "por",
            "las",
            "auto",
            "captions",
            "basura",
        ]
        .iter()
        .enumerate()
        .map(|(index, word)| AsrWord {
            start_s: 1.0 + index as f64 * 0.5,
            end_s: 1.5 + index as f64 * 0.5,
            text: (*word).to_string(),
            probability: 0.9,
        })
        .collect();
        let cues = [Cue { words }];
        assert!(
            build_doc(
                &TrackRef {
                    duration_ms: 60_000,
                    ..TrackRef::default()
                },
                Some(&known),
                &cues,
                false,
            )
            .is_none(),
            "sin anclas no hay documento: las captions no son letra"
        );
    }

    #[test]
    fn generated_captions_only_when_allowed() {
        let words = |items: &[&str], offset: f64| -> Vec<AsrWord> {
            items
                .iter()
                .enumerate()
                .map(|(index, word)| AsrWord {
                    start_s: offset + index as f64 * 0.5,
                    end_s: offset + 0.4 + index as f64 * 0.5,
                    text: (*word).to_string(),
                    probability: 0.8,
                })
                .collect()
        };
        let cues = [
            Cue {
                words: words(&["hola", "mundo", "hermoso"], 1.0),
            },
            Cue {
                words: words(&["la", "noche", "es", "larga"], 5.0),
            },
        ];
        let track = TrackRef {
            duration_ms: 60_000,
            ..TrackRef::default()
        };
        assert!(
            build_doc(&track, None, &cues, false).is_none(),
            "sin permiso no se inventa texto"
        );
        let doc = build_doc(&track, None, &cues, true).expect("captions como texto");
        assert_eq!(doc.source, LyricsSource::Captions);
        assert!(!doc.captions_timed);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "hola mundo hermoso");
        assert_eq!(doc.lines[1].text, "la noche es larga");
    }

    #[tokio::test]
    async fn fetch_without_known_lyrics_returns_none() {
        // Se corta antes de invocar yt-dlp: sin letra conocida no hay nada que
        // alinear y las captions no pueden convertirse en la letra mostrada.
        let track = TrackRef {
            title: "Do I Wanna Know?".into(),
            artist: "Arctic Monkeys".into(),
            duration_ms: 272_000,
            ..TrackRef::default()
        };
        assert!(
            fetch(&track, None, Instant::now() + Duration::from_secs(5), false,)
                .await
                .is_none()
        );
    }

    #[test]
    fn exact_youtube_hint_is_recognized() {
        let track = TrackRef {
            title: "Arctic Monkeys - Do I Wanna Know? (Official Video)".into(),
            artist: "Arctic Monkeys - Topic".into(),
            source_hint: Some(spote_protocol::SourceHint {
                provider: "youtube".into(),
                id: "bpOSn9Q1xQU".into(),
                duration_ms: 290_000,
            }),
            ..TrackRef::default()
        };
        assert_eq!(hint_video_id(&track), Some("bpOSn9Q1xQU"));

        let mut music = track.clone();
        music.source_hint = Some(spote_protocol::SourceHint {
            provider: "ytmusic".into(),
            id: "bpOSn9Q1xQU".into(),
            duration_ms: 0,
        });
        assert_eq!(hint_video_id(&music), Some("bpOSn9Q1xQU"));

        let mut other = track;
        other.source_hint = Some(spote_protocol::SourceHint {
            provider: "spotify".into(),
            id: "abc".into(),
            duration_ms: 0,
        });
        assert_eq!(hint_video_id(&other), None);
    }

    #[test]
    fn subtitle_pick_prefers_the_transcript_that_matches_the_lyrics() {
        // yt-dlp puede bajar la letra original y una traducción; el archivo más
        // grande no siempre es el que comparte palabras con la letra conocida.
        let known = LyricsDoc::from_lrc(
            "[00:00.00]hola mundo\n[00:02.00]otra frase cualquiera",
            true,
        );
        let translated = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nbonjour le monde\n";
        let original = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nhola mundo otra frase\n";
        let texts = vec![translated.to_string(), original.to_string()];
        let picked = pick_subtitle(&texts, Some(&known), false).unwrap();
        assert!(picked.contains("hola mundo"), "{picked}");
    }
}
