#![allow(dead_code)]
//! Composición de visualizadores: capas con blend modes sobre un canvas de
//! celdas RGB. Portado (no copiado) de los efectos de `cli-viz`.

mod dots;
mod effects;

pub use effects::{effect_by_key, EffectKind};

use spote_core::artwork::{Palette, Rgb};
use spote_core::audio::AudioFrame;

/// Celda de un canvas de efectos. `alpha` 0 = no toca nada.
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub alpha: f32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: (0, 0, 0),
            alpha: 0.0,
        }
    }
}

/// Canvas de un efecto antes de componer.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub w: u16,
    pub h: u16,
    pub cells: Vec<Cell>,
}

impl Layer {
    pub fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![Cell::default(); w as usize * h as usize],
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        if self.w != w || self.h != h {
            self.w = w;
            self.h = h;
            self.cells.clear();
            self.cells.resize(w as usize * h as usize, Cell::default());
        }
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, ch: char, fg: Rgb, alpha: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || alpha <= 0.0 {
            return;
        }
        let idx = y as usize * self.w as usize + x as usize;
        if let Some(cell) = self.cells.get_mut(idx) {
            // Lo que se dibuja después gana si tiene más alfa.
            if alpha >= cell.alpha || cell.ch == ' ' {
                cell.ch = ch;
                cell.fg = fg;
                cell.alpha = alpha.clamp(0.0, 1.0);
            }
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<Cell> {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return None;
        }
        self.cells
            .get(y as usize * self.w as usize + x as usize)
            .copied()
    }

    /// Carácter en (x,y), espacio si está fuera.
    pub fn ch_at(&self, x: i32, y: i32) -> char {
        self.get(x, y).map(|c| c.ch).unwrap_or(' ')
    }

    pub fn blit_char(&mut self, x: i32, y: i32, ch: char, fg: Rgb, alpha: f32) {
        self.put(x, y, ch, fg, alpha);
    }

    /// Línea de Bresenham (para estelas y aristas).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, ch: char, fg: Rgb, alpha: f32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let mut guard = 0;
        loop {
            self.put(x, y, ch, fg, alpha);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
            guard += 1;
            if guard > 4096 {
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Add,
    Screen,
    Multiply,
    Overlay,
}

impl Blend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Add => "add",
            Self::Screen => "screen",
            Self::Multiply => "multiply",
            Self::Overlay => "overlay",
        }
    }

    fn blend(self, dst: Rgb, src: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| -> u8 {
            let a = a as f32 / 255.0;
            let b = b as f32 / 255.0;
            let out = match self {
                Self::Normal => b,
                Self::Add => (a + b).min(1.0),
                Self::Screen => 1.0 - (1.0 - a) * (1.0 - b),
                Self::Multiply => a * b,
                Self::Overlay => {
                    if a < 0.5 {
                        2.0 * a * b
                    } else {
                        1.0 - 2.0 * (1.0 - a) * (1.0 - b)
                    }
                }
            };
            let final_value = a + (out - a) * t;
            (final_value * 255.0).clamp(0.0, 255.0) as u8
        };
        (mix(dst.0, src.0), mix(dst.1, src.1), mix(dst.2, src.2))
    }
}

/// Máscara de opacidad sobre el canvas (para que los efectos respiren).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mask {
    Full,
    /// Centro brillante, bordes transparentes. `power` 0.5..4.
    Radial(f32),
    /// Arriba transparente, abajo opaco.
    Bottom,
    /// Solo una banda vertical del canvas (0..1, 0..1).
    Band(f32, f32),
}

impl Mask {
    pub fn alpha(self, x: u16, y: u16, w: u16, h: u16) -> f32 {
        let nx = if w > 1 {
            x as f32 / (w - 1) as f32
        } else {
            0.5
        };
        let ny = if h > 1 {
            y as f32 / (h - 1) as f32
        } else {
            0.5
        };
        match self {
            Self::Full => 1.0,
            Self::Radial(power) => {
                let dx = nx * 2.0 - 1.0;
                let dy = ny * 2.0 - 1.0;
                let distance = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
                (1.0 - distance.powf(power.max(0.2))).clamp(0.0, 1.0)
            }
            Self::Bottom => (0.15 + ny * 0.85).clamp(0.0, 1.0),
            Self::Band(from, to) => {
                if nx >= from && nx <= to {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Parámetros ajustables en caliente por cada efecto.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub sensitivity: f32,
    pub density: f32,
    pub speed: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            density: 1.0,
            speed: 1.0,
        }
    }
}

/// Tamaño de la tabla de colores precalculada. `ramp_at` interpola en OKLab
/// (cbrt + powf por canal); los efectos lo llaman por celda, así que se
/// cuantiza a esta tabla una vez por frame. A 96 pasos no se ve el escalón.
pub const SHADE_STEPS: usize = 96;

/// Rampa de la paleta muestreada para el compositor. La usan el render y los
/// tests de efectos; `ramp_at` interpola en OKLab y por celda es caro.
pub fn shade_lut(palette: &Palette) -> [Rgb; SHADE_STEPS] {
    let mut lut = [palette.bg; SHADE_STEPS];
    for (index, slot) in lut.iter_mut().enumerate() {
        *slot = palette.ramp_at(index as f32 / (SHADE_STEPS - 1) as f32);
    }
    lut
}

/// Contexto de un frame: audio, paleta, tamaño y tiempo.
pub struct VizCtx<'a> {
    pub audio: &'a AudioFrame,
    pub palette: &'a Palette,
    /// Rampa de la paleta ya muestreada (0..1 → color).
    pub lut: [Rgb; SHADE_STEPS],
    pub w: u16,
    pub h: u16,
    pub dt: f32,
    pub time: f32,
}

impl VizCtx<'_> {
    pub fn aspect(&self) -> f32 {
        if self.h == 0 {
            1.0
        } else {
            self.w as f32 / (self.h as f32 * 2.0)
        }
    }

    /// Rama de la rampa de la paleta, con `t` 0..1.
    #[inline]
    pub fn shade(&self, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let index = (t * (SHADE_STEPS - 1) as f32).round() as usize;
        self.lut[index.min(SHADE_STEPS - 1)]
    }
}

pub trait Effect: Send {
    fn key(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// Un frame completo: actualiza estado y dibuja en `layer`.
    fn frame(&mut self, layer: &mut Layer, ctx: &VizCtx);
    fn params(&mut self) -> &mut Params;
    /// Teclas propias del efecto (opcional). Devuelve true si la consumió.
    fn handle_key(&mut self, _key: char) -> bool {
        false
    }
}

/// Una capa del compositor.
pub struct VizLayer {
    pub effect: Box<dyn Effect>,
    pub blend: Blend,
    pub opacity: f32,
    pub mask: Mask,
    pub enabled: bool,
    /// La máscara no cambia mientras no cambie el tamaño del canvas; su valor
    /// por celda se guarda para no recalcular `powf` en cada frame.
    mask_table: Option<(u16, u16, Vec<f32>)>,
}

impl VizLayer {
    pub fn new(effect: Box<dyn Effect>, blend: Blend, opacity: f32, mask: Mask) -> Self {
        Self {
            effect,
            blend,
            opacity,
            mask,
            enabled: true,
            mask_table: None,
        }
    }

    fn mask_alpha(&mut self, w: u16, h: u16) -> &[f32] {
        let fresh = matches!(
            &self.mask_table,
            Some((cw, ch, _)) if *cw == w && *ch == h
        );
        if !fresh {
            let mask = self.mask;
            let mut table = Vec::with_capacity(w as usize * h as usize);
            for y in 0..h {
                for x in 0..w {
                    table.push(mask.alpha(x, y, w, h));
                }
            }
            self.mask_table = Some((w, h, table));
        }
        match &self.mask_table {
            Some((_, _, table)) => table.as_slice(),
            None => &[],
        }
    }
}

pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    /// (efecto, blend, opacidad, máscara)
    pub layers: &'static [(&'static str, Blend, f32, Mask)],
}

/// Presets con nombre. Las máscaras reparten el canvas para que se sumen sin
/// taparse: p. ej. `aurora` deja las ondas abajo y las partículas arriba.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "aurora",
        description: "olas + partículas + estrellas",
        layers: &[
            ("wave", Blend::Screen, 0.85, Mask::Full),
            ("particles", Blend::Add, 0.80, Mask::Bottom),
            ("starfield", Blend::Screen, 0.55, Mask::Full),
        ],
    },
    Preset {
        name: "inferno",
        description: "fuego + barras",
        layers: &[
            ("bars", Blend::Normal, 0.75, Mask::Bottom),
            ("flame", Blend::Add, 0.95, Mask::Bottom),
        ],
    },
    Preset {
        name: "void",
        description: "estrellas + lluvia matrix",
        layers: &[
            ("starfield", Blend::Screen, 0.9, Mask::Radial(1.4)),
            ("matrix", Blend::Add, 0.38, Mask::Full),
        ],
    },
    Preset {
        name: "bloom",
        description: "círculo + púlsar",
        layers: &[
            ("circle", Blend::Normal, 0.9, Mask::Radial(1.2)),
            ("pulsar", Blend::Add, 0.85, Mask::Radial(1.8)),
        ],
    },
    Preset {
        name: "glitch",
        description: "fractal + espectro 3D",
        layers: &[
            ("fractal", Blend::Normal, 0.85, Mask::Full),
            ("spectro3d", Blend::Add, 0.6, Mask::Bottom),
        ],
    },
    Preset {
        name: "clean",
        description: "barras limpias + marea",
        layers: &[
            // El clean original: barras suaves que respiran con el bajo y una
            // marea encima para que no se quede en un ecualizador plano.
            ("bars", Blend::Normal, 0.86, Mask::Bottom),
            ("wave", Blend::Screen, 0.58, Mask::Full),
        ],
    },
    Preset {
        name: "neural",
        description: "red neuronal + túnel",
        layers: &[
            ("neural", Blend::Normal, 0.9, Mask::Full),
            ("tunnel", Blend::Add, 0.55, Mask::Radial(1.3)),
        ],
    },
    Preset {
        name: "crt",
        description: "osciloscopio con estelas",
        // Entrada nueva al final: no altera los presets existentes ni el
        // arranque (el preset por defecto sigue siendo `clean`).
        layers: &[("scope", Blend::Screen, 0.95, Mask::Full)],
    },
    Preset {
        name: "lcd",
        description: "vu segmentado con scanlines + osciloscopio",
        layers: &[("lcd", Blend::Normal, 0.95, Mask::Full)],
    },
    Preset {
        name: "rack",
        description: "escalera led de 16 bandas",
        layers: &[("led", Blend::Normal, 0.95, Mask::Full)],
    },
    Preset {
        name: "braille",
        description: "espectro braille de doble resolución",
        layers: &[("bars_braille", Blend::Normal, 0.95, Mask::Full)],
    },
    Preset {
        name: "stage",
        description: "escenario en perspectiva",
        layers: &[("floor", Blend::Normal, 0.95, Mask::Full)],
    },
];

/// Rotación por defecto de la franja (`v`): los cinco presets activos de
/// fábrica. `lcd`, `rack` y `stage` siguen disponibles a mano en `e` y en
/// ajustes, pero no entran en la rotación.
pub const DEFAULT_ROTATION: [&'static str; 5] = ["clean", "aurora", "bloom", "crt", "braille"];

/// Índice de un preset en `PRESETS` por nombre. La rotación y los ajustes lo
/// usan para resolver un nombre sin recorrer el catálogo a mano.
pub fn preset_by_name(name: &str) -> Option<usize> {
    PRESETS.iter().position(|preset| preset.name == name)
}

/// Compositor: N capas, un canvas final.
pub struct Viz {
    pub layers: Vec<VizLayer>,
    pub preset_index: usize,
    pub time: f32,
    pub scratch: Layer,
    pub output: Layer,
    pub paused: bool,
}

impl Viz {
    pub fn from_preset(name: &str) -> Self {
        let index = preset_by_name(name).unwrap_or(0);
        Self::preset(index)
    }

    pub fn preset(index: usize) -> Self {
        let preset = &PRESETS[index % PRESETS.len()];
        let layers = preset
            .layers
            .iter()
            .map(|(key, blend, opacity, mask)| {
                VizLayer::new(effect_by_key(key), *blend, *opacity, *mask)
            })
            .collect();
        Self {
            layers,
            preset_index: index % PRESETS.len(),
            time: 0.0,
            scratch: Layer::default(),
            output: Layer::default(),
            paused: false,
        }
    }

    pub fn preset_name(&self) -> &'static str {
        PRESETS[self.preset_index].name
    }

    pub fn preset_description(&self) -> &'static str {
        PRESETS[self.preset_index].description
    }

    /// Presets que se ciclan con `v` y en ajustes. Los ruidosos (`void`,
    /// `neural`, `glitch`) viven como fondo ambiental, no en la franja.
    pub const STRIP: [&'static str; 9] = [
        "clean", "aurora", "bloom", "inferno", "crt", "lcd", "rack", "braille", "stage",
    ];

    fn strip_index(&self) -> usize {
        Self::STRIP
            .iter()
            .position(|name| *name == self.preset_name())
            .unwrap_or(0)
    }

    pub fn next_preset(&mut self) {
        let next = (self.strip_index() + 1) % Self::STRIP.len();
        let index = PRESETS
            .iter()
            .position(|preset| preset.name == Self::STRIP[next])
            .unwrap_or(0);
        self.set_preset(index);
    }

    pub fn prev_preset(&mut self) {
        let prev = (self.strip_index() + Self::STRIP.len() - 1) % Self::STRIP.len();
        let index = PRESETS
            .iter()
            .position(|preset| preset.name == Self::STRIP[prev])
            .unwrap_or(0);
        self.set_preset(index);
    }

    pub fn set_preset(&mut self, index: usize) {
        let index = index % PRESETS.len();
        if index == self.preset_index && !self.layers.is_empty() {
            return;
        }
        let enabled: Vec<bool> = self.layers.iter().map(|l| l.enabled).collect();
        let mut next = Self::preset(index);
        for (i, layer) in next.layers.iter_mut().enumerate() {
            layer.enabled = enabled.get(i).copied().unwrap_or(true);
        }
        self.layers = next.layers;
        self.preset_index = index;
    }

    pub fn toggle_layer(&mut self, index: usize) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.enabled = !layer.enabled;
        }
    }

    pub fn layer_summary(&self) -> String {
        self.layers
            .iter()
            .enumerate()
            .map(|(i, layer)| {
                format!(
                    "{}{} {}",
                    i + 1,
                    if layer.enabled { "" } else { "·" },
                    layer.effect.name()
                )
            })
            .collect::<Vec<_>>()
            .join("  ")
    }

    /// Composición de todas las capas activas en `output`.
    pub fn render(&mut self, w: u16, h: u16, audio: &AudioFrame, palette: &Palette, dt: f32) {
        self.time += dt;
        self.output.resize(w, h);
        self.scratch.resize(w, h);
        // Fondo: gradiente vertical de la paleta, muy tenue.
        for y in 0..h {
            let t = y as f32 / (h.max(2) - 1) as f32;
            let color = palette.ramp_at(0.10 + t * 0.06);
            for x in 0..w {
                self.output.put(x as i32, y as i32, ' ', color, 1.0);
            }
        }
        if self.paused || w == 0 || h == 0 {
            return;
        }
        // La rampa solo depende de la paleta (no del tiempo ni del audio).
        let ctx = VizCtx {
            audio,
            palette,
            lut: shade_lut(palette),
            w,
            h,
            dt,
            time: self.time,
        };
        for layer in &mut self.layers {
            if !layer.enabled {
                continue;
            }
            self.scratch.clear();
            layer.effect.frame(&mut self.scratch, &ctx);
            let blend = layer.blend;
            let opacity = layer.opacity.clamp(0.0, 1.0);
            let mask = layer.mask_alpha(w, h);
            for (idx, src) in self.scratch.cells.iter().copied().enumerate() {
                if src.alpha <= 0.01 || src.ch == ' ' {
                    continue;
                }
                let amount = src.alpha * opacity * mask.get(idx).copied().unwrap_or(1.0);
                if amount <= 0.01 {
                    continue;
                }
                let Some(dst) = self.output.cells.get_mut(idx) else {
                    continue;
                };
                dst.fg = blend.blend(dst.fg, src.fg, amount);
                if blend != Blend::Multiply || dst.ch == ' ' {
                    dst.ch = src.ch;
                }
                dst.alpha = dst.alpha.max(amount);
            }
        }
    }

    pub fn handle_key(&mut self, key: char) -> bool {
        self.layers
            .iter_mut()
            .any(|layer| layer.effect.handle_key(key))
    }
}

/// RNG propio (xorshift64*): sin dependencia y suficientemente bueno para
/// partículas y ruido.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    pub fn seeded() -> Self {
        if cfg!(test) {
            // Tests deterministas: mismos glifos y mismas partículas siempre.
            return Self::new(0x5EED_1234);
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self::new(nanos ^ (std::process::id() as u64) << 32)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// 0..1
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    #[inline]
    pub fn range(&mut self, from: f32, to: f32) -> f32 {
        from + (to - from) * self.f32()
    }

    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    #[inline]
    pub fn sign(&mut self) -> f32 {
        if self.f32() < 0.5 {
            -1.0
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_put_and_bounds() {
        let mut layer = Layer::new(10, 4);
        layer.put(3, 2, '█', (255, 0, 0), 1.0);
        assert_eq!(layer.ch_at(3, 2), '█');
        layer.put(-1, 0, 'X', (255, 0, 0), 1.0);
        layer.put(100, 100, 'X', (255, 0, 0), 1.0);
        assert_eq!(layer.ch_at(0, 0), ' ');
    }

    #[test]
    fn alpha_keeps_the_strongest_cell() {
        let mut layer = Layer::new(2, 1);
        layer.put(0, 0, 'a', (255, 0, 0), 0.4);
        layer.put(0, 0, 'b', (0, 255, 0), 0.9);
        layer.put(0, 0, 'c', (0, 0, 255), 0.2);
        assert_eq!(layer.ch_at(0, 0), 'b');
    }

    #[test]
    fn blend_modes_are_bounded() {
        for mode in [
            Blend::Normal,
            Blend::Add,
            Blend::Screen,
            Blend::Multiply,
            Blend::Overlay,
        ] {
            let out = mode.blend((10, 20, 30), (200, 100, 50), 0.5);
            assert!(out.0 < 255 || mode != Blend::Normal);
        }
        assert_eq!(
            Blend::Normal.blend((10, 10, 10), (200, 200, 200), 1.0),
            (200, 200, 200)
        );
        assert_eq!(
            Blend::Normal.blend((10, 10, 10), (200, 200, 200), 0.0),
            (10, 10, 10)
        );
        assert_eq!(
            Blend::Multiply.blend((255, 255, 255), (0, 0, 0), 1.0),
            (0, 0, 0)
        );
    }

    #[test]
    fn masks_fade_outwards() {
        let w = 21;
        let h = 11;
        assert_eq!(Mask::Full.alpha(0, 0, w, h), 1.0);
        assert!(Mask::Radial(1.5).alpha(10, 5, w, h) > Mask::Radial(1.5).alpha(0, 0, w, h));
        assert!(Mask::Bottom.alpha(0, h - 1, w, h) > Mask::Bottom.alpha(0, 0, w, h));
        assert_eq!(Mask::Band(0.0, 0.5).alpha(0, 0, 10, 2), 1.0);
        assert_eq!(Mask::Band(0.0, 0.5).alpha(9, 0, 10, 2), 0.0);
    }

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            let value = a.f32();
            assert!((0.0..1.0).contains(&value));
            assert_eq!(value, b.f32());
        }
        assert_eq!(a.below(0), 0);
        assert!(a.below(5) < 5);
    }

    #[test]
    fn strip_presets_exist_and_roundtrip() {
        for name in Viz::STRIP {
            let viz = Viz::from_preset(name);
            assert_eq!(viz.preset_name(), name, "preset {name} no resuelve");
            assert!(!viz.layers.is_empty(), "preset {name} sin capas");
        }
    }

    #[test]
    fn default_rotation_and_lookup_resolve() {
        assert_eq!(DEFAULT_ROTATION.len(), 5);
        for name in DEFAULT_ROTATION {
            let index = preset_by_name(name).expect("preset de rotación inexistente");
            assert_eq!(PRESETS[index].name, name);
            assert!(
                Viz::STRIP.contains(&name),
                "{name} debería seguir en la franja"
            );
        }
        let clean = preset_by_name("clean").expect("clean existe");
        assert_eq!(PRESETS[clean].name, "clean");
        assert_eq!(preset_by_name("no-existe"), None);
        // Fuera de la rotación, pero presentes en el catálogo.
        for name in ["lcd", "rack", "stage"] {
            assert!(preset_by_name(name).is_some(), "{name} desapareció");
        }
    }

    #[test]
    fn presets_build_and_toggle() {
        let mut viz = Viz::from_preset("aurora");
        assert_eq!(viz.layers.len(), 3);
        assert_eq!(viz.preset_name(), "aurora");
        viz.toggle_layer(0);
        assert!(!viz.layers[0].enabled);
        viz.next_preset();
        assert_eq!(viz.preset_name(), "bloom");
        viz.prev_preset();
        assert_eq!(viz.preset_name(), "aurora");
    }

    #[test]
    fn render_produces_something_with_audio() {
        let pal = Palette::dark();
        let mut audio = AudioFrame::default();
        for (i, band) in audio.bands.iter_mut().enumerate() {
            *band = (i as f32 / 64.0).sin().abs();
        }
        audio.energy = 0.8;
        audio.beat = true;
        audio.beat_pulse = 1.0;
        audio.silent = false;
        let mut viz = Viz::from_preset("clean");
        viz.render(80, 10, &audio, &pal, 1.0 / 60.0);
        let drawn = viz
            .output
            .cells
            .iter()
            .filter(|c| c.ch != ' ' || c.alpha > 0.2)
            .count();
        assert!(drawn > 10, "drawn={drawn}");
    }
}
