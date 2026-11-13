//! Red de letras: LRCLIB, primero por metadatos exactos y si falla por búsqueda
//! libre con ranking de candidatos.
//!
//! Todo lo que habla por la red está aquí; el modelo y el parser viven en el
//! módulo padre. Nunca se devuelve un documento vacío: si no hay nada
//! aprovechable, `Ok(None)`.
//!
//! Las pistas descargadas de YouTube son el caso difícil: el título lleva
//! "Official Video", el artista es un canal y el vídeo tiene intro/outro. Por
//! eso se prueban varias variantes limpias del título/artista, se puntúan todos
//! los candidatos (título, artista, duración y tipo de versión) y se descarta
//! cualquier LRC sincronizado con tiempos imposibles.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::text::{
    artist_similarity, clean_artist, clean_title, primary_artist, similarity, split_artist_title,
    version_kind,
};
use crate::lyrics::{parse_lrc, LyricLine, LyricsDoc, LyricsError, LyricsSource};
use spote_protocol::TrackRef;

/// API pública de LRCLIB (gratis y sin auth).
const API_BASE: &str = "https://lrclib.net/api";
/// Tope por petición: la UI no puede quedarse colgada esperando la letra.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// Presupuesto total de la búsqueda: con varias variantes no compensa esperar
/// más, y menos cuando la red está muerta y cada petición agota su timeout.
const TOTAL_BUDGET: Duration = Duration::from_secs(15);
/// Score mínimo para aceptar el mejor candidato de `/api/search`.
const MIN_SCORE: f64 = 0.55;
/// Sin un mínimo de título no se acepta nada, aunque duración y artista cuadren.
const MIN_TITLE: f64 = 0.45;
/// Con artista conocido, un candidato de otro artista no vale.
const MIN_ARTIST: f64 = 0.34;
/// Con la duración muy desviada hace falta mucha coincidencia de título+artista.
const MIN_SCORE_FAR: f64 = 0.80;
/// Desviación de duración (ms) a partir de la cual se exige [`MIN_SCORE_FAR`].
const FAR_DURATION_MS: f64 = 8_000.0;
/// Similitud mínima de título cuando la duración no cuadra.
const TITLE_STRICT: f64 = 0.90;
/// Similitud mínima de artista cuando la duración no cuadra.
const ARTIST_STRICT: f64 = 0.90;

/// Primer verso más tarde que esto es sospechoso: intro eterna o LRC de otro máster.
const SYNC_MAX_FIRST_MS: u64 = 30_000;
/// Sin duración conocida se usa un tope absoluto, más generoso.
const SYNC_MAX_FIRST_UNKNOWN_MS: u64 = 60_000;
/// Hueco máximo entre dos versos con texto (un puente de 90 s ya no cuela).
const SYNC_MAX_GAP_MS: u64 = 90_000;
/// Margen por el que el último verso puede pasarse del final de la pista.
const SYNC_MAX_OVERRUN_MS: u64 = 15_000;
/// Mínimo de versos con texto para considerar fiable una letra sincronizada.
const SYNC_MIN_LINES: usize = 2;

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
    let variants = query_variants(track);
    let deadline = Instant::now() + TOTAL_BUDGET;
    // 1) Metadatos exactos: si LRCLIB lo tiene, es el bueno sin ranking.
    for variant in &variants {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        if let Ok(Some(doc)) = fetch_exact(client, variant).await {
            return Ok(Some(doc));
        }
    }
    // 2) Búsqueda libre + ranking por título, artista, duración y versión.
    //    Las variantes van de más a menos específica y la última es solo título,
    //    que es lo que salva los artistas colaborativos y los canales raros.
    for variant in &variants {
        if Instant::now() >= deadline {
            break;
        }
        if let Ok(Some(doc)) = fetch_search(client, variant).await {
            return Ok(Some(doc));
        }
    }
    Ok(None)
}

/// Variantes de la pista para probar, de la más específica a la más laxa.
///
/// Se deduplican por título+artista para no repetir peticiones.
fn query_variants(track: &TrackRef) -> Vec<TrackRef> {
    let mut out: Vec<TrackRef> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut push = |candidate: TrackRef| {
        let key = (
            candidate.title.trim().to_lowercase(),
            candidate.artist.trim().to_lowercase(),
        );
        if key.0.is_empty() || !seen.insert(key) {
            return;
        }
        out.push(candidate);
    };

    push(track.clone());

    let title = clean_title(&track.title);
    let title = if title.is_empty() {
        track.title.clone()
    } else {
        title
    };
    let artist = clean_artist(&track.artist);
    let album = track
        .album
        .as_deref()
        .map(clean_title)
        .filter(|album| !album.is_empty());

    // "Artista - Tema": típico de YouTube, donde el título arrastra al artista
    // (o al revés). Se queda el lado que no es el artista como título.
    let mut split_title: Option<String> = None;
    if let Some((left, right)) = split_artist_title(&track.title) {
        let known = if track.artist.trim().is_empty() {
            artist.clone()
        } else {
            track.artist.clone()
        };
        let looks_like_artist = !known.trim().is_empty() && similarity(left, &known) >= 0.6;
        let right_looks_like_artist = !known.trim().is_empty() && similarity(right, &known) >= 0.6;
        let (artist_part, title_part) = if looks_like_artist {
            (Some(left), right)
        } else if right_looks_like_artist {
            (Some(right), left)
        } else {
            (None, "")
        };
        let clean = clean_title(title_part);
        if !clean.is_empty() {
            let mut split = track.clone();
            split.title = clean.clone();
            split.artist = artist_part
                .map(clean_artist)
                .filter(|part| !part.is_empty())
                .unwrap_or_else(|| artist.clone());
            split.album = album.clone();
            push(split);
            split_title = Some(clean);
        }
    }

    let mut cleaned = track.clone();
    cleaned.title = title.clone();
    cleaned.artist = artist.clone();
    cleaned.album = album;
    push(cleaned.clone());

    // Artista principal de una colaboración ("A, B" → "A").
    let primary = primary_artist(&artist);
    if !primary.is_empty() && primary != artist {
        let mut solo = cleaned.clone();
        solo.artist = primary.clone();
        push(solo);

        let mut both = track.clone();
        both.artist = primary;
        push(both);
    }

    // Último recurso: solo título, sin artista ni álbum.
    let mut title_only = track.clone();
    title_only.title = split_title.unwrap_or(title);
    title_only.artist.clear();
    title_only.album = None;
    push(title_only);

    out
}

// ---------------------------------------------------------------------------
// Ranking de candidatos (lógica pura, testeable sin red)
// ---------------------------------------------------------------------------

/// Candidato de `/api/search` normalizado para poder puntuarlo.
#[derive(Debug, Clone, Default)]
pub(crate) struct Candidate {
    /// Posición en la respuesta de LRCLIB, para recuperar el texto original.
    pub source_index: usize,
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

impl Candidate {
    fn from_parts(source_index: usize, raw: &RemoteLyrics) -> Self {
        Self {
            source_index,
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
            // Empate: la sincronizada gana a la plana y, si no, el orden de LRCLIB.
            .then_with(|| b.synced.cmp(&a.synced))
            .then_with(|| a.source_index.cmp(&b.source_index))
    });
    scored
}

/// Puntúa un candidato contra la pista pedida. Devuelve `0..=1`.
///
/// Pesos: título 0.48, artista 0.24, duración 0.28; bonus por llevar letra
/// sincronizada y castigo por ser una versión distinta (directo, remix, cover,
/// karaoke, sped up…) cuando se pidió la de estudio.
pub(crate) fn score_candidate(cand: &Candidate, track: &TrackRef) -> f64 {
    let title = title_similarity(&cand.track_name, track);
    let artist = if track.artist.trim().is_empty() {
        // Sin artista de referencia no hay ni premio ni castigo.
        0.5
    } else {
        artist_similarity(&cand.artist_name, &track.artist)
    };
    let duration = duration_score(cand.duration, track.duration_ms);

    let mut score = 0.48 * title + 0.24 * artist + 0.28 * duration;

    // Versión: un directo o un remix no es la misma letra, y menos con tiempos.
    let requested = version_kind(&track.title, track.album.as_deref());
    let offered = version_kind(&cand.track_name, Some(&cand.album_name));
    if requested != offered {
        score -= if requested == super::text::VersionKind::Studio {
            0.22
        } else {
            0.12
        };
    }

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

/// `true` si el candidato merece devolverse.
fn acceptable(best: &Candidate, track: &TrackRef) -> bool {
    if best.score < MIN_SCORE {
        return false;
    }
    // La duración y el artista pueden coincidir por casualidad; el título no.
    if title_similarity(&best.track_name, track) < MIN_TITLE {
        return false;
    }
    if !track.artist.trim().is_empty()
        && !best.artist_name.trim().is_empty()
        && artist_similarity(&best.artist_name, &track.artist) < MIN_ARTIST
    {
        return false;
    }
    // Karaoke, instrumental o versión de otro artista: la letra no es la pedida.
    let requested = version_kind(&track.title, track.album.as_deref());
    let offered = version_kind(&best.track_name, Some(&best.album_name));
    if offered.is_incompatible() && offered != requested {
        return false;
    }
    let known = track.duration_ms > 0 && best.duration.is_finite() && best.duration > 0.0;
    if !known {
        return true;
    }
    // LRCLIB manda la duración en segundos: se pasa a ms antes de comparar.
    let diff = (best.duration * 1000.0 - track.duration_ms as f64).abs();
    if diff <= FAR_DURATION_MS {
        return true;
    }
    // Duración muy distinta (directo, relleno, vídeo): solo cuela si el título y
    // el artista coinciden casi palabra por palabra.
    let artist_ok = track.artist.trim().is_empty()
        || best.artist_name.trim().is_empty()
        || artist_similarity(&best.artist_name, &track.artist) >= ARTIST_STRICT;
    best.score >= MIN_SCORE_FAR
        && title_similarity(&best.track_name, track) >= TITLE_STRICT
        && artist_ok
}

/// Similitud de título teniendo en cuenta que el título pedido puede arrastrar
/// el nombre del artista ("Artista - Tema", típico de YouTube).
fn title_similarity(candidate_title: &str, track: &TrackRef) -> f64 {
    let mut best = similarity(candidate_title, &track.title);
    let Some((left, right)) = split_artist_title(&track.title) else {
        return best;
    };
    let known = if track.artist.trim().is_empty() {
        String::new()
    } else {
        track.artist.clone()
    };
    if known.trim().is_empty() {
        return best;
    }
    // El lado que se parece al artista es ruido; el otro es el título de verdad.
    if similarity(left, &known) >= 0.6 {
        best = best.max(similarity(candidate_title, right));
    } else if similarity(right, &known) >= 0.6 {
        best = best.max(similarity(candidate_title, left));
    }
    best
}

/// Cuánto se parece la duración del candidato a la de la pista.
///
/// Por tramos: el vídeo suele añadir unos segundos de intro/outro y eso no debe
/// pesar como si fuera otra canción.
fn duration_score(candidate_seconds: f64, track_ms: u64) -> f64 {
    // Sin una de las dos duraciones no se puede comparar: neutro.
    if track_ms == 0 || !candidate_seconds.is_finite() || candidate_seconds <= 0.0 {
        return 0.5;
    }
    let diff = (candidate_seconds * 1000.0 - track_ms as f64).abs();
    match diff {
        diff if diff <= 2_000.0 => 1.0,
        diff if diff <= 5_000.0 => 0.9,
        diff if diff <= 10_000.0 => 0.75,
        diff if diff <= 20_000.0 => 0.5,
        diff if diff <= 40_000.0 => 0.25,
        diff => (1.0 - diff / track_ms as f64).clamp(0.0, 0.2),
    }
}

// ---------------------------------------------------------------------------
// Peticiones
// ---------------------------------------------------------------------------

/// `/api/get` con artista, título, álbum y duración: la vía exacta.
async fn fetch_exact(
    client: &reqwest::Client,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    if track.artist.trim().is_empty() || track.title.trim().is_empty() {
        return Ok(None);
    }
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
    let candidates = entries
        .iter()
        .enumerate()
        .map(|(index, raw)| Candidate::from_parts(index, raw))
        .collect();
    let ranked = rank_candidates(candidates, track);
    // Del mejor al peor: el primero que dé un documento usable gana. Nunca se
    // coge el primero de la lista sin puntuar.
    for best in &ranked {
        if !acceptable(best, track) {
            break;
        }
        // El ranking solo lleva metadatos: el texto se recupera del original.
        let Some(raw) = entries.get(best.source_index) else {
            continue;
        };
        if let Some(doc) = build_doc(raw, track) {
            tracing::debug!(
                target: "spote::lyrics",
                "lrclib: id={} '{} - {}' score={:.2} synced={}",
                best.id,
                best.artist_name,
                best.track_name,
                best.score,
                best.synced
            );
            return Ok(Some(doc));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Construcción del documento
// ---------------------------------------------------------------------------

/// Prefiere `syncedLyrics`; si no hay (o sus tiempos no son de fiar), reparte
/// `plainLyrics` sobre la duración.
fn build_doc(raw: &RemoteLyrics, track: &TrackRef) -> Option<LyricsDoc> {
    let duration_ms = effective_duration_ms(raw, track);
    if let Some(doc) = synced_doc(raw, duration_ms) {
        return Some(doc);
    }
    plain_doc(raw, track)
}

/// Letra con tiempos reales, tal cual viene en LRC. `None` si el LRC no trae
/// tiempos o si los trae imposibles (ver [`sync_problem`]).
fn synced_doc(raw: &RemoteLyrics, duration_ms: u64) -> Option<LyricsDoc> {
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
    if let Some(reason) = sync_problem(&parsed, duration_ms) {
        tracing::debug!(
            target: "spote::lyrics",
            "lrclib: sync descartada ({reason}) para {}",
            raw.track_name
        );
        return None;
    }
    Some(LyricsDoc {
        lines: parsed.lines,
        synced: true,
        instrumental: parsed.instrumental,
        source: LyricsSource::Lrclib,
        offset_ms: parsed.offset_ms,
    })
}

/// Motivo por el que un LRC sincronizado no es de fiar, si lo hay.
///
/// Detecta lo típico de una letra que no es de esta canción o que viene de otro
/// máster: primer verso tardísimo, versos más allá del final de la pista y
/// huecos de minuto y medio sin nada.
pub(crate) fn sync_problem(doc: &LyricsDoc, duration_ms: u64) -> Option<&'static str> {
    let lines: Vec<&LyricLine> = doc
        .lines
        .iter()
        .filter(|line| has_lyric_text(&line.text))
        .collect();
    if lines.len() < SYNC_MIN_LINES {
        return Some("menos de dos versos con texto");
    }
    let first = lines[0].start_ms;
    let last = lines[lines.len() - 1].start_ms;
    if duration_ms == 0 {
        if first > SYNC_MAX_FIRST_UNKNOWN_MS {
            return Some("primer verso demasiado tarde");
        }
    } else {
        if first > duration_ms {
            return Some("primer verso después del final de la pista");
        }
        if last > duration_ms + SYNC_MAX_OVERRUN_MS {
            return Some("versos por debajo del final de la pista");
        }
        // Un cuarto de la canción en silencio antes del primer verso no cuadra.
        if first > SYNC_MAX_FIRST_MS && first > duration_ms / 4 {
            return Some("primer verso demasiado tarde");
        }
    }
    for pair in lines.windows(2) {
        if pair[1].start_ms.saturating_sub(pair[0].start_ms) > SYNC_MAX_GAP_MS {
            return Some("hueco enorme entre versos");
        }
    }
    None
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
    // ([Verse], [00:12.34]…) y las marcas instrumentales que no aportan nada.
    let mut texts: Vec<String> = Vec::new();
    for line in parse_lrc(text).lines {
        let text = line.text.trim();
        if !text.is_empty() && text != "♪" {
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

/// `true` si el texto es letra de verdad y no una marca instrumental.
fn has_lyric_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed != "♪" && trimmed.chars().any(|ch| ch.is_alphanumeric())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::build_test_doc;

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

    fn youtube_track() -> TrackRef {
        TrackRef {
            key: "yt:abc".into(),
            title: "Arctic Monkeys - Do I Wanna Know? (Official Video)".into(),
            artist: "Arctic Monkeys - Topic".into(),
            album: None,
            duration_ms: 290_000,
            ..TrackRef::default()
        }
    }

    fn candidate(id: i64, title: &str, artist: &str, duration: f64, synced: bool) -> Candidate {
        Candidate {
            source_index: id as usize,
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

    fn scored(mut cand: Candidate, track: &TrackRef) -> Candidate {
        cand.score = score_candidate(&cand, track);
        cand
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
    fn best_version_wins_over_the_first_one() {
        // El primero de la lista es un directo: no puede ganar a la de estudio.
        let ranked = rank_candidates(
            vec![
                candidate(
                    1,
                    "Do I Wanna Know? - Live at Reading",
                    "Arctic Monkeys",
                    285.0,
                    true,
                ),
                candidate(
                    2,
                    "Do I Wanna Know? (Karaoke Version)",
                    "Karaoke Hits",
                    272.0,
                    true,
                ),
                candidate(3, "Do I Wanna Know? (Sped Up)", "nightcore", 230.0, true),
                candidate(4, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            ],
            &track(),
        );
        assert_eq!(ranked[0].id, 4, "la versión de estudio es la buena");
        let studio = ranked[0].score;
        assert!(
            ranked[1..].iter().all(|cand| cand.score < studio),
            "{:?}",
            ranked.iter().map(|c| (c.id, c.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn noisy_youtube_metadata_finds_the_same_candidate() {
        let ranked = rank_candidates(
            vec![
                candidate(1, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
                candidate(2, "Do I Wanna Know?", "Some Other Band", 271.0, true),
            ],
            &youtube_track(),
        );
        assert_eq!(ranked[0].id, 1);
        assert!(acceptable(&ranked[0], &youtube_track()));
    }

    #[test]
    fn karaoke_and_live_are_rejected_for_a_studio_request() {
        let karaoke = scored(
            candidate(
                1,
                "Do I Wanna Know? (Karaoke Version)",
                "Arctic Monkeys",
                272.0,
                true,
            ),
            &track(),
        );
        assert!(!acceptable(&karaoke, &track()));

        // Un directo del mismo artista y duración parecida sí se acepta: la
        // letra es la misma, solo pierde en el ranking.
        let live = scored(
            candidate(2, "Do I Wanna Know? (Live)", "Arctic Monkeys", 272.0, true),
            &track(),
        );
        assert!(acceptable(&live, &track()));
        assert!(
            live.score
                < scored(
                    candidate(3, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
                    &track()
                )
                .score
        );
    }

    #[test]
    fn other_artist_with_the_same_title_is_rejected() {
        let impostor = scored(
            candidate(1, "Do I Wanna Know?", "Some Cover Band", 272.0, true),
            &track(),
        );
        assert!(similarity(&impostor.track_name, &track().title) >= MIN_TITLE);
        assert!(!acceptable(&impostor, &track()));
    }

    #[test]
    fn title_only_search_accepts_by_duration() {
        // Regresión: LRCLIB manda la duración en segundos y la pista en ms. Con
        // las unidades mal comparadas, este candidato caía en el camino
        // estricto y se descartaba siempre.
        let mut sin_artista = track();
        sin_artista.artist.clear();
        let near = scored(
            candidate(1, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            &sin_artista,
        );
        assert!(
            acceptable(&near, &sin_artista),
            "sin artista manda título+duración"
        );

        // Una versión 32 s más corta no es la misma grabación.
        let far = scored(
            candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 240.0, true),
            &sin_artista,
        );
        assert!(!acceptable(&far, &sin_artista));
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
    fn variants_clean_youtube_metadata_and_keep_order() {
        let variants = query_variants(&youtube_track());
        // Cruda, limpia y solo título: la primera y la última son las importantes.
        assert_eq!(variants[0].title, youtube_track().title);
        assert!(variants.iter().any(
            |variant| variant.title == "Do I Wanna Know?" && variant.artist == "Arctic Monkeys"
        ));
        let last = variants.last().expect("hay variantes");
        assert!(last.artist.is_empty(), "{last:?}");
        assert_eq!(last.title, "Do I Wanna Know?");
        // Sin duplicados.
        let mut seen = HashSet::new();
        for variant in &variants {
            assert!(seen.insert((variant.title.to_lowercase(), variant.artist.to_lowercase())));
        }
    }

    #[test]
    fn variants_split_artist_and_title() {
        let mut input = track();
        input.title = "Arctic Monkeys - Do I Wanna Know?".into();
        let variants = query_variants(&input);
        assert!(variants
            .iter()
            .any(|variant| variant.title == "Do I Wanna Know?"));
    }

    #[test]
    fn valid_sync_is_accepted() {
        let doc = build_test_doc(&[(0, "uno"), (12_000, "dos"), (24_000, "tres")]);
        assert_eq!(sync_problem(&doc, 272_000), None);
        // Sin duración conocida tampoco hay problema.
        assert_eq!(sync_problem(&doc, 0), None);
    }

    #[test]
    fn sync_is_rejected_when_the_first_line_is_way_too_late() {
        let doc = build_test_doc(&[(45_000, "uno"), (60_000, "dos"), (70_000, "tres")]);
        assert_eq!(
            sync_problem(&doc, 100_000),
            Some("primer verso demasiado tarde")
        );
        // Sin duración, un primer verso a 75 s también es sospechoso.
        let late = build_test_doc(&[(75_000, "uno"), (90_000, "dos")]);
        assert!(sync_problem(&late, 0).is_some());
    }

    #[test]
    fn sync_is_rejected_when_lines_fall_outside_the_track() {
        let doc = build_test_doc(&[(10_000, "uno"), (400_000, "dos")]);
        assert_eq!(
            sync_problem(&doc, 272_000),
            Some("versos por debajo del final de la pista")
        );
    }

    #[test]
    fn sync_is_rejected_on_giant_gaps() {
        let doc = build_test_doc(&[(10_000, "uno"), (200_000, "dos")]);
        assert_eq!(
            sync_problem(&doc, 260_000),
            Some("hueco enorme entre versos")
        );
    }

    #[test]
    fn sync_is_rejected_with_a_single_line() {
        let doc = build_test_doc(&[(10_000, "uno")]);
        assert!(sync_problem(&doc, 200_000).is_some());
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
            synced_lyrics: Some(
                "[00:12.30] Have you got colour in your cheeks?\n\
                 [00:18.90] Do you ever get that fear that you can't shift the tide"
                    .into(),
            ),
        };
        let doc = build_doc(&raw, &track()).expect("hay letra sincronizada");
        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Lrclib);
        assert_eq!(doc.lines[0].start_ms, 12_300);
    }

    #[test]
    fn absurd_sync_falls_back_to_plain_lyrics() {
        let raw = RemoteLyrics {
            id: 9,
            track_name: "Do I Wanna Know?".into(),
            artist_name: "Arctic Monkeys".into(),
            album_name: "AM".into(),
            duration: 272.0,
            plain_lyrics: Some("uno\ndos\ntres".into()),
            synced_lyrics: Some("[00:45.00] uno\n[02:40.00] dos\n[05:00.00] tres".into()),
        };
        let doc = build_doc(&raw, &track()).expect("cae a letra plana");
        assert!(!doc.synced, "la sync con tiempos imposibles se descarta");
        assert_eq!(doc.source, LyricsSource::Plain);
    }
}
