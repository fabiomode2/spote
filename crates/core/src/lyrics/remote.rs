//! Red de letras: LRCLIB, primero por metadatos exactos y si falla por búsqueda
//! libre con ranking de candidatos.
//!
//! Todo lo que habla por la red está aquí; el modelo y el parser viven en el
//! módulo padre. Nunca se devuelve un documento vacío: si no hay nada
//! aprovechable, `Ok(None)`.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::Duration;

use serde::Deserialize;

use crate::lyrics::{parse_lrc, LyricLine, LyricsDoc, LyricsError, LyricsSource};
use crate::util::normalize;
use spote_protocol::TrackRef;

/// API pública de LRCLIB (gratis y sin auth).
const API_BASE: &str = "https://lrclib.net/api";
/// Tope por petición: la UI no puede quedarse colgada esperando la letra.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// Score mínimo para aceptar el mejor candidato de `/api/search`.
const MIN_SCORE: f64 = 0.55;
/// Sin un mínimo de título no se acepta nada, aunque duración y artista cuadren.
const MIN_TITLE: f64 = 0.45;
/// Con la duración muy desviada hace falta mucha coincidencia de título+artista.
const MIN_SCORE_FAR: f64 = 0.80;
/// Desviación de duración (ms) a partir de la cual se exige [`MIN_SCORE_FAR`].
const FAR_DURATION_MS: f64 = 8_000.0;
/// Similitud mínima de título cuando la duración no cuadra.
const TITLE_STRICT: f64 = 0.90;
/// Similitud mínima de artista cuando la duración no cuadra.
const ARTIST_STRICT: f64 = 0.90;

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Busca letra sincronizada en LRCLIB, con fallbacks. `Ok(None)` = no hay nada.
pub async fn fetch_track(
    client: &reqwest::Client,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    if track.title.trim().is_empty() {
        return Ok(None);
    }
    // 1) Metadatos exactos: si LRCLIB lo tiene, es el bueno sin ranking.
    match fetch_exact(client, track).await {
        Ok(Some(doc)) => return Ok(Some(doc)),
        // 404, instrumental o sin letra: queda la búsqueda libre.
        Ok(None) => {}
        // Fallo puntual (red, 5xx, JSON roto): la búsqueda aún puede salvarlo.
        Err(_) => {}
    }
    // 2) Búsqueda libre + ranking por título, artista y duración.
    fetch_search(client, track).await
}

// ---------------------------------------------------------------------------
// Ranking de candidatos (lógica pura, testeable sin red)
// ---------------------------------------------------------------------------

/// Candidato de `/api/search` normalizado para poder puntuarlo.
#[derive(Debug, Clone, Default)]
pub(crate) struct Candidate {
    pub id: i64,
    pub track_name: String,
    pub artist_name: String,
    pub album_name: String,
    /// Duración en segundos, tal cual la manda LRCLIB.
    pub duration: f64,
    pub synced: bool,
    pub plain: bool,
    /// Rellenado por [`rank_candidates`].
    pub score: f64,
}

/// Ordena los candidatos de mejor a peor y deja el score calculado en cada uno.
pub(crate) fn rank_candidates(cands: Vec<Candidate>, track: &TrackRef) -> Vec<Candidate> {
    let mut scored: Vec<Candidate> = cands
        .into_iter()
        .map(|mut cand| {
            cand.score = score_candidate(&cand, track);
            cand
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            // Empate: la sincronizada gana a la plana.
            .then_with(|| b.synced.cmp(&a.synced))
    });
    scored
}

/// Puntúa un candidato contra la pista pedida. Devuelve `0..=1`.
///
/// Pesos: título 0.50, artista 0.22, duración 0.28, y ajustes pequeños por
/// llevar letra sincronizada o compartir álbum.
pub(crate) fn score_candidate(cand: &Candidate, track: &TrackRef) -> f64 {
    let title = similarity(&cand.track_name, &track.title);
    let artist = if track.artist.trim().is_empty() {
        // Sin artista de referencia no hay ni premio ni castigo.
        0.5
    } else {
        similarity(&cand.artist_name, &track.artist)
    };
    let duration = duration_score(cand.duration, track.duration_ms);

    let mut score = 0.50 * title + 0.22 * artist + 0.28 * duration;
    if cand.synced {
        // La sincronizada siempre vale más que la plana.
        score += 0.05;
    } else if !cand.plain {
        // Sin letra no sirve de nada aunque el título cuadre al milímetro.
        score *= 0.35;
    }
    // El álbum solo desempata: dos versiones del mismo título suelen compartirlo.
    if !cand.album_name.trim().is_empty() {
        let album = track.album.as_deref().unwrap_or_default();
        if similarity(&cand.album_name, album) >= 0.99 {
            score += 0.03;
        }
    }
    score.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Peticiones
// ---------------------------------------------------------------------------

/// `/api/get` con artista, título, álbum y duración: la vía exacta.
async fn fetch_exact(
    client: &reqwest::Client,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let mut url = format!(
        "{API_BASE}/get?artist_name={}&track_name={}",
        url_encode(&track.artist),
        url_encode(&track.title)
    );
    if let Some(album) = track
        .album
        .as_deref()
        .map(str::trim)
        .filter(|album| !album.is_empty())
    {
        url.push_str("&album_name=");
        url.push_str(&url_encode(album));
    }
    if track.duration_ms > 0 {
        // LRCLIB trabaja en segundos enteros.
        url.push_str(&format!("&duration={}", track.duration_ms / 1000));
    }

    let response = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let raw: RemoteLyrics = response.json().await.map_err(map_error)?;
    Ok(build_doc(&raw, track))
}

/// `/api/search`: lista de candidatos, se puntúan y se queda el mejor.
async fn fetch_search(
    client: &reqwest::Client,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let mut url = format!("{API_BASE}/search?track_name={}", url_encode(&track.title));
    if !track.artist.trim().is_empty() {
        url.push_str("&artist_name=");
        url.push_str(&url_encode(&track.artist));
    }

    let response = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let entries: Vec<RemoteLyrics> = response.json().await.map_err(map_error)?;
    let candidates = entries.iter().map(Candidate::from).collect();
    let ranked = rank_candidates(candidates, track);
    let Some(best) = ranked.first() else {
        return Ok(None);
    };
    if !acceptable(best, track) {
        return Ok(None);
    }
    // El ranking solo lleva metadatos: el texto se recupera del candidato original.
    let Some(raw) = entries.iter().find(|entry| entry.id == best.id) else {
        return Ok(None);
    };
    Ok(build_doc(raw, track))
}

// ---------------------------------------------------------------------------
// Construcción del documento
// ---------------------------------------------------------------------------

/// Prefiere `syncedLyrics`; si no hay, reparte `plainLyrics` sobre la duración.
fn build_doc(raw: &RemoteLyrics, track: &TrackRef) -> Option<LyricsDoc> {
    if let Some(doc) = synced_doc(raw) {
        return Some(doc);
    }
    plain_doc(raw, track)
}

/// Letra con tiempos reales, tal cual viene en LRC.
fn synced_doc(raw: &RemoteLyrics) -> Option<LyricsDoc> {
    let text = raw
        .synced_lyrics
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    let parsed = parse_lrc(text);
    // Si el LRC no trae ni una marca temporal no es sincronizada: que lo intente
    // el camino de letra plana, que al menos la reparte con la duración.
    if !parsed.synced || parsed.is_empty() {
        return None;
    }
    Some(LyricsDoc {
        lines: parsed.lines,
        synced: true,
        instrumental: false,
        source: LyricsSource::Lrclib,
        offset_ms: parsed.offset_ms,
    })
}

/// Letra sin tiempos: líneas repartidas a lo largo de la canción.
fn plain_doc(raw: &RemoteLyrics, track: &TrackRef) -> Option<LyricsDoc> {
    let text = raw
        .plain_lyrics
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            raw.synced_lyrics
                .as_deref()
                .filter(|text| !text.trim().is_empty())
        })?;

    // `parse_lrc` sirve de extractor: se queda con el texto y tira los tags
    // ([Verse], [00:12.34]…) que no aporten tiempo.
    let mut texts: Vec<String> = Vec::new();
    for line in parse_lrc(text).lines {
        let text = line.text.trim();
        if !text.is_empty() {
            texts.push(text.to_string());
        }
    }
    if texts.is_empty() {
        return None;
    }

    let count = texts.len() as u64;
    let duration_ms = effective_duration_ms(raw, track);
    // Sin duración conocida, 4 s por línea como en el parser.
    let total = if duration_ms > 0 {
        duration_ms
    } else {
        count * 4_000
    };
    let mut doc = LyricsDoc {
        lines: Vec::with_capacity(texts.len()),
        synced: false,
        instrumental: false,
        source: LyricsSource::Plain,
        offset_ms: 0,
    };
    for (index, text) in texts.into_iter().enumerate() {
        let index = index as u64;
        doc.lines.push(LyricLine {
            start_ms: index * total / count,
            end_ms: (index + 1) * total / count,
            text,
            ..LyricLine::default()
        });
    }
    Some(doc)
}

/// Duración de referencia: la de la pista manda, la del candidato es el respaldo.
fn effective_duration_ms(raw: &RemoteLyrics, track: &TrackRef) -> u64 {
    if track.duration_ms > 0 {
        return track.duration_ms;
    }
    if raw.duration.is_finite() && raw.duration > 0.0 {
        return (raw.duration * 1000.0) as u64;
    }
    0
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `true` si el mejor candidato merece devolverse.
fn acceptable(best: &Candidate, track: &TrackRef) -> bool {
    if best.score < MIN_SCORE {
        return false;
    }
    // La duración y el artista pueden coincidir por casualidad; el título no.
    if similarity(&best.track_name, &track.title) < MIN_TITLE {
        return false;
    }
    let known = track.duration_ms > 0 && best.duration.is_finite() && best.duration > 0.0;
    if !known {
        return true;
    }
    let diff = (best.duration - track.duration_ms as f64).abs();
    if diff <= FAR_DURATION_MS {
        return true;
    }
    // Duración muy distinta (directo, relleno, vídeo): solo cuela si el título y
    // el artista coinciden casi palabra por palabra.
    best.score >= MIN_SCORE_FAR
        && similarity(&best.track_name, &track.title) >= TITLE_STRICT
        && similarity(&best.artist_name, &track.artist) >= ARTIST_STRICT
}

/// Similitud `0..=1` entre dos textos: Jaccard sobre palabras normalizadas.
fn similarity(a: &str, b: &str) -> f64 {
    let left = normalize(a);
    let right = normalize(b);
    let left = keywords(&left);
    let right = keywords(&right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let common = left.intersection(&right).count();
    let union = left.union(&right).count();
    if union == 0 {
        return 0.0;
    }
    common as f64 / union as f64
}

/// Palabras significativas de un texto ya normalizado (sin colaboraciones).
fn keywords(normalized: &str) -> HashSet<&str> {
    normalized
        .split_whitespace()
        .take_while(|token| !matches!(*token, "feat" | "ft" | "featuring"))
        .collect()
}

/// Cuánto se parece la duración del candidato a la de la pista.
fn duration_score(candidate_seconds: f64, track_ms: u64) -> f64 {
    // Sin una de las dos duraciones no se puede comparar: neutro.
    if track_ms == 0 || !candidate_seconds.is_finite() || candidate_seconds <= 0.0 {
        return 0.5;
    }
    let diff = (candidate_seconds * 1000.0 - track_ms as f64).abs();
    (1.0 - diff / track_ms as f64).clamp(0.0, 1.0)
}

/// Codifica un valor de query: todo lo que no sea unreserved va como `%XX`.
fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Traduce el error de reqwest: decodificar JSON roto no es un fallo de red.
fn map_error(err: reqwest::Error) -> LyricsError {
    if err.is_decode() {
        LyricsError::Parse(err.to_string())
    } else {
        LyricsError::Http(err.to_string())
    }
}

fn has_text(text: Option<&str>) -> bool {
    text.is_some_and(|value| !value.trim().is_empty())
}

// ---------------------------------------------------------------------------
// Respuesta cruda de LRCLIB
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteLyrics {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    album_name: String,
    /// Segundos, con decimales.
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

impl From<&RemoteLyrics> for Candidate {
    fn from(raw: &RemoteLyrics) -> Self {
        Self {
            id: raw.id,
            track_name: raw.track_name.clone(),
            artist_name: raw.artist_name.clone(),
            album_name: raw.album_name.clone(),
            duration: raw.duration,
            synced: has_text(raw.synced_lyrics.as_deref()),
            plain: has_text(raw.plain_lyrics.as_deref()),
            score: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackRef {
        TrackRef {
            key: "isrc:ABC123".into(),
            title: "Do I Wanna Know?".into(),
            artist: "Arctic Monkeys".into(),
            album: Some("AM".into()),
            duration_ms: 272_000,
            ..TrackRef::default()
        }
    }

    fn candidate(id: i64, title: &str, artist: &str, duration: f64, synced: bool) -> Candidate {
        Candidate {
            id,
            track_name: title.into(),
            artist_name: artist.into(),
            album_name: String::new(),
            duration,
            synced,
            plain: true,
            score: 0.0,
        }
    }

    #[test]
    fn ranking_prefers_matching_title_and_duration() {
        let ranked = rank_candidates(
            vec![
                candidate(1, "R U Mine?", "Arctic Monkeys", 200.0, true),
                candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 272.5, true),
                candidate(3, "Do I Wanna Know?", "Some Cover Band", 271.0, true),
            ],
            &track(),
        );
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].id, 2, "el candidato exacto tiene que ganar");
        assert_eq!(ranked[2].id, 1, "otro título se queda al final");
        assert!(ranked[0].score > ranked[1].score);
        assert!(ranked[1].score > ranked[2].score);
        // El ranking deja el score materializado.
        assert!(ranked.iter().all(|cand| cand.score > 0.0));
    }

    #[test]
    fn score_penalizes_far_durations() {
        let near = score_candidate(
            &candidate(1, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            &track(),
        );
        let far = score_candidate(
            &candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 380.0, true),
            &track(),
        );
        assert!(far < near, "{far} debería quedar por debajo de {near}");
    }

    #[test]
    fn score_stays_in_range() {
        let cases = [
            candidate(1, "", "", 0.0, false),
            candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            candidate(
                3,
                "Do I Wanna Know? (Live)",
                "Arctic Monkeys",
                1_000.0,
                false,
            ),
            candidate(4, "Otra cosa", "Otro", f64::NAN, true),
        ];
        for cand in cases {
            let score = score_candidate(&cand, &track());
            assert!(
                (0.0..=1.0).contains(&score),
                "score fuera de rango: {score}"
            );
        }
        // Sin duración de referencia se usa el neutro en vez de inventar.
        let unknown = TrackRef {
            duration_ms: 0,
            ..track()
        };
        let score = score_candidate(
            &candidate(5, "Do I Wanna Know?", "Arctic Monkeys", 0.0, true),
            &unknown,
        );
        assert!((0.0..=1.0).contains(&score));
    }

    #[test]
    fn unrelated_title_is_rejected_even_with_right_duration() {
        // Mismo artista y misma duración, pero el título no tiene nada que ver.
        let impostor = candidate(1, "R U Mine?", "Arctic Monkeys", 272.0, true);
        let impostor = Candidate {
            score: score_candidate(&impostor, &track()),
            ..impostor
        };
        assert!(!acceptable(&impostor, &track()));

        let real = candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true);
        let real = Candidate {
            score: score_candidate(&real, &track()),
            ..real
        };
        assert!(acceptable(&real, &track()));
    }

    #[test]
    fn noisy_title_scores_like_clean_one() {
        let clean = score_candidate(
            &candidate(1, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            &track(),
        );
        let noisy = score_candidate(
            &candidate(
                2,
                "Do I Wanna Know? (Official Video)",
                "Arctic Monkeys",
                272.0,
                true,
            ),
            &track(),
        );
        assert!((clean - noisy).abs() < 1e-9, "{clean} != {noisy}");
    }

    #[test]
    fn url_encode_escapes_everything_but_unreserved() {
        assert_eq!(url_encode("AC/DC"), "AC%2FDC");
        assert_eq!(url_encode("Déjà Vu"), "D%C3%A9j%C3%A0%20Vu");
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(url_encode("  padded  "), "padded");
    }

    #[test]
    fn plain_lyrics_are_spread_over_duration() {
        let raw = RemoteLyrics {
            id: 7,
            track_name: "Do I Wanna Know?".into(),
            artist_name: "Arctic Monkeys".into(),
            album_name: "AM".into(),
            duration: 272.0,
            plain_lyrics: Some("one\n\ntwo\nthree".into()),
            synced_lyrics: None,
        };
        let doc = build_doc(&raw, &track()).expect("hay letra plana");
        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
        assert_eq!(doc.lines.len(), 3, "las líneas vacías se descartan");
        assert_eq!(doc.lines[0].start_ms, 0);
        assert_eq!(doc.lines[0].end_ms, doc.lines[1].start_ms);
        assert_eq!(doc.lines[2].end_ms, 272_000);
    }

    #[test]
    fn synced_lyrics_win_over_plain() {
        let raw = RemoteLyrics {
            id: 8,
            track_name: "Do I Wanna Know?".into(),
            artist_name: "Arctic Monkeys".into(),
            album_name: "AM".into(),
            duration: 272.0,
            plain_lyrics: Some("plana".into()),
            synced_lyrics: Some("[00:12.30] Have you got colour in your cheeks?".into()),
        };
        let doc = build_doc(&raw, &track()).expect("hay letra sincronizada");
        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Lrclib);
        assert_eq!(doc.lines[0].start_ms, 12_300);
    }
}
