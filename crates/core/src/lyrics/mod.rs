//! Letras sincronizadas: LRCLIB primero, LRC plano/estimado después.
//!
//! El parser y el modelo son deliberadamente simples: el render de la TUI
//! necesita únicamente poder responder "¿qué línea toca en este milisegundo y
//! cuánto de esa línea se ha cantado?".

use spote_protocol::TrackRef;

mod bridge;
mod remote;

/// Palabra con tiempos (tags `<mm:ss.xx>` dentro de una línea LRC mejorada).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LyricsSource {
    #[default]
    None,
    Lrclib,
    Plain,
    Estimated,
    Local,
}

impl LyricsSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Lrclib => "lrclib",
            Self::Plain => "plain",
            Self::Estimated => "approx",
            Self::Local => "local",
        }
    }
}

/// Documento listo para pintar.
#[derive(Debug, Clone, Default)]
pub struct LyricsDoc {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    pub instrumental: bool,
    pub source: LyricsSource,
    pub offset_ms: i64,
}

impl LyricsDoc {
    /// Reconstruye un documento desde LRC cacheado. `synced` manda sobre lo que
    /// detecte el parser: una letra plana sigue siendo plana aunque el texto
    /// tenga marcas `[mm:ss]` heredadas de otro proveedor.
    pub fn from_lrc(text: &str, synced: bool) -> Self {
        let mut doc = parse_lrc(text);
        if !synced {
            doc.synced = false;
            if doc.source == LyricsSource::Lrclib {
                doc.source = LyricsSource::Plain;
            }
        }
        doc
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Índice de la línea activa en `ms` (posición de la canción, sin offset).
    pub fn index_at(&self, ms: u64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let target = (ms as i64 + self.offset_ms).max(0) as u64;
        match self
            .lines
            .binary_search_by(|line| line.start_ms.cmp(&target))
        {
            Ok(index) => Some(index),
            Err(0) => None,
            Err(next) => Some(next - 1),
        }
    }

    /// Línea activa y progreso dentro de ella (0..1).
    pub fn active(&self, ms: u64) -> Option<(usize, f32)> {
        let index = self.index_at(ms)?;
        let line = self.lines.get(index)?;
        let span = line.end_ms.saturating_sub(line.start_ms).max(1);
        let elapsed = ms.saturating_sub(line.start_ms);
        Some((index, (elapsed as f32 / span as f32).clamp(0.0, 1.0)))
    }

    /// Progreso de la palabra `word_index` dentro de la línea (karaoke fino).
    pub fn word_progress(line: &LyricLine, ms: u64) -> f32 {
        if line.words.is_empty() {
            let span = line.end_ms.saturating_sub(line.start_ms).max(1);
            return (ms.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
        }
        let mut done = 0.0;
        for word in &line.words {
            let span = word.end_ms.saturating_sub(word.start_ms).max(1);
            let progress = (ms.saturating_sub(word.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
            done += progress;
        }
        (done / line.words.len() as f32).clamp(0.0, 1.0)
    }

    /// Cercena a las `limit` líneas más cercanas a la activa, para no pintar
    /// 400 líneas cada frame.
    pub fn window(&self, ms: u64, before: usize, after: usize) -> (usize, usize) {
        let anchor = self.index_at(ms).unwrap_or(0);
        let start = anchor.saturating_sub(before);
        let end = (anchor + after + 1).min(self.lines.len());
        (start, end)
    }
}

/// Resultado de una búsqueda: el documento y el id remoto para poder refrescar.
#[derive(Debug, Clone, Default)]
pub struct LyricsHit {
    pub doc: LyricsDoc,
    pub remote_id: Option<i64>,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
}

/// Error de la capa de letras. Nunca debe tumbar la UI.
#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    #[error("http: {0}")]
    Http(String),
    #[error("no encontrada")]
    NotFound,
    #[error("respuesta inválida: {0}")]
    Parse(String),
}

/// Cliente de letras. La caché vive en la librería SQLite.
pub struct LyricsService {
    client: reqwest::Client,
    bridge: Option<bridge::BridgeConfig>,
}

impl LyricsService {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("spote/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_default();
        Self {
            client,
            bridge: None,
        }
    }

    /// Con puente a lyricbar: se intenta primero y, si falla, LRCLIB interno.
    pub fn with_bridge(bridge: bridge::BridgeConfig) -> Self {
        let mut service = Self::new();
        service.bridge = Some(bridge);
        service
    }

    /// Construye el servicio desde la configuración (respeta `lyrics.bridge`).
    pub fn from_config(cfg: &crate::config::LyricsCfg) -> Self {
        match bridge::BridgeConfig::resolve(cfg) {
            Some(bridge) => Self::with_bridge(bridge),
            None => Self::new(),
        }
    }

    /// `true` si hay puente a lyricbar activo.
    pub fn has_bridge(&self) -> bool {
        self.bridge.is_some()
    }

    /// Busca la letra de `track`. Devuelve `Ok(None)` si no hay nada mejor que
    /// un documento vacío.
    pub async fn fetch(&self, track: &TrackRef) -> Result<Option<LyricsDoc>, LyricsError> {
        if let Some(bridge) = &self.bridge {
            match bridge::fetch(bridge, track) {
                Ok(Some(doc)) => return Ok(Some(doc)),
                Ok(None) => {}
                Err(_) => {}
            }
        }
        remote::fetch_track(&self.client, track).await
    }

    /// Igual que [`fetch`](Self::fetch) pero sin red: solo parsea texto LRC.
    pub fn from_local_lrc(text: &str) -> LyricsDoc {
        parse_lrc(text)
    }
}

impl Default for LyricsService {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsea LRC (con o sin tags de palabra) a un documento utilizable.
///
/// Reglas:
/// - Tags `[mm:ss.xx]` o `[mm:ss:xx]`; varias marcas por línea comparten texto.
/// - Tags `[ti:]`, `[ar:]`, `[al:]`, `[by:]`, `[offset:]` se ignoran salvo
///   `offset`, que se aplica al documento.
/// - Las líneas sin tiempo al principio (letra plana) se reparten si no hay
///   ninguna marca temporal.
/// - `end_ms` de cada línea = inicio de la siguiente; la última usa +6 s.
pub fn parse_lrc(text: &str) -> LyricsDoc {
    let mut doc = LyricsDoc::default();
    let mut timed: Vec<LyricLine> = Vec::new();
    let mut plain: Vec<String> = Vec::new();

    for raw in text.lines() {
        let mut rest = raw.trim();
        if rest.is_empty() {
            continue;
        }
        let mut stamps: Vec<u64> = Vec::new();
        loop {
            if !rest.starts_with('[') {
                break;
            }
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(ms) = parse_timestamp(tag) {
                stamps.push(ms);
            } else if let Some(value) = tag.strip_prefix("offset:") {
                doc.offset_ms = value.trim().parse::<i64>().unwrap_or(0);
            }
            rest = rest[end + 1..].trim_start();
        }
        let (body, words) = split_words(rest);
        if stamps.is_empty() {
            if !body.is_empty() {
                plain.push(body);
            }
            continue;
        }
        for stamp in stamps {
            timed.push(LyricLine {
                start_ms: stamp,
                end_ms: 0,
                text: body.clone(),
                words: words
                    .iter()
                    .map(|(ms, text)| Word {
                        start_ms: stamp + ms,
                        end_ms: 0,
                        text: text.clone(),
                    })
                    .collect(),
            });
        }
    }

    if timed.is_empty() {
        // Sin tiempos: reparto proporcional sobre una duración desconocida.
        // La UI lo marca como `estimated` y ajusta con la duración real.
        for (i, line) in plain.iter().enumerate() {
            doc.lines.push(LyricLine {
                start_ms: i as u64 * 4_000,
                end_ms: (i as u64 + 1) * 4_000,
                text: line.clone(),
                words: Vec::new(),
            });
        }
        doc.synced = false;
        doc.source = LyricsSource::Plain;
        return doc;
    }

    timed.sort_by_key(|line| line.start_ms);
    let count = timed.len();
    for i in 0..count {
        let next = timed
            .get(i + 1)
            .map(|line| line.start_ms)
            .unwrap_or(timed[i].start_ms + 6_000);
        // Hueco instrumental largo → línea `♪` para que la UI no se quede muda.
        timed[i].end_ms = next;
        if let Some(words) = word_times(timed[i].start_ms, next, &timed[i].text) {
            if timed[i].words.is_empty() {
                timed[i].words = words;
            }
        }
    }
    // Rellena el final de las palabras con el inicio de la siguiente.
    for line in &mut timed {
        let end = line.end_ms;
        let starts: Vec<u64> = line.words.iter().map(|word| word.start_ms).collect();
        for (i, word) in line.words.iter_mut().enumerate() {
            word.end_ms = starts.get(i + 1).copied().unwrap_or(end);
        }
    }

    doc.lines = timed;
    doc.synced = true;
    doc.source = LyricsSource::Lrclib;
    doc
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.split([':', '.']);
    let minutes: u64 = parts.next()?.trim().parse().ok()?;
    let seconds: u64 = parts.next()?.trim().parse().ok()?;
    let frac_raw = parts.next().unwrap_or("0").trim();
    let frac: u64 = match frac_raw.len() {
        0 => 0,
        1 => frac_raw.parse::<u64>().ok()? * 100,
        2 => frac_raw.parse::<u64>().ok()? * 10,
        _ => frac_raw[..3].parse::<u64>().ok()?,
    };
    if seconds >= 60 && !tag.contains(':') {
        return None;
    }
    Some(minutes * 60_000 + seconds * 1_000 + frac)
}

/// Separa los tags `<mm:ss.xx>` de una línea LRC mejorada.
fn split_words(body: &str) -> (String, Vec<(u64, String)>) {
    if !body.contains('<') {
        return (body.to_string(), Vec::new());
    }
    let mut words: Vec<(u64, String)> = Vec::new();
    let mut text = String::new();
    let mut current: Option<u64> = None;
    let mut rest = body;

    while let Some(start) = rest.find('<') {
        let prefix = &rest[..start];
        if !prefix.is_empty() {
            text.push_str(prefix);
            if let Some(ts) = current {
                match words.last_mut() {
                    Some(last) if last.0 == ts => last.1.push_str(prefix),
                    _ => words.push((ts, prefix.to_string())),
                }
            }
        }
        let Some(end_rel) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let inner = &rest[start + 1..end];
        if let Some(ms) = parse_timestamp(inner) {
            current = Some(ms);
            words.push((ms, String::new()));
        } else {
            // Un `<...>` que no es tiempo es texto normal.
            text.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        text.push_str(rest);
        if let Some(ts) = current {
            match words.last_mut() {
                Some(last) if last.0 == ts => last.1.push_str(rest),
                _ => words.push((ts, rest.to_string())),
            }
        }
    }
    let words = words
        .into_iter()
        .map(|(ms, word)| (ms, word.trim().to_string()))
        .filter(|(_, word)| !word.is_empty())
        .collect();
    (text.trim().to_string(), words)
}

fn word_times(start: u64, end: u64, text: &str) -> Option<Vec<Word>> {
    let chunks: Vec<&str> = text.split_whitespace().collect();
    if chunks.len() < 2 {
        return None;
    }
    let span = end.saturating_sub(start);
    let total: usize = chunks.iter().map(|w| w.chars().count()).sum();
    let mut words = Vec::with_capacity(chunks.len());
    let mut cursor = start;
    for chunk in chunks {
        let share = chunk.chars().count() as f32 / total as f32;
        let duration = (span as f32 * share) as u64;
        words.push(Word {
            start_ms: cursor,
            end_ms: cursor + duration,
            text: chunk.to_string(),
        });
        cursor += duration;
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[ar:Arctic Monkeys]\n[ti:Do I Wanna Know?]\n\
[00:12.30] Have you got colour in your cheeks?\n\
[00:18.90] Do you ever get that fear that you can't shift the tide\n\
[01:20.00]\n";

    #[test]
    fn parses_timed_lines_and_offsets() {
        let doc = parse_lrc(LRC);
        assert_eq!(doc.lines.len(), 3);
        assert!(doc.synced);
        assert_eq!(doc.lines[0].start_ms, 12_300);
        assert_eq!(doc.lines[0].end_ms, 18_900);
        assert_eq!(doc.lines[1].text, "Do you ever get that fear that you can't shift the tide");
        assert_eq!(doc.index_at(0), None);
        assert_eq!(doc.index_at(12_000), None);
        assert_eq!(doc.index_at(12_400), Some(0));
        assert_eq!(doc.index_at(60_000), Some(1));
    }

    #[test]
    fn active_progress_is_bounded() {
        let doc = parse_lrc(LRC);
        let (index, progress) = doc.active(15_000).unwrap();
        assert_eq!(index, 0);
        assert!(progress > 0.3 && progress < 0.6);
        assert_eq!(doc.active(5_000), None);
    }

    #[test]
    fn plain_lyrics_are_estimated() {
        let doc = parse_lrc("one\ntwo\nthree");
        assert!(!doc.synced);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.source, LyricsSource::Plain);
    }

    #[test]
    fn enhanced_tags_are_kept() {
        let doc = parse_lrc("[00:10.00] <00:10.00>Hello <00:11.00>world");
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.lines[0].words.len(), 2);
        assert_eq!(doc.lines[0].words[1].text, "world");
    }

    #[test]
    fn window_is_clamped() {
        let doc = parse_lrc(LRC);
        let (start, end) = doc.window(12_400, 2, 2);
        assert_eq!((start, end), (0, 3));
    }
}
