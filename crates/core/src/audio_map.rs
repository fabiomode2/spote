//! Mapa de "feeling" de una canción completa.
//!
//! `audio::Analyzer` está pensado para tiempo real; aquí se le pasa el fichero
//! entero decodificado por ffmpeg tan rápido como permita la CPU y se guarda un
//! resumen cada ~255 ms (≈4 muestras por segundo) más la lista de secciones
//! (intro, subida, drop, coro...). La TUI consulta por milisegundo y no toca el
//! hilo de dibujo: `analyze_file` es síncrono y quien lo llama decide lanzarlo
//! en un hilo aparte.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use crate::audio::{Analyzer, AudioFrame};

/// Muestreo de decodificación (mono, s16le). Coincide con `audio::RATE`.
const RATE: u32 = 22_050;
/// HOP interno del analizador: 512 muestras ≈ 23.2 ms.
const HOP: usize = 512;
/// 11 frames ≈ 255 ms → unas 4 muestras por segundo.
const FRAMES_PER_BUCKET: usize = 11;
/// Muestras que cubre un bucket.
const SAMPLES_PER_BUCKET: usize = HOP * FRAMES_PER_BUCKET;
/// Separación entre buckets en milisegundos (aproximada; los buckets reales
/// se etiquetan con `bucket_at_ms`, sin truncar).
pub const BUCKET_MS: u64 = SAMPLES_PER_BUCKET as u64 * 1000 / RATE as u64;

/// Milisegundos exactos en que empieza el bucket `index`. `BUCKET_MS` trunca
/// 255.46 ms a 255: usar `i * BUCKET_MS` acumula ~430 ms de desfase en 4 min.
pub fn bucket_at_ms(index: usize) -> u64 {
    (index as u64 * SAMPLES_PER_BUCKET as u64 * 1000) / RATE as u64
}

// Umbrales del detector de secciones, todos relativos a la energía media.
const MIN_SECTION_BUCKETS: usize = 8; // ~2 s: lo más corto se funde
const SMOOTH_BUCKETS: usize = 4; // ~1 s de media móvil centrada
const SLOPE_BUCKETS: usize = 8; // ~2 s para medir la pendiente
const SUSTAIN_BUCKETS: usize = 12; // ~3 s de energía sostenida
const DROP_WINDOW: usize = 16; // el drop dura como mucho ~4 s
const RISE_THRESHOLD: f32 = 0.12; // subida en "medias de canción"
const QUIET_LEVEL: f32 = 0.35;
const VERSE_LEVEL: f32 = 0.72;
const CHORUS_LEVEL: f32 = 0.95;
const DROP_LEVEL: f32 = 1.15;

/// Resumen de ~255 ms de canción. Todo 0..1 salvo el tiempo y el flag de golpe.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SongBucket {
    pub at_ms: u64,
    pub energy: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub vocal: f32,
    pub beat: bool,
}

impl SongBucket {
    fn from_frame(at_ms: u64, frame: &AudioFrame, beat: bool) -> Self {
        Self {
            at_ms,
            energy: frame.energy.clamp(0.0, 1.0),
            bass: frame.bass.clamp(0.0, 1.0),
            mid: frame.mid.clamp(0.0, 1.0),
            treble: frame.treble.clamp(0.0, 1.0),
            vocal: frame.vocal.clamp(0.0, 1.0),
            beat,
        }
    }
}

/// Papel de una sección dentro del tema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SongSectionKind {
    #[default]
    Intro,
    Verse,
    Build,
    Drop,
    Chorus,
    Bridge,
    Quiet,
    Outro,
}

impl SongSectionKind {
    /// Identificador estable para logs y caché (la UI decide cómo pintarlo).
    pub fn label(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Verse => "verso",
            Self::Build => "subida",
            Self::Drop => "drop",
            Self::Chorus => "estribillo",
            Self::Bridge => "puente",
            Self::Quiet => "tranquilo",
            Self::Outro => "salida",
        }
    }
}

/// Tramo continuo con un mismo papel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SongSection {
    pub start_ms: u64,
    pub end_ms: u64,
    pub kind: SongSectionKind,
    pub energy: f32,
}

/// Mapa precalculado de una canción.
#[derive(Debug, Clone, Default)]
pub struct SongMap {
    pub duration_ms: u64,
    pub buckets: Vec<SongBucket>,
    pub sections: Vec<SongSection>,
}

impl SongMap {
    /// Construye el mapa desde buckets ya calculados (tests, caché en disco).
    pub fn from_buckets(duration_ms: u64, buckets: Vec<SongBucket>) -> Self {
        // Si no nos dan duración, la deducimos del último bucket.
        let duration_ms = if duration_ms == 0 {
            buckets
                .last()
                .map(|_| bucket_at_ms(buckets.len()))
                .unwrap_or_default()
        } else {
            duration_ms
        };
        let sections = detect_sections(&buckets, duration_ms);
        Self {
            duration_ms,
            buckets,
            sections,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Bucket que cubre `ms`; si se pasa del final devuelve el último.
    pub fn at(&self, ms: u64) -> Option<&SongBucket> {
        self.index_at(ms).map(|i| &self.buckets[i])
    }

    pub fn energy_at(&self, ms: u64) -> f32 {
        self.at(ms).map(|b| b.energy).unwrap_or(0.0)
    }

    pub fn section_at(&self, ms: u64) -> Option<&SongSection> {
        if self.sections.is_empty() {
            return None;
        }
        let next = self.sections.partition_point(|s| s.start_ms <= ms);
        let section = self.sections.get(next.saturating_sub(1))?;
        (ms >= section.start_ms && ms < section.end_ms).then_some(section)
    }

    /// Energía suavizada (±0.5 s) del momento, 0..1.
    pub fn intensity_at(&self, ms: u64) -> f32 {
        let Some(index) = self.index_at(ms) else {
            return 0.0;
        };
        let lo = index.saturating_sub(2);
        let hi = (index + 3).min(self.buckets.len());
        let window = &self.buckets[lo..hi];
        let sum: f32 = window.iter().map(|b| b.energy).sum();
        (sum / window.len() as f32).clamp(0.0, 1.0)
    }

    fn index_at(&self, ms: u64) -> Option<usize> {
        if self.buckets.is_empty() {
            return None;
        }
        let next = self.buckets.partition_point(|b| b.at_ms <= ms);
        Some(next.saturating_sub(1).min(self.buckets.len() - 1))
    }
}

// ---------------------------------------------------------------------------
// Análisis de un fichero
// ---------------------------------------------------------------------------

/// Analiza `path` de principio a fin. Bloquea: úsalo desde un hilo de trabajo.
pub fn analyze_file(path: &Path) -> Result<SongMap> {
    if path.as_os_str().is_empty() {
        bail!("ruta de audio vacía");
    }
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-map",
            "a:0",
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ac",
            "1",
            "-ar",
            &RATE.to_string(),
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("no se pudo lanzar ffmpeg ({})", path.display()))?;

    let mut stdout = child.stdout.take().context("ffmpeg sin stdout")?;
    // ffmpeg escribe poco por stderr, pero si el fichero está roto puede soltar
    // bastante: se drena en paralelo para no bloquear el pipe.
    let stderr = child
        .stderr
        .take()
        .map(|stderr| std::thread::spawn(move || drain(stderr)));

    let mut collector = Collector::new();
    let mut decoder = PcmDecoder::default();
    let mut buffer = [0u8; 16 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = stdout
            .read(&mut buffer)
            .with_context(|| format!("leyendo PCM de {}", path.display()))?;
        if read == 0 {
            break;
        }
        let samples = decoder.push(&buffer[..read]);
        total += samples.len() as u64;
        collector.push(samples);
    }

    let status = child.wait().context("ffmpeg no terminó")?;
    let detail = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if !status.success() {
        let detail = detail.trim();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(anyhow!(
            "ffmpeg falló con {status} ({}){suffix}",
            path.display()
        ));
    }
    if total == 0 {
        bail!("ffmpeg no devolvió audio para {}", path.display());
    }
    Ok(collector.finish(total))
}

/// Análisis puro sobre muestras mono ya decodificadas a 22050 Hz (-1..1).
pub fn analyze_samples(samples: &[f32]) -> SongMap {
    let mut collector = Collector::new();
    collector.push(samples);
    collector.finish(samples.len() as u64)
}

/// Acumula frames del analizador y los cierra en buckets de ~255 ms.
struct Collector {
    analyzer: Analyzer,
    buckets: Vec<SongBucket>,
    frames: usize,
    beat: bool,
    last: AudioFrame,
}

impl Collector {
    fn new() -> Self {
        Self {
            analyzer: Analyzer::new(RATE),
            buckets: Vec::new(),
            frames: 0,
            beat: false,
            last: AudioFrame::default(),
        }
    }

    /// Acepta cualquier longitud; lo que no complete frame queda en el analizador.
    fn push(&mut self, samples: &[f32]) {
        for chunk in samples.chunks(HOP) {
            if !self.analyzer.push(chunk) {
                continue; // todavía no hay frame completo
            }
            self.frames += 1;
            let frame = self.analyzer.frame();
            self.beat |= frame.beat;
            self.last = frame;
            if self.frames >= FRAMES_PER_BUCKET {
                self.close();
            }
        }
    }

    fn close(&mut self) {
        let at_ms = bucket_at_ms(self.buckets.len());
        self.buckets
            .push(SongBucket::from_frame(at_ms, &self.last, self.beat));
        self.frames = 0;
        self.beat = false;
    }

    fn finish(mut self, total_samples: u64) -> SongMap {
        if self.frames > 0 {
            self.close();
        }
        let duration_ms = total_samples * 1000 / RATE as u64;
        SongMap::from_buckets(duration_ms, self.buckets)
    }
}

/// Convierte s16le a f32 arrastrando el byte impar entre lecturas.
#[derive(Default)]
struct PcmDecoder {
    carry: Option<u8>,
    samples: Vec<f32>,
}

impl PcmDecoder {
    fn push(&mut self, bytes: &[u8]) -> &[f32] {
        self.samples.clear();
        let mut bytes = bytes;
        if let Some(low) = self.carry.take() {
            match bytes.split_first() {
                Some((&high, rest)) => {
                    self.samples.push(scale(i16::from_le_bytes([low, high])));
                    bytes = rest;
                }
                None => {
                    self.carry = Some(low);
                    return &self.samples;
                }
            }
        }
        let mut pairs = bytes.chunks_exact(2);
        for pair in pairs.by_ref() {
            self.samples
                .push(scale(i16::from_le_bytes([pair[0], pair[1]])));
        }
        if let Some(&last) = pairs.remainder().first() {
            self.carry = Some(last);
        }
        &self.samples
    }
}

fn scale(sample: i16) -> f32 {
    sample as f32 / 32_768.0
}

fn drain(mut stderr: impl Read) -> String {
    let mut out = Vec::new();
    let mut buffer = [0u8; 4096];
    while let Ok(read) = stderr.read(&mut buffer) {
        if read == 0 {
            break;
        }
        if out.len() < 8 * 1024 {
            out.extend_from_slice(&buffer[..read]);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Detección de secciones
// ---------------------------------------------------------------------------

/// Etiqueta cada bucket comparando su energía con la media de la canción y
/// funde los tramos cortos hasta dejar secciones de al menos ~2 s.
fn detect_sections(buckets: &[SongBucket], duration_ms: u64) -> Vec<SongSection> {
    if buckets.is_empty() {
        return Vec::new();
    }
    let energies: Vec<f32> = buckets.iter().map(|b| b.energy.clamp(0.0, 1.0)).collect();
    let average = mean(&energies).max(0.01); // una canción en silencio no divide por cero
    let smooth = smooth(&energies, SMOOTH_BUCKETS);

    let mut runs: Vec<(SongSectionKind, usize)> = Vec::new();
    for i in 0..smooth.len() {
        let kind = classify(&smooth, i, average);
        match runs.last_mut() {
            Some((last, len)) if *last == kind => *len += 1,
            _ => runs.push((kind, 1)),
        }
    }

    let mut runs = merge_runs(runs);
    // Solo tiene sentido llamar intro/outro a los extremos si hay algo en medio.
    if runs.len() > 1 {
        runs[0].0 = SongSectionKind::Intro;
        let last = runs.len() - 1;
        runs[last].0 = SongSectionKind::Outro;
    }

    let mut sections = Vec::with_capacity(runs.len());
    let mut cursor = 0usize;
    for (kind, len) in runs {
        let start = cursor;
        let end = (cursor + len).min(buckets.len());
        if end <= start {
            break;
        }
        cursor = end;
        let start_ms = buckets[start].at_ms;
        let end_ms = buckets
            .get(end)
            .map(|b| b.at_ms)
            .unwrap_or(duration_ms)
            .max(start_ms);
        sections.push(SongSection {
            start_ms,
            end_ms,
            kind,
            energy: mean(&energies[start..end]),
        });
    }
    sections
}

fn classify(smooth: &[f32], i: usize, average: f32) -> SongSectionKind {
    let relative = smooth[i] / average;
    let back = smooth[i.saturating_sub(SLOPE_BUCKETS)];
    let slope = (smooth[i] - back) / average;
    let sustained = mean(&smooth[i.saturating_sub(SUSTAIN_BUCKETS)..=i]) / average;
    let rising = slope > RISE_THRESHOLD;
    // El drop es corto: alta energía justo después de una subida.
    let rose_recently = (i.saturating_sub(DROP_WINDOW)..i).any(|j| {
        let back = smooth[j.saturating_sub(SLOPE_BUCKETS)];
        (smooth[j] - back) / average > RISE_THRESHOLD
    });

    if relative < QUIET_LEVEL {
        SongSectionKind::Quiet
    } else if rising && relative < DROP_LEVEL {
        SongSectionKind::Build
    } else if relative >= DROP_LEVEL && rose_recently {
        SongSectionKind::Drop
    } else if sustained >= CHORUS_LEVEL {
        SongSectionKind::Chorus
    } else if relative >= VERSE_LEVEL {
        SongSectionKind::Verse
    } else {
        SongSectionKind::Bridge
    }
}

/// Funde tramos cortos con el vecino anterior y une vecinos del mismo tipo.
fn merge_runs(runs: Vec<(SongSectionKind, usize)>) -> Vec<(SongSectionKind, usize)> {
    let mut out: Vec<(SongSectionKind, usize)> = Vec::with_capacity(runs.len());
    for (kind, len) in runs {
        match out.last_mut() {
            Some((prev, prev_len)) if *prev == kind || len < MIN_SECTION_BUCKETS => {
                *prev_len += len
            }
            _ => out.push((kind, len)),
        }
    }
    // El primero no tiene anterior: si es corto se lo come el siguiente.
    while out.len() > 1 && out[0].1 < MIN_SECTION_BUCKETS {
        let (_, len) = out.remove(0);
        out[0].1 += len;
    }
    out
}

/// Media móvil centrada, recortada en los extremos.
fn smooth(values: &[f32], window: usize) -> Vec<f32> {
    let half = window / 2;
    (0..values.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(values.len());
            mean(&values[lo..hi])
        })
        .collect()
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(at_ms: u64, energy: f32) -> SongBucket {
        SongBucket {
            at_ms,
            energy,
            bass: energy * 0.5,
            mid: energy * 0.3,
            treble: energy * 0.2,
            vocal: energy * 0.4,
            beat: false,
        }
    }

    fn map_of(energies: &[f32]) -> SongMap {
        let buckets: Vec<SongBucket> = energies
            .iter()
            .enumerate()
            .map(|(i, energy)| bucket(i as u64 * BUCKET_MS, *energy))
            .collect();
        let duration = buckets.len() as u64 * BUCKET_MS;
        SongMap::from_buckets(duration, buckets)
    }

    fn kinds(map: &SongMap) -> Vec<SongSectionKind> {
        map.sections.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn bucket_timestamps_do_not_accumulate_truncation() {
        // El bucket real dura 5632/22050 s ≈ 255.46 ms. Etiquetar con
        // `i * BUCKET_MS` (255) adelantaba ~430 ms al final de una canción.
        let index = 900usize;
        let drift = bucket_at_ms(index) - index as u64 * BUCKET_MS;
        assert!(drift >= 350, "drift={drift}");
        assert_eq!(
            bucket_at_ms(index),
            index as u64 * SAMPLES_PER_BUCKET as u64 * 1000 / RATE as u64
        );
    }

    #[test]
    fn empty_map_is_inert() {
        let map = SongMap::default();
        assert!(map.is_empty());
        assert!(map.at(0).is_none());
        assert_eq!(map.energy_at(1_000), 0.0);
        assert_eq!(map.intensity_at(1_000), 0.0);
        assert!(map.section_at(0).is_none());
        assert!(map.sections.is_empty());
    }

    #[test]
    fn at_returns_the_covering_bucket_and_clamps() {
        let map = map_of(&[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(map.at(0).unwrap().energy, 0.1);
        assert_eq!(map.at(BUCKET_MS - 1).unwrap().energy, 0.1);
        assert_eq!(map.at(BUCKET_MS).unwrap().energy, 0.2);
        assert_eq!(map.at(BUCKET_MS * 3).unwrap().energy, 0.4);
        // Más allá del último bucket se queda en el último.
        assert_eq!(map.at(60_000).unwrap().energy, 0.4);
        assert_eq!(map.energy_at(BUCKET_MS * 2), 0.3);
    }

    #[test]
    fn intensity_is_smoothed_and_clamped() {
        // Un pico aislado se reparte entre vecinos y no llega a 1.
        let map = map_of(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        let peak = map.intensity_at(BUCKET_MS * 2);
        assert!(peak > 0.0 && peak < 0.5, "peak {peak}");

        // Energías fuera de rango se recortan.
        assert_eq!(map_of(&[2.0, 2.0, 2.0, 2.0]).intensity_at(0), 1.0);
        assert_eq!(map_of(&[-1.0, -1.0]).intensity_at(0), 0.0);
    }

    #[test]
    fn sections_cover_the_whole_song() {
        let energies: Vec<f32> = [0.10f32; 20]
            .into_iter()
            .chain(std::iter::repeat_n(0.50, 25))
            .chain((0..15).map(|i| 0.50 + i as f32 * 0.04))
            .chain(std::iter::repeat_n(1.00, 10))
            .chain(std::iter::repeat_n(0.90, 30))
            .chain(std::iter::repeat_n(0.12, 20))
            .collect();
        let map = map_of(&energies);

        assert_eq!(map.sections.first().unwrap().start_ms, 0);
        assert_eq!(map.sections.last().unwrap().end_ms, map.duration_ms);
        for pair in map.sections.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
        }
        assert_eq!(map.sections[0].kind, SongSectionKind::Intro);
        assert_eq!(map.sections.last().unwrap().kind, SongSectionKind::Outro);
        assert!(map.sections[0].energy < 0.3, "intro floja");
        for section in &map.sections {
            assert!((0.0..=1.0).contains(&section.energy));
            assert!(section.end_ms > section.start_ms);
        }

        let kinds = kinds(&map);
        assert!(kinds.contains(&SongSectionKind::Chorus), "{kinds:?}");
        assert!(kinds.contains(&SongSectionKind::Build), "{kinds:?}");
        assert!(kinds.len() >= 4, "{kinds:?}");
    }

    #[test]
    fn rising_energy_then_plateau_marks_a_drop() {
        let energies: Vec<f32> = std::iter::repeat_n(0.10, 20)
            .chain((0..30).map(|i| 0.20 + i as f32 * 0.03))
            .chain(std::iter::repeat_n(1.00, 12))
            .chain(std::iter::repeat_n(0.10, 16))
            .collect();
        let map = map_of(&energies);
        let kinds = kinds(&map);
        assert!(kinds.contains(&SongSectionKind::Build), "{kinds:?}");
        assert!(kinds.contains(&SongSectionKind::Drop), "{kinds:?}");
        assert_eq!(kinds.first(), Some(&SongSectionKind::Intro));
        assert_eq!(kinds.last(), Some(&SongSectionKind::Outro));
    }

    #[test]
    fn section_at_covers_the_timeline() {
        let map = map_of(&[0.1f32; 60]);
        let mid = map.duration_ms / 2;
        let section = map.section_at(mid).unwrap();
        assert!(section.start_ms <= mid && mid < section.end_ms);
        assert!(map.section_at(map.duration_ms + BUCKET_MS).is_none());
    }

    #[test]
    fn analyze_samples_builds_buckets_without_ffmpeg() {
        let tone: Vec<f32> = (0..RATE as usize * 2)
            .map(|i| (std::f32::consts::TAU * 110.0 * i as f32 / RATE as f32).sin() * 0.5)
            .chain(std::iter::repeat_n(0.0, RATE as usize))
            .collect();
        let map = analyze_samples(&tone);

        assert!(!map.is_empty());
        assert!(
            (map.duration_ms as i64 - 3_000).abs() <= 5,
            "{}",
            map.duration_ms
        );
        // ~4 buckets por segundo (±2 por el arranque del analizador).
        assert!(
            (map.buckets.len() as i64 - 12).abs() <= 2,
            "{}",
            map.buckets.len()
        );
        assert!(map.buckets.iter().all(|b| (0.0..=1.0).contains(&b.energy)));
        assert!(map.buckets.windows(2).all(|w| w[0].at_ms < w[1].at_ms));
        assert!(!map.sections.is_empty());
    }
}
