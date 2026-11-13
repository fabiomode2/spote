#![allow(dead_code)]
//! Los efectos. Mecanismos portados de `cli-viz` (spectrum bars, wave, circle,
//! particles, flame, matrix, starfield, fractal, neural, pulsar, tunnel,
//! spectro3d), recoloreados con la paleta del álbum.

use super::{Effect, Layer, Params, Rng, VizCtx};
use spote_core::audio::{AudioFrame, BANDS};
use spote_core::util::mix_oklab;

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];
const DOTS: [char; 6] = ['·', '•', '∙', '◦', '○', '●'];

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
}

impl EffectKind {
    pub const ALL: [EffectKind; 12] = [
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
                // Degradado de color por columna: sin esto, en el modo de
                // fondo todas las barras salían del mismo tono y quedaba plano.
                let hue = (t * 1.35 + ctx.time * 0.04) % 1.0;
                let base = if hue < 0.5 {
                    mix_oklab(ctx.palette.primary, ctx.palette.secondary, hue * 2.0)
                } else {
                    mix_oklab(ctx.palette.secondary, ctx.palette.accent, (hue - 0.5) * 2.0)
                };
                for row in 0..full {
                    let y = h - 1 - row;
                    let level = row as f32 / h as f32;
                    let ramp = ctx.shade(0.38 + level * 0.5);
                    let mut color = mix_oklab(ramp, base, 0.55);
                    if level > 0.72 {
                        color = mix_oklab(color, ctx.palette.text, 0.45);
                    }
                    layer.put(x, y, '█', color, 1.0);
                }
                // Sub-celda: el bloque parcial da la sensación de altura continua.
                let fraction = height - height.floor();
                let extra = (fraction * 8.0) as usize;
                if extra > 0 {
                    let y = h - 1 - full;
                    layer.put(x, y, BLOCKS[extra.min(8)], mix_oklab(base, ctx.palette.text, 0.35), 1.0);
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
                let (a, b) = if prev < y { (prev + 1, y) } else { (y + 1, prev) };
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
        layer.put(cx as i32, cy as i32, '◉', ctx.shade(1.0), (0.5 + core * 0.3).min(1.0));
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
        let glyphs = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz<>[]{}/\\|=+-*#%&$@!?"
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
        self.pulse = (self.pulse * 0.92).max(ctx.audio.beat_pulse * 0.7 + ctx.audio.snare_pulse * 0.6);
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
                layer.line(x1, y1, x2, y2, '·', ctx.shade(0.32 + strength * 0.35), alpha);
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
}
