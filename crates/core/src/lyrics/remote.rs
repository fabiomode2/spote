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
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::text::{
    artist_similarity, clean_artist, clean_title, is_curator_artist, is_youtube_provider,
    primary_artist, search_title, similarity, split_artist_title, starts_with_version_marker,
    version_kind, VersionKind,
};
use crate::lyrics::{parse_lrc, LyricLine, LyricsCandidate, LyricsDoc, LyricsError, LyricsSource};
use spote_protocol::TrackRef;

/// API pública de LRCLIB (gratis y sin auth).
const API_BASE: &str = "https://lrclib.net/api";
/// Tope por petición: la UI no puede quedarse colgada esperando la letra.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(7);
/// Presupuesto total de la búsqueda: con varias variantes no compensa esperar
/// más, y menos cuando la red está muerta y cada petición agota su timeout.
const TOTAL_BUDGET: Duration = Duration::from_secs(24);
/// Tramos por proveedor. Ninguno puede comerse el presupuesto de los demás:
/// antes lrcmux/Genius/YouTube podían sumar más de un minuto de espera.
const LRCLIB_BUDGET: Duration = Duration::from_secs(9);
const LRCMUX_BUDGET: Duration = Duration::from_secs(6);
const GENIUS_BUDGET: Duration = Duration::from_secs(5);
const YOUTUBE_BUDGET: Duration = Duration::from_secs(12);
/// Por debajo de esto no se lanza una petición condenada a expirar.
const MIN_REQUEST_BUDGET: Duration = Duration::from_millis(700);
/// Score mínimo para aceptar el mejor candidato de `/api/search`.
const MIN_SCORE: f64 = 0.55;
/// Sin un mínimo de título no se acepta nada, aunque duración y artista cuadren.
const MIN_TITLE: f64 = 0.50;
/// Títulos muy cortos ("Love", "Halo") exigen más parecido: un candidato tipo
/// "Love Story" comparte una palabra y no es la misma canción.
const GENERIC_TITLE: f64 = 0.72;
/// Con artista conocido, un candidato de otro artista no vale.
const MIN_ARTIST: f64 = 0.34;
/// Desviación de duración (ms) a partir de la cual se descarta el candidato.
/// Es otra grabación (directo, remix, versión de vídeo) y sus tiempos no sirven.
const FAR_DURATION_MS: f64 = 12_000.0;
/// YouTube añade intro/outro y silencios de vídeo: se tolera algo más.
const YOUTUBE_FAR_DURATION_MS: f64 = 35_000.0;

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

/// Parecido mínimo (Dice sobre palabras) entre el texto de Genius y el de un
/// proveedor sincronizado para aceptar sus tiempos. Por debajo se considera
/// otra letra y se prefiere el texto de Genius sin sincronizar antes que una
/// letra ajena con karaoke.
const GENIUS_TEXT_MATCH: f64 = 0.55;
/// Parecido mínimo entre dos líneas para anclarlas durante la alineación.
const LINE_MATCH_MIN: f64 = 0.55;
/// Fracción mínima de líneas de Genius que deben quedar ancladas a un tiempo.
const MIN_TEXT_COVERAGE: f64 = 0.28;

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Busca letra con esta prioridad:
///
/// 1. **Genius primero** y autoritativo en texto, con su propio tramo de
///    presupuesto. Si devuelve letra, ese texto es el que se pinta: LRCLIB,
///    lrcmux y las captions solo pueden prestarle tiempos.
/// 2. Sin Genius, LRCLIB y lrcmux siguen siendo fuentes de texto fiables. Un
///    LRC sincronizado se devuelve tal cual; uno plano queda como respaldo.
/// 3. Las captions nunca aportan texto: solo alinean una letra ya conocida
///    (Genius, LRCLIB o lrcmux) contra el audio del vídeo. Sin letra conocida
///    no se consultan, porque las auto-captions pueden inventar líneas.
///
/// Cuando Genius es plano y un proveedor trae LRC sincronizado, se comprueba
/// que el texto coincida (Dice sobre palabras y alineación por líneas). Si
/// coincide, se conservan las líneas de Genius y se les aplican esos tiempos;
/// si no, se devuelve Genius plano antes que una letra sincronizada de otra
/// canción. El ASR, más abajo, solo puede reafinar esos mismos tiempos.
pub async fn fetch_track(
    client: &reqwest::Client,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    if track.title.trim().is_empty() {
        return Ok(None);
    }
    let target = MatchTarget::from_track(track);
    let variants = query_variants(track);
    let deadline = Instant::now() + TOTAL_BUDGET;
    let prefer_captions = youtube_hint(track).is_some();

    // 1) Genius: texto autoritativo. Se consulta antes que nadie para que un
    //    LRC de otra versión no pueda decidir qué letra se muestra.
    let mut genius = {
        let genius_deadline = slice(deadline, GENIUS_BUDGET);
        if has_budget(genius_deadline, MIN_REQUEST_BUDGET) {
            match fetch_genius(client, track, &target, genius_deadline).await {
                Ok(doc) => doc,
                Err(err) => {
                    tracing::debug!(target: "spote::lyrics", "genius: {err}");
                    None
                }
            }
        } else {
            None
        }
    };
    if let Some(doc) = &genius {
        tracing::info!(
            target: "spote::lyrics",
            "genius: texto autoritativo, {} líneas (synced={})",
            doc.lines.len(),
            doc.synced
        );
    }

    // 2) Tiempos y, si no hay Genius, texto de los proveedores fiables.
    let lrclib_deadline = slice(deadline, LRCLIB_BUDGET);
    let (mut timing, mut fallback) =
        fetch_lrclib(client, &variants, &target, lrclib_deadline).await;
    if timing.is_none() {
        let lrcmux_deadline = slice(deadline, LRCMUX_BUDGET);
        if has_budget(lrcmux_deadline, MIN_REQUEST_BUDGET) {
            // Se consulta con la variante limpia: un título de YouTube con
            // "Official Video" no encaja en el endpoint compatible.
            let query = variants.first().unwrap_or(track);
            if let Ok(Some(doc)) = super::lrcmux::fetch_before(client, query, lrcmux_deadline).await
            {
                if doc.synced {
                    timing = Some(doc);
                } else if genius.is_none() && fallback.is_none() {
                    fallback = Some(doc);
                }
            }
        }
    }

    // Sin Genius y con tiempos ya resueltos, no hace falta YouTube salvo que
    // la pista venga de un vídeo exacto: ahí las captions ajustan mejor.
    if !prefer_captions {
        // Genius manda: si hay tiempos de proveedor, se mezclan ya y se evita
        // gastar el tramo de YouTube.
        if genius.is_some() && timing.is_some() {
            let genius = genius.take().expect("genius comprobado");
            let timing = timing.take();
            let (doc, resolution) = resolve_genius(genius, None, timing);
            log_resolution(&doc, resolution);
            return Ok(Some(doc));
        }
        if genius.is_none() {
            if let Some(doc) = timing.take() {
                tracing::info!(
                    target: "spote::lyrics",
                    "letra: {} sincronizada (sin genius)",
                    doc.source.label()
                );
                return Ok(Some(doc));
            }
        }
    } else if genius.is_some() {
        if let Some(timing) = timing.as_ref() {
            tracing::debug!(
                target: "spote::lyrics",
                "genius: tiempos candidatos de {}; se intentan captions del vídeo",
                timing.source.label()
            );
        }
    }

    // 3) Captions: solo con letra conocida y solo para poner tiempos. Nunca
    //    se usan como fuente de texto, ni siquiera como último recurso.
    let known = genius.as_ref().or(timing.as_ref()).or(fallback.as_ref());
    let known_source = known.map(|doc| doc.source.label()).unwrap_or("");
    let mut captions = None;
    if known.is_some() {
        let youtube_deadline = slice(deadline, YOUTUBE_BUDGET);
        if has_budget(youtube_deadline, Duration::from_secs(2)) {
            captions = super::youtube::fetch(track, known, youtube_deadline).await;
        }
    }

    if let Some(genius) = genius {
        let (doc, resolution) = resolve_genius(genius, captions, timing);
        log_resolution(&doc, resolution);
        return Ok(Some(doc));
    }
    if let Some(doc) = captions {
        tracing::info!(
            target: "spote::lyrics",
            "letra: captions aportan tiempos sobre texto de {known_source}"
        );
        return Ok(Some(doc));
    }
    let doc = timing.or(fallback);
    if let Some(doc) = &doc {
        tracing::info!(
            target: "spote::lyrics",
            "letra: {} (sin genius ni captions)",
            doc.source.label()
        );
    } else {
        tracing::info!(
            target: "spote::lyrics",
            "letra: sin texto fiable; no se usan captions como letra"
        );
    }
    Ok(doc)
}

/// Busca candidatos libres en LRCLIB para elegir a mano. Devuelve todos los
/// que tengan texto, en el orden del proveedor y sin filtro de duración.
pub(crate) async fn search_candidates(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<LyricsCandidate>, LyricsError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!("{API_BASE}/search?q={}", url_encode(query));
    let response = client
        .get(url)
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Ok(Vec::new());
    }
    let entries: Vec<RemoteLyrics> = response.json().await.map_err(map_error)?;
    let mut out = Vec::new();
    for raw in entries {
        if !has_text(raw.synced_lyrics.as_deref()) && !has_text(raw.plain_lyrics.as_deref()) {
            continue;
        }
        out.push(LyricsCandidate {
            id: raw.id,
            title: raw.track_name,
            artist: raw.artist_name,
            album: raw.album_name,
            duration_ms: (raw.duration.max(0.0) * 1000.0).round() as u64,
            synced: has_text(raw.synced_lyrics.as_deref()),
            source: "lrclib".to_string(),
            lrc: raw.synced_lyrics.filter(|text| !text.trim().is_empty()),
            plain: raw.plain_lyrics.filter(|text| !text.trim().is_empty()),
        });
        if out.len() >= 14 {
            break;
        }
    }
    Ok(out)
}

/// Recorre las variantes de LRCLIB (metadatos exactos y luego búsqueda libre)
/// y devuelve el primer documento sincronizado y el primer texto plano. Se
/// detiene en cuanto encuentra una sincronizada: es la que mejores tiempos da
/// y no compensa seguir gastando presupuesto.
async fn fetch_lrclib(
    client: &reqwest::Client,
    variants: &[TrackRef],
    target: &MatchTarget,
    deadline: Instant,
) -> (Option<LyricsDoc>, Option<LyricsDoc>) {
    let mut synced: Option<LyricsDoc> = None;
    let mut plain: Option<LyricsDoc> = None;

    for variant in variants {
        if !has_budget(deadline, MIN_REQUEST_BUDGET) {
            break;
        }
        if let Ok(Some(doc)) = fetch_exact(client, variant, target, deadline).await {
            if doc.synced {
                synced = Some(doc);
                break;
            }
            if plain.is_none() {
                plain = Some(doc);
            }
        }
    }
    if synced.is_none() {
        for variant in variants {
            if !has_budget(deadline, MIN_REQUEST_BUDGET) {
                break;
            }
            if let Ok(Some(doc)) = fetch_search(client, variant, target, deadline).await {
                if doc.synced {
                    synced = Some(doc);
                    break;
                }
                if plain.is_none() {
                    plain = Some(doc);
                }
            }
        }
    }
    (synced, plain)
}

// ---------------------------------------------------------------------------
// Genius como texto autoritativo
// ---------------------------------------------------------------------------

/// Cómo se resolvió el texto de Genius contra los tiempos disponibles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// No hubo tiempos compatibles: se conserva la letra de Genius tal cual.
    Plain,
    /// Las captions del vídeo solo aportaron tiempos; el texto es de Genius.
    CaptionTiming,
    /// Un LRC sincronizado (LRCLIB/lrcmux) solo aportó tiempos; texto Genius.
    ProviderTiming,
}

impl Resolution {
    fn label(self) -> &'static str {
        match self {
            Self::Plain => "genius: texto plano conservado (sin tiempos fiables)",
            Self::CaptionTiming => "genius: texto + tiempos de captions",
            Self::ProviderTiming => "genius: texto + tiempos de proveedor",
        }
    }
}

/// Decide el documento final cuando Genius dio texto. Las captions y el LRC
/// de otros proveedores son solo una fuente de tiempos: sus líneas nunca
/// sustituyen a las de Genius.
fn resolve_genius(
    genius: LyricsDoc,
    captions: Option<LyricsDoc>,
    timing: Option<LyricsDoc>,
) -> (LyricsDoc, Resolution) {
    if let Some(mut doc) = captions {
        // `youtube::build_doc` devuelve exactamente las líneas del texto
        // conocido. Si por cualquier motivo no fuera así, se reutilizan sus
        // tiempos sobre el texto de Genius en vez de aceptar sus líneas.
        if doc.synced && same_lyric_lines(&genius, &doc) {
            doc.source = genius.source;
            doc.instrumental = genius.instrumental;
            doc.offset_ms = 0;
            return (doc, Resolution::CaptionTiming);
        }
        if doc.synced {
            if let Some(mut merged) = merge_text_with_timing(&genius, &doc) {
                merged.source = genius.source;
                return (merged, Resolution::CaptionTiming);
            }
        }
        tracing::debug!(
            target: "spote::lyrics",
            "genius: captions descartadas; su texto no coincide y no puede sustituir a Genius"
        );
    }
    if let Some(timing) = timing {
        if let Some(mut doc) = merge_text_with_timing(&genius, &timing) {
            doc.source = genius.source;
            return (doc, Resolution::ProviderTiming);
        }
        tracing::debug!(
            target: "spote::lyrics",
            "genius: {} no coincide con el texto (similitud {:.2}); se mantiene Genius plano",
            timing.source.label(),
            doc_text_similarity(&genius, &timing)
        );
    }
    (genius, Resolution::Plain)
}

fn log_resolution(doc: &LyricsDoc, resolution: Resolution) {
    tracing::info!(
        target: "spote::lyrics",
        "letra: {} · {} líneas · synced={} · offset={}",
        resolution.label(),
        doc.lines.len(),
        doc.synced,
        doc.offset_ms
    );
}

/// Similitud de texto entre dos documentos (Dice multiconjunto sobre palabras
/// normalizadas). Es insensible al orden de las líneas y a la puntuación, que
/// es justo lo que cambia entre Genius y un LRC de otro proveedor.
pub(crate) fn doc_text_similarity(left: &LyricsDoc, right: &LyricsDoc) -> f64 {
    let left = doc_tokens(left);
    let right = doc_tokens(right);
    token_dice(&left, &right)
}

/// Aplica los tiempos de `timing_doc` al texto de `text_doc`, conservando las
/// líneas del primero. Devuelve `None` si el texto no coincide lo suficiente:
/// en ese caso es preferible mostrar la letra plana antes que una sincronizada
/// de otra canción.
pub(crate) fn merge_text_with_timing(
    text_doc: &LyricsDoc,
    timing_doc: &LyricsDoc,
) -> Option<LyricsDoc> {
    if !timing_doc.synced || text_doc.lines.len() < 2 || timing_doc.lines.len() < 2 {
        return None;
    }
    let text_lines = comparable_lines(text_doc);
    let timing_lines = comparable_lines(timing_doc);
    if text_lines.len() < 2 || timing_lines.len() < 2 {
        return None;
    }
    let text_tokens: Vec<String> = text_lines
        .iter()
        .flat_map(|(_, tokens)| tokens.iter().cloned())
        .collect();
    let timing_tokens: Vec<String> = timing_lines
        .iter()
        .flat_map(|(_, tokens)| tokens.iter().cloned())
        .collect();
    if token_dice(&text_tokens, &timing_tokens) < GENIUS_TEXT_MATCH {
        return None;
    }

    let pairs = align_lyric_lines(&text_lines, &timing_lines);
    if pairs.len() < 2 {
        return None;
    }
    if pairs.len() as f64 / (text_lines.len() as f64) < MIN_TEXT_COVERAGE {
        return None;
    }
    let anchors: Vec<(usize, usize)> = pairs
        .into_iter()
        .map(|(text_index, timing_index)| (text_lines[text_index].0, timing_lines[timing_index].0))
        .collect();
    Some(build_timed_from_anchors(text_doc, timing_doc, &anchors))
}

/// Líneas con texto comparable, con sus palabras normalizadas.
fn comparable_lines(doc: &LyricsDoc) -> Vec<(usize, Vec<String>)> {
    doc.lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if !has_lyric_text(&line.text) {
                return None;
            }
            let tokens = lyric_tokens(&line.text);
            (!tokens.is_empty()).then_some((index, tokens))
        })
        .collect()
}

/// `true` si los dos documentos tienen exactamente las mismas líneas de letra
/// (normalizadas), en el mismo orden. Es la garantía de que las captions
/// devolvieron el texto conocido y no otro.
fn same_lyric_lines(left: &LyricsDoc, right: &LyricsDoc) -> bool {
    let normalize = |doc: &LyricsDoc| -> Vec<String> {
        comparable_lines(doc)
            .into_iter()
            .map(|(_, tokens)| tokens.join(" "))
            .collect()
    };
    let left = normalize(left);
    let right = normalize(right);
    !left.is_empty() && left == right
}

fn doc_tokens(doc: &LyricsDoc) -> Vec<String> {
    comparable_lines(doc)
        .into_iter()
        .flat_map(|(_, tokens)| tokens)
        .collect()
}

/// Palabras de una línea para comparar: minúsculas, sin acentos, sin
/// puntuación y sin adlibs entre paréntesis (no son la frase principal).
fn lyric_tokens(text: &str) -> Vec<String> {
    let mut stripped = String::with_capacity(text.len());
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            _ => stripped.push(ch),
        }
    }
    stripped
        .split_whitespace()
        .map(super::align::normalize_word)
        .filter(|token| !token.is_empty())
        .collect()
}

/// Coeficiente de Dice sobre multiconjuntos de palabras. Cuenta las repeticiones
/// (un estribillo repetido no se colapsa) y no depende del orden.
fn token_dice(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for token in left {
        *counts.entry(token.as_str()).or_default() += 1;
    }
    let mut common = 0usize;
    for token in right {
        if let Some(count) = counts.get_mut(token.as_str()) {
            if *count > 0 {
                *count -= 1;
                common += 1;
            }
        }
    }
    2.0 * common as f64 / (left.len() + right.len()) as f64
}

/// Alineación global (Needleman-Wunsch) entre líneas de dos letras. Solo
/// empareja líneas con parecido suficiente y deja huecos para las que faltan,
/// así una línea extra o un estribillo repetido no desplaza toda la rejilla.
fn align_lyric_lines(
    text: &[(usize, Vec<String>)],
    timing: &[(usize, Vec<String>)],
) -> Vec<(usize, usize)> {
    const GAP: f64 = -0.35;
    const MISMATCH: f64 = -0.9;
    let rows = text.len();
    let columns = timing.len();
    if rows == 0 || columns == 0 || rows.saturating_mul(columns) > 1_000_000 {
        return Vec::new();
    }
    let stride = columns + 1;
    let mut scores = vec![0.0f64; (rows + 1) * stride];
    for row in 1..=rows {
        scores[row * stride] = GAP * row as f64;
    }
    for (column, score) in scores.iter_mut().enumerate().take(columns + 1).skip(1) {
        *score = GAP * column as f64;
    }
    for row in 1..=rows {
        for column in 1..=columns {
            let similarity = token_dice(&text[row - 1].1, &timing[column - 1].1);
            let diagonal = scores[(row - 1) * stride + column - 1]
                + if similarity >= LINE_MATCH_MIN {
                    similarity
                } else {
                    MISMATCH
                };
            let up = scores[(row - 1) * stride + column] + GAP;
            let left = scores[row * stride + column - 1] + GAP;
            scores[row * stride + column] = diagonal.max(up).max(left);
        }
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut row, mut column) = (rows, columns);
    while row > 0 && column > 0 {
        let similarity = token_dice(&text[row - 1].1, &timing[column - 1].1);
        let diagonal = scores[(row - 1) * stride + column - 1]
            + if similarity >= LINE_MATCH_MIN {
                similarity
            } else {
                MISMATCH
            };
        let up = scores[(row - 1) * stride + column] + GAP;
        let left = scores[row * stride + column - 1] + GAP;
        let current = scores[row * stride + column];
        if similarity >= LINE_MATCH_MIN && (current - diagonal).abs() < 1e-9 {
            pairs.push((row - 1, column - 1));
            row -= 1;
            column -= 1;
        } else if (current - up).abs() < 1e-9 {
            row -= 1;
        } else if (current - left).abs() < 1e-9 {
            column -= 1;
        } else if similarity >= LINE_MATCH_MIN {
            pairs.push((row - 1, column - 1));
            row -= 1;
            column -= 1;
        } else {
            break;
        }
    }
    pairs.reverse();
    pairs
}

/// Construye el documento final: líneas de `text_doc` con la rejilla temporal
/// que marcan las líneas ancladas de `timing_doc`. Las líneas sin ancla se
/// interpolan entre las vecinas usando el reparto estimado original.
fn build_timed_from_anchors(
    text_doc: &LyricsDoc,
    timing_doc: &LyricsDoc,
    anchors: &[(usize, usize)],
) -> LyricsDoc {
    const MIN_LINE_GAP_MS: u64 = 120;
    let mut lines = text_doc.lines.clone();
    let count = lines.len();
    let mut starts = vec![0u64; count];
    let mut ends: Vec<Option<u64>> = vec![None; count];
    let mut anchored = vec![false; count];

    for &(text_index, timing_index) in anchors {
        let Some(timing) = timing_doc.lines.get(timing_index) else {
            continue;
        };
        let start = timing.start_ms;
        starts[text_index] = start;
        ends[text_index] = Some(timing.end_ms.max(start.saturating_add(100)));
        anchored[text_index] = true;
    }

    let anchor_positions: Vec<usize> = (0..count).filter(|index| anchored[*index]).collect();
    if anchor_positions.is_empty() {
        return text_doc.clone();
    }

    let mut previous_anchor: Option<usize> = None;
    for index in 0..count {
        if anchored[index] {
            previous_anchor = Some(index);
            continue;
        }
        let next_anchor = anchor_positions
            .iter()
            .copied()
            .find(|candidate| *candidate > index);
        match (previous_anchor, next_anchor) {
            (Some(previous), Some(next)) => {
                let previous_end = ends[previous].unwrap_or(starts[previous] + 1_000);
                let next_start = starts[next];
                let span = next_start.saturating_sub(previous_end);
                let estimate_previous = lines[previous].start_ms;
                let estimate_next = lines[next].start_ms;
                let estimate_here = lines[index].start_ms;
                let weight = estimate_next.saturating_sub(estimate_previous).max(1);
                let offset = estimate_here.saturating_sub(estimate_previous).min(weight);
                starts[index] =
                    previous_end + (span as u128 * offset as u128 / weight as u128) as u64;
            }
            (None, Some(next)) => {
                let next_start = starts[next];
                let estimate_next = lines[next].start_ms;
                let estimate_here = lines[index].start_ms;
                starts[index] =
                    next_start.saturating_sub(estimate_next.saturating_sub(estimate_here));
            }
            (Some(previous), None) => {
                let previous_end = ends[previous].unwrap_or(starts[previous] + 1_000);
                let estimate_previous = lines[previous].start_ms;
                let estimate_here = lines[index].start_ms;
                starts[index] =
                    previous_end.saturating_add(estimate_here.saturating_sub(estimate_previous));
            }
            (None, None) => starts[index] = index as u64 * 4_000,
        }
    }

    // La rejilla tiene que ser monótona aunque el proveedor traiga marcas
    // repetidas o una interpolación se quede corta.
    let mut previous = starts.first().copied().unwrap_or(0);
    for (index, start) in starts.iter_mut().enumerate() {
        if index > 0 {
            *start = (*start).max(previous.saturating_add(MIN_LINE_GAP_MS));
            previous = *start;
        }
    }

    for index in 0..count {
        if ends[index].is_some() {
            continue;
        }
        let start = starts[index];
        let estimate = lines[index]
            .end_ms
            .saturating_sub(lines[index].start_ms)
            .max(1_000);
        let end = starts
            .get(index + 1)
            .copied()
            .map(|next| next.saturating_sub(40).max(start.saturating_add(250)))
            .unwrap_or(start.saturating_add(estimate));
        ends[index] = Some(end.max(start.saturating_add(100)));
    }
    for index in 0..count.saturating_sub(1) {
        let next = starts[index + 1];
        let start = starts[index];
        if next > start {
            let end = ends[index]
                .unwrap_or(start.saturating_add(100))
                .min(next.saturating_sub(40).max(start.saturating_add(50)))
                .max(start.saturating_add(50));
            ends[index] = Some(end);
        }
    }
    if let Some(last) = ends.last_mut() {
        *last = Some(
            last.unwrap_or(starts[count - 1].saturating_add(250))
                .max(starts[count - 1].saturating_add(250)),
        );
    }

    for (index, line) in lines.iter_mut().enumerate() {
        line.start_ms = starts[index];
        line.end_ms = ends[index].unwrap_or(starts[index] + 1_000);
        line.words.clear();
        line.words_exact = false;
    }

    LyricsDoc {
        lines,
        synced: true,
        instrumental: text_doc.instrumental,
        source: text_doc.source,
        offset_ms: 0,
        aligned: false,
    }
}

/// Final del tramo de un proveedor, sin pasarse del presupuesto global.
fn slice(deadline: Instant, budget: Duration) -> Instant {
    let end = Instant::now() + budget;
    if end < deadline {
        end
    } else {
        deadline
    }
}

fn has_budget(deadline: Instant, minimum: Duration) -> bool {
    deadline.saturating_duration_since(Instant::now()) >= minimum
}

/// Id de vídeo exacto si la pista se descargó de YouTube/YT Music. Sin esto no
/// se puede garantizar que las captions pertenezcan al mismo audio.
fn youtube_hint(track: &TrackRef) -> Option<&str> {
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

/// Metadatos contra los que se puntúa un candidato. A diferencia de la pista
/// cruda, el título viene limpio y el artista se deduce del patrón
/// "Artista - Tema" cuando el canal de YouTube no es el artista.
#[derive(Debug, Clone)]
pub(crate) struct MatchTarget {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub version: VersionKind,
    /// El artista procede del canal o del título y se puede exigir. Si es un
    /// canal de YouTube sin relación (recopilatorio, "Lyrics"), se relaja la
    /// puerta de artista y se compensa con un título más estricto.
    pub artist_trusted: bool,
    /// La pista se descargó de YouTube: la duración del vídeo incluye
    /// intro/outro y la tolerancia debe ser mayor.
    pub youtube: bool,
}

impl MatchTarget {
    pub(crate) fn from_track(track: &TrackRef) -> Self {
        let channel = track.artist.to_ascii_lowercase();
        let title_lower = track.title.to_ascii_lowercase();
        let looks_like_video = [
            "official video",
            "official music video",
            "official audio",
            "lyric video",
            "lyrics video",
            "video oficial",
            "audio oficial",
            "visualizer",
        ]
        .iter()
        .any(|marker| title_lower.contains(*marker));
        let youtube = track
            .source_hint
            .as_ref()
            .is_some_and(|hint| is_youtube_provider(&hint.provider))
            // Pistas viejas/importadas sin source_hint pero con el canal típico
            // de YouTube: "Artista - Topic" o "ArtistaVEVO".
            || channel.contains("- topic")
            || channel.ends_with("vevo")
            // O con el ruido de vídeo en el título.
            || looks_like_video;
        let mut artist = clean_artist(&track.artist);
        let mut title = clean_title(&track.title);
        let mut artist_trusted = !artist.is_empty();

        if let Some((left, right)) = split_artist_title(&track.title) {
            let left_clean = clean_title(left);
            let right_clean = clean_title(right);
            let left_is_artist = !artist.is_empty() && similarity(&left_clean, &artist) >= 0.60;
            let right_is_artist = !artist.is_empty() && similarity(&right_clean, &artist) >= 0.60;
            if left_is_artist && !right_clean.is_empty() {
                artist = clean_artist(left);
                title = right_clean;
                artist_trusted = true;
            } else if right_is_artist && !left_clean.is_empty() {
                artist = clean_artist(right);
                title = left_clean;
                artist_trusted = true;
            } else if !left_clean.is_empty()
                && !right_clean.is_empty()
                && starts_with_version_marker(right)
            {
                // "Tema - Live at Reading": el lado derecho es la versión, no
                // el título. Se queda el tema de la izquierda y el canal como
                // artista (fiable fuera de YouTube; allí puede ser un upload).
                title = left_clean;
                if youtube {
                    artist_trusted = false;
                }
            } else if (youtube || artist.is_empty())
                && !left_clean.is_empty()
                && !right_clean.is_empty()
            {
                // Canal de YouTube que no es el artista: el patrón habitual
                // "Artista - Tema (Official Video)" sigue valiendo.
                artist = clean_artist(left);
                title = right_clean;
                artist_trusted = true;
            } else if youtube && !artist.is_empty() {
                // El canal no casa con ningún lado del título: probablemente
                // sea un canal de recopilación. No se puede exigir artista.
                artist_trusted = false;
            }
        }
        if title.is_empty() {
            title = clean_title(&track.title);
        }
        // Un canal de discográfica/recopilatorio no identifica al artista: se
        // relaja la puerta de artista y manda título+duración+versión.
        if youtube && artist_trusted && is_curator_artist(&artist) {
            artist_trusted = false;
        }
        Self {
            title,
            artist,
            album: track
                .album
                .as_deref()
                .map(clean_title)
                .filter(|album| !album.is_empty()),
            duration_ms: track.duration_ms,
            version: version_kind(&track.title, track.album.as_deref()),
            artist_trusted,
            youtube,
        }
    }

    fn far_duration_ms(&self) -> f64 {
        if self.youtube {
            YOUTUBE_FAR_DURATION_MS
        } else {
            FAR_DURATION_MS
        }
    }
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

    let target = MatchTarget::from_track(track);
    let album = target.album.clone();

    // La variante limpia primero: es la que más probabilidad tiene de acertar
    // en `/api/get` y evita gastar el tramo de LRCLIB en títulos de YouTube.
    if !target.title.is_empty() && !target.artist.is_empty() {
        let mut cleaned = track.clone();
        cleaned.title = target.title.clone();
        cleaned.artist = target.artist.clone();
        cleaned.album = album.clone();
        push(cleaned);
    }
    push(track.clone());

    // "Artista - Tema": se prueban las dos orientaciones. La que coincide con
    // el canal ya la usó `MatchTarget`, pero un canal recopilatorio no.
    if let Some((left, right)) = split_artist_title(&track.title) {
        let left_clean = clean_title(left);
        let right_clean = clean_title(right);
        if !left_clean.is_empty() && !right_clean.is_empty() {
            let mut left_artist = track.clone();
            left_artist.title = right_clean.clone();
            left_artist.artist = clean_artist(left);
            left_artist.album = album.clone();
            push(left_artist);

            // En YouTube el patrón es "Artista - Tema"; la orientación
            // inversa solo añade una petición que nunca va a pasar la puerta.
            if !target.youtube {
                let mut right_artist = track.clone();
                right_artist.title = left_clean;
                right_artist.artist = clean_artist(right);
                right_artist.album = album.clone();
                push(right_artist);
            }
        }
    }

    // Título limpio con el canal tal cual: cubre canales que sí son artistas.
    // Si el artista se dedujo del título, el canal es ruido y no se consulta.
    if !target.title.is_empty() && clean_artist(&track.artist) == target.artist {
        let mut title_channel = track.clone();
        title_channel.title = target.title.clone();
        title_channel.artist = clean_artist(&track.artist);
        title_channel.album = album.clone();
        push(title_channel);
    }

    // Artista principal de una colaboración ("A, B" → "A").
    let primary = primary_artist(&target.artist);
    if !primary.is_empty() && primary != target.artist {
        let mut solo = track.clone();
        solo.title = target.title.clone();
        solo.artist = primary;
        solo.album = album;
        push(solo);
    }

    // Recurso intermedio: título plegado (comillas y guiones tipográficos).
    if !target.title.is_empty() {
        let folded_title = search_title(&target.title);
        if !folded_title.is_empty() && folded_title != target.title {
            let mut folded = track.clone();
            folded.title = folded_title;
            folded.artist = target.artist.clone();
            push(folded);
        }
    }

    // Último recurso: solo título, sin artista ni álbum.
    let mut title_only = track.clone();
    title_only.title = target.title;
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
#[cfg(test)]
pub(crate) fn rank_candidates(cands: Vec<Candidate>, track: &TrackRef) -> Vec<Candidate> {
    rank_for(cands, &MatchTarget::from_track(track))
}

fn rank_for(cands: Vec<Candidate>, target: &MatchTarget) -> Vec<Candidate> {
    let mut scored: Vec<Candidate> = cands
        .into_iter()
        .map(|mut cand| {
            cand.score = score_for(&cand, target);
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
/// Pesos: título 0.48, artista 0.24, duración 0.28; bonus pequeño por llevar
/// letra sincronizada y castigo por ser una versión distinta (directo, remix,
/// cover, karaoke, sped up…) cuando se pidió la de estudio. Los timestamps no
/// pueden compensar un título o un artista que no son los pedidos: eso lo
/// decide [`acceptable`], no el score.
#[cfg(test)]
pub(crate) fn score_candidate(cand: &Candidate, track: &TrackRef) -> f64 {
    score_for(cand, &MatchTarget::from_track(track))
}

fn score_for(cand: &Candidate, target: &MatchTarget) -> f64 {
    let title = title_similarity_for(&cand.track_name, target);
    let artist = if target.artist.trim().is_empty() {
        0.5
    } else if cand.artist_name.trim().is_empty() {
        0.0
    } else {
        artist_similarity(&cand.artist_name, &target.artist)
    };
    let duration = duration_score(cand.duration, target.duration_ms, target.youtube);

    let mut score = 0.48 * title + 0.24 * artist + 0.28 * duration;

    let offered = version_kind(&cand.track_name, Some(&cand.album_name));
    if offered != target.version {
        score -= if target.version == VersionKind::Studio {
            0.22
        } else {
            0.12
        };
    }

    if cand.synced {
        // La sincronizada vale un poco más, pero solo como desempate.
        score += 0.05;
    } else if !cand.plain {
        // Sin letra no sirve de nada aunque el título cuadre al milímetro.
        score *= 0.35;
    }
    if !cand.album_name.trim().is_empty() {
        let album = target.album.as_deref().unwrap_or_default();
        if similarity(&cand.album_name, album) >= 0.99 {
            score += 0.03;
        }
    }
    score.clamp(0.0, 1.0)
}

/// `true` si el candidato merece devolverse.
#[cfg(test)]
fn acceptable(best: &Candidate, track: &TrackRef) -> bool {
    acceptable_for(best, &MatchTarget::from_track(track))
}

fn acceptable_for(best: &Candidate, target: &MatchTarget) -> bool {
    if best.score < MIN_SCORE {
        return false;
    }
    let title = title_similarity_for(&best.track_name, target);
    let minimum = if target.artist.trim().is_empty() {
        // Sin artista solo queda el título y la duración: se exige más.
        0.70
    } else {
        MIN_TITLE
    };
    if title < minimum {
        return false;
    }
    if is_generic_title(&target.title) && title < GENERIC_TITLE {
        return false;
    }

    // Puerta de artista. Un canal de YouTube que no es el artista no puede
    // bloquear la única coincidencia buena; en ese caso manda el título.
    let artist = if best.artist_name.trim().is_empty() {
        0.0
    } else {
        artist_similarity(&best.artist_name, &target.artist)
    };
    if !target.artist.trim().is_empty() {
        if target.artist_trusted {
            if best.artist_name.trim().is_empty() || artist < MIN_ARTIST {
                return false;
            }
        } else if title < GENERIC_TITLE {
            return false;
        }
    }

    // Una versión distinta con tiempos propios (directo, remix, cover) no vale
    // aunque el título se limpie igual: los timestamps son de otro audio.
    let offered = version_kind(&best.track_name, Some(&best.album_name));
    if offered != target.version {
        if offered.is_incompatible() || best.synced {
            return false;
        }
    }

    let known = target.duration_ms > 0 && best.duration.is_finite() && best.duration > 0.0;
    // Un título genérico ("Regular", "Love", "Halo") con una versión de la que
    // no sabemos la duración es una moneda al aire: puede ser otro máster del
    // mismo artista. Mejor no enseñar nada que una letra de otra canción.
    if target.duration_ms > 0 && !known && is_generic_title(&target.title) {
        return false;
    }
    if !known {
        return true;
    }
    // LRCLIB manda la duración en segundos: se pasa a ms antes de comparar.
    let diff = (best.duration * 1000.0 - target.duration_ms as f64).abs();
    diff <= target.far_duration_ms()
}

/// Comprueba los metadatos de una respuesta de un proveedor que no pasa por el
/// ranking de `/api/search` (lrcmux, `/api/get`). Devuelve `true` cuando no hay
/// metadatos suficientes para juzgar.
pub(crate) fn metadata_matches(
    track: &TrackRef,
    title: &str,
    artist: &str,
    album: &str,
    duration_seconds: f64,
) -> bool {
    if title.trim().is_empty() && artist.trim().is_empty() {
        // Un proveedor sin metadatos no puede validarse contra una pista que
        // sí los tiene. Antes se aceptaba cualquier letra en este caso.
        return track.title.trim().is_empty() && track.artist.trim().is_empty();
    }
    let target = MatchTarget::from_track(track);
    let cand = Candidate {
        source_index: 0,
        id: 0,
        track_name: if title.trim().is_empty() {
            target.title.clone()
        } else {
            title.to_string()
        },
        artist_name: artist.to_string(),
        album_name: album.to_string(),
        duration: duration_seconds,
        synced: true,
        plain: true,
        score: 0.0,
    };
    let mut cand = cand;
    cand.score = score_for(&cand, &target);
    acceptable_for(&cand, &target)
}

/// Similitud de título teniendo en cuenta que el candidato o la pista pueden
/// arrastrar el nombre del artista ("Artista - Tema", típico de YouTube).
fn title_similarity_for(candidate_title: &str, target: &MatchTarget) -> f64 {
    if target.title.trim().is_empty() {
        return 0.0;
    }
    let mut best = similarity(candidate_title, &target.title);
    if let Some((left, right)) = split_artist_title(candidate_title) {
        // El candidato también puede venir con "Artista - Tema".
        best = best.max(similarity(right, &target.title));
        best = best.max(similarity(left, &target.title));
    }
    best
}

/// Título con muy pocas palabras significativas ("Love", "Halo"): exige un
/// parecido mayor para que "Love Story" no pase por "Love".
fn is_generic_title(title: &str) -> bool {
    let cleaned = clean_title(title);
    let words = cleaned.split_whitespace().count();
    let letters = cleaned.chars().filter(|ch| ch.is_alphanumeric()).count();
    words <= 2 || letters <= 8
}

/// Título con símbolos que lo identifican aunque tenga pocas letras o palabras
/// (`(_8(|)`, `#1`, `R U Mine?`). No es genérico: el símbolo forma parte del
/// nombre. Exige un núcleo alfanumérico (`$$$` no identifica nada).
pub(crate) fn is_symbolic_title(title: &str) -> bool {
    let cleaned = clean_title(title);
    if !cleaned.chars().any(|ch| ch.is_alphanumeric()) {
        return false;
    }
    cleaned
        .chars()
        .any(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
}

/// Compara títulos conservando los símbolos. [`similarity`] normaliza el texto
/// y se come los paréntesis, así que `(_8(|)` quedaría vacío; para títulos
/// simbólicos hace falta una igualdad literal (sin espacios ni mayúsculas).
pub(crate) fn titles_match_exactly(left: &str, right: &str) -> bool {
    fn canonical(text: &str) -> String {
        clean_title(text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
    let left = canonical(left);
    let right = canonical(right);
    !left.is_empty() && left == right
}

/// Cuánto se parece la duración del candidato a la de la pista.
///
/// Por tramos: el vídeo suele añadir unos segundos de intro/outro y eso no debe
/// pesar como si fuera otra canción.
fn duration_score(candidate_seconds: f64, track_ms: u64, youtube: bool) -> f64 {
    // Sin una de las dos duraciones no se puede comparar: neutro.
    if track_ms == 0 || !candidate_seconds.is_finite() || candidate_seconds <= 0.0 {
        return 0.5;
    }
    let diff = (candidate_seconds * 1000.0 - track_ms as f64).abs();
    if youtube {
        // El vídeo añade intro/outro: se tolera más antes de castigar.
        return match diff {
            diff if diff <= 4_000.0 => 1.0,
            diff if diff <= 12_000.0 => 0.9,
            diff if diff <= 25_000.0 => 0.75,
            diff if diff <= 35_000.0 => 0.5,
            diff => (1.0 - diff / track_ms as f64).clamp(0.0, 0.2),
        };
    }
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
    target: &MatchTarget,
    deadline: Instant,
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

    let Some(response) = get_with_budget(client, &url, deadline).await? else {
        return Ok(None);
    };
    let raw: RemoteLyrics = response.json().await.map_err(map_error)?;
    // `/api/get` devuelve un único registro, pero también puede devolver una
    // versión mal etiquetada. Se valida con la misma puerta que la búsqueda.
    let mut cand = Candidate::from_parts(0, &raw);
    cand.score = score_for(&cand, target);
    if !acceptable_for(&cand, target) {
        tracing::debug!(
            target: "spote::lyrics",
            "lrclib get descartado: '{} - {}' score={:.2}",
            cand.artist_name,
            cand.track_name,
            cand.score
        );
        return Ok(None);
    }
    Ok(build_doc(&raw, track))
}

/// `/api/search`: lista de candidatos, se puntúan y se queda el mejor.
async fn fetch_search(
    client: &reqwest::Client,
    track: &TrackRef,
    target: &MatchTarget,
    deadline: Instant,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let mut url = format!("{API_BASE}/search?track_name={}", url_encode(&track.title));
    if !track.artist.trim().is_empty() {
        url.push_str("&artist_name=");
        url.push_str(&url_encode(&track.artist));
    }

    let Some(response) = get_with_budget(client, &url, deadline).await? else {
        return Ok(None);
    };
    let entries: Vec<RemoteLyrics> = response.json().await.map_err(map_error)?;
    let candidates = entries
        .iter()
        .enumerate()
        .map(|(index, raw)| Candidate::from_parts(index, raw))
        .collect();
    let ranked = rank_for(candidates, target);
    // Del mejor al peor: el primero que pase la puerta gana. Nunca se coge el
    // primero de la lista sin puntuar, y un synced con mal título no compensa.
    for best in &ranked {
        if !acceptable_for(best, target) {
            continue;
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

/// Lanza una petición GET respetando el presupuesto del proveedor. Devuelve
/// `None` (sin error) cuando ya no queda tiempo: no es un fallo de la fuente.
async fn get_with_budget(
    client: &reqwest::Client,
    url: &str,
    deadline: Instant,
) -> Result<Option<reqwest::Response>, LyricsError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining < MIN_REQUEST_BUDGET {
        return Ok(None);
    }
    let response = client
        .get(url)
        .timeout(remaining.min(REQUEST_TIMEOUT))
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(Some(response))
}

// ---------------------------------------------------------------------------
// Construcción del documento
// ---------------------------------------------------------------------------

/// Prefiere `syncedLyrics`; si no hay (o sus tiempos no son de fiar), reparte
/// `plainLyrics` sobre la duración.
fn build_doc(raw: &RemoteLyrics, track: &TrackRef) -> Option<LyricsDoc> {
    build_doc_with_source(raw, track, LyricsSource::Lrclib)
}

fn build_doc_with_source(
    raw: &RemoteLyrics,
    track: &TrackRef,
    source: LyricsSource,
) -> Option<LyricsDoc> {
    let duration_ms = effective_duration_ms(raw, track);
    if let Some(doc) = synced_doc(raw, duration_ms, source) {
        return Some(doc);
    }
    plain_doc(raw, track)
}

/// Letra con tiempos reales, tal cual viene en LRC. `None` si el LRC no trae
/// tiempos o si los trae imposibles (ver [`sync_problem`]).
fn synced_doc(raw: &RemoteLyrics, duration_ms: u64, source: LyricsSource) -> Option<LyricsDoc> {
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
        source,
        offset_ms: parsed.offset_ms,
        aligned: false,
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
    plain_doc_with_source(raw, track, LyricsSource::Plain)
}

fn plain_doc_with_source(
    raw: &RemoteLyrics,
    track: &TrackRef,
    source: LyricsSource,
) -> Option<LyricsDoc> {
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
        source,
        offset_ms: 0,
        aligned: false,
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

// ---------------------------------------------------------------------------
// Genius (letra plana)
// ---------------------------------------------------------------------------

const GENIUS_SEARCH: &str = "https://genius.com/api/search/multi";
const GENIUS_BASE: &str = "https://genius.com";
const GENIUS_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const GENIUS_ACCEPT_JSON: &str = "application/json";
const GENIUS_ACCEPT_HTML: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

#[derive(Debug, Deserialize)]
struct GeniusSearch {
    #[serde(default)]
    response: GeniusResponse,
}

#[derive(Debug, Default, Deserialize)]
struct GeniusResponse {
    #[serde(default)]
    sections: Vec<GeniusSection>,
}

#[derive(Debug, Deserialize)]
struct GeniusSection {
    #[serde(default)]
    hits: Vec<GeniusHit>,
}

#[derive(Debug, Deserialize)]
struct GeniusHit {
    #[serde(default)]
    result: Option<GeniusSong>,
}

#[derive(Debug, Deserialize)]
struct GeniusSong {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    title: String,
    /// Genius pone aquí todos los artistas: "Guxo, Flako NKO & yoyo.stay".
    #[serde(default)]
    artist_names: String,
    #[serde(default)]
    path: String,
    #[serde(rename = "_type", default)]
    kind: String,
    #[serde(default)]
    lyrics_state: String,
}

/// Busca una canción en Genius, elige la mejor coincidencia y extrae su letra.
async fn fetch_genius(
    client: &reqwest::Client,
    track: &TrackRef,
    target: &MatchTarget,
    deadline: Instant,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let query = format!("{} {}", target.title.trim(), target.artist.trim());
    let query = query.trim();
    if query.is_empty() {
        return Ok(None);
    }
    let url = format!("{GENIUS_SEARCH}?per_page=5&q={}", url_encode(query));
    let Some(response) = genius_get(client, &url, GENIUS_ACCEPT_JSON, deadline).await? else {
        return Ok(None);
    };
    let body: GeniusSearch = response.json().await.map_err(map_error)?;
    let Some(song) = best_genius_song(&body, target) else {
        if std::env::var_os("SPOTE_GENIUS_DEBUG").is_some() {
            eprintln!(
                "[genius] sin candidato en {} secciones",
                body.response.sections.len()
            );
        }
        return Ok(None);
    };
    if std::env::var_os("SPOTE_GENIUS_DEBUG").is_some() {
        eprintln!("[genius] candidato {} · {}", song.title, song.path);
    }
    let path = if song.path.starts_with("http://") || song.path.starts_with("https://") {
        song.path.clone()
    } else if song.path.starts_with('/') {
        format!("{GENIUS_BASE}{}", song.path)
    } else {
        format!("{GENIUS_BASE}/{}", song.path)
    };
    let Some(response) = genius_get(client, &path, GENIUS_ACCEPT_HTML, deadline).await? else {
        return Ok(None);
    };
    let status = response.status();
    let page = response.text().await.map_err(map_error)?;
    let text = extract_genius_lyrics(&page);
    if std::env::var_os("SPOTE_GENIUS_DEBUG").is_some() {
        eprintln!(
            "[genius] página {} · {} bytes · {} de letra",
            status,
            page.len(),
            text.len()
        );
    }
    if text.is_empty() {
        return Ok(None);
    }
    let raw = RemoteLyrics {
        id: song.id,
        track_name: song.title.clone(),
        artist_name: song.artist_names.clone(),
        album_name: String::new(),
        duration: 0.0,
        plain_lyrics: Some(text),
        synced_lyrics: None,
    };
    let doc = plain_doc_with_source(&raw, track, LyricsSource::Genius);
    if let Some(doc) = &doc {
        tracing::debug!(
            target: "spote::lyrics",
            "genius: {} líneas extraídas de {}",
            doc.lines.len(),
            song.title
        );
    }
    Ok(doc)
}

async fn genius_get(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
    deadline: Instant,
) -> Result<Option<reqwest::Response>, LyricsError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining < MIN_REQUEST_BUDGET {
        return Ok(None);
    }
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, GENIUS_UA)
        .header(reqwest::header::ACCEPT, accept)
        .timeout(remaining.min(REQUEST_TIMEOUT))
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(Some(response))
}

fn best_genius_song<'a>(search: &'a GeniusSearch, target: &MatchTarget) -> Option<&'a GeniusSong> {
    let generic = is_generic_title(&target.title);
    // Un título con símbolos y poco texto (`(_8(|)`) es genérico por longitud
    // pero el símbolo lo identifica. En ese caso se exige coincidencia literal
    // con Genius; el resto de títulos siguen el parecido normal.
    let symbolic_generic = generic && is_symbolic_title(&target.title);
    // Genius no publica duración: para títulos de una sola palabra no hay
    // forma de distinguir dos canciones homónimas del mismo artista. Es
    // exactamente el caso de «Regular» (dos versiones de Biberon), así que se
    // prefiere no arriesgar una letra ajena.
    if target.duration_ms > 0 && generic && !symbolic_generic {
        return None;
    }
    let mut best: Option<(f64, &GeniusSong)> = None;
    for section in &search.response.sections {
        for hit in &section.hits {
            let Some(song) = hit.result.as_ref() else {
                continue;
            };
            if !song.kind.is_empty() && song.kind != "song" {
                continue;
            }
            if song.title.trim().is_empty() || song.path.trim().is_empty() {
                continue;
            }
            if song.lyrics_state == "unavailable" {
                continue;
            }
            // Genius no da duración, así que la versión se juzga por el título:
            // una letra de directo/remix no debe colarse por la de estudio.
            let offered = version_kind(&song.title, None);
            if offered != target.version && offered.is_incompatible() {
                continue;
            }
            let mut title_score = title_similarity_for(&song.title, target);
            if symbolic_generic {
                // `similarity` normaliza y descarta paréntesis: para `(_8(|)`
                // daría 0 incluso consigo mismo. Solo vale el título idéntico.
                if !titles_match_exactly(&song.title, &target.title) {
                    continue;
                }
                title_score = 1.0;
            }
            let minimum = if symbolic_generic {
                0.95
            } else if target.artist.trim().is_empty() {
                0.80
            } else {
                MIN_TITLE
            };
            if title_score < minimum {
                continue;
            }
            let artist_score = if target.artist.trim().is_empty() {
                0.5
            } else {
                artist_similarity(&song.artist_names, &target.artist)
            };
            // Mismo título de otro artista no vale. Con un canal de YouTube no
            // fiable se compensa exigiendo un título casi exacto.
            if !target.artist.trim().is_empty() {
                if target.artist_trusted && artist_score < 0.30 {
                    continue;
                }
                if !target.artist_trusted && title_score < 0.80 {
                    continue;
                }
            }
            let score = title_score * 0.72 + artist_score * 0.28;
            if best.as_ref().is_none_or(|(best, _)| score > *best) {
                best = Some((score, song));
            }
        }
    }
    best.map(|(_, song)| song)
}

/// Extrae la letra de los `div[data-lyrics-container="true"]` de Genius.
///
/// Sin dependencias de HTML: se recorre el marcado buscando el div, se anidan
/// las etiquetas y se convierte `<br>`/bloques en saltos de línea. Los bloques
/// `data-exclude-from-selection` ("You might also like" y similares) se
/// descartan antes para no colarlos como verso.
fn extract_genius_lyrics(page: &str) -> String {
    let page = strip_excluded_blocks(page);
    let mut lines: Vec<String> = Vec::new();
    for marker in [
        "data-lyrics-container=\"true\"",
        "data-lyrics-container='true'",
    ] {
        let mut cursor = 0;
        while let Some(found) = page[cursor..].find(marker) {
            let at = cursor + found;
            let Some(open) = page[..at].rfind('<') else {
                break;
            };
            if !page[open..].to_ascii_lowercase().starts_with("<div") {
                cursor = at + marker.len();
                continue;
            }
            let Some(close) = page[at..].find('>') else {
                break;
            };
            let body_start = at + close + 1;
            let Some(end) = matching_div_end(&page, body_start) else {
                break;
            };
            for line in html_to_text(&page[body_start..end]).lines() {
                let line = line.trim();
                if line.is_empty() || is_section_header(line) {
                    continue;
                }
                lines.push(line.to_string());
            }
            cursor = end + 6;
        }
        if !lines.is_empty() {
            break;
        }
    }
    lines.join("\n")
}

fn is_section_header(line: &str) -> bool {
    line.len() >= 2
        && line.starts_with('[')
        && line.ends_with(']')
        && !line[1..line.len() - 1].contains('[')
}

fn strip_excluded_blocks(html: &str) -> String {
    let mut page = html.to_string();
    loop {
        let Some(at) = page.find("data-exclude-from-selection") else {
            return page;
        };
        let Some(open) = page[..at].rfind('<') else {
            return page;
        };
        let Some(close) = page[open..].find('>') else {
            return page;
        };
        let body_start = open + close + 1;
        let Some(end) = matching_div_end(&page, body_start) else {
            return page;
        };
        page.replace_range(open..end + 6, "");
    }
}

/// Índice del `</div>` que cierra un div cuyo contenido empieza en `start`.
fn matching_div_end(html: &str, start: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut cursor = start;
    while cursor < html.len() {
        let lt = html[cursor..].find('<')? + cursor;
        let gt = html[lt..].find('>')? + lt + 1;
        let tag = html[lt..gt].to_ascii_lowercase();
        if tag.starts_with("</div") {
            depth -= 1;
            if depth == 0 {
                return Some(lt);
            }
        } else if tag.starts_with("<div") {
            depth += 1;
        }
        cursor = gt;
    }
    None
}

/// Convierte el interior de un contenedor de letra a texto: saltos en `<br>` y
/// cierres de bloque, etiquetas fuera, entidades decodificadas.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            out.push_str(&rest[lt..]);
            return decode_entities(&out);
        };
        let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
        let is_break = tag.starts_with("br")
            || tag.starts_with("/p")
            || tag.starts_with("/div")
            || tag.starts_with("/li")
            || tag.starts_with("p")
            || tag.starts_with("li");
        if is_break {
            out.push('\n');
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        if semi > 10 {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        match decoded {
            Some(char) => out.push(char),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    out
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

    /// Letra plana con reparto estimado, como la que devuelve Genius.
    fn plain_lyrics(lines: &[&str], source: LyricsSource) -> LyricsDoc {
        let mut doc = LyricsDoc {
            synced: false,
            source,
            ..LyricsDoc::default()
        };
        for (index, text) in lines.iter().enumerate() {
            let start = index as u64 * 4_000;
            doc.lines.push(LyricLine {
                start_ms: start,
                end_ms: start + 4_000,
                text: (*text).to_string(),
                ..LyricLine::default()
            });
        }
        doc
    }

    /// Letra sincronizada de otro proveedor, solo para prestar tiempos.
    fn timed_lyrics(lines: &[(u64, &str)], source: LyricsSource) -> LyricsDoc {
        let mut doc = build_test_doc(lines);
        doc.source = source;
        doc
    }

    const GENIUS_LINES: [&str; 6] = [
        "Cuando salgo de fiesta con mi gente",
        "La noche se nos pasa volando",
        "Bailando hasta que salga el sol",
        "No pienso parar hasta el amanecer",
        "Porque la vida es una sola",
        "Y hay que vivirla sin miedo",
    ];

    const OTHER_LINES: [&str; 4] = [
        "I love you forever and ever",
        "You are the only one for me",
        "Nothing else matters in this world",
        "Stay with me until the morning light",
    ];

    #[test]
    fn genius_text_keeps_provider_timing_when_it_matches() {
        let genius = plain_lyrics(&GENIUS_LINES, LyricsSource::Genius);
        let timing = timed_lyrics(
            &[
                (12_000, "cuando salgo de fiesta con mi gente"),
                (16_000, "la noche se nos pasa volando"),
                (20_000, "bailando hasta que salga el sol"),
                (24_000, "no pienso parar hasta el amanecer"),
                (28_000, "porque la vida es una sola"),
                (32_000, "y hay que vivirla sin miedo"),
            ],
            LyricsSource::Lrclib,
        );
        let merged = merge_text_with_timing(&genius, &timing).expect("el texto coincide");
        assert!(merged.synced);
        assert_eq!(merged.source, LyricsSource::Genius);
        let texts: Vec<&str> = merged.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, GENIUS_LINES);
        // Los tiempos sí son los del proveedor.
        assert_eq!(merged.lines[0].start_ms, 12_000);
        assert_eq!(merged.lines[5].start_ms, 32_000);
    }

    #[test]
    fn genius_plain_is_not_replaced_by_unrelated_synced_lyrics() {
        let genius = plain_lyrics(&GENIUS_LINES, LyricsSource::Genius);
        let timing = timed_lyrics(
            &[
                (10_000, OTHER_LINES[0]),
                (14_000, OTHER_LINES[1]),
                (18_000, OTHER_LINES[2]),
                (22_000, OTHER_LINES[3]),
            ],
            LyricsSource::Lrclib,
        );
        assert!(
            merge_text_with_timing(&genius, &timing).is_none(),
            "otra letra no puede prestar tiempos"
        );
        let (doc, resolution) = resolve_genius(genius, None, Some(timing));
        assert_eq!(resolution, Resolution::Plain);
        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Genius);
        let texts: Vec<&str> = doc.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, GENIUS_LINES);
    }

    #[test]
    fn captions_with_other_text_cannot_override_genius() {
        let genius = plain_lyrics(&GENIUS_LINES, LyricsSource::Genius);
        let captions = timed_lyrics(
            &[
                (10_000, OTHER_LINES[0]),
                (14_000, OTHER_LINES[1]),
                (18_000, OTHER_LINES[2]),
            ],
            LyricsSource::Captions,
        );
        let (doc, resolution) = resolve_genius(genius, Some(captions), None);
        assert_eq!(resolution, Resolution::Plain);
        let texts: Vec<&str> = doc.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, GENIUS_LINES);
        assert!(!doc.synced);
    }

    #[test]
    fn captions_timing_over_genius_text_is_kept() {
        let genius = plain_lyrics(&GENIUS_LINES, LyricsSource::Genius);
        let captions = timed_lyrics(
            &[
                (1_000, GENIUS_LINES[0]),
                (2_000, GENIUS_LINES[1]),
                (3_000, GENIUS_LINES[2]),
                (4_000, GENIUS_LINES[3]),
                (5_000, GENIUS_LINES[4]),
                (6_000, GENIUS_LINES[5]),
            ],
            LyricsSource::Captions,
        );
        let (doc, resolution) = resolve_genius(genius, Some(captions), None);
        assert_eq!(resolution, Resolution::CaptionTiming);
        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Genius);
        assert_eq!(doc.lines[0].start_ms, 1_000);
        assert_eq!(doc.lines[5].start_ms, 6_000);
    }

    #[test]
    fn merge_skips_an_extra_line_without_losing_genius_text() {
        let mut lines = GENIUS_LINES.to_vec();
        lines.insert(2, "Oh oh");
        let genius = plain_lyrics(&lines, LyricsSource::Genius);
        let timing = timed_lyrics(
            &[
                (12_000, GENIUS_LINES[0]),
                (16_000, GENIUS_LINES[1]),
                (20_000, GENIUS_LINES[2]),
                (24_000, GENIUS_LINES[3]),
                (28_000, GENIUS_LINES[4]),
                (32_000, GENIUS_LINES[5]),
            ],
            LyricsSource::Lrcmux,
        );
        let merged = merge_text_with_timing(&genius, &timing).expect("coincide lo suficiente");
        assert_eq!(merged.lines.len(), lines.len());
        assert_eq!(merged.source, LyricsSource::Genius);
        let texts: Vec<&str> = merged.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, lines);
        // El adlib interpolado no rompe la monotonía de la rejilla.
        assert!(merged
            .lines
            .windows(2)
            .all(|pair| pair[0].start_ms < pair[1].start_ms));
        assert!(merged.lines.iter().all(|line| line.end_ms > line.start_ms));
    }

    #[test]
    fn lyric_text_similarity_ignores_punctuation_and_line_order() {
        let original = plain_lyrics(&GENIUS_LINES, LyricsSource::Genius);
        let mut reordered = plain_lyrics(&GENIUS_LINES, LyricsSource::Plain);
        reordered.lines.reverse();
        for line in &mut reordered.lines {
            line.text = line.text.replace('a', "á").replace('e', "é");
        }
        assert!(
            doc_text_similarity(&original, &reordered) > 0.95,
            "la puntuación y el orden no deben cambiar la similitud"
        );
        let other = plain_lyrics(&OTHER_LINES, LyricsSource::Lrclib);
        assert!(doc_text_similarity(&original, &other) < GENIUS_TEXT_MATCH);
    }

    #[tokio::test]
    #[ignore = "diagnóstico manual con red: Genius/LRCLIB/YouTube"]
    async fn live_biberon_symbol_lyrics_prefer_genius() {
        let track = TrackRef {
            key: "diag:biberon-symbol".into(),
            title: "(_8(|) (feat. superreservao)".into(),
            artist: "Bíberon".into(),
            duration_ms: 139_000,
            ..TrackRef::default()
        };
        let service = crate::lyrics::LyricsService::new();
        let doc = service
            .fetch(&track)
            .await
            .expect("sin error de red")
            .expect("debe haber letra");
        eprintln!(
            "biberon: fuente={} synced={} lineas={}",
            doc.source.label(),
            doc.synced,
            doc.lines.len()
        );
        assert_eq!(doc.source, LyricsSource::Genius);
        assert!(doc.lines.len() >= 2);
    }

    #[tokio::test]
    #[ignore = "diagnóstico manual con red: Genius/LRCLIB/YouTube"]
    async fn diagnose_superreservao_sources() {
        let service = crate::lyrics::LyricsService::new();
        for (title, artist) in [
            ("Superreservao", "Biberon"),
            ("(_8(|) (feat. superreservao)", "Biberon"),
        ] {
            let track = TrackRef {
                key: format!("diag:{title}"),
                title: title.into(),
                artist: artist.into(),
                ..TrackRef::default()
            };
            println!("\n== {title} · {artist} ==");
            match service.fetch(&track).await {
                Ok(Some(doc)) => {
                    println!(
                        "fuente={} synced={} lineas={} offset={}",
                        doc.source.label(),
                        doc.synced,
                        doc.lines.len(),
                        doc.offset_ms
                    );
                    for line in doc.lines.iter().take(12) {
                        println!("{:>7} ms | {}", line.start_ms, line.text);
                    }
                }
                Ok(None) => println!("sin letra"),
                Err(err) => println!("error: {err}"),
            }
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

        // Un directo con timestamps propios no se acepta para una pista de
        // estudio: la letra puede coincidir, pero el karaoke iría desfasado.
        let live = scored(
            candidate(2, "Do I Wanna Know? (Live)", "Arctic Monkeys", 272.0, true),
            &track(),
        );
        assert!(!acceptable(&live, &track()));
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
    fn search_response_fixture_picks_the_studio_recording() {
        // Respuesta representativa de `/api/search`: directo, karaoke, remix y
        // la grabación buena, en ese orden.
        let entries: Vec<RemoteLyrics> = serde_json::from_str(
            r#"[
                {"id":1,"trackName":"Do I Wanna Know? (Live at Reading)","artistName":"Arctic Monkeys","albumName":"Live at Reading","duration":285.0,"syncedLyrics":"[00:10.00]uno\n[00:20.00]dos"},
                {"id":2,"trackName":"Do I Wanna Know? (Karaoke Version)","artistName":"Karaoke Hits","albumName":"","duration":272.0,"syncedLyrics":"[00:10.00]uno\n[00:20.00]dos"},
                {"id":3,"trackName":"Do I Wanna Know?","artistName":"Arctic Monkeys","albumName":"AM","duration":272.2,"syncedLyrics":"[00:12.30]Have you got colour\n[00:18.90]Do you ever get that fear"},
                {"id":4,"trackName":"Do I Wanna Know? (Remix)","artistName":"DJ X","albumName":"","duration":270.0,"syncedLyrics":"[00:10.00]uno\n[00:20.00]dos"}
            ]"#,
        )
        .expect("fixture válida");
        let candidates = entries
            .iter()
            .enumerate()
            .map(|(index, raw)| Candidate::from_parts(index, raw))
            .collect();
        let ranked = rank_for(candidates, &MatchTarget::from_track(&track()));
        assert_eq!(
            ranked[0].id,
            3,
            "{:?}",
            ranked.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        assert!(acceptable(&ranked[0], &track()));
        let raw = &entries[ranked[0].source_index];
        let doc = build_doc(raw, &track()).expect("hay letra");
        assert!(doc.synced);
        assert_eq!(doc.lines[0].start_ms, 12_300);
    }

    #[test]
    fn youtube_junk_channel_still_matches_the_artist_in_the_title() {
        let mut input = TrackRef {
            title: "Tame Impala - The Less I Know the Better (Official Video)".into(),
            artist: "Indie Uploads".into(),
            duration_ms: 220_000,
            ..TrackRef::default()
        };
        input.source_hint = Some(spote_protocol::SourceHint {
            provider: "youtube".into(),
            id: "sBzrzS1Ag_g".into(),
            duration_ms: 220_000,
        });
        let exact = scored(
            candidate(1, "The Less I Know the Better", "Tame Impala", 216.0, true),
            &input,
        );
        assert!(acceptable(&exact, &input), "{exact:?}");

        // El canal no es el artista: una versión de otro artista no puede
        // colarse solo porque el título coincide.
        let cover = scored(
            candidate(
                2,
                "The Less I Know the Better (Cover)",
                "Some Cover Band",
                220.0,
                true,
            ),
            &input,
        );
        assert!(!acceptable(&cover, &input));
    }

    #[test]
    fn youtube_curator_channel_does_not_block_the_real_song() {
        let mut input = TrackRef {
            title: "The Less I Know the Better (Official Video)".into(),
            artist: "Sony Music".into(),
            duration_ms: 220_000,
            ..TrackRef::default()
        };
        input.source_hint = Some(spote_protocol::SourceHint {
            provider: "youtube".into(),
            id: "sBzrzS1Ag_g".into(),
            duration_ms: 220_000,
        });
        let exact = scored(
            candidate(1, "The Less I Know the Better", "Tame Impala", 220.0, true),
            &input,
        );
        assert!(acceptable(&exact, &input), "{exact:?}");
    }

    #[test]
    fn youtube_live_title_without_artist_still_finds_the_live_version() {
        let mut input = TrackRef {
            title: "Do I Wanna Know? - Live at Reading".into(),
            artist: "Random Uploads".into(),
            duration_ms: 285_000,
            ..TrackRef::default()
        };
        input.source_hint = Some(spote_protocol::SourceHint {
            provider: "youtube".into(),
            id: "liveVid1234".into(),
            duration_ms: 285_000,
        });
        let live = scored(
            candidate(
                1,
                "Do I Wanna Know? (Live at Reading)",
                "Arctic Monkeys",
                285.0,
                true,
            ),
            &input,
        );
        assert!(acceptable(&live, &input), "{live:?}");
        let studio = scored(
            candidate(2, "Do I Wanna Know?", "Arctic Monkeys", 272.0, true),
            &input,
        );
        assert!(!acceptable(&studio, &input));
    }

    #[test]
    fn synced_remix_and_live_are_rejected_but_plain_text_is_usable() {
        let remix = scored(
            candidate(1, "Do I Wanna Know? (Remix)", "Arctic Monkeys", 272.0, true),
            &track(),
        );
        assert!(!acceptable(&remix, &track()));
        let remix_plain = scored(
            candidate(
                2,
                "Do I Wanna Know? (Remix)",
                "Arctic Monkeys",
                272.0,
                false,
            ),
            &track(),
        );
        assert!(acceptable(&remix_plain, &track()));
    }

    #[test]
    fn collaboration_artist_matches_when_one_side_is_the_requested_artist() {
        let mut input = track();
        input.title = "One Kiss".into();
        input.artist = "Calvin Harris, Dua Lipa".into();
        input.duration_ms = 215_000;
        let collab = scored(candidate(1, "One Kiss", "Dua Lipa", 215.0, true), &input);
        assert!(acceptable(&collab, &input), "{collab:?}");
    }

    #[test]
    fn generic_titles_require_a_near_exact_match() {
        let input = TrackRef {
            title: "Love".into(),
            artist: "Artist".into(),
            duration_ms: 200_000,
            ..TrackRef::default()
        };
        let other = scored(candidate(1, "Love Story", "Artist", 200.0, true), &input);
        assert!(!acceptable(&other, &input), "{other:?}");
        let same = scored(candidate(2, "Love", "Artist", 200.0, true), &input);
        assert!(acceptable(&same, &input));
    }

    #[test]
    fn metadata_gate_rejects_a_different_artist_from_lrcmux() {
        assert!(!metadata_matches(
            &track(),
            "Do I Wanna Know?",
            "Some Cover Band",
            "AM",
            272.0,
        ));
        assert!(metadata_matches(
            &track(),
            "Do I Wanna Know? - Remastered",
            "Arctic Monkeys",
            "AM",
            272.0,
        ));
        // Una respuesta sin metadatos no puede validarse contra una pista
        // que sí los tiene.
        assert!(!metadata_matches(&track(), "", "", "", 0.0));
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
    fn extracts_genius_lyrics_and_drops_excluded_blocks() {
        let page = r#"<div class="Lyrics__Container" data-lyrics-container="true">[Verse 1]<br/>Hola &amp; adiós<br/>Otra línea<div data-exclude-from-selection="true">You might also like</div></div>"#;
        let text = extract_genius_lyrics(page);
        assert_eq!(text, "Hola & adiós\nOtra línea");
    }

    fn genius_search(title: &str, artist: &str) -> GeniusSearch {
        GeniusSearch {
            response: GeniusResponse {
                sections: vec![GeniusSection {
                    hits: vec![GeniusHit {
                        result: Some(GeniusSong {
                            id: 1,
                            title: title.into(),
                            artist_names: artist.into(),
                            path: "/biberon/song".into(),
                            kind: "song".into(),
                            lyrics_state: "complete".into(),
                        }),
                    }],
                }],
            },
        }
    }

    #[test]
    fn symbolic_titles_are_distinguished_from_generic_ones() {
        assert!(is_symbolic_title("(_8(|)"));
        assert!(is_symbolic_title("#1"));
        assert!(is_symbolic_title("R U Mine?"));
        // Sin núcleo alfanumérico o sin símbolos no hay nada distintivo.
        assert!(!is_symbolic_title("$$$"));
        assert!(!is_symbolic_title("(((("));
        // Una palabra corta sin símbolos sigue siendo genérica.
        assert!(!is_symbolic_title("Regular"));
        assert!(!is_symbolic_title("Love"));
        assert!(titles_match_exactly("(_8(|)", "(_8(|)"));
        assert!(!titles_match_exactly("(_8(|)", "(_9(|)"));
    }

    #[test]
    fn symbolic_genius_title_is_accepted_while_regular_stays_ambiguous() {
        // Caso real: Genius tiene `(_8(|)` de Biberon & JOHNNYFUU. El título
        // limpio es `(_8(|)`: una sola «palabra» de un carácter alfanumérico,
        // pero el símbolo lo identifica y debe aceptarse.
        let search = genius_search("(_8(|)", "Biberon & JOHNNYFUU (Ft. superreservao)");
        let mut input = TrackRef {
            title: "(_8(|) (feat. superreservao)".into(),
            artist: "Biberon".into(),
            duration_ms: 180_000,
            ..TrackRef::default()
        };
        let target = MatchTarget::from_track(&input);
        assert_eq!(target.title, "(_8(|)");
        let chosen = best_genius_song(&search, &target).expect("el título simbólico se acepta");
        assert_eq!(chosen.title, "(_8(|)");

        // El título exacto no basta si el artista no es compatible.
        let impostor = genius_search("(_8(|)", "Otro Artista Cualquiera");
        assert!(best_genius_song(&impostor, &target).is_none());

        // «Regular» sigue siendo un homónimo sin duración en Genius: no se
        // arriesga una letra ajena.
        let regular = genius_search("Regular", "Biberon");
        input.title = "Regular".into();
        let target = MatchTarget::from_track(&input);
        assert!(best_genius_song(&regular, &target).is_none());
    }

    #[test]
    fn genius_entities_and_inline_tags() {
        let page = r#"<div data-lyrics-container="true">Te <i>quiero</i> &#39;mucho&#39;<br/>y &quot;más&quot;</div>"#;
        let text = extract_genius_lyrics(page);
        assert_eq!(text, "Te quiero 'mucho'\ny \"más\"");
    }

    #[test]
    fn variants_clean_youtube_metadata_and_keep_order() {
        let variants = query_variants(&youtube_track());
        // Limpia primero, cruda después y solo título al final.
        assert_eq!(variants[0].title, "Do I Wanna Know?");
        assert_eq!(variants[0].artist, "Arctic Monkeys");
        assert!(variants
            .iter()
            .any(|variant| variant.title == youtube_track().title));
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
