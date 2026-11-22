//! Proveedor de letras lrcmux.
//!
//! lrcmux es un agregador compatible con la API de LRCLIB. Se consulta primero
//! el endpoint de compatibilidad (`/compat/lrclib/api/get`), que devuelve
//! `syncedLyrics` y `plainLyrics` con el mismo formato que ya entiende el
//! parser del módulo padre. Si la pista no trae artista o duración, o si esa
//! vía falla, se usa la API nativa (`/get`), que entrega las líneas ya
//! separadas y con un `start` opcional en segundos.
//!
//! La respuesta pasa por la misma puerta de metadatos que LRCLIB: el endpoint
//! compatible puede devolver la versión de otro artista si el título coincide,
//! y un LRC sincronizado de otra grabación desincroniza el karaoke.

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::lyrics::{parse_lrc, LyricLine, LyricsDoc, LyricsError, LyricsSource};
use spote_protocol::TrackRef;

/// Endpoint compatible con LRCLIB: devuelve `syncedLyrics`/`plainLyrics`.
const COMPAT_URL: &str = "https://api.lrcmux.dev/compat/lrclib/api/get";
/// Endpoint nativo: líneas ya separadas, con `start` opcional en segundos.
const NATIVE_URL: &str = "https://api.lrcmux.dev/get";
/// Tope por petición: la UI no puede quedarse colgada esperando la letra.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// Duración estimada de una línea cuando no hay duración de la pista.
const PLAIN_LINE_MS: u64 = 4_000;

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Igual que [`fetch`], pero con el plazo que fija la cascada. El presupuesto
/// de lrcmux nunca puede superar el global: antes una respuesta lenta de este
/// proveedor dejaba a Genius y YouTube sin turno.
pub async fn fetch_before(
    client: &reqwest::Client,
    track: &TrackRef,
    deadline: Instant,
) -> Result<Option<LyricsDoc>, LyricsError> {
    if track.title.trim().is_empty() {
        return Ok(None);
    }
    if deadline.saturating_duration_since(Instant::now()).is_zero() {
        return Ok(None);
    }

    let mut compat_error: Option<LyricsError> = None;

    // La vía de compatibilidad necesita artista y duración para que el endpoint
    // de LRCLIB pueda emparejar la pista. Sin ellos se va directo al nativo.
    if !track.artist.trim().is_empty() && track.duration_ms > 0 {
        match fetch_compat(client, track, deadline).await {
            Ok(Some(doc)) => return Ok(Some(doc)),
            Ok(None) => {}
            Err(err) => {
                tracing::debug!(target: "spote::lyrics", "lrcmux compat: {err}");
                compat_error = Some(err);
            }
        }
    }

    // Si compat agotó el presupuesto, no se lanza una petición condenada a
    // expirar: se devuelve el error para que el llamante pruebe otra fuente.
    if deadline.saturating_duration_since(Instant::now()).is_zero() {
        return match compat_error {
            Some(err) => Err(err),
            None => Ok(None),
        };
    }

    match fetch_native(client, track, deadline).await {
        Ok(Some(doc)) => Ok(Some(doc)),
        Ok(None) => Ok(None),
        Err(err) => {
            tracing::debug!(target: "spote::lyrics", "lrcmux nativo: {err}");
            Err(err)
        }
    }
}

// ---------------------------------------------------------------------------
// Peticiones
// ---------------------------------------------------------------------------

/// `GET /compat/lrclib/api/get`: letra en el formato clásico de LRCLIB.
async fn fetch_compat(
    client: &reqwest::Client,
    track: &TrackRef,
    deadline: Instant,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let mut url = format!(
        "{COMPAT_URL}?track_name={}&artist_name={}",
        url_encode(&track.title),
        url_encode(&track.artist)
    );
    if track.duration_ms > 0 {
        // lrcmux trabaja en segundos enteros, igual que LRCLIB.
        url.push_str(&format!("&duration={}", track.duration_ms / 1_000));
    }
    let value = request_json(client, &url, deadline).await?;
    parse_compat(&value, track)
}

/// `GET /get`: respuesta nativa con las líneas ya separadas.
async fn fetch_native(
    client: &reqwest::Client,
    track: &TrackRef,
    deadline: Instant,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let mut url = format!(
        "{NATIVE_URL}?artist={}&title={}",
        url_encode(&track.artist),
        url_encode(&track.title)
    );
    if track.duration_ms > 0 {
        url.push_str(&format!("&duration={}", track.duration_ms / 1_000));
    }
    url.push_str("&format=json");
    let value = request_json(client, &url, deadline).await?;
    parse_native(&value, track)
}

/// Lanza una petición y decodifica el JSON, respetando el presupuesto global.
async fn request_json(
    client: &reqwest::Client,
    url: &str,
    deadline: Instant,
) -> Result<serde_json::Value, LyricsError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(LyricsError::Http("presupuesto de lrcmux agotado".into()));
    }
    let timeout = remaining.min(REQUEST_TIMEOUT);
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(map_error)?;
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(map_error)
}

// ---------------------------------------------------------------------------
// Parseo (lógica pura, testeable sin red)
// ---------------------------------------------------------------------------

/// Convierte la respuesta de compatibilidad en un documento.
///
/// Prefiere `syncedLyrics` si el LRC tiene tiempos y al menos dos líneas con
/// texto; si no, reparte `plainLyrics` sobre la duración conocida.
pub(crate) fn parse_compat(
    value: &serde_json::Value,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let raw = CompatResponse::deserialize(value).map_err(parse_error)?;

    // El endpoint compatible puede devolver la versión de otro artista si el
    // título coincide; se valida antes de aceptar tiempos.
    if !super::remote::metadata_matches(
        track,
        &raw.track_name,
        &raw.artist_name,
        &raw.album_name,
        raw.duration,
    ) {
        tracing::debug!(
            target: "spote::lyrics",
            "lrcmux: metadatos descartados para '{} - {}'",
            raw.artist_name,
            raw.track_name
        );
        return Ok(None);
    }

    let duration_ms = effective_duration_ms(track.duration_ms, raw.duration);
    if let Some(text) = raw
        .synced_lyrics
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let parsed = parse_lrc(text);
        // Una sola marca (o un LRC lleno de huecos instrumentales) no da para
        // karaoke: se prefiere la letra plana, que al menos se puede repartir.
        if parsed.synced
            && lyric_line_count(&parsed) >= 2
            && super::remote::sync_problem(&parsed, duration_ms).is_none()
        {
            return Ok(Some(LyricsDoc {
                lines: parsed.lines,
                synced: true,
                instrumental: parsed.instrumental,
                source: lrcmux_source(),
                offset_ms: parsed.offset_ms,
                aligned: false,
            }));
        }
    }

    let Some(plain) = raw
        .plain_lyrics
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    else {
        return Ok(None);
    };
    let texts = split_plain_lines(plain);
    Ok(build_plain_doc(texts, duration_ms))
}

/// Convierte la respuesta nativa (`lines` + `start` opcional) en un documento.
///
/// Hay sincronización cuando al menos dos líneas con texto traen un `start`
/// numérico **y** la mayoría de las líneas están cronometradas. En caso
/// contrario las líneas se reparten como letra plana.
pub(crate) fn parse_native(
    value: &serde_json::Value,
    track: &TrackRef,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let raw = NativeResponse::deserialize(value).map_err(parse_error)?;
    if !super::remote::metadata_matches(
        track,
        &raw.track.title,
        &raw.track.artist,
        &raw.track.album,
        raw.track.duration,
    ) {
        tracing::debug!(
            target: "spote::lyrics",
            "lrcmux nativo: metadatos descartados para '{} - {}'",
            raw.track.artist,
            raw.track.title
        );
        return Ok(None);
    }
    let rows: Vec<(Option<u64>, String)> = raw
        .lines
        .iter()
        .filter_map(|line| {
            let text = clean_text(&line.text);
            if text.is_empty() {
                return None;
            }
            let start = line
                .start
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .map(|seconds| (seconds * 1_000.0).round() as u64);
            Some((start, text))
        })
        .collect();
    if rows.is_empty() {
        return Ok(None);
    }

    let duration_ms = effective_duration_ms(track.duration_ms, raw.track.duration);
    let timed = rows.iter().filter(|(start, _)| start.is_some()).count();
    // Dos líneas con tiempo entre cincuenta sin él no es una letra sincronizada:
    // el resto se colocaría a saltos de 4 s y el karaoke iría a trompicones.
    let mostly_timed = timed >= 2 && timed * 100 >= rows.len() * 60;
    if mostly_timed {
        let doc = build_native_synced(rows, duration_ms);
        if super::remote::sync_problem(&doc, duration_ms).is_none() {
            return Ok(Some(doc));
        }
        // Tiempos imposibles (primer verso tardísimo, fuera de la pista):
        // mejor letra plana que un karaoke mintiendo.
        let texts = doc.lines.into_iter().map(|line| line.text).collect();
        return Ok(build_plain_doc(texts, duration_ms));
    }
    let texts = rows.into_iter().map(|(_, text)| text).collect();
    Ok(build_plain_doc(texts, duration_ms))
}

// ---------------------------------------------------------------------------
// Construcción del documento
// ---------------------------------------------------------------------------

/// Construye el documento sincronizado de la API nativa.
///
/// La API devuelve las líneas en orden, pero se fuerza monotonía en los
/// inicios por si una marca viene repetida o desordenada. Una línea sin `start`
/// hereda un hueco de [`PLAIN_LINE_MS`] desde el inicio anterior.
fn build_native_synced(rows: Vec<(Option<u64>, String)>, duration_ms: u64) -> LyricsDoc {
    let count = rows.len();
    let mut starts = Vec::with_capacity(count);
    let mut last_start = 0u64;
    let mut next_guess = 0u64;
    for (start, _) in &rows {
        let value = match start {
            Some(ms) => (*ms).max(last_start),
            None => next_guess.max(last_start),
        };
        starts.push(value);
        last_start = value;
        next_guess = value.saturating_add(PLAIN_LINE_MS);
    }

    let mut lines = Vec::with_capacity(count);
    for (index, (_, text)) in rows.into_iter().enumerate() {
        let start_ms = starts[index];
        let end_ms = starts.get(index + 1).copied().unwrap_or_else(|| {
            let natural = start_ms.saturating_add(PLAIN_LINE_MS);
            if duration_ms > start_ms {
                natural.min(duration_ms)
            } else {
                natural
            }
        });
        lines.push(LyricLine {
            start_ms,
            end_ms,
            text,
            words: Vec::new(),
            words_exact: false,
        });
    }

    LyricsDoc {
        lines,
        synced: true,
        instrumental: false,
        source: lrcmux_source(),
        offset_ms: 0,
        aligned: false,
    }
}

/// Reparte líneas sin tiempos sobre la duración total.
///
/// Si no hay duración, cada línea ocupa [`PLAIN_LINE_MS`] (4 s). El documento
/// queda como no sincronizado y con origen plano.
fn build_plain_doc(texts: Vec<String>, duration_ms: u64) -> Option<LyricsDoc> {
    if texts.is_empty() {
        return None;
    }
    let count = texts.len() as u64;
    let total = if duration_ms > 0 {
        duration_ms
    } else {
        count * PLAIN_LINE_MS
    };

    let mut doc = LyricsDoc {
        lines: Vec::with_capacity(texts.len()),
        synced: false,
        instrumental: false,
        source: LyricsSource::Plain,
        ..LyricsDoc::default()
    };
    for (index, text) in texts.into_iter().enumerate() {
        let index = index as u64;
        doc.lines.push(LyricLine {
            start_ms: index * total / count,
            end_ms: (index + 1) * total / count,
            text,
            words: Vec::new(),
            words_exact: false,
        });
    }
    Some(doc)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Origen de todo documento servido por lrcmux.
fn lrcmux_source() -> LyricsSource {
    LyricsSource::Lrcmux
}

/// Duración de referencia: manda la de la pista; si no, la de la respuesta.
fn effective_duration_ms(track_ms: u64, fallback_seconds: f64) -> u64 {
    if track_ms > 0 {
        return track_ms;
    }
    if fallback_seconds.is_finite() && fallback_seconds > 0.0 {
        return (fallback_seconds * 1_000.0).round() as u64;
    }
    0
}

/// Normaliza CRLF y recorta una línea suelta.
fn clean_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

/// Parte un texto plano en líneas no vacías, normalizando CRLF.
fn split_plain_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `true` si el texto es letra de verdad y no una marca instrumental.
fn has_lyric_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed != "♪" && trimmed.chars().any(|ch| ch.is_alphanumeric())
}

/// Número de líneas con texto aprovechable (los `♪` no cuentan).
fn lyric_line_count(doc: &LyricsDoc) -> usize {
    doc.lines
        .iter()
        .filter(|line| has_lyric_text(&line.text))
        .count()
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

/// Decodificar JSON roto no es un fallo de red.
fn map_error(err: reqwest::Error) -> LyricsError {
    if err.is_decode() {
        LyricsError::Parse(err.to_string())
    } else {
        LyricsError::Http(err.to_string())
    }
}

/// Traduce un HTTP no exitoso al error de la capa de letras.
fn status_error(status: reqwest::StatusCode) -> LyricsError {
    if status == reqwest::StatusCode::NOT_FOUND {
        LyricsError::NotFound
    } else {
        LyricsError::Http(format!("lrcmux respondió {}", status.as_u16()))
    }
}

fn parse_error(err: serde_json::Error) -> LyricsError {
    LyricsError::Parse(err.to_string())
}

// ---------------------------------------------------------------------------
// Respuestas crudas
// ---------------------------------------------------------------------------

/// Respuesta de `/compat/lrclib/api/get`. Los metadatos se usan solo para
/// validar que la letra pertenece a la pista pedida.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompatResponse {
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

/// Respuesta de `/get`.
#[derive(Debug, Clone, Default, Deserialize)]
struct NativeResponse {
    #[serde(default)]
    track: NativeTrack,
    #[serde(default)]
    lines: Vec<NativeLine>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NativeTrack {
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: String,
    /// Segundos, con decimales.
    #[serde(default)]
    duration: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NativeLine {
    #[serde(default)]
    text: String,
    /// Segundos desde el inicio de la pista; ausente en la letra plana.
    #[serde(default)]
    start: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track() -> TrackRef {
        TrackRef {
            key: "isrc:TEST123".into(),
            title: "Do I Wanna Know?".into(),
            artist: "Arctic Monkeys".into(),
            album: Some("AM".into()),
            duration_ms: 272_000,
            ..TrackRef::default()
        }
    }

    /// Pista sin duración para que la respuesta JSON sea la que marque el
    /// reparto de la letra plana.
    fn track_without_duration() -> TrackRef {
        TrackRef {
            duration_ms: 0,
            ..track()
        }
    }

    #[test]
    fn compat_parses_synced_lyrics() {
        let value = json!({
            "trackName": "Do I Wanna Know?",
            "artistName": "Arctic Monkeys",
            "albumName": "AM",
            "duration": 272.0,
            "plainLyrics": "uno\ndos",
            "syncedLyrics": "[00:12.30] Have you got colour in your cheeks?\n[00:18.90] Do you ever get that fear"
        });
        let doc = parse_compat(&value, &track())
            .expect("respuesta válida")
            .expect("hay letra");

        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Lrcmux);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].start_ms, 12_300);
        assert_eq!(doc.lines[0].end_ms, 18_900);
        assert!(doc.lines.iter().all(|line| !line.text.is_empty()));
    }

    #[test]
    fn compat_parses_plain_lyrics() {
        let value = json!({
            "duration": 120.0,
            "plainLyrics": "uno\r\n\r\ndos\r\ntres",
            "syncedLyrics": null
        });
        let doc = parse_compat(&value, &track_without_duration())
            .expect("respuesta válida")
            .expect("hay letra");

        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0].text, "uno");
        assert_eq!(doc.lines[0].start_ms, 0);
        assert_eq!(doc.lines[0].end_ms, 40_000);
        assert_eq!(doc.lines[2].end_ms, 120_000);
    }

    #[test]
    fn compat_without_usable_sync_falls_back_to_plain() {
        // Una sola línea sincronizada no da para karaoke: se usa la plana.
        let value = json!({
            "duration": 20.0,
            "syncedLyrics": "[00:01.00]solo una línea",
            "plainLyrics": "uno\ndos"
        });
        let doc = parse_compat(&value, &track_without_duration())
            .expect("respuesta válida")
            .expect("hay letra");

        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "uno");
        assert_eq!(doc.lines[0].start_ms, 0);
        assert_eq!(doc.lines[0].end_ms, 10_000);
    }

    #[test]
    fn native_parses_lines_with_start() {
        let value = json!({
            "track": { "title": "Do I Wanna Know?", "artist": "Arctic Monkeys", "album": "AM", "duration": 272.0 },
            "lines": [
                { "text": "uno", "start": 1.5 },
                { "text": "dos", "start": 5.25 }
            ]
        });
        let doc = parse_native(&value, &track())
            .expect("respuesta válida")
            .expect("hay letra");

        assert!(doc.synced);
        assert_eq!(doc.source, LyricsSource::Lrcmux);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].start_ms, 1_500);
        assert_eq!(doc.lines[0].end_ms, 5_250);
        assert_eq!(doc.lines[1].start_ms, 5_250);
        assert_eq!(doc.lines[1].end_ms, 9_250);
        assert!(doc
            .lines
            .iter()
            .all(|line| line.words.is_empty() && !line.words_exact));
    }

    #[test]
    fn native_without_start_is_plain_text() {
        let value = json!({
            "track": { "duration": 90.0 },
            "lines": [
                { "text": "uno" },
                { "text": "dos" },
                { "text": "tres" }
            ]
        });
        let doc = parse_native(&value, &track_without_duration())
            .expect("respuesta válida")
            .expect("hay letra");

        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0].start_ms, 0);
        assert_eq!(doc.lines[1].start_ms, 30_000);
        assert_eq!(doc.lines[2].start_ms, 60_000);
        assert_eq!(doc.lines[2].end_ms, 90_000);
    }

    #[test]
    fn empty_lines_and_crlf_are_normalized() {
        // Compat: CRLF, líneas en blanco y espacios se descartan.
        let compat = json!({
            "duration": 8.0,
            "plainLyrics": "\r\nuno\r\n\r\n  dos  \r\n"
        });
        let doc = parse_compat(&compat, &track_without_duration())
            .expect("respuesta válida")
            .expect("hay letra");
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "uno");
        assert_eq!(doc.lines[1].text, "dos");

        // Nativo: una línea vacía no rompe la detección de sincronizada ni el
        // reparto temporal de las que sí tienen texto.
        let native = json!({
            "track": { "duration": 20.0 },
            "lines": [
                { "text": "", "start": 0.0 },
                { "text": "  uno  ", "start": 2.0 },
                { "text": "\r\n", "start": 3.0 },
                { "text": "dos", "start": 6.0 }
            ]
        });
        let doc = parse_native(&native, &track_without_duration())
            .expect("respuesta válida")
            .expect("hay letra");
        assert!(doc.synced);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "uno");
        assert_eq!(doc.lines[0].start_ms, 2_000);
        assert_eq!(doc.lines[1].text, "dos");
    }

    #[test]
    fn compat_without_lyrics_returns_none() {
        let value = json!({
            "duration": 10.0,
            "plainLyrics": "\r\n   \r\n",
            "syncedLyrics": null
        });
        assert!(parse_compat(&value, &track())
            .expect("respuesta válida")
            .is_none());
    }

    #[test]
    fn native_without_lyrics_returns_none() {
        let value = json!({
            "track": { "duration": 10.0 },
            "lines": []
        });
        assert!(parse_native(&value, &track())
            .expect("respuesta válida")
            .is_none());
    }

    #[test]
    fn compat_rejects_a_response_from_another_artist() {
        let value = json!({
            "trackName": "Do I Wanna Know?",
            "artistName": "Some Cover Band",
            "albumName": "Covers",
            "duration": 272.0,
            "syncedLyrics": "[00:12.30] Have you got colour in your cheeks?\n[00:18.90] Do you ever get that fear"
        });
        assert!(parse_compat(&value, &track())
            .expect("respuesta válida")
            .is_none());
    }

    #[test]
    fn compat_rejects_sync_with_impossible_times() {
        let value = json!({
            "trackName": "Do I Wanna Know?",
            "artistName": "Arctic Monkeys",
            "duration": 272.0,
            "syncedLyrics": "[00:45.00] uno\n[02:40.00] dos\n[05:00.00] tres",
            "plainLyrics": "uno\ndos\ntres"
        });
        let doc = parse_compat(&value, &track())
            .expect("respuesta válida")
            .expect("hay letra plana");
        assert!(!doc.synced, "la sync imposible se descarta");
        assert_eq!(doc.source, LyricsSource::Plain);
    }

    #[test]
    fn native_with_only_a_few_timed_lines_is_not_synced() {
        // Dos líneas con tiempo y cuatro sin él no es karaoke: las demás se
        // colocarían a saltos de 4 s y el resultado iría a trompicones.
        let value = json!({
            "track": { "title": "Do I Wanna Know?", "artist": "Arctic Monkeys", "duration": 272.0 },
            "lines": [
                { "text": "uno", "start": 1.0 },
                { "text": "dos", "start": 5.0 },
                { "text": "tres" },
                { "text": "cuatro" },
                { "text": "cinco" },
                { "text": "seis" }
            ]
        });
        let doc = parse_native(&value, &track())
            .expect("respuesta válida")
            .expect("hay letra");
        assert!(!doc.synced);
        assert_eq!(doc.source, LyricsSource::Plain);
    }

    #[tokio::test]
    #[ignore = "diagnóstico manual con red"]
    async fn live_lrcmux_returns_something() {
        let client = reqwest::Client::new();
        let mut t = track();
        t.title = "Borderline".into();
        t.artist = "Tame Impala".into();
        t.duration_ms = 238_000;
        let result = fetch_before(&client, &t, Instant::now() + Duration::from_secs(12)).await;
        eprintln!(
            "lrcmux live => {:?}",
            result.map(|doc| doc.map(|d| (d.synced, d.lines.len(), d.source)))
        );
    }

    #[tokio::test]
    async fn empty_title_returns_none_without_touching_the_network() {
        let client = reqwest::Client::new();
        let mut blank = track();
        blank.title = "   ".into();
        let result = fetch_before(&client, &blank, Instant::now() + Duration::from_secs(12))
            .await
            .expect("no hay error");
        assert!(result.is_none());
    }
}
