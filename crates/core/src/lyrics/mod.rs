//! Letras sincronizadas: Genius manda en el texto, los proveedores con tiempos
//! (LRCLIB, lrcmux, captions) solo aportan la rejilla temporal.
//!
//! El parser y el modelo son deliberadamente simples: el render de la TUI
//! necesita únicamente poder responder "¿qué línea toca en este milisegundo y
//! cuánto de esa línea se ha cantado?".
//!
//! Regla de la cascada: si Genius devuelve letra, ese es el texto que se pinta.
//! Ningún otro proveedor, caption ni ASR puede sustituirlo; como mucho le
//! prestan sus tiempos cuando el texto coincide de forma razonable. El ASR
//! únicamente alinea un documento que ya existe: nunca inventa líneas.

use spote_protocol::TrackRef;

mod align;
mod asr;
mod lrcmux;
mod remote;
mod text;
mod youtube;

/// Palabra con tiempos (tags `<mm:ss.xx>` dentro de una línea LRC mejorada).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Va entre paréntesis/corchetes: adlib o coro de fondo. Se muestra, pero
    /// no bloquea el progreso ni obliga a esperar para pasar a la siguiente.
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<Word>,
    /// `true` si los tiempos de palabra son fiables (tags LRC mejorados o
    /// alineación contra el audio). La UI no debe volver a repartirlos.
    pub words_exact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LyricsSource {
    #[default]
    None,
    /// Texto autoritativo de Genius. Si está sincronizado, los tiempos vienen
    /// de otro proveedor (LRCLIB, lrcmux o captions) y el texto sigue siendo el
    /// de Genius.
    Genius,
    Lrclib,
    Lrcmux,
    Captions,
    Plain,
    Estimated,
    /// Letra generada transcribiendo el audio con ASR: texto inventado, no
    /// una letra de un proveedor. Solo aparece si el usuario lo permite.
    Asr,
    Local,
    Manual,
}

impl LyricsSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Genius => "genius",
            Self::Lrclib => "lrclib",
            Self::Lrcmux => "lrcmux",
            Self::Captions => "captions",
            Self::Plain => "plain",
            Self::Estimated => "approx",
            Self::Asr => "asr",
            Self::Local => "local",
            Self::Manual => "manual",
        }
    }

    /// Recupera la procedencia guardada en caché. Si no se conoce se conserva
    /// la que dedujo el parser, para no perder el estado al recargar.
    pub fn from_label(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "genius" => Some(Self::Genius),
            "lrclib" => Some(Self::Lrclib),
            "lrcmux" => Some(Self::Lrcmux),
            "captions" => Some(Self::Captions),
            "plain" => Some(Self::Plain),
            "approx" => Some(Self::Estimated),
            "asr" => Some(Self::Asr),
            "local" => Some(Self::Local),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }

    /// Texto inventado: captions usadas como letra o transcripción ASR. No es
    /// lo mismo que una letra real cuyos tiempos vienen de captions.
    pub fn is_generated(self) -> bool {
        matches!(self, Self::Captions | Self::Asr)
    }

    /// Etiqueta para la UI, marcando las fuentes generadas.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::Captions => "captions · inventada",
            Self::Asr => "asr · inventada",
            source => source.label(),
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
    /// Ajuste manual del usuario. El `[offset:]` que trae un LRC de proveedor
    /// se hornea en los tiempos de las líneas al parsear (ver [`parse_lrc`]),
    /// así que aquí no se suma dos veces. `0` = sin ajuste.
    pub offset_ms: i64,
    /// Tiempos refinados contra la voz real (ASR/audio), no solo LRC.
    pub aligned: bool,
    /// Los tiempos vienen de captions del vídeo exacto, pero el texto es de
    /// una fuente real. Evita volver a pasar ASR o un offset automático sobre
    /// una sincronización que ya era buena.
    pub captions_timed: bool,
}

/// Una línea es "de fondo" cuando todo su contenido va entre paréntesis o
/// corchetes: `(oh oh)`, `[yeah]`, `(background vocals)`. Esas líneas no deben
/// robar la línea activa a la letra principal; si no, el karaoke se queda
/// esperando adlibs mientras la voz principal ya avanzó.
pub fn is_background_line(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text == INSTRUMENTAL_MARK {
        return false;
    }
    let mut depth = 0i32;
    let mut has_outside = false;
    for ch in text.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 && ch.is_alphanumeric() => {
                has_outside = true;
            }
            _ => {}
        }
    }
    !has_outside
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

    /// Tiempo de documento: posición de la canción con el offset **de usuario**
    /// aplicado. El offset del proveedor ya está horneado en `start_ms`.
    ///
    /// Toda comparación contra `LyricLine.start_ms` / `Word.start_ms` debe usar
    /// esta base. Si la línea se elige con offset pero el karaoke se pinta sin
    /// él, la línea salta cuando la frase aún va por la mitad.
    pub fn aligned_ms(&self, ms: u64) -> u64 {
        (ms as i64 + self.offset_ms).max(0) as u64
    }

    /// Índice de la línea activa para un tiempo de documento **ya alineado**.
    /// No vuelve a sumar el offset: úsalo cuando ya llamaste a [`Self::aligned_ms`].
    pub fn index_at_target(&self, target: u64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let mut index = match self
            .lines
            .binary_search_by(|line| line.start_ms.cmp(&target))
        {
            Ok(index) => index,
            Err(0) => return None,
            Err(next) => next - 1,
        };
        // Si la marca actual es un adlib entre paréntesis, la línea activa
        // sigue siendo la última letra principal. Así no se queda "atascada"
        // esperando coros de fondo.
        while index > 0 && is_background_line(&self.lines[index].text) {
            index -= 1;
        }
        Some(index)
    }

    /// Índice de la línea activa en `ms` (posición de la canción).
    pub fn index_at(&self, ms: u64) -> Option<usize> {
        self.index_at_target(self.aligned_ms(ms))
    }

    /// Línea activa y progreso dentro de ella (0..1). Ambos comparten la misma
    /// base temporal: el tiempo de documento con offset.
    pub fn active(&self, ms: u64) -> Option<(usize, f32)> {
        self.active_at_target(self.aligned_ms(ms))
    }

    /// Igual que [`Self::active`] pero para un tiempo ya alineado.
    pub fn active_at_target(&self, target: u64) -> Option<(usize, f32)> {
        let index = self.index_at_target(target)?;
        let line = self.lines.get(index)?;
        let span = line.end_ms.saturating_sub(line.start_ms).max(1);
        let elapsed = target.saturating_sub(line.start_ms);
        Some((index, (elapsed as f32 / span as f32).clamp(0.0, 1.0)))
    }

    /// Progreso de la línea `index` en `ms` de canción, con offset aplicado.
    pub fn word_progress_at(&self, ms: u64, index: usize) -> f32 {
        let target = self.aligned_ms(ms);
        self.lines
            .get(index)
            .map_or(0.0, |line| Self::word_progress(line, target))
    }

    /// Progreso de las palabras de una línea. `ms` es tiempo de **documento**
    /// (ya con offset); para tiempo de canción usa [`Self::word_progress_at`].
    pub fn word_progress(line: &LyricLine, ms: u64) -> f32 {
        if line.words.is_empty() {
            let span = line.end_ms.saturating_sub(line.start_ms).max(1);
            return (ms.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
        }
        let mut done = 0.0;
        let mut count = 0;
        for word in &line.words {
            // Los adlibs entre paréntesis no cuentan: la frase principal puede
            // terminar (y empezar la siguiente) mientras todavía se oyen.
            if word.optional {
                continue;
            }
            let span = word.end_ms.saturating_sub(word.start_ms).max(1);
            let progress = (ms.saturating_sub(word.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
            done += progress;
            count += 1;
        }
        if count == 0 {
            let span = line.end_ms.saturating_sub(line.start_ms).max(1);
            return (ms.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
        }
        (done / count as f32).clamp(0.0, 1.0)
    }

    /// Cercena a las `limit` líneas más cercanas a la activa, para no pintar
    /// 400 líneas cada frame.
    pub fn window(&self, ms: u64, before: usize, after: usize) -> (usize, usize) {
        let anchor = self.index_at(ms).unwrap_or(0);
        let start = anchor.saturating_sub(before);
        let end = (anchor + after + 1).min(self.lines.len());
        (start, end)
    }

    /// Estima un desplazamiento global comparando los inicios de las líneas
    /// principales con la actividad vocal/ritmo del mapa de audio de la pista.
    ///
    /// Es conservador a propósito: solo devuelve algo si el pico destaca sobre
    /// el resto de candidatos. Un LRC de otra versión no debe desplazarse a
    /// ciegas; para eso ya está el ajuste manual por pista.
    pub fn estimate_offset_from_map(&self, map: &crate::audio_map::SongMap) -> Option<i64> {
        if !self.synced || map.buckets.is_empty() || self.offset_ms != 0 {
            return None;
        }
        let starts: Vec<u64> = self
            .lines
            .iter()
            .filter(|line| line.text != INSTRUMENTAL_MARK && !is_background_line(&line.text))
            .map(|line| line.start_ms)
            .collect();
        if starts.len() < 4 {
            return None;
        }
        let max_offset = 25_000i64;
        let step = 100i64;
        let mut best: Option<(f32, i64)> = None;
        let mut runner_up = 0.0f32;
        let mut all_scores: Vec<f32> = Vec::new();
        let mut offset = -max_offset;
        while offset <= max_offset {
            let mut total = 0.0f32;
            let mut used = 0usize;
            for start in &starts {
                let at = *start as i64 + offset;
                if at < 0 {
                    continue;
                }
                // Los buckets reales se etiquetan con ms exactos (255.46...),
                // no con `i * BUCKET_MS` truncado: indexar por división
                // acumula casi medio segundo al final de una canción.
                let next = map.buckets.partition_point(|b| b.at_ms <= at as u64);
                if next == 0 || next > map.buckets.len() {
                    continue;
                }
                let idx = next - 1;
                let current = &map.buckets[idx];
                let before = idx.saturating_sub(2);
                let rise = (current.vocal - map.buckets[before].vocal).max(0.0);
                let beat = if current.beat { 0.28 } else { 0.0 };
                total += current.vocal * 0.70 + rise * 1.5 + beat;
                used += 1;
            }
            if used >= 3 {
                // Penaliza desplazamientos enormes: la intro de un vídeo suele
                // ser de segundos, no de medio minuto.
                let prior = 1.0 - (offset.abs() as f32 / max_offset as f32) * 0.18;
                let score = total / used as f32 * prior;
                all_scores.push(score);
                match best {
                    Some((best_score, _)) if score > best_score => {
                        runner_up = best_score;
                        best = Some((score, offset));
                    }
                    Some((_, best_offset))
                        if (offset - best_offset).abs() >= 2_000 && score > runner_up =>
                    {
                        runner_up = score;
                    }
                    None => best = Some((score, offset)),
                    _ => {}
                }
            }
            offset += step;
        }
        let (score, offset) = best?;
        // Sin una separación clara frente a la mediana (y frente al segundo
        // pico lejano), se prefiere no tocar nada antes que empeorar una letra
        // que ya estaba bien. Los estribillos repetidos producen picos falsos
        // separados media frase; exigir un segundo pico evita adoptarlos.
        let mut sorted = all_scores.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
        if score < 0.18
            || score < median * 1.35
            || (runner_up > 0.0 && score < runner_up * 1.15)
            || offset.abs() < 250
        {
            return None;
        }
        Some(offset)
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

/// Candidato de una búsqueda manual de letras. No pasa por el filtro
/// automático: lo elige y lo acepta el usuario.
#[derive(Debug, Clone, Default)]
pub struct LyricsCandidate {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub synced: bool,
    pub source: String,
    pub lrc: Option<String>,
    pub plain: Option<String>,
}

impl LyricsCandidate {
    /// Texto que se enseña en la previsualización, sin marcas de tiempo.
    pub fn preview_lines(&self) -> Vec<String> {
        let text = self
            .lrc
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .or(self.plain.as_deref())
            .unwrap_or("");
        let doc = parse_lrc(text);
        let mut lines: Vec<String> = doc
            .lines
            .into_iter()
            .map(|line| line.text.trim().to_string())
            .filter(|line| !line.is_empty() && line != INSTRUMENTAL_MARK)
            .collect();
        if lines.is_empty() {
            lines = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
        }
        lines
    }
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

/// Busca un script auxiliar en los sitios razonables: ruta configurada,
/// `SPOTE_ROOT`, instalación junto a los datos o checkout del repo.
pub(crate) fn resolve_script(configured: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::PathBuf::from(configured);
    if candidate.is_file() {
        return Some(candidate);
    }
    if let Some(root) = std::env::var_os("SPOTE_ROOT") {
        let path = std::path::PathBuf::from(root).join(configured);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(name) = candidate.file_name() {
        let installed = crate::config::data_dir().join("scripts").join(name);
        if installed.is_file() {
            return Some(installed);
        }
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(configured);
    repo.is_file().then_some(repo)
}

/// Cliente de letras. La caché vive en la librería SQLite.
pub struct LyricsService {
    client: reqwest::Client,
    asr: Option<asr::AsrConfig>,
    /// Si es `false`, captions y ASR solo aportan tiempos. Si es `true`,
    /// pueden generar texto cuando no hay una letra fiable.
    allow_generated: bool,
}

impl LyricsService {
    pub fn new() -> Self {
        crate::crypto::install();
        let client = reqwest::Client::builder()
            .user_agent(concat!("spote/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_default();
        Self {
            client,
            asr: None,
            allow_generated: false,
        }
    }

    /// Cambia en caliente si se permite inventar letra con captions/ASR.
    pub fn set_allow_generated(&mut self, allow: bool) {
        self.allow_generated = allow;
    }

    /// Construye el servicio desde la configuración.
    pub fn from_config(cfg: &crate::config::LyricsCfg) -> Self {
        let mut service = Self::new();
        service.asr = asr::AsrConfig::resolve(cfg);
        service.allow_generated = cfg.allow_generated;
        service
    }

    /// Busca la letra de `track`. Devuelve `Ok(None)` si no hay nada mejor que
    /// un documento vacío.
    pub async fn fetch(&self, track: &TrackRef) -> Result<Option<LyricsDoc>, LyricsError> {
        self.fetch_with_audio(track, None, false).await
    }

    /// Igual que [`fetch`](Self::fetch), pero afina la letra contra el audio
    /// local: lee el audio con ASR y alinea la letra conocida en Rust.
    pub async fn fetch_with_audio(
        &self,
        track: &TrackRef,
        audio: Option<&std::path::Path>,
        force_align: bool,
    ) -> Result<Option<LyricsDoc>, LyricsError> {
        self.fetch_with_audio_cancellable(track, audio, force_align, None)
            .await
    }

    /// Variante cancelable que usa el worker de la TUI. `cancel` se consulta
    /// entre lecturas del ASR; al cambiar de pista se activa y el proceso del
    /// transcriptor se mata en vez de bloquear la cola.
    ///
    /// El ASR solo aporta tiempos si hay documento. Cuando el usuario activa
    /// `allow_generated`, captions y ASR pueden inventar el texto: captions
    /// como último recurso en `remote` y ASR transcribiendo el audio local.
    /// Las líneas que se alinean sobre una letra existente son siempre las del
    /// documento de entrada ([`asr_reference_lines`]).
    pub async fn fetch_with_audio_cancellable(
        &self,
        track: &TrackRef,
        audio: Option<&std::path::Path>,
        force_align: bool,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Option<LyricsDoc>, LyricsError> {
        let remote = remote::fetch_track(&self.client, track, self.allow_generated).await;
        let doc = match remote {
            Ok(doc) => doc,
            // Sin red, el modo "inventar" aún puede transcribir el audio local.
            Err(err) if self.allow_generated && audio.is_some() => {
                tracing::debug!(target: "spote::lyrics", "red de letras falló: {err}");
                None
            }
            Err(err) => return Err(err),
        };
        // El usuario pudo apagar el modo mientras la petición estaba en vuelo.
        if doc.as_ref().is_some_and(|doc| doc.source.is_generated()) && !self.allow_generated {
            return Ok(None);
        }
        let Some(mut doc) = doc else {
            if !self.allow_generated {
                return Ok(None);
            }
            // Sin nada fiable, el modo "inventar" transcribe el audio local.
            let (Some(audio), Some(config)) = (audio, self.asr.as_ref()) else {
                return Ok(None);
            };
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
                return Ok(None);
            }
            let duration_s = track.duration_ms as f64 / 1000.0;
            let Some((lines, confidence)) =
                asr::transcribe_track_with(config, audio, duration_s, cancel, None)
            else {
                return Ok(None);
            };
            if lines.len() < 2 || confidence < 0.30 {
                tracing::debug!(
                    target: "spote::lyrics",
                    "ASR generado con confianza {confidence:.2} insuficiente; sin letra"
                );
                return Ok(None);
            }
            tracing::info!(
                target: "spote::lyrics",
                "letra generada por ASR: {} líneas, confianza {:.2}",
                lines.len(),
                confidence
            );
            return Ok(Some(LyricsDoc {
                lines,
                synced: true,
                instrumental: false,
                source: LyricsSource::Asr,
                offset_ms: 0,
                aligned: true,
                captions_timed: false,
            }));
        };
        let (Some(audio), Some(config)) = (audio, self.asr.as_ref()) else {
            return Ok(Some(doc));
        };
        if !needs_alignment(&doc, force_align) {
            return Ok(Some(doc));
        }
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
            return Ok(Some(doc));
        }
        let lines = asr_reference_lines(&doc);
        if lines.len() < 2 {
            return Ok(Some(doc));
        }
        let duration_s = track.duration_ms as f64 / 1000.0;
        // Una letra ya sincronizada solo se sustituye si el ASR mejora con
        // claridad; una plana se acepta con menos confianza porque cualquier
        // cosa es mejor que el reparto a ojo.
        let minimum = if doc.synced { 0.45 } else { 0.35 };
        match asr::align_track_with(config, audio, duration_s, &lines, cancel, None) {
            Some((aligned, confidence)) if confidence >= minimum && aligned.len() >= 2 => {
                tracing::info!(
                    target: "spote::lyrics",
                    "letra alineada con ASR: {} líneas, confianza {:.2}",
                    aligned.len(),
                    confidence
                );
                doc.lines = aligned;
                doc.aligned = true;
                doc.synced = true;
                // Los tiempos del ASR son absolutos del audio; el offset del
                // proveedor ya no aplica (antes se sumaba otra vez al pintar).
                doc.offset_ms = 0;
            }
            Some((_, confidence)) => {
                tracing::debug!(
                    target: "spote::lyrics",
                    "ASR con confianza {confidence:.2} insuficiente; se conserva la letra anterior"
                );
            }
            None => {
                tracing::debug!(
                    target: "spote::lyrics",
                    "ASR sin una alineación fiable; se mantiene la letra estimada"
                );
            }
        }
        Ok(Some(doc))
    }

    /// Igual que [`fetch`](Self::fetch) pero sin red: solo parsea texto LRC.
    pub fn from_local_lrc(text: &str) -> LyricsDoc {
        parse_lrc(text)
    }

    /// Busca candidatos para la elección manual desde la pestaña de letra.
    /// No aplica el ranking automático: el usuario decide cuál es la buena.
    pub async fn search_candidates(
        &self,
        query: &str,
    ) -> Result<Vec<LyricsCandidate>, LyricsError> {
        remote::search_candidates(&self.client, query).await
    }
}

/// Líneas para el ASR en tiempo de audio: sin marcas instrumentales y sin el
/// offset de usuario (el ASR devuelve tiempos absolutos del fichero).
fn asr_reference_lines(doc: &LyricsDoc) -> Vec<LyricLine> {
    doc.lines
        .iter()
        .filter(|line| {
            let text = line.text.trim();
            !text.is_empty()
                && text != INSTRUMENTAL_MARK
                && text.chars().any(|ch| ch.is_alphanumeric())
        })
        .map(|line| {
            let mut line = line.clone();
            line.start_ms = (line.start_ms as i64 - doc.offset_ms).max(0) as u64;
            line.end_ms = (line.end_ms as i64 - doc.offset_ms).max(0) as u64;
            line.words.clear();
            line.words_exact = false;
            line
        })
        .collect()
}

/// ¿Merece la pena re-alinear? Un LRC sincronizado de LRCLIB suele ser mejor
/// que el ASR; solo se toca si viene de una versión vieja (`force_align`) o si
/// trae ráfagas imposibles (varias líneas a menos de 400 ms).
fn needs_alignment(doc: &LyricsDoc, force_align: bool) -> bool {
    if doc.aligned {
        return false;
    }
    // Captions/generado ya tienen su propia rejilla. Y si los tiempos vienen
    // de captions sobre un texto real, repetir ASR o auto-offset empeora una
    // sincronización exacta.
    if doc.captions_timed || doc.source.is_generated() {
        return false;
    }
    // Una letra con tiempos de palabra reales o captions del vídeo exacto ya
    // está alineada contra el audio: repetir ASR solo añade minutos de espera
    // y puede empeorar una sincronización que ya era buena. Esto gana incluso
    // a `force_align` (versión vieja de la caché).
    if doc.synced
        && (matches!(doc.source, LyricsSource::Captions)
            || doc.lines.iter().any(|line| line.words_exact))
    {
        return false;
    }
    if force_align {
        return true;
    }
    if !doc.synced || matches!(doc.source, LyricsSource::Estimated | LyricsSource::Plain) {
        return true;
    }
    let mut previous: Option<u64> = None;
    let mut tight = 0usize;
    for line in &doc.lines {
        if let Some(previous) = previous {
            if line.start_ms.saturating_sub(previous) < 400 {
                tight += 1;
            }
        }
        previous = Some(line.start_ms);
    }
    tight >= 2
}

impl Default for LyricsService {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsea LRC (con o sin tags de palabra) a un documento utilizable.
///
/// Reglas:
/// - Varias marcas por línea (`[00:12.34][00:30.00]`) comparten texto.
/// - Timestamps `[mm:ss]`, con 1, 2 o 3 decimales y también con coma o con
///   `[mm:ss:xx]` (variante antigua).
/// - `[offset:±NNN]` se aplica a los tiempos al parsear y se guarda a cero: el
///   único offset que queda en el documento es el ajuste manual del usuario.
/// - `[ti:]`, `[ar:]`, `[al:]`, `[by:]`, `[length:]` y similares son metadatos y
///   no llegan a las líneas; `[Verse 1]`, `[Chorus]`… tampoco.
/// - Los tags `<mm:ss.xx>` de palabra se aceptan absolutos (lo normal) y
///   relativos al inicio de la línea (lo que escriben algunos editores).
/// - Las marcas sin texto son huecos instrumentales: se pintan con `♪`, igual
///   que los silencios largos entre dos versos.
/// - Si no hay ninguna marca temporal, las líneas se reparten de forma
///   aproximada (4 s por línea) y el documento queda como no sincronizado.
pub fn parse_lrc(text: &str) -> LyricsDoc {
    let mut doc = LyricsDoc::default();
    let mut timed: Vec<LyricLine> = Vec::new();
    let mut plain: Vec<String> = Vec::new();

    for raw in text.lines() {
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw).trim();
        if raw.is_empty() {
            continue;
        }
        // Etiquetas `[...]` al principio de la línea: tiempos, metadatos o
        // títulos de sección. Solo los tiempos cuentan; el resto se tira.
        let mut tags: Vec<&str> = Vec::new();
        let mut rest = raw;
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            tags.push(&rest[1..end]);
            rest = rest[end + 1..].trim_start();
        }
        let mut stamps: Vec<u64> = Vec::new();
        let mut leftover = String::new();
        for tag in tags {
            if let Some(ms) = parse_timestamp(tag) {
                stamps.push(ms);
            } else if let Some(offset) = parse_offset(tag) {
                doc.offset_ms = offset;
            } else if is_metadata_tag(tag) || is_section_label(tag) {
                // Metadato o marca de sección: no es letra.
            } else {
                // Un corchete que no reconocemos se conserva como texto.
                leftover.push('[');
                leftover.push_str(tag);
                leftover.push(']');
                leftover.push(' ');
            }
        }
        let source = if leftover.is_empty() {
            rest.to_string()
        } else {
            format!("{leftover}{rest}")
        };
        let (body, words) = split_words(&source);
        if stamps.is_empty() {
            if !body.is_empty() {
                plain.push(body);
            }
            continue;
        }
        for stamp in stamps {
            timed.push(build_line(stamp, &body, &words));
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
                words_exact: false,
            });
        }
        doc.synced = false;
        doc.source = LyricsSource::Plain;
        // El offset de un LRC sin tiempos no se puede hornear en un reparto
        // estimado: se descarta para no sumarlo dos veces al ajustar.
        doc.offset_ms = 0;
        return doc;
    }

    timed.sort_by_key(|line| line.start_ms);
    // Marcas repetidas con el mismo texto (LRC generados por herramientas).
    timed.dedup_by(|a, b| a.start_ms == b.start_ms && a.text == b.text);
    for line in &mut timed {
        if line.text.is_empty() || is_instrumental_text(&line.text) {
            line.text = INSTRUMENTAL_MARK.to_string();
            line.words.clear();
            line.words_exact = false;
        }
    }
    // Un `[mm:ss]` vacío al final es la convención de "aquí acaba el último
    // verso": se usa como final en vez de pintarse como una línea más.
    let closing = match timed.last() {
        Some(last)
            if last.text == INSTRUMENTAL_MARK
                && timed.iter().any(|line| line.text != INSTRUMENTAL_MARK) =>
        {
            let start = last.start_ms;
            timed.pop();
            Some(start)
        }
        _ => None,
    };

    // El final de un verso llega hasta la siguiente línea **principal**: un
    // adlib `(oh oh)` intercalado no debe recortarlo ni llenar su progreso a
    // mitad de frase mientras la voz sigue cantando.
    let boundaries: Vec<Option<u64>> = (0..timed.len())
        .map(|i| {
            ((i + 1)..timed.len())
                .find(|&j| !is_background_line(&timed[j].text))
                .map(|j| timed[j].start_ms)
                .filter(|end| *end > timed[i].start_ms)
                .or(closing.filter(|end| *end > timed[i].start_ms))
        })
        .collect();

    let mut out: Vec<LyricLine> = Vec::with_capacity(timed.len() + 4);
    for (index, mut line) in timed.into_iter().enumerate() {
        let boundary = boundaries[index];
        let estimate = line_estimate_ms(&line);
        let Some(end) = boundary else {
            line.end_ms = line.start_ms + estimate;
            out.push(line);
            continue;
        };
        // Hueco instrumental largo → línea `♪` para que la UI no se quede
        // mostrando un verso ya cantado durante medio minuto.
        if end.saturating_sub(line.start_ms) > GAP_MARK_MS && line.text != INSTRUMENTAL_MARK {
            let mut line_end = line.start_ms + estimate.min(end - line.start_ms);
            // Un verso con tags de palabra reales puede durar más de lo que
            // sugieren sus caracteres: nunca lo cortamos antes de su última
            // palabra (antes se cortaba a 8 s a mitad de frase).
            if line.words_exact {
                if let Some(last) = line.words.last() {
                    line_end = line_end.max((last.start_ms + word_avg_span(&line)).min(end));
                }
            }
            line.end_ms = line_end;
            out.push(line);
            out.push(LyricLine {
                start_ms: line_end,
                end_ms: end,
                text: INSTRUMENTAL_MARK.to_string(),
                ..LyricLine::default()
            });
        } else {
            line.end_ms = end;
            out.push(line);
        }
    }
    // El orden de inserción del hueco tiene que quedar creciente y sin dos
    // marcas instrumentales pegadas.
    out.sort_by_key(|line| line.start_ms);
    out.dedup_by(|a, b| a.text == INSTRUMENTAL_MARK && b.text == INSTRUMENTAL_MARK);

    for line in &mut out {
        if line.words.is_empty() {
            if let Some(words) = word_times(line.start_ms, line.end_ms, &line.text) {
                line.words = words;
                line.words_exact = false;
            }
        }
        // El último word de un LRC mejorado termina tras su propia duración,
        // no necesariamente en la marca de la línea siguiente.
        let default_end = if line.words_exact && line.words.len() >= 2 {
            line.words.last().unwrap().start_ms + word_avg_span(line)
        } else {
            line.end_ms
        };
        // Rellena el final de las palabras con el inicio de la siguiente. La
        // última termina un pelín antes del borde del verso: si acaba justo
        // cuando salta la línea, en la práctica nunca se ve llena.
        let starts: Vec<u64> = line.words.iter().map(|word| word.start_ms).collect();
        let count = line.words.len();
        let tail_end = line
            .end_ms
            .saturating_sub(WORD_TAIL_MS)
            .max(line.start_ms + 1);
        for (i, word) in line.words.iter_mut().enumerate() {
            let boundary = if i + 1 == count {
                tail_end.max(word.start_ms + 1)
            } else {
                line.end_ms
            };
            word.end_ms = starts
                .get(i + 1)
                .copied()
                .unwrap_or(default_end)
                .min(boundary);
        }
        mark_optional_words(line);
    }

    // El `[offset:]` del proveedor se aplica aquí, una sola vez, y se guarda a
    // cero: así el único offset que queda es el ajuste manual del usuario. Antes
    // el documento arrastraba el offset del LRC y la app lo sustituía por el
    // del usuario (se perdía el del proveedor) o lo sumaba al del ASR (doble).
    if doc.offset_ms != 0 {
        let offset = doc.offset_ms;
        for line in &mut out {
            line.start_ms = shift_to_audio(line.start_ms, offset);
            line.end_ms = shift_to_audio(line.end_ms, offset);
            for word in &mut line.words {
                word.start_ms = shift_to_audio(word.start_ms, offset);
                word.end_ms = shift_to_audio(word.end_ms, offset);
            }
        }
    }
    doc.offset_ms = 0;

    doc.instrumental = !out.is_empty() && out.iter().all(|line| line.text == INSTRUMENTAL_MARK);
    doc.lines = out;
    doc.synced = true;
    doc.source = LyricsSource::Lrclib;
    doc
}

/// Marca de hueco instrumental.
const INSTRUMENTAL_MARK: &str = "♪";
/// Silencio a partir del cual se pinta una línea instrumental entre dos versos.
const GAP_MARK_MS: u64 = 12_000;
/// Margen al final de cada verso para que la última palabra llegue a llenarse
/// antes de que la línea activa salte a la siguiente.
const WORD_TAIL_MS: u64 = 150;
/// Duración estimada de un verso sin tags de palabra.
const LINE_ESTIMATE_MIN_MS: u64 = 2_500;
const LINE_ESTIMATE_MAX_MS: u64 = 8_000;
const LINE_MS_PER_CHAR: u64 = 75;

/// Construye una línea temporal con sus palabras (si el LRC las trae).
fn build_line(stamp: u64, body: &str, words: &[(u64, String)]) -> LyricLine {
    // Los tags de palabra suelen ser absolutos (`<00:12.00>` = 12 s de canción).
    // Son relativos cuando empiezan cerca del principio de la canción y muy
    // por debajo de la marca de línea. Un tag absoluto de "pickup" (sílaba
    // débil justo antes del `[mm:ss]`) no debe clasificarse como relativo:
    // antes se desplazaba al futuro medio minuto.
    let base = match words.first() {
        Some((first, _)) if *first < 30_000 && *first + 2_000 < stamp => stamp,
        _ => 0,
    };
    let mut timed: Vec<Word> = words
        .iter()
        .map(|(ms, text)| Word {
            // Un pre-roll corto (pickup) es legítimo: no lo recortamos al
            // inicio de línea, pero tampoco admitimos tags 30 s antes.
            start_ms: (base + ms).max(stamp.saturating_sub(1_500)),
            end_ms: 0,
            text: text.clone(),
            optional: false,
        })
        .collect();
    timed.sort_by_key(|word| word.start_ms);
    timed.dedup_by(|a, b| a.start_ms == b.start_ms && a.text == b.text);
    LyricLine {
        start_ms: stamp,
        end_ms: 0,
        text: body.to_string(),
        words_exact: !timed.is_empty(),
        words: timed,
    }
}

/// Duración media de una palabra con tags reales. Se usa para prolongar el
/// final del verso hasta la última palabra cantada.
fn word_avg_span(line: &LyricLine) -> u64 {
    if line.words.len() < 2 {
        return 600;
    }
    let first = line.words.first().unwrap().start_ms;
    let last = line.words.last().unwrap().start_ms;
    (last.saturating_sub(first) / (line.words.len() as u64 - 1)).clamp(250, 3_000)
}

fn line_estimate_ms(line: &LyricLine) -> u64 {
    // Ojo: aquí `word.end_ms` todavía está a 0 (se rellena más abajo), así que
    // la estimación con tags tiene que apoyarse en `start_ms`, no en `end_ms`.
    if line.words_exact && line.words.len() >= 2 {
        let last = line.words.last().unwrap().start_ms;
        return (last + word_avg_span(line))
            .saturating_sub(line.start_ms)
            .clamp(LINE_ESTIMATE_MIN_MS, 30_000);
    }
    let chars = line.text.chars().count() as u64;
    (chars * LINE_MS_PER_CHAR).clamp(LINE_ESTIMATE_MIN_MS, LINE_ESTIMATE_MAX_MS)
}

fn is_instrumental_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    let lower = lower.trim().trim_matches(|ch: char| !ch.is_alphanumeric());
    matches!(lower, "instrumental" | "instrumental break" | "interludio")
}

/// Pasa un tiempo de documento a tiempo de audio restando el `[offset:]` del
/// proveedor. Positivo adelanta la letra, así que el audio llega antes.
fn shift_to_audio(ms: u64, offset: i64) -> u64 {
    (ms as i64 - offset).max(0) as u64
}

/// `[offset:+500]`, `[offset:-120]` o `[offset: 30 ]`.
fn parse_offset(tag: &str) -> Option<i64> {
    let (key, value) = tag.split_once(':')?;
    if !key.trim().eq_ignore_ascii_case("offset") {
        return None;
    }
    let value = value
        .trim()
        .trim_start_matches('+')
        .replace([' ', '\t'], "");
    value.parse::<i64>().ok()
}

/// Claves de metadatos del estándar LRC (y primas de ID3).
const METADATA_KEYS: &[&str] = &[
    "ti", "ar", "al", "au", "by", "re", "ve", "length", "tool", "encoding", "lang", "la", "id",
    "album", "artist", "title", "offset",
];

fn is_metadata_tag(tag: &str) -> bool {
    let Some((key, _)) = tag.split_once(':') else {
        return false;
    };
    let key = key.trim().to_lowercase();
    METADATA_KEYS.contains(&key.as_str())
}

/// Nombres de sección que algunos LRC cuelan como si fueran versos.
const SECTION_WORDS: &[&str] = &[
    "intro",
    "verse",
    "verse1",
    "verse2",
    "chorus",
    "prechorus",
    "pre",
    "post",
    "bridge",
    "outro",
    "hook",
    "refrain",
    "instrumental",
    "interlude",
    "interludio",
    "spoken",
    "skit",
    "adlib",
    "part",
    "repetition",
    "coda",
    "break",
    "drop",
    "solo",
    "vocal",
    "vocals",
];

fn is_section_label(tag: &str) -> bool {
    let normalized: String = tag
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect();
    let mut saw_word = false;
    for token in normalized.split_whitespace() {
        let token = token.to_lowercase();
        if token.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        if token.len() == 1 {
            // "Verse A", "Chorus 2x".
            continue;
        }
        if SECTION_WORDS.contains(&token.as_str()) {
            saw_word = true;
            continue;
        }
        return false;
    }
    saw_word
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let tag = tag.trim();
    // Separador decimal con punto o con coma, 1..=3 dígitos.
    let (head, frac) = match tag.rsplit_once(['.', ',']) {
        Some((head, frac))
            if (1..=3).contains(&frac.len()) && frac.chars().all(|c| c.is_ascii_digit()) =>
        {
            (head, frac)
        }
        _ => (tag, ""),
    };
    let mut parts = head.split(':');
    let minutes_raw = parts.next()?;
    let seconds_raw = parts.next()?;
    let frac_raw = match parts.next() {
        // `[mm:ss:xx]`, variante antigua de `[mm:ss.xx]`.
        Some(tail) => {
            if !frac.is_empty() || parts.next().is_some() {
                return None;
            }
            tail
        }
        None => frac,
    };
    let minutes: u64 = minutes_raw.trim().parse().ok()?;
    let seconds: u64 = seconds_raw.trim().parse().ok()?;
    // 600 min son 10 h: por encima es basura de un tag que no era un tiempo.
    if seconds >= 60 || minutes > 600 {
        return None;
    }
    let frac: u64 = match frac_raw.trim().len() {
        0 => 0,
        1 => frac_raw.trim().parse::<u64>().ok()? * 100,
        2 => frac_raw.trim().parse::<u64>().ok()? * 10,
        _ => frac_raw.trim().parse::<u64>().ok()?,
    };
    if frac >= 1_000 {
        return None;
    }
    Some(minutes * 60_000 + seconds * 1_000 + frac)
}

/// Separa los tags `<mm:ss.xx>` de una línea LRC mejorada. Devuelve el texto
/// plano y los tiempos de palabra tal cual vienen en el fichero.
fn split_words(body: &str) -> (String, Vec<(u64, String)>) {
    if !body.contains('<') {
        return (body.trim().to_string(), Vec::new());
    }
    let mut words: Vec<(u64, String)> = Vec::new();
    let mut text = String::new();
    let mut current: Option<u64> = None;
    let mut rest = body;

    while let Some(start) = rest.find('<') {
        let prefix = &rest[..start];
        if !prefix.is_empty() {
            text.push_str(prefix);
            push_word(&mut words, current, prefix);
        }
        let Some(end_rel) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let inner = &rest[start + 1..end];
        match parse_timestamp(inner) {
            Some(ms) => {
                current = Some(ms);
                words.push((ms, String::new()));
            }
            None => {
                // Un `<...>` que no es tiempo es texto normal.
                text.push_str(&rest[start..=end]);
            }
        }
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        text.push_str(rest);
        push_word(&mut words, current, rest);
    }
    let words = words
        .into_iter()
        .map(|(ms, word)| (ms, word.trim().to_string()))
        .filter(|(_, word)| !word.is_empty())
        .collect();
    (text.trim().to_string(), words)
}

/// Acumula texto en la última palabra con ese tiempo (o abre una nueva).
fn push_word(words: &mut Vec<(u64, String)>, stamp: Option<u64>, text: &str) {
    let Some(stamp) = stamp else { return };
    match words.last_mut() {
        Some(last) if last.0 == stamp => last.1.push_str(text),
        _ => words.push((stamp, text.to_string())),
    }
}

/// Marca las palabras que caen dentro de paréntesis o corchetes. El parser
/// conserva esos textos (se ven en la línea), pero el progreso y la selección
/// de línea los tratan como opcionales.
pub(crate) fn mark_optional_words(line: &mut LyricLine) {
    if line.words.is_empty() {
        return;
    }
    let mut optional_ranges: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, ch) in line.text.char_indices() {
        match ch {
            '(' | '[' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
            }
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        optional_ranges.push((start, index + ch.len_utf8()));
                    }
                }
            }
            _ => {}
        }
    }
    let mut cursor = 0usize;
    for word in &mut line.words {
        let Some(relative) = line.text[cursor..].find(&word.text) else {
            continue;
        };
        let at = cursor + relative;
        word.optional = optional_ranges
            .iter()
            .any(|(from, to)| at >= *from && at < *to);
        cursor = at + word.text.len();
    }
}

pub(crate) fn word_times(start: u64, end: u64, text: &str) -> Option<Vec<Word>> {
    let chunks: Vec<&str> = text.split_whitespace().collect();
    if chunks.len() < 2 {
        return None;
    }
    // Peso por sílabas (grupos de vocales) + signos: una palabra larga o con
    // pausa detrás tarda más en cantarse que una corta. Es más exacto que
    // repartir por caracteres, sobre todo en español.
    let weights: Vec<f32> = chunks
        .iter()
        .map(|chunk| {
            let lower = chunk.to_lowercase();
            let vowels = lower
                .chars()
                .filter(|c| "aeiouáéíóúüàèìòù".contains(*c))
                .count()
                .max(1);
            let pause = if chunk.ends_with([',', '.', '!', '?', ';', '…']) {
                1.8
            } else {
                0.0
            };
            vowels as f32 * 1.7 + chunk.chars().count() as f32 * 0.3 + pause
        })
        .collect();
    let total: f32 = weights.iter().sum::<f32>().max(0.001);
    let span = end.saturating_sub(start);
    let mut words = Vec::with_capacity(chunks.len());
    let mut cursor = start;
    for (chunk, weight) in chunks.iter().zip(weights) {
        let duration = (span as f32 * weight / total) as u64;
        words.push(Word {
            start_ms: cursor,
            end_ms: cursor + duration,
            text: chunk.to_string(),
            optional: false,
        });
        cursor += duration;
    }
    Some(words)
}

#[cfg(test)]
pub(crate) fn build_test_doc(lines: &[(u64, &str)]) -> LyricsDoc {
    let mut doc = LyricsDoc {
        synced: true,
        source: LyricsSource::Lrclib,
        ..LyricsDoc::default()
    };
    for (index, (start, text)) in lines.iter().enumerate() {
        let end = lines
            .get(index + 1)
            .map(|(next, _)| *next)
            .unwrap_or(start + 4_000);
        doc.lines.push(LyricLine {
            start_ms: *start,
            end_ms: end,
            text: (*text).to_string(),
            ..LyricLine::default()
        });
    }
    doc
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
        assert!(doc.synced);
        assert_eq!(doc.lines[0].start_ms, 12_300);
        assert_eq!(doc.lines[0].end_ms, 18_900);
        assert_eq!(
            doc.lines[1].text,
            "Do you ever get that fear that you can't shift the tide"
        );
        assert_eq!(doc.index_at(0), None);
        assert_eq!(doc.index_at(12_000), None);
        assert_eq!(doc.index_at(12_400), Some(0));
        // A mitad del hueco instrumental ya no manda el segundo verso.
        assert_eq!(doc.index_at(60_000), Some(2));
        assert_eq!(doc.lines[2].text, "♪");
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
    fn long_gap_becomes_an_instrumental_line() {
        let doc = parse_lrc(LRC);
        // Niño, verso y ♪ que cubre el hueco hasta las 1:20.
        assert_eq!(doc.lines.len(), 3, "{:?}", doc.lines);
        let marker = doc
            .lines
            .iter()
            .find(|line| line.text == "♪")
            .expect("hay marca instrumental");
        assert_eq!(marker.start_ms, doc.lines[1].end_ms);
        assert!(marker.end_ms > marker.start_ms);
        // El verso no se queda activo 60 s esperando al siguiente.
        assert!(doc.lines[1].end_ms - doc.lines[1].start_ms <= 8_000);
    }

    #[test]
    fn metadata_tags_do_not_become_lyrics() {
        let text = "[ti:Do I Wanna Know?]\n[ar:Arctic Monkeys]\n[al:AM]\n\
[by:someone]\n[length:04:32]\n\
[00:01.00]primera\n[Verse 1]\n[00:02.00]segunda";
        let doc = parse_lrc(text);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "primera");
        assert_eq!(doc.lines[1].text, "segunda");
    }

    #[test]
    fn provider_offset_is_baked_into_the_timeline() {
        let doc = parse_lrc("[offset:+250]\n[00:10.00]hola");
        // El offset del proveedor ya no vive en `offset_ms`: se aplica una vez
        // a las líneas para que no se pueda sumar dos veces con el del usuario.
        assert_eq!(doc.offset_ms, 0);
        assert_eq!(doc.lines[0].start_ms, 9_750);
        assert_eq!(doc.index_at(9_749), None);
        assert_eq!(doc.index_at(9_750), Some(0));

        let negative = parse_lrc("[offset:-500]\n[00:10.00]hola");
        assert_eq!(negative.offset_ms, 0);
        assert_eq!(negative.lines[0].start_ms, 10_500);
        assert_eq!(negative.index_at(10_499), None);
        assert_eq!(negative.index_at(10_500), Some(0));
    }

    #[test]
    fn provider_and_user_offsets_compose_once_each() {
        let doc = parse_lrc("[offset:+1000]\n[00:10.00]primera frase\n[00:12.00]segunda frase");
        assert_eq!(doc.lines[0].start_ms, 9_000);
        assert_eq!(doc.offset_ms, 0);
        // El ajuste manual se suma sobre el tiempo ya corregido por el
        // proveedor; antes uno de los dos se perdía.
        let mut doc = doc;
        doc.offset_ms = 500;
        assert_eq!(doc.aligned_ms(8_500), 9_000);
        assert_eq!(doc.index_at(8_500), Some(0));
        assert_eq!(doc.index_at(8_499), None);
        // El progreso de palabra usa la misma base que el índice.
        let baseline = parse_lrc("[00:10.00]primera frase\n[00:12.00]segunda frase");
        assert!(
            (doc.word_progress_at(10_000, 0) - baseline.word_progress_at(11_500, 0)).abs() < 1e-3
        );
    }

    #[test]
    fn timestamps_accept_one_to_three_decimals() {
        let cases = [
            ("[00:12.3]a", 12_300),
            ("[00:12.34]a", 12_340),
            ("[00:12.345]a", 12_345),
            ("[00:12,5]a", 12_500),
            ("[00:12]a", 12_000),
            ("[00:12:50]a", 12_500),
            ("[01:02.00]a", 62_000),
        ];
        for (lrc, expected) in cases {
            let doc = parse_lrc(lrc);
            assert_eq!(doc.lines.len(), 1, "{lrc}");
            assert_eq!(doc.lines[0].start_ms, expected, "{lrc}");
            assert_eq!(doc.lines[0].text, "a", "{lrc}");
        }
        // Basura que parece un tiempo no debe colarse.
        let doc = parse_lrc("[00:99.00]a\n[00:10.9999]b");
        assert!(!doc.synced);
    }

    #[test]
    fn several_time_tags_share_the_text() {
        let doc = parse_lrc("[00:10.00][00:20.00]estribillo\n[00:30.00]final");
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0].text, "estribillo");
        assert_eq!(doc.lines[1].text, "estribillo");
        assert_eq!(doc.lines[1].start_ms, 20_000);
        assert_eq!(doc.lines[2].start_ms, 30_000);
    }

    #[test]
    fn enhanced_tags_are_kept() {
        let doc = parse_lrc("[00:10.00] <00:10.00>Hello <00:11.00>world");
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.lines[0].words.len(), 2);
        assert_eq!(doc.lines[0].words[1].text, "world");
    }

    #[test]
    fn enhanced_tags_are_absolute() {
        // Formato estándar: el tag de palabra es la marca absoluta de la canción.
        let doc = parse_lrc("[01:00.00]<01:00.00>Hello <01:00.50>world");
        assert!(doc.lines[0].words_exact);
        assert_eq!(doc.lines[0].words[0].start_ms, 60_000);
        assert_eq!(doc.lines[0].words[1].start_ms, 60_500);
        assert!(doc.lines[0].words[0].start_ms >= doc.lines[0].start_ms);
    }

    #[test]
    fn last_word_finishes_before_the_next_line() {
        let doc = parse_lrc("[00:00.00]one two three\n[00:05.00]four five six");
        let line = &doc.lines[0];
        let last = line.words.last().expect("palabras sintetizadas");
        assert!(
            last.end_ms < line.end_ms,
            "última palabra {} termina en {} y la línea en {}",
            last.text,
            last.end_ms,
            line.end_ms
        );
        assert!(last.end_ms + WORD_TAIL_MS >= line.end_ms);
    }

    #[test]
    fn pickup_word_tags_stay_absolute() {
        // Una sílaba débil 1,5 s antes de la marca es tiempo absoluto, no un
        // tag relativo: no debe moverse a los 118 s.
        let doc = parse_lrc("[01:00.00]<00:58.50>Yeah <01:00.00>hello");
        assert!(doc.lines[0].words_exact);
        assert_eq!(doc.lines[0].words[0].start_ms, 58_500);
        assert_eq!(doc.lines[0].words[1].start_ms, 60_000);
    }

    #[test]
    fn relative_word_tags_get_shifted_to_the_line() {
        let doc = parse_lrc("[01:00.00]<00:00.00>Hello <00:00.50>world");
        assert_eq!(doc.lines[0].words[0].start_ms, 60_000);
        assert_eq!(doc.lines[0].words[1].start_ms, 60_500);
    }

    #[test]
    fn trailing_mark_extends_the_last_verse() {
        // Convención LRC: una marca vacía al final cierra el último verso.
        let doc = parse_lrc("[00:05.00]hola\n[00:09.00]");
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.lines[0].end_ms, 9_000);

        // Con un silencio largo de por medio sí se pinta el hueco.
        let doc = parse_lrc("[00:05.00]hola\n[00:40.00]");
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[1].text, "♪");
        assert_eq!(doc.lines[1].start_ms, doc.lines[0].end_ms);
        assert_eq!(doc.lines[1].end_ms, 40_000);
    }

    #[test]
    fn empty_timed_lines_are_instrumental() {
        let doc = parse_lrc("[00:00.00]\n[00:05.00]");
        assert!(doc.synced);
        assert!(doc.instrumental);
        assert!(doc.lines.iter().all(|line| line.text == "♪"));
    }

    #[test]
    fn plain_lyrics_are_estimated() {
        let doc = parse_lrc("one\ntwo\nthree");
        assert!(!doc.synced);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.source, LyricsSource::Plain);
    }

    #[test]
    fn alignment_is_only_requested_for_lyrics_that_need_it() {
        let synced = LyricsDoc::from_lrc(LRC, true);
        assert!(!needs_alignment(&synced, false));

        let plain = LyricsDoc::from_lrc("one\ntwo\nthree", false);
        assert!(needs_alignment(&plain, false));

        // Ráfagas imposibles: dos líneas a menos de 400 ms.
        let broken = LyricsDoc::from_lrc(
            "[00:10.00]uno\n[00:10.10]dos\n[00:10.20]tres\n[00:20.00]cuatro",
            true,
        );
        assert!(needs_alignment(&broken, false));

        // Captions ya vienen del audio exacto; no hace falta ASR.
        let mut captions = synced.clone();
        captions.source = LyricsSource::Captions;
        assert!(!needs_alignment(&captions, false));

        // Un LRC mejorado con palabras ya está alineado.
        let word_timed = LyricsDoc::from_lrc(
            "[00:10.00]<00:10.00>uno <00:11.00>dos\n[00:12.00]tres",
            true,
        );
        assert!(!needs_alignment(&word_timed, false));

        // `force_align` manda: es una petición explícita de re-alinear.
        assert!(needs_alignment(&synced, true));
    }

    #[test]
    fn window_is_clamped() {
        let doc = parse_lrc(LRC);
        let (start, end) = doc.window(12_400, 2, 2);
        assert_eq!((start, end), (0, 3));
    }

    #[test]
    fn genius_source_survives_cache_labels() {
        // La caché guarda `doc.source.label()`: Genius debe distinguirse de un
        // `plain` o un `captions` genérico para poder invalidar solo estos.
        assert_eq!(LyricsSource::Genius.label(), "genius");
        assert_eq!(
            LyricsSource::from_label("genius"),
            Some(LyricsSource::Genius)
        );
        assert_eq!(
            LyricsSource::from_label("  GENIUS "),
            Some(LyricsSource::Genius)
        );
        // Un texto de Genius con tiempos de otro proveedor conserva la fuente
        // Genius; el parser devuelve Lrclib y la capa de caché la restaura.
        let mut doc = LyricsDoc::from_lrc("[00:10.00]uno\n[00:12.00]dos", true);
        assert!(doc.synced);
        doc.source = LyricsSource::Genius;
        assert_eq!(doc.source.label(), "genius");
    }

    #[test]
    fn generated_sources_and_caption_timing_skip_alignment() {
        assert!(LyricsSource::Captions.is_generated());
        assert!(LyricsSource::Asr.is_generated());
        assert!(!LyricsSource::Lrclib.is_generated());
        let mut doc = build_test_doc(&[(0, "uno"), (2_000, "dos")]);
        doc.synced = false;
        doc.source = LyricsSource::Plain;
        assert!(needs_alignment(&doc, false));
        doc.captions_timed = true;
        assert!(!needs_alignment(&doc, true));
        doc.captions_timed = false;
        doc.source = LyricsSource::Asr;
        assert!(!needs_alignment(&doc, true));
    }

    #[test]
    fn asr_reference_keeps_the_existing_text_and_times() {
        // El ASR recibe exactamente las líneas ya escritas, sin palabras
        // sintetizadas: no puede convertirse en una fuente de texto.
        let mut doc = LyricsDoc::from_lrc("[00:10.00]uno\n[00:12.00](oh oh)\n[00:14.00]dos", true);
        doc.offset_ms = 500;
        let reference = asr_reference_lines(&doc);
        let texts: Vec<&str> = reference.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, vec!["uno", "(oh oh)", "dos"]);
        assert!(reference.iter().all(|line| line.words.is_empty()));
        assert!(!reference.iter().any(|line| line.words_exact));
        // Se quita el offset de usuario: el ASR devuelve tiempos del audio.
        assert_eq!(reference[0].start_ms, 9_500);
        assert_eq!(reference[2].start_ms, 13_500);
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    #[test]
    fn parenthetical_lines_do_not_steal_the_active_line() {
        let doc = LyricsDoc::from_lrc(
            "[00:01.00]Main line\n[00:04.00](oh oh)\n[00:07.00]Next main",
            true,
        );
        assert!(is_background_line("(oh oh)"));
        assert!(!is_background_line("Main (oh oh)"));
        assert_eq!(doc.index_at(5_000), Some(0));
        assert_eq!(doc.index_at(8_000), Some(2));
    }

    #[test]
    fn audio_map_can_estimate_the_global_offset() {
        use crate::audio_map::{SongBucket, SongMap, BUCKET_MS};
        let mut buckets = Vec::new();
        for index in 0..240u64 {
            let at = index * BUCKET_MS;
            // Cuatro picos, uno por verso: con solo dos, un desplazamiento de
            // -6 s cuadraba casi igual de bien (+2 s) y el estimador debía
            // abstenerse. Así la respuesta es inequívoca.
            let vocal = if (12_000..12_500).contains(&at)
                || (20_000..20_500).contains(&at)
                || (28_000..28_500).contains(&at)
                || (36_000..36_500).contains(&at)
            {
                1.0
            } else {
                0.0
            };
            buckets.push(SongBucket {
                at_ms: at,
                vocal,
                ..SongBucket::default()
            });
        }
        let map = SongMap::from_buckets(60_000, buckets);
        let doc = LyricsDoc::from_lrc(
            "[00:10.00]primera\n[00:18.00]segunda\n[00:26.00]tercera\n[00:34.00]cuarta",
            true,
        );
        let offset = doc.estimate_offset_from_map(&map).expect("debe estimar");
        assert!((offset - 2_000).abs() <= 500, "offset={offset}");
    }

    #[test]
    fn ambiguous_vocal_map_does_not_move_the_lyrics() {
        use crate::audio_map::{SongBucket, SongMap, BUCKET_MS};
        let mut buckets = Vec::new();
        for index in 0..240u64 {
            let at = index * BUCKET_MS;
            let vocal = if (12_000..12_500).contains(&at) || (20_000..20_500).contains(&at) {
                1.0
            } else {
                0.0
            };
            buckets.push(SongBucket {
                at_ms: at,
                vocal,
                ..SongBucket::default()
            });
        }
        let map = SongMap::from_buckets(60_000, buckets);
        let doc = LyricsDoc::from_lrc(
            "[00:10.00]primera\n[00:18.00]segunda\n[00:26.00]tercera\n[00:34.00]cuarta",
            true,
        );
        // +2 s y -6 s explican el mismo número de versos: mejor no tocar nada.
        assert_eq!(doc.estimate_offset_from_map(&map), None);
    }

    #[test]
    fn parenthetical_words_are_optional_for_progress() {
        let doc = LyricsDoc::from_lrc("[00:01.00]Main line (oh oh)\n[00:02.00]Next main", true);
        let line = &doc.lines[0];
        let optional: Vec<bool> = line.words.iter().map(|word| word.optional).collect();
        assert!(optional.iter().any(|value| *value), "{optional:?}");
        assert!(!optional.iter().all(|value| *value), "{optional:?}");
        // A mitad de la parte principal, el progreso no se queda esperando el adlib.
        assert!(LyricsDoc::word_progress(line, 1_500) > 0.45);
        // La siguiente línea ya manda aunque el adlib siga sonando.
        assert_eq!(doc.index_at(2_100), Some(1));
    }
}

/// Regresiones del síntoma "cambia a la siguiente a mitad de frase".
#[cfg(test)]
mod timeline_tests {
    use super::*;

    /// Por debajo de este progreso, un cambio de línea es prematuro.
    const EARLY_SWITCH_PROGRESS: f32 = 0.85;

    #[derive(Debug)]
    struct Transition {
        at_ms: u64,
        from: usize,
        to: usize,
        outgoing_progress: f32,
        outgoing: String,
        incoming: String,
    }

    /// Recorre la línea de tiempo y registra cada cambio de línea activa con
    /// el progreso de la frase saliente. Es el detector del bug: antes del
    /// arreglo, con offset ≠ 0, el progreso saliente era ~0.5.
    fn scan(doc: &LyricsDoc, from_ms: u64, to_ms: u64, step: u64) -> Vec<Transition> {
        let mut changes = Vec::new();
        let mut previous = doc.index_at(from_ms);
        let mut ms = from_ms + step;
        while ms <= to_ms {
            let current = doc.index_at(ms);
            if let (Some(from), Some(to)) = (previous, current) {
                if from != to {
                    let outgoing = doc.lines[from].text.clone();
                    changes.push(Transition {
                        at_ms: ms,
                        from,
                        to,
                        outgoing_progress: LyricsDoc::word_progress(
                            &doc.lines[from],
                            doc.aligned_ms(ms),
                        ),
                        outgoing,
                        incoming: doc.lines[to].text.clone(),
                    });
                }
            }
            previous = current;
            ms += step;
        }
        changes
    }

    fn assert_clean_timeline(doc: &LyricsDoc, from_ms: u64, to_ms: u64) {
        let changes = scan(doc, from_ms, to_ms, 50);
        let early: Vec<&Transition> = changes
            .iter()
            .filter(|change| {
                change.outgoing_progress < EARLY_SWITCH_PROGRESS
                    && change.outgoing != INSTRUMENTAL_MARK
                    && !is_background_line(&change.outgoing)
            })
            .collect();
        let report: Vec<_> = early
            .iter()
            .map(|change| {
                (
                    change.at_ms,
                    change.from,
                    change.to,
                    change.outgoing.as_str(),
                    change.incoming.as_str(),
                    change.outgoing_progress,
                )
            })
            .collect();
        assert!(early.is_empty(), "cambios prematuros: {report:#?}");
    }

    const VERSES: &str = "[00:10.00]primera frase\n[00:12.00]segunda frase\n\
                          [00:14.00]tercera frase\n[00:16.00]cuarta frase\n[00:18.00]";

    #[test]
    fn user_offset_applies_to_index_and_progress() {
        let baseline = LyricsDoc::from_lrc(VERSES, true);
        for offset in [0i64, 1_000, -1_000, 500] {
            let mut doc = baseline.clone();
            doc.offset_ms = offset;
            // Con offset de usuario, el estado en `ms` debe ser el mismo que
            // sin offset en `ms + offset`.
            for ms in [10_000u64, 10_500, 11_000, 11_500, 12_000, 13_500] {
                let shift = (ms as i64 + offset).max(0) as u64;
                if shift < 10_000 {
                    continue; // antes del primer verso no hay línea activa
                }
                let (index, progress) = doc.active(ms).unwrap();
                let (base_index, base_progress) = baseline.active(shift).unwrap();
                assert_eq!(Some(index), doc.index_at(ms), "offset={offset} ms={ms}");
                assert_eq!(index, base_index, "offset={offset} ms={ms}");
                assert!(
                    (progress - base_progress).abs() < 1e-3,
                    "offset={offset} ms={ms}"
                );
                assert!((0.0..=1.0).contains(&progress));
            }
        }
        // Caso exacto del reporte: con +1 s, el cambio ocurre cuando la frase
        // anterior, medida con el reloj sin corregir, iría por la mitad.
        let mut doc = baseline;
        doc.offset_ms = 1_000;
        let (index, progress) = doc.active(11_000).unwrap();
        assert_eq!(index, 1);
        assert!(progress < 0.01, "justo al cambiar empieza la nueva frase");
        let outgoing_raw = LyricsDoc::word_progress(&doc.lines[0], 11_000);
        assert!((0.2..0.85).contains(&outgoing_raw), "raw={outgoing_raw}");
        let outgoing_aligned = LyricsDoc::word_progress(&doc.lines[0], doc.aligned_ms(11_000));
        assert!(outgoing_aligned > 0.99, "aligned={outgoing_aligned}");
    }

    #[test]
    fn provider_offset_bakes_an_equivalent_timeline() {
        let baseline = LyricsDoc::from_lrc(VERSES, true);
        for offset in [1_000i64, -1_000, 500] {
            let doc = LyricsDoc::from_lrc(&format!("[offset:{offset}]\n{VERSES}"), true);
            assert_eq!(doc.offset_ms, 0, "el proveedor no deja offset pendiente");
            for ms in [9_000u64, 10_000, 11_000, 12_000, 13_500] {
                let shift = (ms as i64 + offset).max(0) as u64;
                if shift < 10_000 {
                    continue;
                }
                let (index, progress) = doc.active(ms).unwrap();
                let (base_index, base_progress) = baseline.active(shift).unwrap();
                assert_eq!(index, base_index, "offset={offset} ms={ms}");
                assert!(
                    (progress - base_progress).abs() < 1e-3,
                    "offset={offset} ms={ms}"
                );
            }
        }
    }

    #[test]
    fn timeline_has_no_early_switch_with_any_offset() {
        for offset in [0i64, 500, 1_000, -500, -1_000] {
            let doc = LyricsDoc::from_lrc(&format!("[offset:{offset}]\n{VERSES}"), true);
            assert_clean_timeline(&doc, 9_000, 19_000);
        }
    }

    #[test]
    fn raw_clock_differs_from_document_clock_with_user_offset() {
        // Con un offset de usuario, comparar las palabras contra el reloj de
        // canción sin corregir da un progreso falso; el documento usa
        // `aligned_ms` en todas las rutas.
        let mut doc = LyricsDoc::from_lrc(VERSES, true);
        doc.offset_ms = 1_000;
        let transitions = scan(&doc, 9_000, 19_000, 50);
        assert!(!transitions.is_empty());
        // Reloj de canción: al cambiar, la frase saliente va por la mitad.
        let raw = LyricsDoc::word_progress(&doc.lines[0], 11_000);
        assert!((0.2..0.85).contains(&raw), "raw={raw}");
        // Reloj de documento: ya está terminada, que es lo que debe pintarse.
        let aligned = LyricsDoc::word_progress(&doc.lines[0], doc.aligned_ms(11_000));
        assert!(aligned > 0.99, "aligned={aligned}");
    }

    #[test]
    fn adlib_does_not_shorten_the_main_verse() {
        let doc = LyricsDoc::from_lrc(
            "[00:10.00]Main line\n[00:12.00](oh oh)\n[00:14.00]Next main",
            true,
        );
        assert_eq!(doc.lines[0].end_ms, 14_000);
        assert!(doc.word_progress_at(13_000, 0) < 1.0);
        assert_eq!(doc.index_at(13_000), Some(0));
        assert_clean_timeline(&doc, 9_000, 16_000);
    }

    #[test]
    fn long_verse_with_word_tags_is_not_cut_at_eight_seconds() {
        let doc = LyricsDoc::from_lrc(
            "[00:00.00]<00:00.00>Una <00:02.00>frase <00:04.00>larga \
             <00:06.00>que <00:08.00>sigue <00:10.00>hasta <00:12.00>aquí\n\
             [00:20.00]\n",
            true,
        );
        assert!(doc.lines[0].end_ms >= 13_000, "{:?}", doc.lines[0]);
        let marker = &doc.lines[1];
        assert_eq!(marker.text, INSTRUMENTAL_MARK);
        assert!(marker.start_ms >= 13_000, "{marker:?}");
        assert!(marker.end_ms > marker.start_ms);
    }

    #[test]
    fn words_never_leave_their_line() {
        let corpus = [
            VERSES.to_string(),
            "[00:10.00]Main line\n[00:12.00](oh oh)\n[00:14.00]Next main".to_string(),
            "[00:00.00]<00:00.00>Una <00:02.00>frase <00:04.00>larga \
             <00:06.00>que <00:08.00>sigue <00:10.00>hasta <00:12.00>aquí\n[00:20.00]\n"
                .to_string(),
            "[offset:+500]\n[00:10.00]a\n[00:20.00]b\n[00:30.00]c".to_string(),
        ];
        for lrc in corpus {
            let doc = LyricsDoc::from_lrc(&lrc, true);
            for line in &doc.lines {
                for word in &line.words {
                    assert!(word.end_ms >= word.start_ms, "{word:?}");
                    assert!(word.start_ms >= line.start_ms, "{word:?} en {line:?}");
                    assert!(word.end_ms <= line.end_ms, "{word:?} en {line:?}");
                }
            }
        }
    }
}
