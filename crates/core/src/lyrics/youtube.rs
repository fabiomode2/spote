//! Captions de YouTube.
//!
//! Es el último proveedor que faltaba para que spote no dependa de nada
//! externo: se buscan vídeos candidatos con `yt-dlp`, se baja su VTT y se
//! alinean las palabras contra la letra conocida. Si no hay letra conocida, se
//! usan las propias líneas de los captions.
//!
//! Si `yt-dlp` no está, el vídeo no tiene subtítulos o el VTT no se puede
//! leer, se devuelve `None` y sigue el fallback normal (letra plana + ASR).

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use spote_protocol::TrackRef;
use tokio::process::Command;

use super::align::{align_lines, AsrWord};
use super::text::{artist_similarity, similarity};
use super::{LyricLine, LyricsDoc, LyricsSource};
use crate::util;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const SUBS_TIMEOUT: Duration = Duration::from_secs(18);
const MAX_CANDIDATES: usize = 2;

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
}

/// Busca captions para `track`. `known` es la letra que ya tenemos (LRCLIB o
/// Genius): si existe, se usa para alinear; si no, se devuelven las líneas del
/// propio VTT.
pub async fn fetch(track: &TrackRef, known: Option<&LyricsDoc>) -> Option<LyricsDoc> {
    let ytdlp = util::which("yt-dlp")?;
    let candidates = candidates(&ytdlp, track).await;
    for candidate in candidates.iter().take(MAX_CANDIDATES) {
        let Some(vtt) = subtitles(&ytdlp, &candidate.id).await else {
            continue;
        };
        let cues = parse_vtt(&vtt);
        if let Some(doc) = build_doc(track, known, &cues) {
            return Some(doc);
        }
    }
    None
}

async fn candidates(ytdlp: &PathBuf, track: &TrackRef) -> Vec<Candidate> {
    let mut out: Vec<(f64, Candidate)> = Vec::new();
    // Si la pista viene de YouTube, ese vídeo es el primero: sus captions
    // cuadran con el audio descargado y no hay que buscar nada.
    if let Some(hint) = &track.source_hint {
        let provider = hint.provider.to_ascii_lowercase();
        if (provider.contains("youtube") || provider == "ytmusic")
            && !hint.id.trim().is_empty()
            && !hint.id.contains(' ')
        {
            out.push((
                100.0,
                Candidate {
                    id: hint.id.clone(),
                },
            ));
        }
    }

    let query = format!(
        "ytsearch6:{} {} lyrics",
        track.artist.trim(),
        track.title.trim()
    );
    let output = match tokio::time::timeout(
        SEARCH_TIMEOUT,
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
            let title_score = similarity(&title, &track.title);
            if title_score < 0.45 {
                continue;
            }
            let artist_score = artist_similarity(&channel, &track.artist);
            let duration_score = match (duration_s, track.duration_ms) {
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
            out.push((
                score,
                Candidate {
                    id: id.to_string(),
                },
            ));
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    out.into_iter()
        .filter(|(_, candidate)| seen.insert(candidate.id.clone()))
        .map(|(_, candidate)| candidate)
        .collect()
}

async fn subtitles(ytdlp: &PathBuf, video_id: &str) -> Option<String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("spote-subs-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let pattern = dir.join("%(id)s.%(ext)s");
    let url = format!("https://www.youtube.com/watch?v={video_id}");
    let _ = tokio::time::timeout(
        SUBS_TIMEOUT,
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
    let text = files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .max_by_key(String::len);
    let _ = std::fs::remove_dir_all(&dir);
    text
}

#[derive(Debug, Clone, Default)]
struct Cue {
    start: f64,
    end: f64,
    text: String,
    words: Vec<AsrWord>,
}

fn build_doc(track: &TrackRef, known: Option<&LyricsDoc>, cues: &[Cue]) -> Option<LyricsDoc> {
    let duration_s = track.duration_ms as f64 / 1000.0;
    let words: Vec<AsrWord> = cues
        .iter()
        .flat_map(|cue| cue.words.iter().cloned())
        .collect();
    if words.len() >= 6 {
        if let Some(known) = known.filter(|doc| doc.lines.len() >= 2) {
            if let Some((lines, confidence, _)) = align_lines(&known.lines, &words, duration_s) {
                if confidence >= 0.35 {
                    return Some(LyricsDoc {
                        lines,
                        synced: true,
                        instrumental: known.instrumental,
                        source: LyricsSource::Captions,
                        offset_ms: 0,
                        aligned: false,
                    });
                }
            }
        }
    }

    // Si ya había letra conocida y los captions no han alineado, no se
    // sustituye por el texto del VTT (podría estar en otro idioma).
    if known.is_some() {
        return None;
    }

    let mut lines: Vec<LyricLine> = Vec::new();
    let mut previous_text = String::new();
    for cue in cues {
        let text = cue.text.trim();
        if text.is_empty() || text == previous_text {
            continue;
        }
        previous_text = text.to_string();
        let start_ms = (cue.start.max(0.0) * 1000.0) as u64;
        let end_ms = (cue.end.max(cue.start + 0.2) * 1000.0) as u64;
        lines.push(LyricLine {
            start_ms,
            end_ms,
            text: text.to_string(),
            ..LyricLine::default()
        });
    }
    if lines.len() < 2 {
        return None;
    }
    // Captions de YouTube suelen venir con la misma línea repetida en varios
    // cues; también se cortan solapadas. Se deja una línea por texto seguido.
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
    })
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
    cues.push(Cue {
        start,
        end: end.max(start + 0.2),
        text,
        words,
    });
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
        [minutes, seconds] => Some(
            minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?,
        ),
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
        assert_eq!(cues[0].text, "Hola mundo");
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
    fn cue_document_skips_repeated_text() {
        let cues = vec![
            Cue {
                start: 1.0,
                end: 2.0,
                text: "hola".into(),
                ..Cue::default()
            },
            Cue {
                start: 2.0,
                end: 3.0,
                text: "hola".into(),
                ..Cue::default()
            },
            Cue {
                start: 3.0,
                end: 4.0,
                text: "mundo".into(),
                ..Cue::default()
            },
        ];
        let doc = build_doc(
            &TrackRef {
                duration_ms: 10_000,
                ..TrackRef::default()
            },
            None,
            &cues,
        )
        .unwrap();
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.source, LyricsSource::Captions);
    }
}
