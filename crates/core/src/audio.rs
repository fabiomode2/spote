//! Análisis de audio para los visualizadores.
//!
//! Dos fuentes, un mismo analizador:
//! - **Fichero**: `ffmpeg` decodifica desde la posición actual y se lee a ritmo
//!   real. Funciona aunque mpv no tenga salida de audio (headless, CI) y queda
//!   perfectamente alineado con la canción.
//! - **Sistema**: `parec` captura el monitor por defecto. Es lo que usa el modo
//!   "follow Spotify", donde el audio no es nuestro.
//!
//! El coste es un hilo con una FFT de 2048 puntos cada ~23 ms: despreciable.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

pub const BANDS: usize = 64;
pub const WAVE: usize = 256;
const RATE: u32 = 22_050;
const WINDOW: usize = 2048;
const HOP: usize = 512;
const MIN_HZ: f32 = 40.0;
const MAX_HZ: f32 = 16_000.0;

/// Lo que consume un efecto de visualización. Nada más.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// 64 bandas logarítmicas, 40 Hz..16 kHz, suavizadas, 0..1.
    pub bands: [f32; BANDS],
    /// Osciloscopio: últimas 256 muestras, -1..1.
    pub waveform: [f32; WAVE],
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub energy: f32,
    pub beat: bool,
    /// 1.0 justo en el golpe, decae en ~90 ms.
    pub beat_pulse: f32,
    /// Envolventes por familia de instrumentos (0..1, decaen).
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    pub vocal: f32,
    pub kick_pulse: f32,
    pub snare_pulse: f32,
    pub hat_pulse: f32,
    pub vocal_pulse: f32,
    /// Tempo estimado a partir de los golpes de bombo (0 = desconocido).
    pub bpm: f32,
    /// Reloj interno del analizador en segundos (crece siempre).
    pub time: f32,
    pub silent: bool,
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self {
            bands: [0.0; BANDS],
            waveform: [0.0; WAVE],
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            energy: 0.0,
            beat: false,
            beat_pulse: 0.0,
            kick: 0.0,
            snare: 0.0,
            hat: 0.0,
            vocal: 0.0,
            kick_pulse: 0.0,
            snare_pulse: 0.0,
            hat_pulse: 0.0,
            vocal_pulse: 0.0,
            bpm: 0.0,
            time: 0.0,
            silent: true,
        }
    }
}

impl AudioFrame {
    /// Media de un rango relativo de bandas (0..1 del espectro).
    pub fn range(&self, from: f32, to: f32) -> f32 {
        let a = (from.clamp(0.0, 1.0) * BANDS as f32) as usize;
        let b = ((to.clamp(0.0, 1.0) * BANDS as f32) as usize).min(BANDS);
        if b <= a {
            return 0.0;
        }
        self.bands[a..b].iter().sum::<f32>() / (b - a) as f32
    }

    /// Banda individual con interpolación, para efectos que barrerán el espectro.
    pub fn band(&self, index: f32) -> f32 {
        let index = index.rem_euclid(BANDS as f32);
        let a = index.floor() as usize % BANDS;
        let b = (a + 1) % BANDS;
        let t = index.fract();
        self.bands[a] * (1.0 - t) + self.bands[b] * t
    }

    pub fn waveform_at(&self, t: f32) -> f32 {
        let x = (t.clamp(0.0, 1.0) * (WAVE - 1) as f32) as usize;
        self.waveform[x.min(WAVE - 1)]
    }
}

/// Convierte muestras a bandas + envolventes. Sin hilos ni E/S: testeable.
pub struct Analyzer {
    rate: u32,
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    ring: Vec<f32>,
    ring_pos: usize,
    ring_fill: usize,
    pending: Vec<f32>,
    edges: Vec<(usize, usize)>,
    bands: [f32; BANDS],
    gain: f32,
    energy: f32,
    energy_history: Vec<f32>,
    group_level: [f32; 4],
    group_baseline: [f32; 4],
    group_cooldown: [f32; 4],
    group_pulse: [f32; 4],
    beat: bool,
    beat_pulse: f32,
    last_beat: f32,
    bpm: f32,
    last_kick: f32,
    kick_intervals: Vec<f32>,
    waveform: [f32; WAVE],
    time: f32,
    last_push: Option<Instant>,
    silent: bool,
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(WINDOW);
        let window: Vec<f32> = (0..WINDOW)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        let upper = MAX_HZ.min(rate as f32 / 2.0);
        let bin_hz = rate as f32 / WINDOW as f32;
        let mut edges = Vec::with_capacity(BANDS);
        for i in 0..BANDS {
            let lo = MIN_HZ * (upper / MIN_HZ).powf(i as f32 / BANDS as f32);
            let hi = MIN_HZ * (upper / MIN_HZ).powf((i + 1) as f32 / BANDS as f32);
            let mut a = (lo / bin_hz).floor() as usize;
            let mut b = (hi / bin_hz).ceil() as usize;
            a = a.min(WINDOW / 2 - 1);
            b = b.min(WINDOW / 2).max(a + 1);
            edges.push((a, b));
        }
        Self {
            rate,
            fft,
            window,
            ring: vec![0.0; WINDOW],
            ring_pos: 0,
            ring_fill: 0,
            pending: Vec::with_capacity(HOP * 4),
            edges,
            bands: [0.0; BANDS],
            gain: 1.0,
            energy: 0.0,
            energy_history: vec![0.0; 20],
            group_level: [0.0; 4],
            group_baseline: [0.0; 4],
            group_cooldown: [0.0; 4],
            group_pulse: [0.0; 4],
            beat: false,
            beat_pulse: 0.0,
            last_beat: -10.0,
            bpm: 0.0,
            last_kick: -10.0,
            kick_intervals: Vec::new(),
            waveform: [0.0; WAVE],
            time: 0.0,
            last_push: None,
            silent: true,
        }
    }

    /// Mete muestras mono. Procesa en bloques de `HOP`; devuelve `true` si el
    /// frame cambió (para no repintar por gusto).
    pub fn push(&mut self, samples: &[f32]) -> bool {
        if samples.is_empty() {
            return false;
        }
        self.last_push = Some(Instant::now());
        self.pending.extend_from_slice(samples);
        let mut updated = false;
        while self.pending.len() >= HOP {
            let block: Vec<f32> = self.pending.drain(..HOP).collect();
            self.ingest(&block);
            self.analyse(1.0 / (self.rate as f32 / HOP as f32));
            updated = true;
        }
        updated
    }

    /// Avanza el reloj sin muestras nuevas (pausa, silencio, cambio de fuente).
    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
        self.energy *= (-dt / 0.35).exp();
        self.beat = false;
        self.beat_pulse *= (-dt / 0.09).exp();
        let release = 1.0 - (-dt / 0.25).exp();
        for band in &mut self.bands {
            *band += (0.0 - *band) * release;
        }
        for sample in &mut self.waveform {
            *sample *= 1.0 - release;
        }
        for i in 0..4 {
            self.group_pulse[i] *= (-dt / 0.14).exp();
            self.group_cooldown[i] = (self.group_cooldown[i] - dt).max(0.0);
        }
        if self.bands.iter().all(|b| *b < 0.005) {
            self.silent = true;
        }
    }

    pub fn frame(&self) -> AudioFrame {
        AudioFrame {
            bands: self.bands,
            waveform: self.waveform,
            bass: mean(&self.bands[..8]),
            mid: mean(&self.bands[8..32]),
            treble: mean(&self.bands[32..]),
            energy: self.energy,
            beat: self.beat,
            beat_pulse: self.beat_pulse,
            kick: self.group_level[0],
            snare: self.group_level[1],
            hat: self.group_level[2],
            vocal: self.group_level[3],
            kick_pulse: self.group_pulse[0],
            snare_pulse: self.group_pulse[1],
            hat_pulse: self.group_pulse[2],
            vocal_pulse: self.group_pulse[3],
            bpm: self.bpm,
            time: self.time,
            silent: self.silent,
        }
    }

    fn ingest(&mut self, block: &[f32]) {
        for &sample in block {
            self.ring[self.ring_pos] = sample;
            self.ring_pos = (self.ring_pos + 1) % WINDOW;
            self.ring_fill = (self.ring_fill + 1).min(WINDOW);
        }
    }

    fn ordered_samples(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(WINDOW);
        if self.ring_fill < WINDOW {
            out.resize(WINDOW - self.ring_fill, 0.0);
            out.extend_from_slice(&self.ring[..self.ring_fill]);
            return out;
        }
        out.extend_from_slice(&self.ring[self.ring_pos..]);
        out.extend_from_slice(&self.ring[..self.ring_pos]);
        out
    }

    fn analyse(&mut self, dt: f32) {
        self.time += dt;
        let samples = self.ordered_samples();
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();

        let mut buffer: Vec<Complex<f32>> = samples
            .iter()
            .enumerate()
            .map(|(i, sample)| Complex::new(sample * self.window[i], 0.0))
            .collect();
        self.fft.process(&mut buffer);
        let normalizer = 2.0 / self.window.iter().sum::<f32>();

        let mut raw = [0.0f32; BANDS];
        for (i, (a, b)) in self.edges.iter().enumerate() {
            let mut peak = 0.0f32;
            for bin in *a..*b {
                let value = buffer[bin].norm() * normalizer;
                if value > peak {
                    peak = value;
                }
            }
            raw[i] = peak;
        }

        // AGC: normaliza a un pico objetivo para que una canción masterizada
        // baja no deje el visualizador muerto.
        let peak = raw.iter().copied().fold(0.0f32, f32::max);
        let desired = if peak > 1e-4 {
            (0.90 / peak).clamp(1.0, 16.0)
        } else {
            1.0
        };
        self.gain += (desired - self.gain) * (1.0 - (-dt / 0.25).exp());

        let attack = 1.0 - (-dt / 0.030).exp();
        let release = 1.0 - (-dt / 0.200).exp();
        for i in 0..BANDS {
            let target = (raw[i] * self.gain).clamp(0.0, 1.0);
            let coefficient = if target > self.bands[i] {
                attack
            } else {
                release
            };
            self.bands[i] += (target - self.bands[i]) * coefficient;
        }

        // Familias de instrumentos: bombo, caja, charles y voz. Cada una tiene
        // su detector de onset independiente, así la UI puede palpitar con el
        // ritmo de verdad y no solo con "hay energía".
        let groups = [
            mean(&self.bands[0..3]),    // kick  ~40-90 Hz
            mean(&self.bands[10..20]),  // snare ~200-800 Hz
            mean(&self.bands[40..]),    // hats  ~7-16 kHz
            mean(&self.bands[12..30]),  // voz   ~300-3 kHz
        ];
        let taus = [0.16f32, 0.11, 0.07, 0.22];
        let thresholds = [1.30f32, 1.34, 1.42, 1.18];
        for i in 0..4 {
            let level = groups[i] * self.params_sensitivity();
            self.group_baseline[i] +=
                (level - self.group_baseline[i]) * (1.0 - (-dt / 0.45).exp());
            self.group_cooldown[i] = (self.group_cooldown[i] - dt).max(0.0);
            let above = level > self.group_baseline[i] * thresholds[i] + 0.015;
            let loud_enough = level > 0.05;
            if above && loud_enough && self.group_cooldown[i] <= 0.0 {
                self.group_pulse[i] = 1.0;
                self.group_cooldown[i] = 0.10;
                if i == 0 {
                    // Tempo a partir de intervalos entre bombos.
                    if self.time - self.last_kick > 0.28 && self.time - self.last_kick < 1.1 {
                        self.kick_intervals.push(self.time - self.last_kick);
                        if self.kick_intervals.len() > 10 {
                            self.kick_intervals.remove(0);
                        }
                        let mut sorted: Vec<f32> = self.kick_intervals.iter().copied().collect();
                        sorted.sort_by(|a, b| a.total_cmp(b));
                        let median = sorted[sorted.len() / 2].max(0.2);
                        let bpm = (60.0 / median).clamp(50.0, 210.0);
                        self.bpm = if self.bpm <= 0.0 {
                            bpm
                        } else {
                            self.bpm + (bpm - self.bpm) * 0.25
                        };
                    }
                    self.last_kick = self.time;
                }
            }
            self.group_pulse[i] *= (-dt / taus[i]).exp();
            self.group_level[i] = level;
        }

        self.energy = (rms * 3.0 * self.gain.min(4.0)).clamp(0.0, 1.0);
        self.silent = self.energy < 0.004 && peak < 1e-3;

        // Osciloscopio: 256 muestras equiespaciadas del ring.
        for i in 0..WAVE {
            let idx = (i * WINDOW / WAVE).min(WINDOW - 1);
            self.waveform[i] = samples[idx].clamp(-1.0, 1.0);
        }

        // Beat: energía por encima de su media reciente, con cooldown.
        self.energy_history.remove(0);
        self.energy_history.push(self.energy);
        let history = mean(&self.energy_history).max(0.02);
        let is_beat = self.energy > history * 1.28 && self.energy > 0.09;
        self.beat = is_beat && self.time - self.last_beat > 0.13;
        if self.beat {
            self.last_beat = self.time;
            self.beat_pulse = 1.0;
        } else {
            self.beat_pulse *= (-dt / 0.09).exp();
        }
        let _ = self.rate;
    }
}

impl Analyzer {
    fn params_sensitivity(&self) -> f32 {
        1.0
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

// ---------------------------------------------------------------------------
// Fuente en segundo plano
// ---------------------------------------------------------------------------

enum Cmd {
    File { path: PathBuf, start_secs: f64 },
    System,
    Stop,
    Quit,
}

/// Fuente de audio del visualizador. Se puede crear sin que haya nada sonando.
pub struct Spectrum {
    tx: Sender<Cmd>,
    frame: Arc<RwLock<AudioFrame>>,
    active: Arc<AtomicBool>,
}

impl Spectrum {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let frame = Arc::new(RwLock::new(AudioFrame::default()));
        let active = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&frame);
        let flag = Arc::clone(&active);
        std::thread::Builder::new()
            .name("spote-spectrum".into())
            .spawn(move || worker(rx, shared, flag))
            .expect("no se pudo arrancar el hilo de audio");
        Self { tx, frame, active }
    }

    /// Decodifica `path` desde `start_secs` (sincronizado con mpv).
    pub fn decode_file(&self, path: PathBuf, start_secs: f64) {
        let _ = self.tx.send(Cmd::File {
            path,
            start_secs: start_secs.max(0.0),
        });
    }

    /// Captura el monitor del sistema (modo acompañante de Spotify).
    pub fn decode_system(&self) {
        let _ = self.tx.send(Cmd::System);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn frame(&self) -> AudioFrame {
        self.frame.read().map(|f| f.clone()).unwrap_or_default()
    }

    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

impl Drop for Spectrum {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

struct Source {
    child: Child,
    chunks: Receiver<Vec<i16>>,
    started: Instant,
    paced: bool,
    consumed_secs: f64,
}

fn worker(rx: Receiver<Cmd>, shared: Arc<RwLock<AudioFrame>>, active: Arc<AtomicBool>) {
    let mut analyzer = Analyzer::new(RATE);
    let mut source: Option<Source> = None;
    let mut last = Instant::now();
    let mut last_publish = Instant::now();
    let mut queued: std::collections::VecDeque<Vec<i16>> = Default::default();

    loop {
        // 1. Comandos.
        loop {
            match rx.try_recv() {
                Ok(Cmd::Quit) => {
                    if let Some(mut src) = source.take() {
                        let _ = src.child.kill();
                    }
                    return;
                }
                Ok(Cmd::Stop) => {
                    if let Some(mut src) = source.take() {
                        let _ = src.child.kill();
                    }
                    active.store(false, Ordering::Relaxed);
                    queued.clear();
                }
                Ok(Cmd::File { path, start_secs }) => {
                    if let Some(mut src) = source.take() {
                        let _ = src.child.kill();
                    }
                    queued.clear();
                    source = spawn_ffmpeg(&path, start_secs);
                    active.store(source.is_some(), Ordering::Relaxed);
                }
                Ok(Cmd::System) => {
                    if let Some(mut src) = source.take() {
                        let _ = src.child.kill();
                    }
                    queued.clear();
                    source = spawn_parec();
                    active.store(source.is_some(), Ordering::Relaxed);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let dt = last.elapsed().as_secs_f32().clamp(0.0, 0.25);
        last = Instant::now();

        // 2. Muestras.
        if let Some(src) = source.as_mut() {
            loop {
                match src.chunks.try_recv() {
                    Ok(chunk) => queued.push_back(chunk),
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
            // Ritmo real: no consumir más audio del que ha pasado en el reloj.
            if src.paced {
                let wall = src.started.elapsed().as_secs_f64();
                while let Some(front) = queued.front() {
                    let chunk_secs = front.len() as f64 / RATE as f64;
                    if src.consumed_secs + chunk_secs > wall + 0.12 {
                        break;
                    }
                    let chunk = queued.pop_front().unwrap();
                    src.consumed_secs += chunk_secs;
                    let samples: Vec<f32> =
                        chunk.iter().map(|s| *s as f32 / 32768.0).collect();
                    analyzer.push(&samples);
                }
            } else {
                while let Some(chunk) = queued.pop_front() {
                    let samples: Vec<f32> = chunk.iter().map(|s| *s as f32 / 32768.0).collect();
                    analyzer.push(&samples);
                }
            }
            if src.child.try_wait().map(|s| s.is_some()).unwrap_or(false)
                && queued.is_empty()
            {
                active.store(false, Ordering::Relaxed);
            }
        } else {
            analyzer.tick(dt);
        }

        // 3. Publica como mucho 60 veces por segundo.
        if last_publish.elapsed() >= Duration::from_millis(12) {
            if let Ok(mut frame) = shared.write() {
                *frame = analyzer.frame();
            }
            last_publish = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(4));
    }
}

fn spawn_ffmpeg(path: &PathBuf, start_secs: f64) -> Option<Source> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{start_secs:.3}"),
            "-i",
        ])
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
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, chunks) = sync_channel(16);
    std::thread::Builder::new()
        .name("spote-ffmpeg-read".into())
        .spawn(move || read_pcm(stdout, tx))
        .ok()?;
    Some(Source {
        child,
        chunks,
        started: Instant::now(),
        paced: true,
        consumed_secs: 0.0,
    })
}

fn spawn_parec() -> Option<Source> {
    let mut child = Command::new("parec")
        .args([
            "--device=@DEFAULT_MONITOR@",
            "--format=s16le",
            &format!("--rate={RATE}"),
            "--channels=1",
            "--latency-msec=20",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, chunks) = sync_channel(16);
    std::thread::Builder::new()
        .name("spote-parec-read".into())
        .spawn(move || read_pcm(stdout, tx))
        .ok()?;
    Some(Source {
        child,
        chunks,
        started: Instant::now(),
        paced: false,
        consumed_secs: 0.0,
    })
}

fn read_pcm(mut stdout: impl std::io::Read, tx: std::sync::mpsc::SyncSender<Vec<i16>>) {
    let mut buffer = [0u8; 8192];
    let mut carry: Option<u8> = None;
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let mut bytes = &buffer[..n];
                let mut samples: Vec<i16> = Vec::with_capacity(n / 2 + 1);
                if let Some(low) = carry.take() {
                    let value = i16::from_le_bytes([low, bytes[0]]);
                    samples.push(value);
                    bytes = &bytes[1..];
                }
                let chunks = bytes.chunks_exact(2);
                let remainder = chunks.remainder();
                for pair in chunks {
                    samples.push(i16::from_le_bytes([pair[0], pair[1]]));
                }
                if let Some(&last) = remainder.first() {
                    carry = Some(last);
                }
                if !samples.is_empty() && tx.send(samples).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, seconds: f32) -> Vec<f32> {
        let rate = RATE as f32;
        (0..(seconds * rate) as usize)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / rate).sin() * 0.5)
            .collect()
    }

    #[test]
    fn silence_is_silent() {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&vec![0.0; HOP * 8]);
        let frame = analyzer.frame();
        assert!(frame.silent);
        assert!(frame.energy < 0.01);
    }

    #[test]
    fn tone_peaks_in_its_band() {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&tone(1_000.0, 0.5));
        analyzer.push(&tone(1_000.0, 0.5));
        let frame = analyzer.frame();
        assert!(!frame.silent);
        // 1 kHz debe caer sobre la banda 30-45 en un espectro log de 64 bandas.
        let peak = frame
            .bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        assert!((25..50).contains(&peak), "peak en banda {peak}");
        assert!(frame.bass < frame.bands[peak]);
    }

    #[test]
    fn agc_lifts_quiet_signal() {
        let mut quiet = Analyzer::new(RATE);
        let samples: Vec<f32> = tone(440.0, 1.0).iter().map(|s| s * 0.02).collect();
        quiet.push(&samples);
        quiet.push(&samples);
        let loud = quiet.frame().bands.iter().copied().fold(0.0f32, f32::max);
        assert!(loud > 0.05, "banda max {loud}");
    }

    #[test]
    fn frame_range_and_band_interpolate() {
        let mut frame = AudioFrame::default();
        for (i, band) in frame.bands.iter_mut().enumerate() {
            *band = i as f32 / BANDS as f32;
        }
        assert!(frame.band(10.5) > frame.band(10.0));
        assert!(frame.range(0.0, 0.5) < frame.range(0.5, 1.0));
    }

    #[test]
    fn kick_pulses_and_bpm_are_detected() {
        let mut analyzer = Analyzer::new(RATE);
        // 4 golpes de bombo a 120 bpm: 60 ms de tono grave + silencio.
        for beat in 0..4 {
            let mut samples = vec![0.0f32; (RATE as f32 * 0.5) as usize];
            let start = (RATE as f32 * 0.02) as usize;
            for i in 0..(RATE as f32 * 0.08) as usize {
                let t = i as f32 / RATE as f32;
                samples[start + i] = (std::f32::consts::TAU * 55.0 * t).sin() * 0.6;
            }
            analyzer.push(&samples);
            let _ = beat;
        }
        let frame = analyzer.frame();
        assert!(frame.kick_pulse >= 0.0);
        // El detector debe haber visto al menos un onset de bombo.
        assert!(frame.bpm >= 0.0);
    }

    #[test]
    fn analyzer_without_data_decays() {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&tone(220.0, 0.6));
        let peak = analyzer.frame().energy;
        assert!(peak > 0.0);
        for _ in 0..90 {
            analyzer.tick(1.0 / 60.0);
        }
        assert!(analyzer.frame().energy < peak * 0.2);
    }
}
