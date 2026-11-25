#![allow(dead_code)]
//! Los efectos. Mecanismos portados de `cli-viz` (spectrum bars, wave, circle,
//! particles, flame, matrix, starfield, fractal, neural, pulsar, tunnel,
//! spectro3d, scope, lcd, led, braille, floor), recoloreados con la paleta del
//! álbum.

use std::collections::VecDeque;
use std::sync::OnceLock;

use super::{Effect, Layer, Params, Rng, VizCtx};
use spote_core::artwork::Rgb;
use spote_core::audio::{AudioFrame, BANDS, WAVE};
use spote_core::util::{mix, mix_oklab};

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];
const DOTS: [char; 6] = ['·', '•', '∙', '◦', '○', '●'];

/// Glifos Braille U+2800..U+28FF indexados por su máscara de 8 puntos. El
/// índice 0 es un espacio: una celda sin puntos no llega a pintarse.
static BRAILLE_LUT: [char; 256] = braille_lut();

const fn braille_lut() -> [char; 256] {
    let mut table = [' '; 256];
    let mut mask = 1usize;
    while mask < 256 {
        table[mask] = match char::from_u32(0x2800 + mask as u32) {
            Some(ch) => ch,
            None => ' ',
        };
        mask += 1;
    }
    table
}

/// Bits de una columna Braille (izquierda/derecha) con los puntos encendidos
/// desde `top` (0 = fila superior) hasta el borde inferior. Índice 4 = vacía.
const BRAILLE_COL: [[u8; 5]; 2] = [
    [0x47, 0x46, 0x44, 0x40, 0x00],
    [0xB8, 0xB0, 0xA0, 0x80, 0x00],
];

/// Bit de cada subpunto Braille, indexado por `[columna][fila]` de la celda.
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Resolución de la tabla de `t^1.15`: reparte las bandas del espectro sin
/// llamar a `powf` por subcolumna.
const POW_STEPS: usize = 1024;

fn spectrum_curve() -> &'static [f32; POW_STEPS] {
    static CURVE: OnceLock<[f32; POW_STEPS]> = OnceLock::new();
    CURVE.get_or_init(|| {
        let mut lut = [0.0; POW_STEPS];
        for (index, slot) in lut.iter_mut().enumerate() {
            *slot = (index as f32 / (POW_STEPS - 1) as f32).powf(1.15);
        }
        lut
    })
}

/// Construye un efecto por su clave. Claves desconocidas → barras.
pub fn effect_by_key(key: &str) -> Box<dyn Effect> {
    match key {
        "wave" => Box::new(Wave::default()),
        "circle" => Box::new(Circle::default()),
        "particles" => Box::new(Particles::default()),
        "flame" => Box::new(Flame::default()),
        "matrix" => Box::new(Matrix::default()),
        "starfield" => Box::new(Starfield::default()),
        "fractal" => Box::new(Fractal::default()),
        "neural" => Box::new(Neural::default()),
        "pulsar" => Box::new(Pulsar::default()),
        "tunnel" => Box::new(Tunnel::default()),
        "spectro3d" => Box::new(Spectro3d::default()),
        "scope" => Box::new(Scope::default()),
        "lcd" => Box::new(Lcd::default()),
        "led" => Box::new(Led::default()),
        "bars_braille" => Box::new(BarsBraille::default()),
        "floor" => Box::new(Floor::default()),
        _ => Box::new(Bars::default()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Bars,
    Wave,
    Circle,
    Particles,
    Flame,
    Matrix,
    Starfield,
    Fractal,
    Neural,
    Pulsar,
    Tunnel,
    Spectro3d,
    Scope,
    Lcd,
    Led,
    BarsBraille,
    Floor,
}

impl EffectKind {
    pub const ALL: [EffectKind; 17] = [
        EffectKind::Bars,
        EffectKind::Wave,
        EffectKind::Circle,
        EffectKind::Particles,
        EffectKind::Flame,
        EffectKind::Matrix,
        EffectKind::Starfield,
        EffectKind::Fractal,
        EffectKind::Neural,
        EffectKind::Pulsar,
        EffectKind::Tunnel,
        EffectKind::Spectro3d,
        EffectKind::Scope,
        EffectKind::Lcd,
        EffectKind::Led,
        EffectKind::BarsBraille,
        EffectKind::Floor,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Bars => "bars",
            Self::Wave => "wave",
            Self::Circle => "circle",
            Self::Particles => "particles",
            Self::Flame => "flame",
            Self::Matrix => "matrix",
            Self::Starfield => "starfield",
            Self::Fractal => "fractal",
            Self::Neural => "neural",
            Self::Pulsar => "pulsar",
            Self::Tunnel => "tunnel",
            Self::Spectro3d => "spectro3d",
            Self::Scope => "scope",
            Self::Lcd => "lcd",
            Self::Led => "led",
            Self::BarsBraille => "bars_braille",
            Self::Floor => "floor",
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Barras
// ---------------------------------------------------------------------------

pub struct Bars {
    params: Params,
    peaks: Vec<f32>,
    boost: f32,
}

impl Default for Bars {
    fn default() -> Self {
        Self {
            params: Params::default(),
            peaks: vec![0.0; 128],
            boost: 1.35,
        }
    }
}

impl Effect for Bars {
    fn key(&self) -> &'static str {
        "bars"
    }
    fn name(&self) -> &'static str {
        "barras"
    }

    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn handle_key(&mut self, key: char) -> bool {
        match key {
            'b' => {
                self.boost = (self.boost + 0.15).min(3.0);
                true
            }
            'B' => {
                self.boost = (self.boost - 0.15).max(0.6);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let h = ctx.h as i32;
        if h <= 0 || layer.w == 0 {
            return;
        }
        let count = (layer.w as usize).min(96).max(4);
        if self.peaks.len() < count {
            self.peaks.resize(count, 0.0);
        }
        let dt = ctx.dt;
        let width_per_bar = layer.w as f32 / count as f32;
        // Precalcular sombras por fila y color base por barra: antes se hacía
        // `mix_oklab` + `shade` por celda y en pantalla completa (200x56) el
        // preset clean perdía fps. Con tablas es el mismo dibujo, mucho menos
        // trabajo por frame.
        let row_shades: Vec<Rgb> = (0..h as usize)
            .map(|row| ctx.shade(0.30 + (row as f32 / h as f32) * 0.62))
            .collect();
        let bases: Vec<Rgb> = (0..count)
            .map(|i| {
                // Colores de la propia portada, no solo primary/secondary: en
                // pantalla completa las barras se veían todas del mismo tono.
                // Ciclo rápido por barra: con solo unas pocas barras con señal
                // un gradiente suave las dejaba todas del mismo tono. Así cada
                // barra recorre la paleta de la portada.
                let hue = (i as f32 * 0.37
                    + ctx.time * 0.11
                    + ctx.audio.bass * 0.18
                    + ctx.audio.kick_pulse * 0.08)
                    % 1.0;
                mix_oklab(ctx.palette.ramp_at(hue), ctx.palette.accent, 0.15)
            })
            .collect();
        for i in 0..count {
            let t = i as f32 / count as f32;
            let band_index = (t.powf(1.35) * BANDS as f32).min(BANDS as f32 - 1.0);
            let value = (ctx.audio.band(band_index)
                * self.params.sensitivity
                * self.boost
                * (1.0 + ctx.audio.kick * 0.22))
                .clamp(0.0, 1.0);
            let height = value * h as f32 * 0.98;
            let peak = self.peaks[i].max(height);
            // Caída del pico: rápida al principio, luego lenta (feel de EQ).
            self.peaks[i] = (peak - dt * (1.5 + peak * 2.5)).max(height);

            let x0 = (i as f32 * width_per_bar) as i32;
            let x1 = (((i + 1) as f32 * width_per_bar).ceil() as i32).max(x0 + 1);
            for x in x0..x1 {
                let full = height.floor() as i32;
                let base = bases[i];
                for row in 0..full {
                    let y = h - 1 - row;
                    let level = row as f32 / h as f32;
                    let ramp = row_shades[row as usize];
                    // `mix` lineal en vez de oklab: es un degradado interno de
                    // barra con miles de celdas por frame y no se nota.
                    let mut color = mix(ramp, base, 0.5);
                    if level > 0.72 {
                        color = mix(color, ctx.palette.text, 0.42);
                    }
                    layer.put(x, y, '█', color, 1.0);
                }
                // Sub-celda: el bloque parcial da la sensación de altura continua.
                let fraction = height - height.floor();
                let extra = (fraction * 8.0) as usize;
                if extra > 0 {
                    let y = h - 1 - full;
                    layer.put(
                        x,
                        y,
                        BLOCKS[extra.min(8)],
                        mix(base, ctx.palette.text, 0.35),
                        1.0,
                    );
                }
                // Pico flotante.
                let peak_row = self.peaks[i];
                if peak_row > 1.5 && (peak_row - height) > 0.8 {
                    let y = h - 1 - peak_row as i32;
                    layer.put(x, y, '▔', ctx.shade(0.98), 0.9);
                }
            }
        }
        if ctx.audio.beat {
            let y = h - 1;
            for x in 0..layer.w as i32 {
                let t = x as f32 / layer.w.max(1) as f32;
                layer.put(x, y, '▁', ctx.shade(0.98), 0.35 * ctx.audio.beat_pulse);
                let _ = t;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Onda
// ---------------------------------------------------------------------------

pub struct Wave {
    params: Params,
    phases: [f32; 8],
}

impl Default for Wave {
    fn default() -> Self {
        Self {
            params: Params::default(),
            phases: [0.0; 8],
        }
    }
}

impl Effect for Wave {
    fn key(&self) -> &'static str {
        "wave"
    }
    fn name(&self) -> &'static str {
        "onda"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w < 2 || h < 2 {
            return;
        }
        let mid = h as f32 / 2.0;
        for (i, phase) in self.phases.iter_mut().enumerate() {
            *phase += ctx.dt * (0.6 + i as f32 * 0.22) * (0.5 + ctx.audio.mid);
        }
        let mut previous: Option<i32> = None;
        for x in 0..w {
            let nx = x as f32 / w as f32;
            let mut value = 0.0;
            for i in 0..8 {
                let amplitude = ctx.audio.bands[i * 2] * self.params.sensitivity;
                let freq = (i as f32 + 1.0) * 1.7;
                value += amplitude * (std::f32::consts::TAU * freq * nx + self.phases[i]).sin();
            }
            // Mezcla con el osciloscopio real: la onda sigue la forma del audio.
            let scope = ctx.audio.waveform_at(nx) * 0.9;
            let y = mid - value * h as f32 * 0.52 - scope * h as f32 * 0.22;
            let y = y.clamp(0.0, (h - 1) as f32) as i32;
            let brightness = 0.55 + (value.abs() * 0.6).min(0.42) + ctx.audio.energy * 0.15;
            let color = ctx.shade(brightness.min(1.0));
            let ch = match previous {
                Some(prev) if y < prev => '╱',
                Some(prev) if y > prev => '╲',
                _ => '─',
            };
            layer.put(x, y, ch, color, 0.95);
            if let Some(prev) = previous {
                // Rellena el hueco vertical para que la onda no se corte.
                let (a, b) = if prev < y {
                    (prev + 1, y)
                } else {
                    (y + 1, prev)
                };
                for fill in a..b {
                    layer.put(x, fill, '│', color, 0.55);
                }
            }
            previous = Some(y);
        }
        // Línea base tenue.
        let base = mid as i32;
        for x in 0..w {
            if layer.ch_at(x, base) == ' ' {
                layer.put(x, base, '·', ctx.shade(0.28), 0.25);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Círculo
// ---------------------------------------------------------------------------

pub struct Circle {
    params: Params,
    rotation: f32,
}

impl Default for Circle {
    fn default() -> Self {
        Self {
            params: Params::default(),
            rotation: 0.0,
        }
    }
}

impl Effect for Circle {
    fn key(&self) -> &'static str {
        "circle"
    }
    fn name(&self) -> &'static str {
        "círculo"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w < 4 || h < 3 {
            return;
        }
        self.rotation += ctx.dt * 0.35;
        let cx = w as f32 / 2.0;
        let cy = h as f32 / 2.0;
        let base = (h as f32 * 0.42).min(w as f32 * 0.24);
        let segments = (w.min(h * 4) as i32).max(48);
        let energy = ctx.audio.bass * 1.4 + ctx.audio.energy * 0.4;
        for layer_index in 0..3 {
            let spread = 1.0 - layer_index as f32 * 0.16;
            let points = segments;
            for step in 0..points {
                let angle = step as f32 / points as f32 * std::f32::consts::TAU + self.rotation;
                let band = (step as f32 / points as f32 * 24.0) as usize % 48;
                let amplitude = ctx.audio.bands[band] * self.params.sensitivity;
                let wobble = (angle * 3.0 + ctx.time * 1.7).sin() * 0.06;
                let radius = (base * spread) * (1.0 + wobble) + amplitude * base * 0.55;
                let x = cx + angle.cos() * radius * 1.9;
                let y = cy + angle.sin() * radius;
                let color = ctx.shade((0.45 + amplitude * 0.55).clamp(0.0, 1.0));
                let ch = if amplitude > 0.55 {
                    '●'
                } else if amplitude > 0.25 {
                    '•'
                } else {
                    '·'
                };
                layer.put(x as i32, y as i32, ch, color, 0.9);
                // Radio tenue hacia el centro: da sensación de espectro radial.
                if amplitude > 0.35 && layer_index == 0 {
                    let inner = radius * 0.55;
                    let ix = cx + angle.cos() * inner * 1.9;
                    let iy = cy + angle.sin() * inner;
                    layer.line(
                        ix as i32,
                        iy as i32,
                        x as i32,
                        y as i32,
                        '·',
                        ctx.shade(0.35 + amplitude * 0.3),
                        0.35,
                    );
                }
            }
        }
        // Núcleo que late.
        let core = 0.5 + ctx.audio.beat_pulse * 1.5 + energy * 0.5;
        layer.put(
            cx as i32,
            cy as i32,
            '◉',
            ctx.shade(1.0),
            (0.5 + core * 0.3).min(1.0),
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Partículas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max_life: f32,
    hue: f32,
    size: u8,
}

pub struct Particles {
    params: Params,
    particles: Vec<Particle>,
    rng: Rng,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            params: Params::default(),
            particles: Vec::with_capacity(512),
            rng: Rng::seeded(),
        }
    }
}

impl Effect for Particles {
    fn key(&self) -> &'static str {
        "particles"
    }
    fn name(&self) -> &'static str {
        "partículas"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as f32, layer.h as f32);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let dt = ctx.dt;
        let cap = (220.0 * self.params.density).clamp(20.0, 900.0) as usize;
        let mut spawn = if ctx.audio.beat {
            14 + (ctx.audio.energy * 48.0) as usize
        } else {
            (ctx.audio.energy * 6.0) as usize
        };
        // Cada golpe de caja añade una ráfaga.
        if ctx.audio.snare_pulse > 0.55 {
            spawn += (ctx.audio.snare_pulse * 26.0) as usize;
        }
        for _ in 0..spawn {
            if self.particles.len() >= cap {
                break;
            }
            let max_life = self.rng.range(0.6, 2.2);
            self.particles.push(Particle {
                x: w * 0.5 + self.rng.range(-w * 0.22, w * 0.22),
                y: h * self.rng.range(0.75, 1.0),
                vx: self.rng.range(-3.0, 3.0) * (0.4 + ctx.audio.mid),
                vy: self.rng.range(-9.0, -3.0) * (0.5 + ctx.audio.bass),
                life: max_life,
                max_life,
                hue: self.rng.f32(),
                size: self.rng.below(4) as u8,
            });
        }
        let gravity = 4.5 * self.params.speed;
        let mut index = 0;
        while index < self.particles.len() {
            let particle = &mut self.particles[index];
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            particle.vy += gravity * dt;
            particle.vx *= 1.0 - 0.35 * dt;
            particle.life -= dt;
            if particle.life <= 0.0
                || particle.y > h + 1.0
                || particle.x < -1.0
                || particle.x > w + 1.0
            {
                self.particles.swap_remove(index);
                continue;
            }
            let life = (particle.life / particle.max_life).clamp(0.0, 1.0);
            let color = ctx.shade(0.45 + particle.hue * 0.35 + life * 0.2);
            let ch = match particle.size {
                0 => '·',
                1 => '•',
                2 => '*',
                _ => '+',
            };
            layer.put(
                particle.x as i32,
                particle.y as i32,
                ch,
                color,
                (0.25 + life * 0.75).min(1.0),
            );
            index += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Fuego
// ---------------------------------------------------------------------------

pub struct Flame {
    params: Params,
    heat: Vec<f32>,
    width: u16,
    height: u16,
    rng: Rng,
}

impl Default for Flame {
    fn default() -> Self {
        Self {
            params: Params::default(),
            heat: Vec::new(),
            width: 0,
            height: 0,
            rng: Rng::seeded(),
        }
    }
}

impl Effect for Flame {
    fn key(&self) -> &'static str {
        "flame"
    }
    fn name(&self) -> &'static str {
        "fuego"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w, layer.h);
        if w == 0 || h < 2 {
            return;
        }
        if self.width != w || self.height != h {
            self.width = w;
            self.height = h;
            self.heat = vec![0.0; w as usize * h as usize];
        }
        let bass = (ctx.audio.bass * 1.6 * self.params.sensitivity
            + ctx.audio.kick * 0.8
            + ctx.audio.energy * 0.3)
            .clamp(0.0, 1.2);
        let last = (h - 1) as usize;
        let mut consumed = 0.0;
        for x in 0..w as usize {
            let noise = self.rng.f32();
            let seed = (bass * (0.45 + noise * 0.7)).clamp(0.0, 1.0);
            self.heat[last * w as usize + x] = seed.clamp(0.0, 1.2);
            consumed += seed;
        }
        let cooling = (0.86 - ctx.dt * 0.10).clamp(0.62, 0.94);
        for y in (0..h as usize - 1).rev() {
            for x in 0..w as usize {
                let left = if x == 0 { 0 } else { x - 1 };
                let right = if x + 1 >= w as usize { x } else { x + 1 };
                let below = (y + 1) * w as usize;
                let row = y * w as usize;
                let sum = self.heat[below + left] * 0.30
                    + self.heat[below + x] * 0.45
                    + self.heat[below + right] * 0.18
                    + self.heat[((y + 2) * w as usize + x).min(self.heat.len() - 1)] * 0.07;
                self.heat[row + x] = (sum * cooling).min(1.2);
            }
        }
        for y in 0..h as usize {
            for x in 0..w as usize {
                let value = self.heat[y * w as usize + x];
                if value <= 0.10 {
                    continue;
                }
                let index = ((value * 4.6) as usize).min(4);
                let color = ctx.shade((0.35 + value * 0.62).min(1.0));
                layer.put(x as i32, y as i32, SHADES[index], color, value.min(1.0));
            }
        }
        let _ = consumed;
    }
}

// ---------------------------------------------------------------------------
// 6. Matrix
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Drop {
    y: f32,
    speed: f32,
    length: usize,
    glyphs: Vec<u8>,
    offset: usize,
}

pub struct Matrix {
    params: Params,
    columns: Vec<Option<Drop>>,
    rng: Rng,
    glyphs: Vec<char>,
}

impl Default for Matrix {
    fn default() -> Self {
        // ASCII y bloques: todos de ancho 1 en cualquier terminal.
        let glyphs =
            "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz<>[]{}/\\|=+-*#%&$@!?"
                .chars()
                .collect();
        Self {
            params: Params::default(),
            columns: Vec::new(),
            rng: Rng::seeded(),
            glyphs,
        }
    }
}

impl Effect for Matrix {
    fn key(&self) -> &'static str {
        "matrix"
    }
    fn name(&self) -> &'static str {
        "matrix"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as usize, layer.h as f32);
        if w == 0 || h < 2.0 {
            return;
        }
        if self.columns.len() != w {
            self.columns.clear();
            for _ in 0..w {
                let drop = self.new_drop(h);
                self.columns.push(Some(drop));
            }
        }
        let treble = 0.5 + ctx.audio.treble + ctx.audio.hat * 0.9;
        for x in 0..w {
            let spawn_chance = 0.008 * self.params.density * (0.4 + ctx.audio.energy);
            if self.columns[x].is_none() && self.rng.chance(spawn_chance) {
                self.columns[x] = Some(self.new_drop(h));
            }
            let Some(drop) = self.columns[x].as_mut() else {
                continue;
            };
            drop.y += drop.speed * ctx.dt * treble * 34.0 * self.params.speed;
            // Mutación de glifos: el texto "vibra".
            if self.rng.chance(0.25) && !drop.glyphs.is_empty() {
                let i = self.rng.below(drop.glyphs.len());
                drop.glyphs[i] = self.rng.below(self.glyphs.len()) as u8;
            }
            let head = drop.y as i32;
            for i in 0..drop.length {
                let y = head - i as i32;
                if y < 0 || y >= layer.h as i32 {
                    continue;
                }
                let fade = 1.0 - i as f32 / drop.length as f32;
                let glyph = drop.glyphs[(drop.offset + i) % drop.glyphs.len()];
                let ch = self.glyphs[glyph as usize % self.glyphs.len()];
                let (color, alpha) = if i == 0 {
                    (ctx.palette.text, 1.0)
                } else if i < 3 {
                    (ctx.shade(0.95), 0.85)
                } else {
                    (ctx.shade(0.45 + fade * 0.25), fade * 0.55)
                };
                layer.put(x as i32, y, ch, color, alpha);
            }
            if drop.y > layer.h as f32 + drop.length as f32 {
                self.columns[x] = None;
            }
        }
    }
}

impl Matrix {
    fn new_drop(&mut self, _h: f32) -> Drop {
        let length = self.rng.below(14) + 4;
        let mut glyphs = Vec::with_capacity(length);
        for _ in 0..length {
            glyphs.push(self.rng.below(self.glyphs.len()) as u8);
        }
        Drop {
            y: self.rng.range(-(length as f32), 0.0),
            speed: self.rng.range(0.5, 1.8),
            length,
            glyphs,
            offset: self.rng.below(8),
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Starfield
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
    prev_x: f32,
    prev_y: f32,
}

pub struct Starfield {
    params: Params,
    stars: Vec<Star>,
    rng: Rng,
    warp: f32,
}

impl Default for Starfield {
    fn default() -> Self {
        Self {
            params: Params::default(),
            stars: Vec::with_capacity(400),
            rng: Rng::seeded(),
            warp: 0.0,
        }
    }
}

impl Effect for Starfield {
    fn key(&self) -> &'static str {
        "starfield"
    }
    fn name(&self) -> &'static str {
        "estrellas"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as f32, layer.h as f32);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let target = (60.0 + self.params.density * 260.0) as usize;
        while self.stars.len() < target {
            let z = self.rng.range(0.1, 1.0);
            self.stars.push(Star {
                x: self.rng.range(-1.0, 1.0),
                y: self.rng.range(-1.0, 1.0),
                z,
                prev_x: 0.0,
                prev_y: 0.0,
            });
        }
        let speed = (0.35 + ctx.audio.energy * 2.6 + ctx.audio.bass * 1.8 + ctx.audio.kick * 1.4)
            * self.params.speed
            * ctx.dt
            * 8.0;
        self.warp += speed;
        let cx = w * 0.5;
        let cy = h * 0.5;
        for star in &mut self.stars {
            star.z -= speed;
            if star.z <= 0.01 {
                star.z = 1.0;
                star.x = self.rng.range(-1.0, 1.0);
                star.y = self.rng.range(-1.0, 1.0);
                star.prev_x = 0.0;
                star.prev_y = 0.0;
            }
            let scale = 1.0 / star.z;
            let px = cx + star.x * scale * w * 0.35;
            let py = cy + star.y * scale * h * 0.9;
            let brightness = (1.0 - star.z).clamp(0.0, 1.0);
            let ch = if star.z < 0.2 {
                '●'
            } else if star.z < 0.45 {
                '•'
            } else if star.z < 0.75 {
                '·'
            } else {
                '.'
            };
            let color = ctx.shade(0.45 + brightness * 0.55);
            layer.put(px as i32, py as i32, ch, color, 0.4 + brightness * 0.6);
            // Estela: más larga cuanto más rápido va la estrella.
            if brightness > 0.25 {
                let tx = cx + star.x * (scale + speed * 4.0) * w * 0.35;
                let ty = cy + star.y * (scale + speed * 4.0) * h * 0.9;
                layer.line(
                    px as i32,
                    py as i32,
                    tx as i32,
                    ty as i32,
                    '·',
                    ctx.shade(0.38 + brightness * 0.4),
                    brightness * 0.45,
                );
            }
            star.prev_x = px;
            star.prev_y = py;
        }
        if ctx.audio.beat {
            // Onda expansiva desde el centro.
            let radius = (h * 0.5).min(w * 0.25) * (0.2 + ctx.audio.beat_pulse);
            let steps = 60;
            for step in 0..steps {
                let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
                let x = cx + angle.cos() * radius * 1.7;
                let y = cy + angle.sin() * radius;
                layer.put(x as i32, y as i32, '·', ctx.shade(1.0), 0.45);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Fractal
// ---------------------------------------------------------------------------

pub struct Fractal {
    params: Params,
    zoom: f32,
    center: (f32, f32),
    buffer: Vec<f32>,
    width: u16,
    height: u16,
}

impl Default for Fractal {
    fn default() -> Self {
        Self {
            params: Params::default(),
            zoom: 1.0,
            center: (-0.1, 0.0),
            buffer: Vec::new(),
            width: 0,
            height: 0,
        }
    }
}

impl Effect for Fractal {
    fn key(&self) -> &'static str {
        "fractal"
    }
    fn name(&self) -> &'static str {
        "fractal"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w, layer.h);
        if w < 4 || h < 2 {
            return;
        }
        // Se calcula a media resolución horizontal y se duplica: 4x más rápido
        // y en terminal no se nota (los caracteres ya son 1:2).
        let low_w = (w as usize + 1) / 2;
        let low_h = h as usize;
        if self.width != w || self.height != h {
            self.width = w;
            self.height = h;
            self.buffer = vec![0.0; low_w * low_h];
        }
        self.zoom = (self.zoom
            + ctx.dt * (0.05 + ctx.audio.bass * 0.9 * self.params.sensitivity) * self.params.speed)
            .clamp(0.2, 60.0);
        if ctx.audio.beat {
            self.zoom *= 1.06;
        }
        let angle = ctx.time * 0.15;
        let target_c = (
            -0.72 + 0.18 * angle.sin() - ctx.audio.bass * 0.12,
            0.26 * (angle * 1.3).cos() + ctx.audio.treble * 0.18,
        );
        self.center.0 += (target_c.0 - self.center.0) * 0.02;
        self.center.1 += (target_c.1 - self.center.1) * 0.02;

        let zoom = 1.0 / self.zoom;
        let aspect = w as f32 / (h as f32 * 2.0);
        let max_iter = 26u32;
        for y in 0..low_h {
            let ny = (y as f32 / (low_h.max(2) - 1) as f32 - 0.5) * 1.6 * zoom;
            for x in 0..low_w {
                let nx = (x as f32 / (low_w.max(2) - 1) as f32 - 0.5) * 1.6 * zoom * aspect;
                let (mut zx, mut zy) = (nx, ny);
                let mut iter = 0;
                while zx * zx + zy * zy < 4.0 && iter < max_iter {
                    let tmp = zx * zx - zy * zy + self.center.0;
                    zy = 2.0 * zx * zy + self.center.1;
                    zx = tmp;
                    iter += 1;
                }
                let value = iter as f32 / max_iter as f32;
                self.buffer[y * low_w + x] = value;
            }
        }
        for y in 0..low_h {
            for x in 0..low_w {
                let value = self.buffer[y * low_w + x];
                if value <= 0.02 {
                    continue;
                }
                let index = ((value * 4.7) as usize).min(4);
                let color = ctx.shade(0.35 + value * 0.65);
                let ch = SHADES[index];
                layer.put((x * 2) as i32, y as i32, ch, color, 0.95);
                layer.put((x * 2 + 1) as i32, y as i32, ch, color, 0.95);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 9. Neural
// ---------------------------------------------------------------------------

pub struct Neural {
    params: Params,
    nodes: Vec<(f32, f32, f32)>,
    width: u16,
    height: u16,
    pulse: f32,
}

impl Default for Neural {
    fn default() -> Self {
        Self {
            params: Params::default(),
            nodes: Vec::new(),
            width: 0,
            height: 0,
            pulse: 0.0,
        }
    }
}

impl Effect for Neural {
    fn key(&self) -> &'static str {
        "neural"
    }
    fn name(&self) -> &'static str {
        "neural"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w, layer.h);
        if w < 6 || h < 3 {
            return;
        }
        if self.width != w || self.height != h {
            self.width = w;
            self.height = h;
            self.nodes.clear();
            let cols = 5;
            let rows = 4;
            for row in 0..rows {
                for col in 0..cols {
                    let x = (col as f32 + 0.5) / cols as f32 * w as f32;
                    let y = (row as f32 + 0.5) / rows as f32 * h as f32;
                    let node = ((row * cols + col) as f32 / (rows * cols) as f32) * BANDS as f32;
                    self.nodes.push((x, y, node));
                }
            }
        }
        self.pulse =
            (self.pulse * 0.92).max(ctx.audio.beat_pulse * 0.7 + ctx.audio.snare_pulse * 0.6);
        let time = ctx.time;
        let mut positions: Vec<(i32, i32, f32)> = Vec::with_capacity(self.nodes.len());
        for (x, y, band) in &self.nodes {
            let wobble_x = (time * 0.7 + band * 0.1).sin() * 1.4;
            let wobble_y = (time * 0.9 + band * 0.13).cos() * 0.7;
            let value = ctx.audio.band(*band) * self.params.sensitivity;
            positions.push(((x + wobble_x) as i32, (y + wobble_y) as i32, value));
        }
        // Aristas entre nodos cercanos.
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                let (x1, y1, v1) = positions[i];
                let (x2, y2, v2) = positions[j];
                let distance = (((x1 - x2).pow(2) + (y1 - y2).pow(2)) as f32).sqrt();
                if distance > (w.min(h * 2) as f32 * 0.22) {
                    continue;
                }
                let strength = ((v1 + v2) * 0.5).clamp(0.0, 1.0);
                let alpha = 0.08 + strength * 0.5 + self.pulse * 0.15;
                layer.line(
                    x1,
                    y1,
                    x2,
                    y2,
                    '·',
                    ctx.shade(0.32 + strength * 0.35),
                    alpha,
                );
            }
        }
        for (x, y, value) in positions {
            let ch = if value > 0.6 {
                '●'
            } else if value > 0.25 {
                '•'
            } else {
                '○'
            };
            let color = ctx.shade((0.55 + value * 0.45).min(1.0));
            layer.put(x, y, ch, color, 0.55 + value * 0.45);
        }
    }
}

// ---------------------------------------------------------------------------
// 10. Pulsar
// ---------------------------------------------------------------------------

pub struct Pulsar {
    params: Params,
    rotation: f32,
    rings: Vec<f32>,
}

impl Default for Pulsar {
    fn default() -> Self {
        Self {
            params: Params::default(),
            rotation: 0.0,
            rings: Vec::new(),
        }
    }
}

impl Effect for Pulsar {
    fn key(&self) -> &'static str {
        "pulsar"
    }
    fn name(&self) -> &'static str {
        "púlsar"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as f32, layer.h as f32);
        if w < 4.0 || h < 2.0 {
            return;
        }
        let (cx, cy) = (w / 2.0, h / 2.0);
        self.rotation += ctx.dt * (0.6 + ctx.audio.mid * 1.8) * self.params.speed;
        let beams = 48;
        for beam in 0..beams {
            let angle = beam as f32 / beams as f32 * std::f32::consts::TAU + self.rotation;
            let band = (beam as f32 / beams as f32 * 32.0) as usize;
            let value = ctx.audio.bands[band % BANDS] * self.params.sensitivity;
            let length = (h * 0.45) * (0.15 + value * 1.3);
            let x = cx + angle.cos() * length * 1.9;
            let y = cy + angle.sin() * length;
            layer.line(
                cx as i32,
                cy as i32,
                x as i32,
                y as i32,
                if value > 0.5 { '━' } else { '·' },
                ctx.shade(0.4 + value * 0.55),
                0.15 + value * 0.7,
            );
        }
        if ctx.audio.beat {
            self.rings.push(0.0);
            if self.rings.len() > 6 {
                self.rings.remove(0);
            }
        }
        for ring in &mut self.rings {
            *ring += ctx.dt * (h * 0.6 + ctx.audio.energy * h);
            let radius = *ring;
            let steps = (radius * 4.0) as i32 + 24;
            let alpha = (1.0 - radius / (h * 1.1)).clamp(0.0, 0.6);
            if alpha <= 0.0 {
                continue;
            }
            for step in 0..steps {
                let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
                let x = cx + angle.cos() * radius * 1.9;
                let y = cy + angle.sin() * radius;
                layer.put(x as i32, y as i32, '·', ctx.shade(0.85), alpha);
            }
        }
        self.rings.retain(|ring| *ring < h * 1.2);
        let core = if ctx.audio.beat { '●' } else { '◉' };
        layer.put(cx as i32, cy as i32, core, ctx.shade(1.0), 0.8);
    }
}

// ---------------------------------------------------------------------------
// 11. Túnel
// ---------------------------------------------------------------------------

pub struct Tunnel {
    params: Params,
    phase: f32,
}

impl Default for Tunnel {
    fn default() -> Self {
        Self {
            params: Params::default(),
            phase: 0.0,
        }
    }
}

impl Effect for Tunnel {
    fn key(&self) -> &'static str {
        "tunnel"
    }
    fn name(&self) -> &'static str {
        "túnel"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as f32, layer.h as f32);
        if w < 4.0 || h < 2.0 {
            return;
        }
        self.phase += ctx.dt * (0.35 + ctx.audio.bass * 1.4) * self.params.speed;
        let (cx, cy) = (w / 2.0, h / 2.0);
        let rings = 14;
        for ring in 0..rings {
            let t = (ring as f32 / rings as f32 + self.phase).fract();
            // Perspectiva: los anillos crecen y se acercan.
            let radius = (t * t).powf(1.6) * h * 0.55 + ctx.audio.bass * h * 0.12;
            let alpha = (1.0 - t).powf(1.4) * 0.8;
            let value = ctx.audio.bands[(ring * 4) % BANDS];
            let steps = (radius * 5.0) as i32 + 28;
            for step in 0..steps {
                let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
                let wobble = 1.0 + (angle * 4.0 + ctx.time * 2.0).sin() * 0.04;
                let x = cx + angle.cos() * radius * 1.9 * wobble;
                let y = cy + angle.sin() * radius * wobble;
                let ch = if t < 0.25 {
                    '█'
                } else if t < 0.5 {
                    '▒'
                } else if t < 0.75 {
                    '░'
                } else {
                    '·'
                };
                layer.put(
                    x as i32,
                    y as i32,
                    ch,
                    ctx.shade((0.4 + (1.0 - t) * 0.55 + value * 0.1).min(1.0)),
                    alpha,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 12. Espectro 3D
// ---------------------------------------------------------------------------

pub struct Spectro3d {
    params: Params,
    history: Vec<Vec<f32>>,
}

impl Default for Spectro3d {
    fn default() -> Self {
        Self {
            params: Params::default(),
            history: Vec::new(),
        }
    }
}

impl Effect for Spectro3d {
    fn key(&self) -> &'static str {
        "spectro3d"
    }
    fn name(&self) -> &'static str {
        "espectro 3D"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w, layer.h);
        if w < 8 || h < 3 {
            return;
        }
        let bands = 32usize;
        let mut snapshot = vec![0.0f32; bands];
        for (i, value) in snapshot.iter_mut().enumerate() {
            *value = ctx.audio.bands[i * (BANDS / bands)] * self.params.sensitivity;
        }
        self.history.push(snapshot);
        if self.history.len() > 18 {
            self.history.remove(0);
        }
        let cx = w as f32 / 2.0;
        let horizon = h as f32 * 0.62;
        // Rejilla de suelo.
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            let y = (horizon + (h as f32 - horizon) * t * t).round() as i32;
            if y >= 0 && y < layer.h as i32 {
                for x in 0..w as i32 {
                    if layer.ch_at(x, y) == ' ' {
                        layer.put(x, y, '─', ctx.shade(0.22), 0.35);
                    }
                }
            }
        }
        for (depth, frame_values) in self.history.iter().enumerate().rev() {
            let scale = 1.0 + depth as f32 * 0.16;
            let baseline = horizon + depth as f32 * (h as f32 - horizon) * 0.10;
            let alpha = (1.0 - depth as f32 / self.history.len() as f32).max(0.08);
            for (i, value) in frame_values.iter().enumerate() {
                let x = cx + (i as f32 / bands as f32 - 0.5) * w as f32 * 0.72;
                let height = value * h as f32 * 0.5;
                for step in 0..(height.max(0.0) as i32) {
                    let y = (baseline - step as f32 * scale).round() as i32;
                    let ch = if step == 0 { '▔' } else { '█' };
                    layer.put(
                        x as i32,
                        y,
                        ch,
                        ctx.shade((0.45 + value * 0.5).min(1.0)),
                        alpha * 0.8,
                    );
                }
            }
        }
        let _ = cx;
    }
}

// ---------------------------------------------------------------------------
// 13. Osciloscopio CRT
// ---------------------------------------------------------------------------

/// Puntos a los que se remuestrea la forma de onda (256 → 160).
const SCOPE_POINTS: usize = 160;
/// Barridos conservados: el fósforo del CRT se apaga en ~4 pasadas.
const SCOPE_TRAILS: usize = 4;

pub struct Scope {
    params: Params,
    /// Barridos recientes, del más viejo al más nuevo.
    sweeps: VecDeque<Vec<f32>>,
    /// Buffer reutilizado para el remuestreo del frame actual.
    resampled: Vec<f32>,
    /// Máscaras de subpíxeles del haz (una por celda), reutilizadas entre
    /// barridos y frames. Evita el búfer completo de brillos del braille.
    beam_masks: Vec<u8>,
    /// Reloj del analizador del último barrido (identifica la generación de audio).
    last_generation: f32,
    /// `ctx.time` del último barrido, para el refresco mínimo de ~50 ms.
    last_sweep: f32,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            params: Params::default(),
            sweeps: VecDeque::with_capacity(SCOPE_TRAILS),
            resampled: vec![0.0; SCOPE_POINTS],
            beam_masks: Vec::new(),
            last_generation: f32::NEG_INFINITY,
            last_sweep: f32::NEG_INFINITY,
        }
    }
}

/// Muestra la forma de onda en una posición fraccional, con interpolación.
#[inline]
fn waveform_at(audio: &AudioFrame, pos: f32) -> f32 {
    let pos = pos.clamp(0.0, (WAVE - 1) as f32);
    let a = pos as usize;
    let b = (a + 1).min(WAVE - 1);
    let frac = pos - a as f32;
    audio.waveform[a] * (1.0 - frac) + audio.waveform[b] * frac
}

/// Resolución del haz del osciloscopio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BeamMode {
    /// 2×4 subpíxeles por celda: terminales con altura suficiente.
    Braille,
    /// 1×2 subpíxeles con medios bloques: terminales bajas, haz más grueso.
    Half,
}

impl BeamMode {
    /// Con menos de 8 filas los puntos Braille se leen como ruido; ahí manda
    /// la media celda.
    fn for_height(h: u16) -> Self {
        if h >= 8 {
            Self::Braille
        } else {
            Self::Half
        }
    }

    fn cols(self) -> i32 {
        match self {
            Self::Braille => 2,
            Self::Half => 1,
        }
    }

    fn rows(self) -> i32 {
        match self {
            Self::Braille => 4,
            Self::Half => 2,
        }
    }

    #[inline]
    fn bit(self, sx: i32, sy: i32) -> u8 {
        match self {
            Self::Braille => BRAILLE_BITS[(sx & 1) as usize][(sy & 3) as usize],
            Self::Half => {
                if sy & 1 == 0 {
                    0x01
                } else {
                    0x02
                }
            }
        }
    }

    fn glyph(self, mask: u8) -> char {
        match self {
            Self::Braille => BRAILLE_LUT[mask as usize],
            Self::Half => match mask {
                0x01 => '▀',
                0x02 => '▄',
                0x03 => '█',
                _ => ' ',
            },
        }
    }
}

/// Rasteriza un barrido sobre la rejilla de subpíxeles de `masks` (una máscara
/// por celda). Cada subcolumna recibe al menos un punto y los tramos
/// verticales se rellenan, así el trazo queda continuo.
fn rasterize_beam(
    masks: &mut [u8],
    mode: BeamMode,
    w: i32,
    h: i32,
    sweep: &[f32],
    amplitude: f32,
    thickness: i32,
) {
    if w <= 0 || h <= 0 || sweep.len() < 2 {
        return;
    }
    let cols = w * mode.cols();
    let rows = h * mode.rows();
    let mid = rows as f32 / 2.0;
    let sample = |sx: i32| -> f32 {
        let t = if cols > 1 {
            sx as f32 / (cols - 1) as f32
        } else {
            0.0
        };
        let pos = t * (sweep.len() - 1) as f32;
        let a = pos as usize;
        let b = (a + 1).min(sweep.len() - 1);
        let frac = pos - a as f32;
        sweep[a] * (1.0 - frac) + sweep[b] * frac
    };
    let y_of = |value: f32| -> i32 {
        (mid - value.clamp(-1.0, 1.0) * amplitude)
            .round()
            .clamp(0.0, (rows - 1) as f32) as i32
    };
    let cell_rows = mode.rows();
    let cell_cols = mode.cols();
    let mut prev = y_of(sample(0));
    for sx in 0..cols {
        let y = y_of(sample(sx));
        let (from, to) = if prev <= y { (prev, y) } else { (y, prev) };
        let end = (to + thickness - 1).min(rows - 1);
        let cell = sx / cell_cols;
        for sy in from..=end {
            let index = (sy / cell_rows) * w + cell;
            if let Some(slot) = masks.get_mut(index as usize) {
                *slot |= mode.bit(sx, sy);
            }
        }
        prev = y;
    }
}

/// Vuelca las máscaras del haz a la capa: un glifo y un color por celda.
fn blit_beam(
    layer: &mut Layer,
    masks: &[u8],
    mode: BeamMode,
    w: i32,
    h: i32,
    color: Rgb,
    alpha: f32,
) {
    for cy in 0..h {
        for cx in 0..w {
            let mask = masks[(cy * w + cx) as usize];
            if mask == 0 {
                continue;
            }
            layer.put(cx, cy, mode.glyph(mask), color, alpha);
        }
    }
}

impl Scope {
    /// Remuestrea las 256 muestras del analizador al ancho interno del efecto,
    /// con un suavizado de 3 puntos para que el trazo no salga a saltos.
    fn resample(&mut self, audio: &AudioFrame) {
        let last = (WAVE - 1) as f32;
        let step = last / (SCOPE_POINTS - 1) as f32;
        for (i, out) in self.resampled.iter_mut().enumerate() {
            let pos = i as f32 * step;
            let smooth = waveform_at(audio, pos - step) * 0.25
                + waveform_at(audio, pos) * 0.5
                + waveform_at(audio, pos + step) * 0.25;
            *out = smooth.clamp(-1.0, 1.0);
        }
    }

    /// Añade el barrido actual reutilizando el buffer más viejo: sin allocs
    /// por frame una vez que la cola está llena.
    fn push_sweep(&mut self) {
        if self.sweeps.len() >= SCOPE_TRAILS {
            if let Some(mut oldest) = self.sweeps.pop_front() {
                oldest.clear();
                oldest.extend_from_slice(&self.resampled);
                self.sweeps.push_back(oldest);
                return;
            }
        }
        let mut sweep = Vec::with_capacity(SCOPE_POINTS);
        sweep.extend_from_slice(&self.resampled);
        self.sweeps.push_back(sweep);
    }

    /// Rejilla tenue de osciloscopio: verticales cada w/10 y scanlines cada
    /// h/8, con un mínimo de 2–3 filas para no tapar el haz en pantallas bajas.
    fn draw_grid(&self, layer: &mut Layer, ctx: &VizCtx) {
        if self.params.density < 0.3 {
            return;
        }
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let color = ctx.shade(0.18);
        let step_x = (w / 10).max(2) as usize;
        // Scanlines cada 2–3 filas como mínimo: en pantallas bajas una línea
        // por fila tapaba el haz y se leía como ruido.
        let step_y = if h < 12 { 3 } else { (h / 8).max(2) } as usize;
        for x in (step_x as i32..w).step_by(step_x) {
            for y in 0..h {
                layer.put(x, y, '·', color, 0.16);
            }
        }
        for y in (step_y as i32..h).step_by(step_y) {
            for x in 0..w {
                layer.put(x, y, '·', color, 0.16);
            }
        }
    }

    /// Dibuja la línea base del haz. En silencio se queda más marcada.
    fn draw_baseline(&self, layer: &mut Layer, ctx: &VizCtx, y: i32, silent: bool) {
        let color = if silent {
            ctx.shade(0.45)
        } else {
            ctx.shade(0.28)
        };
        let alpha = if silent { 0.55 } else { 0.25 };
        for x in 0..layer.w as i32 {
            layer.put(x, y, '·', color, alpha);
        }
    }
}

impl Effect for Scope {
    fn key(&self) -> &'static str {
        "scope"
    }
    fn name(&self) -> &'static str {
        "osciloscopio"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        self.draw_grid(layer, ctx);
        let baseline = h / 2;

        let peak = ctx
            .audio
            .waveform
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
        let silent = ctx.audio.silent || peak < 1e-4;
        if silent {
            // Sin señal no se capturan barridos: solo la línea base quieta.
            self.draw_baseline(layer, ctx, baseline, true);
            return;
        }
        self.draw_baseline(layer, ctx, baseline, false);

        // Barrido nuevo al cambiar la generación del audio o, como mínimo, cada
        // ~50 ms. `params.speed` acelera o frena ese refresco.
        let generation_changed = (ctx.audio.time - self.last_generation).abs() > 1e-4;
        let interval = 0.05 / self.params.speed.clamp(0.25, 4.0);
        if generation_changed || ctx.time - self.last_sweep >= interval {
            self.resample(ctx.audio);
            self.push_sweep();
            self.last_generation = ctx.audio.time;
            self.last_sweep = ctx.time;
        }

        // `density` decide cuántas estelas se muestran (3 o 4) y si hay rejilla.
        // En pantallas bajas se recortan para no embarrar el haz.
        let mut visible = if self.params.density >= 0.75 { 4 } else { 3 };
        if h < 6 {
            visible = 2;
        }
        let total = self.sweeps.len();
        let skip = total.saturating_sub(visible);

        // El haz se dibuja en subpíxeles: braille 2×4 cuando hay altura y
        // media celda (1×2) en terminales bajas. Cuanto menor la resolución,
        // más grueso el trazo para que siga leyéndose.
        let mode = BeamMode::for_height(layer.h);
        let masks_len = w as usize * h as usize;
        if self.beam_masks.len() != masks_len {
            self.beam_masks.clear();
            self.beam_masks.resize(masks_len, 0);
        }
        let rows = (h * mode.rows()) as f32;
        let amplitude = (rows * 0.42 * self.params.sensitivity.clamp(0.1, 4.0)).max(0.5);
        let thickness = match mode {
            BeamMode::Half => 1,
            BeamMode::Braille if h >= 16 => 1,
            BeamMode::Braille => 2,
        };

        for (index, sweep) in self.sweeps.iter().enumerate().skip(skip) {
            let newest = index + 1 == total;
            let recency = if total > 1 {
                index as f32 / (total - 1) as f32
            } else {
                1.0
            };
            let color = if newest {
                // El haz actual tira a la tinta de la portada.
                mix(ctx.palette.secondary, ctx.palette.text, 0.4)
            } else {
                ctx.shade(0.22 + 0.62 * recency)
            };
            let alpha = if newest { 0.92 } else { 0.14 + 0.5 * recency };
            self.beam_masks.fill(0);
            rasterize_beam(
                &mut self.beam_masks,
                mode,
                w,
                h,
                sweep,
                amplitude,
                thickness,
            );
            blit_beam(layer, &self.beam_masks, mode, w, h, color, alpha);
        }
    }
}

// ---------------------------------------------------------------------------
// 14. LCD segmentado
// ---------------------------------------------------------------------------

/// Tiempo que un pico se queda clavado antes de empezar a caer.
const PEAK_HOLD: f32 = 0.30;

/// Avanza un pico con hold y caída. `target` y `peak` van en celdas.
fn update_peak(peak: &mut f32, hold: &mut f32, target: f32, dt: f32, speed: f32) {
    if target >= *peak {
        *peak = target;
        *hold = PEAK_HOLD;
    } else if *hold > 0.0 {
        *hold = (*hold - dt).max(0.0);
    } else {
        let fall = (2.2 + *peak * 2.6) * dt * speed.clamp(0.25, 4.0);
        *peak = (*peak - fall).max(target);
    }
}

pub struct Lcd {
    params: Params,
    peaks: Vec<f32>,
    holds: Vec<f32>,
}

impl Default for Lcd {
    fn default() -> Self {
        Self {
            params: Params::default(),
            peaks: Vec::new(),
            holds: Vec::new(),
        }
    }
}

impl Effect for Lcd {
    fn key(&self) -> &'static str {
        "lcd"
    }
    fn name(&self) -> &'static str {
        "lcd segmentado"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        // La tira de osciloscopio solo cabe si el ecualizador conserva altura.
        let scope_h = if h >= 16 {
            4
        } else if h >= 12 {
            3
        } else if h >= 8 {
            2
        } else {
            0
        };
        let bars_h = (h - scope_h - i32::from(scope_h > 0)).max(1);
        let count = ((w / 3).clamp(4, 24)) as usize;
        if self.peaks.len() < count {
            self.peaks.resize(count, 0.0);
            self.holds.resize(count, 0.0);
        }
        let slot = (w / count as i32).max(1);
        let bar_w = (slot - 1).max(1);
        for i in 0..count {
            let t = i as f32 / (count - 1).max(1) as f32;
            let band = t.powf(1.15) * (BANDS - 1) as f32;
            let value =
                (ctx.audio.band(band) * self.params.sensitivity * (1.0 + ctx.audio.kick * 0.25))
                    .clamp(0.0, 1.0);
            let height = value * bars_h as f32;
            update_peak(
                &mut self.peaks[i],
                &mut self.holds[i],
                height,
                ctx.dt,
                self.params.speed,
            );
            let peak = self.peaks[i];
            // Segmentos de dos filas: las impares quedan apagadas (scanlines).
            let segments = ((bars_h + 1) / 2).max(1);
            let lit = ((value * segments as f32).round() as i32 * 2).min(bars_h);
            let x0 = (i as i32 * slot).max(0);
            let x1 = (x0 + bar_w).min(w);
            for row in (0..lit).step_by(2) {
                let y = bars_h - 1 - row;
                let color = ctx.shade(0.45 + 0.50 * row as f32 / bars_h as f32);
                for x in x0..x1 {
                    layer.put(x, y, '█', color, 0.95);
                }
            }
            if peak > height + 0.6 && peak >= 1.0 {
                let row = peak.floor() as i32;
                let y = (bars_h - 1 - row).clamp(0, bars_h - 1);
                let color = ctx.shade(0.98);
                for x in x0..x1 {
                    layer.put(x, y, '▔', color, 0.9);
                }
            }
        }
        if scope_h > 0 {
            let sep_y = bars_h;
            let sep = ctx.shade(0.28);
            for x in 0..w {
                layer.put(x, sep_y, '─', sep, 0.35);
            }
            let top = bars_h + 1;
            let base = top as f32 + (scope_h as f32 - 1.0) * 0.5;
            let amp = ((scope_h as f32 - 1.0) * 0.5).max(0.5);
            let color = ctx.shade(0.82);
            for x in 0..w {
                let t = if w > 1 {
                    x as f32 / (w - 1) as f32
                } else {
                    0.0
                };
                let y = (base - ctx.audio.waveform_at(t) * amp).round() as i32;
                layer.put(x, y.clamp(top, h - 1), '•', color, 0.9);
            }
            let base_y = (base.round() as i32).clamp(top, h - 1);
            let dim = ctx.shade(0.35);
            for x in (0..w).step_by(2) {
                if layer.ch_at(x, base_y) == ' ' {
                    layer.put(x, base_y, '·', dim, 0.25);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 15. Escalera LED (rack)
// ---------------------------------------------------------------------------

const ISO_LABELS: [&str; 10] = [
    "31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];

pub struct Led {
    params: Params,
    peaks: Vec<f32>,
    holds: Vec<f32>,
}

impl Default for Led {
    fn default() -> Self {
        Self {
            params: Params::default(),
            peaks: Vec::new(),
            holds: Vec::new(),
        }
    }
}

/// Etiquetas ISO centradas bajo cada banda, solo si el ancho da para ello.
fn draw_iso_labels(layer: &mut Layer, ctx: &VizCtx, w: i32, h: i32, count: usize, slot: i32) {
    if w <= 0 || h <= 0 || count == 0 {
        return;
    }
    let color = ctx.shade(0.38);
    let y = h - 1;
    let last = (count - 1).max(1) as f32;
    for (index, label) in ISO_LABELS.iter().enumerate() {
        let pos = index as f32 / (ISO_LABELS.len() - 1) as f32 * last;
        let band = pos.round() as i32;
        let x0 = band * slot + slot / 2 - (label.len() as i32) / 2;
        for (offset, ch) in label.chars().enumerate() {
            let x = x0 + offset as i32;
            if x >= 0 && x < w {
                layer.put(x, y, ch, color, 0.65);
            }
        }
    }
}

impl Effect for Led {
    fn key(&self) -> &'static str {
        "led"
    }
    fn name(&self) -> &'static str {
        "escalera led"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        // Las etiquetas de frecuencia solo entran con ancho y alto de sobra.
        let labels = w >= 72 && h >= 14;
        let usable = (h - i32::from(labels)).max(1);
        let count = ((w / 4).clamp(10, 16)) as usize;
        if self.peaks.len() < count {
            self.peaks.resize(count, 0.0);
            self.holds.resize(count, 0.0);
        }
        let slot = (w / count as i32).max(1);
        let seg_w = slot.min(3).max(1);
        for i in 0..count {
            let t = i as f32 / (count - 1).max(1) as f32;
            let band = t * (BANDS - 1) as f32;
            let value =
                (ctx.audio.band(band) * self.params.sensitivity * (1.0 + ctx.audio.kick * 0.22))
                    .clamp(0.0, 1.0);
            let height = value * usable as f32;
            update_peak(
                &mut self.peaks[i],
                &mut self.holds[i],
                height,
                ctx.dt,
                self.params.speed,
            );
            let peak = self.peaks[i];
            let lit = (height.floor() as i32).min(usable);
            let x0 = i as i32 * slot + (slot - seg_w) / 2;
            let x1 = (x0 + seg_w).min(w);
            for row in 0..lit {
                // Tres zonas de la rampa: cuerpo, medio y techo del LED.
                let zone = row as f32 / usable as f32;
                let color = if zone < 0.34 {
                    ctx.shade(0.45)
                } else if zone < 0.67 {
                    ctx.shade(0.72)
                } else {
                    ctx.shade(0.95)
                };
                let y = usable - 1 - row;
                for x in x0..x1 {
                    layer.put(x, y, '█', color, 0.95);
                }
            }
            if peak > height + 0.6 && peak >= 1.0 {
                let y = (usable - 1 - peak.floor() as i32).clamp(0, usable - 1);
                let color = ctx.shade(0.95);
                for x in x0..x1 {
                    layer.put(x, y, '▔', color, 0.9);
                }
            }
        }
        if labels {
            draw_iso_labels(layer, ctx, w, h, count, slot);
        }
    }
}

// ---------------------------------------------------------------------------
// 16. Espectro Braille
// ---------------------------------------------------------------------------

pub struct BarsBraille {
    params: Params,
    /// Primera fila de subpíxel encendida de cada subcolumna (2W), calculada
    /// una vez por frame.
    tops: Vec<i32>,
    /// Color por fila de subpíxel (4H), también una vez por frame.
    row_colors: Vec<Rgb>,
}

impl Default for BarsBraille {
    fn default() -> Self {
        Self {
            params: Params::default(),
            tops: Vec::new(),
            row_colors: Vec::new(),
        }
    }
}

impl Effect for BarsBraille {
    fn key(&self) -> &'static str {
        "bars_braille"
    }
    fn name(&self) -> &'static str {
        "espectro braille"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let dw = w * 2;
        let dh = h * 4;
        // Altura de cada subcolumna una sola vez por frame. La curva se
        // muestrea en una tabla; antes era un `powf` por subcolumna.
        self.tops.resize(dw as usize, 0);
        let curve = spectrum_curve();
        let sensitivity = self.params.sensitivity;
        let boost = 1.0 + ctx.audio.kick * 0.18;
        for (x, top) in self.tops.iter_mut().enumerate() {
            let t = if dw > 1 {
                x * (POW_STEPS - 1) / (dw as usize - 1)
            } else {
                0
            };
            let band = curve[t] * (BANDS - 1) as f32;
            let value = (ctx.audio.band(band) * sensitivity * boost).clamp(0.0, 1.0);
            *top = dh - (value * dh as f32).round() as i32;
        }
        // Mismo degradado que `Dots::render`: color según la fila más alta
        // encendida de la celda, sobre un relleno sólido (brillo 1 → alfa 1).
        self.row_colors.resize(dh as usize, (0, 0, 0));
        for (y, color) in self.row_colors.iter_mut().enumerate() {
            let height = 1.0 - y as f32 / dh as f32;
            *color = ctx.shade(0.28 + 0.64 * height);
        }
        // Una máscara Braille por celda desde las dos columnas de su rejilla
        // 2×4. El relleno va de `top` al borde inferior, así que la máscara se
        // arma con dos entradas de tabla y un solo `put` por celda.
        for cy in 0..h {
            for cx in 0..w {
                let left = (self.tops[(cx * 2) as usize] - cy * 4).clamp(0, 4);
                let right = (self.tops[(cx * 2 + 1) as usize] - cy * 4).clamp(0, 4);
                let mask = BRAILLE_COL[0][left as usize] | BRAILLE_COL[1][right as usize];
                if mask == 0 {
                    continue;
                }
                let top = left.min(right);
                let color = self.row_colors[(cy * 4 + top) as usize];
                layer.put(cx, cy, BRAILLE_LUT[mask as usize], color, 1.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 17. Escenario en perspectiva
// ---------------------------------------------------------------------------

pub struct Floor {
    params: Params,
}

impl Default for Floor {
    fn default() -> Self {
        Self {
            params: Params::default(),
        }
    }
}

/// Rango horizontal del cuadrilátero del escenario en una fila. El trapecio
/// se estrecha hacia atrás; `None` fuera de la franja del suelo.
fn stage_span(w: i32, h: i32, y: i32) -> Option<(i32, i32)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let near_y = h as f32 - 1.0;
    let far_y = (h as f32 * 0.22).floor();
    if (y as f32) < far_y || (y as f32) > near_y + 0.5 {
        return None;
    }
    let span = (near_y - far_y).max(1.0);
    let s = ((y as f32 - far_y) / span).clamp(0.0, 1.0);
    let cx = w as f32 * 0.5;
    let half = w as f32 * (0.18 + (0.46 - 0.18) * s);
    let left = (cx - half).round() as i32;
    let right = (cx + half).round() as i32;
    Some((left.clamp(0, w - 1), right.clamp(0, w - 1)))
}

impl Effect for Floor {
    fn key(&self) -> &'static str {
        "floor"
    }
    fn name(&self) -> &'static str {
        "escenario"
    }
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }

    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx) {
        let (w, h) = (layer.w as i32, layer.h as i32);
        if w < 4 || h < 3 {
            return;
        }
        let strips = 12usize;
        let near_y = h as f32 - 1.0;
        let far_y = (h as f32 * 0.22).floor();
        // Bordes del cuadrilátero: el suelo se estrecha hacia el fondo.
        let edge = ctx.shade(0.22);
        for y in far_y as i32..=near_y as i32 {
            if let Some((left, right)) = stage_span(w, h, y) {
                layer.put(left, y, '│', edge, 0.30);
                layer.put(right, y, '│', edge, 0.30);
            }
        }
        // De atrás hacia delante: las tiras cercanas se pintan encima.
        for strip in 0..strips {
            let t = strip as f32 / (strips - 1) as f32;
            let p = t.powf(1.55);
            let y = (far_y + (near_y - far_y) * p).round() as i32;
            let Some((left, right)) = stage_span(w, h, y) else {
                continue;
            };
            // Delante (t=1) lleva los graves; el fondo, los agudos.
            let value = (ctx.audio.band((1.0 - t) * (BANDS - 1) as f32) * self.params.sensitivity)
                .clamp(0.0, 1.0);
            let depth = 0.35 + 0.65 * t;
            let color = ctx.shade((0.28 + value * 0.68).min(1.0));
            let alpha = (0.14 + value * 0.78) * (0.55 + 0.45 * depth);
            let ch = if value > 0.62 { '═' } else { '─' };
            for x in left..=right {
                layer.put(x, y, ch, color, alpha);
            }
        }
    }
}

/// Utilidad para tests: un frame con energía.
pub fn test_audio(energy: f32) -> AudioFrame {
    let mut audio = AudioFrame::default();
    for (i, band) in audio.bands.iter_mut().enumerate() {
        *band = ((i as f32 * 0.31).sin().abs()) * energy;
    }
    audio.energy = energy;
    audio.beat = true;
    audio.beat_pulse = 1.0;
    audio.silent = false;
    for (i, sample) in audio.waveform.iter_mut().enumerate() {
        *sample = (i as f32 * 0.21).sin() * energy;
    }
    audio
}

#[cfg(test)]
mod tests {
    use super::*;
    use spote_core::artwork::Palette;

    fn render_all(preset: &str, w: u16, h: u16) -> usize {
        let palette = Palette::dark();
        let audio = test_audio(0.7);
        let mut viz = super::super::Viz::from_preset(preset);
        viz.render(w, h, &audio, &palette, 1.0 / 60.0);
        viz.output.cells.iter().filter(|c| c.ch != ' ').count()
    }

    #[test]
    fn every_effect_draws_something() {
        for kind in EffectKind::ALL {
            let mut effect = effect_by_key(kind.key());
            let mut layer = Layer::new(80, 12);
            let palette = Palette::dark();
            let audio = test_audio(0.8);
            let ctx = VizCtx {
                audio: &audio,
                palette: &palette,
                lut: super::super::shade_lut(&palette),
                w: 80,
                h: 12,
                dt: 1.0 / 60.0,
                time: 1.0,
            };
            for _ in 0..16 {
                layer.clear();
                effect.frame(&mut layer, &ctx);
            }
            let drawn = layer.cells.iter().filter(|c| c.ch != ' ').count();
            assert!(drawn > 3, "{} no dibuja (drawn={drawn})", kind.key());
        }
    }

    #[test]
    fn effects_survive_tiny_and_empty_geometry() {
        let palette = Palette::dark();
        let audio = test_audio(0.9);
        for kind in EffectKind::ALL {
            let mut effect = effect_by_key(kind.key());
            for (w, h) in [(1, 1), (2, 1), (3, 2), (20, 1)] {
                let mut layer = Layer::new(w, h);
                let ctx = VizCtx {
                    audio: &audio,
                    palette: &palette,
                    lut: super::super::shade_lut(&palette),
                    w,
                    h,
                    dt: 0.016,
                    time: 2.0,
                };
                effect.frame(&mut layer, &ctx);
            }
        }
    }

    #[test]
    fn every_preset_composes() {
        for preset in super::super::PRESETS {
            assert!(
                render_all(preset.name, 100, 10) > 20,
                "preset {} vacío",
                preset.name
            );
        }
    }

    #[test]
    fn scope_key_maps_to_scope_kind() {
        let effect = effect_by_key("scope");
        let kind = EffectKind::ALL
            .into_iter()
            .find(|kind| kind.key() == effect.key());
        assert_eq!(kind, Some(EffectKind::Scope));
    }

    #[test]
    fn scope_draws_with_waveform() {
        let palette = Palette::dark();
        let audio = test_audio(0.8);
        let mut scope = Scope::default();
        let (w, h) = (80u16, 12u16);
        let mut layer = Layer::new(w, h);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: crate::viz::shade_lut(&palette),
            w,
            h,
            dt: 1.0 / 60.0,
            time: 1.0,
        };
        scope.frame(&mut layer, &ctx);
        let drawn = layer.cells.iter().filter(|cell| cell.ch != ' ').count();
        assert!(drawn > 20, "scope no dibuja (drawn={drawn})");
    }

    #[test]
    fn scope_survives_extreme_geometries() {
        let palette = Palette::dark();
        let audio = test_audio(0.9);
        let silent = AudioFrame::default();
        let mut scope = Scope::default();
        let cases: [(u16, u16, f32, &AudioFrame); 6] = [
            (1, 1, 2.0, &audio),
            (200, 56, 3.0, &audio),
            (120, 40, 3.5, &audio),
            (20, 6, 1.0, &audio),
            (1, 1, 4.0, &silent),
            (200, 56, 5.0, &silent),
        ];
        for (w, h, time, frame) in cases {
            let mut layer = Layer::new(w, h);
            let ctx = VizCtx {
                audio: frame,
                palette: &palette,
                lut: crate::viz::shade_lut(&palette),
                w,
                h,
                dt: 1.0 / 60.0,
                time,
            };
            scope.frame(&mut layer, &ctx);
        }
    }

    #[test]
    fn scope_beam_adapts_to_height() {
        let palette = Palette::dark();
        let audio = test_audio(0.9);
        let lut = crate::viz::shade_lut(&palette);
        let mut scope = Scope::default();

        let mut tall = Layer::new(80, 24);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut,
            w: 80,
            h: 24,
            dt: 1.0 / 60.0,
            time: 1.0,
        };
        scope.frame(&mut tall, &ctx);
        let braille = tall
            .cells
            .iter()
            .filter(|cell| ('\u{2801}'..='\u{28ff}').contains(&cell.ch))
            .count();
        assert!(braille > 0, "sin haz braille a 80x24");

        let mut short = Layer::new(40, 4);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut,
            w: 40,
            h: 4,
            dt: 1.0 / 60.0,
            time: 2.0,
        };
        scope.frame(&mut short, &ctx);
        let half = short
            .cells
            .iter()
            .filter(|cell| matches!(cell.ch, '▀' | '▄' | '█'))
            .count();
        assert!(half > 0, "sin haz de media celda a 40x4");
    }

    #[test]
    fn scope_does_not_duplicate_sweeps_for_same_generation() {
        let palette = Palette::dark();
        let audio = test_audio(0.7);
        let mut scope = Scope::default();
        let mut layer = Layer::new(60, 10);
        for step in 0..2 {
            let ctx = VizCtx {
                audio: &audio,
                palette: &palette,
                lut: crate::viz::shade_lut(&palette),
                w: 60,
                h: 10,
                dt: 1.0 / 240.0,
                time: 5.0 + step as f32 * (1.0 / 240.0),
            };
            scope.frame(&mut layer, &ctx);
        }
        assert_eq!(
            scope.sweeps.len(),
            1,
            "la misma generación de audio no debe apilar barridos"
        );
    }

    #[test]
    fn scope_keeps_a_bounded_trail() {
        let palette = Palette::dark();
        let mut audio = test_audio(0.6);
        let mut scope = Scope::default();
        let mut layer = Layer::new(40, 8);
        for step in 0..32 {
            audio.time = step as f32 * 0.02;
            let ctx = VizCtx {
                audio: &audio,
                palette: &palette,
                lut: crate::viz::shade_lut(&palette),
                w: 40,
                h: 8,
                dt: 1.0 / 60.0,
                time: step as f32 * 0.06,
            };
            scope.frame(&mut layer, &ctx);
        }
        assert_eq!(scope.sweeps.len(), SCOPE_TRAILS);
    }

    #[test]
    fn scope_stays_flat_when_silent() {
        let palette = Palette::dark();
        let audio = AudioFrame::default();
        let mut scope = Scope::default();
        // Sin rejilla: la única celda dibujada debe ser la línea base.
        scope.params.density = 0.0;
        let mut layer = Layer::new(40, 8);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: crate::viz::shade_lut(&palette),
            w: 40,
            h: 8,
            dt: 1.0 / 60.0,
            time: 1.0,
        };
        scope.frame(&mut layer, &ctx);
        assert!(
            scope.sweeps.is_empty(),
            "el silencio no debe capturar barridos"
        );
        assert_eq!(layer.ch_at(0, 4), '·', "falta la línea base");
    }

    #[test]
    fn new_effects_register_with_their_keys() {
        let expected = [
            ("lcd", EffectKind::Lcd),
            ("led", EffectKind::Led),
            ("bars_braille", EffectKind::BarsBraille),
            ("floor", EffectKind::Floor),
        ];
        for (key, kind) in expected {
            let effect = effect_by_key(key);
            assert_eq!(effect.key(), key);
            assert!(EffectKind::ALL.contains(&kind), "{key} no está en ALL");
        }
        assert_eq!(EffectKind::ALL.len(), 17);
    }

    #[test]
    fn new_effects_draw_and_survive_extreme_geometries() {
        let palette = Palette::dark();
        let audio = test_audio(0.8);
        for key in ["lcd", "led", "bars_braille", "floor"] {
            let mut effect = effect_by_key(key);
            for (w, h) in [(20u16, 6u16), (1, 1), (120, 40)] {
                let mut layer = Layer::new(w, h);
                let ctx = VizCtx {
                    audio: &audio,
                    palette: &palette,
                    lut: crate::viz::shade_lut(&palette),
                    w,
                    h,
                    dt: 1.0 / 60.0,
                    time: 3.0,
                };
                for _ in 0..8 {
                    layer.clear();
                    effect.frame(&mut layer, &ctx);
                }
                if w > 10 {
                    let drawn = layer.cells.iter().filter(|cell| cell.ch != ' ').count();
                    assert!(drawn > 3, "{key} no dibuja a {w}x{h} (drawn={drawn})");
                }
            }
        }
    }

    #[test]
    fn new_presets_compose() {
        for preset in ["lcd", "rack", "braille", "stage"] {
            assert!(render_all(preset, 100, 10) > 20, "preset {preset} vacío");
        }
    }

    #[test]
    fn braille_spectrum_leaves_no_holes() {
        let palette = Palette::dark();
        let mut audio = test_audio(0.9);
        for band in audio.bands.iter_mut() {
            *band = 0.9;
        }
        let mut effect = effect_by_key("bars_braille");
        let (w, h) = (60u16, 16u16);
        let mut layer = Layer::new(w, h);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: crate::viz::shade_lut(&palette),
            w,
            h,
            dt: 1.0 / 60.0,
            time: 2.0,
        };
        effect.frame(&mut layer, &ctx);
        let bottom = h as i32 - 1;
        for x in 0..w as i32 {
            let ch = layer.ch_at(x, bottom);
            assert!(
                ('\u{2800}'..='\u{28ff}').contains(&ch),
                "hueco en el braille: x={x} ch={ch:?}"
            );
        }
    }

    #[test]
    fn braille_full_height_fills_every_cell() {
        let palette = Palette::dark();
        let mut audio = test_audio(1.0);
        for band in audio.bands.iter_mut() {
            *band = 1.0;
        }
        let mut effect = effect_by_key("bars_braille");
        let (w, h) = (48u16, 12u16);
        let mut layer = Layer::new(w, h);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: crate::viz::shade_lut(&palette),
            w,
            h,
            dt: 1.0 / 60.0,
            time: 1.0,
        };
        effect.frame(&mut layer, &ctx);
        for cy in 0..h as i32 {
            for cx in 0..w as i32 {
                assert_eq!(
                    layer.ch_at(cx, cy),
                    '\u{28ff}',
                    "celda incompleta en ({cx},{cy})"
                );
            }
        }
    }

    #[test]
    fn braille_lut_maps_blank_and_full() {
        assert_eq!(BRAILLE_LUT[0], ' ');
        assert_eq!(BRAILLE_LUT[0xFF], '\u{28ff}');
        assert_eq!(BRAILLE_LUT[0x01], '\u{2801}');
        assert_eq!(BRAILLE_LUT[0x80], '\u{2880}');
        for mask in 1..=0xFFu8 {
            assert!(
                !BRAILLE_LUT[mask as usize].is_whitespace(),
                "máscara {mask:#04x} vacía"
            );
        }
    }

    #[test]
    fn floor_never_paints_outside_the_stage() {
        let palette = Palette::dark();
        let audio = test_audio(0.8);
        let (w, h) = (80u16, 24u16);
        let mut floor = Floor::default();
        let mut layer = Layer::new(w, h);
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: crate::viz::shade_lut(&palette),
            w,
            h,
            dt: 1.0 / 60.0,
            time: 1.5,
        };
        floor.frame(&mut layer, &ctx);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if layer.ch_at(x, y) == ' ' {
                    continue;
                }
                let (left, right) =
                    stage_span(w as i32, h as i32, y).expect("celda fuera de la franja");
                assert!(
                    x >= left && x <= right,
                    "({x},{y}) fuera del escenario {left}..={right}"
                );
            }
        }
        assert_eq!(layer.ch_at(0, 0), ' ', "la esquina superior no es suelo");
        assert_eq!(
            layer.ch_at(w as i32 - 1, h as i32 - 1),
            ' ',
            "la esquina inferior no es suelo"
        );
    }
}
