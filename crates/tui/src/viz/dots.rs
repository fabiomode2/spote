//! Retícula de puntos Braille (2×4 por celda) con buffer reutilizable.
//!
//! El efecto escribe un brillo por subpunto y el helper decide el glifo: un
//! punto se enciende si pasa el umbral y la celda usa el brillo medio de sus
//! puntos encendidos como alfa. El búfer se dimensiona al cambiar de tamaño;
//! después no hay allocs por frame.

use super::{Layer, VizCtx};

/// Bit Unicode de cada punto, indexado por `[columna][fila]` dentro de la celda.
const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

const BASE: u32 = 0x2800;

#[derive(Debug, Clone, Default)]
pub struct Dots {
    /// Ancho en celdas de terminal.
    w: u16,
    /// Alto en celdas de terminal.
    h: u16,
    /// Brillo 0..255 de cada subpunto, fila a fila.
    levels: Vec<u8>,
}

impl Dots {
    /// Dimensiona la retícula si cambió el canvas. No toca nada si no cambió.
    pub fn resize(&mut self, w: u16, h: u16) {
        if self.w == w && self.h == h {
            return;
        }
        self.w = w;
        self.h = h;
        let len = w as usize * 2 * h as usize * 4;
        self.levels.clear();
        self.levels.resize(len, 0);
    }

    pub fn clear(&mut self) {
        self.levels.fill(0);
    }

    /// Enciende un subpunto. Se queda con el brillo mayor si ya estaba pintado.
    pub fn plot(&mut self, x: i32, y: i32, level: f32) {
        let dw = self.w as i32 * 2;
        let dh = self.h as i32 * 4;
        if x < 0 || y < 0 || x >= dw || y >= dh {
            return;
        }
        let value = (level.clamp(0.0, 1.0) * 255.0).round() as u8;
        let index = y as usize * dw as usize + x as usize;
        if let Some(slot) = self.levels.get_mut(index) {
            *slot = (*slot).max(value);
        }
    }

    /// Glifo Braille de una celda, brillo medio y fila del punto más alto.
    /// `None` si ningún punto llega al umbral.
    pub fn cell(&self, cx: i32, cy: i32, threshold: u8) -> Option<(char, f32, u8)> {
        if cx < 0 || cy < 0 || cx >= self.w as i32 || cy >= self.h as i32 {
            return None;
        }
        let dw = self.w as usize * 2;
        let mut bits = 0u32;
        let mut lit = 0u32;
        let mut sum = 0u32;
        let mut top = 4u8;
        for row in 0..4 {
            for (col, column_bits) in BITS.iter().enumerate() {
                let index = (cy as usize * 4 + row) * dw + cx as usize * 2 + col;
                let level = self.levels.get(index).copied().unwrap_or(0);
                if level >= threshold {
                    bits |= column_bits[row];
                    lit += 1;
                    sum += level as u32;
                    if (row as u8) < top {
                        top = row as u8;
                    }
                }
            }
        }
        if bits == 0 {
            return None;
        }
        let ch = char::from_u32(BASE | bits).unwrap_or(' ');
        let brightness = if lit == 0 {
            0.0
        } else {
            sum as f32 / (lit as f32 * 255.0)
        };
        Some((ch, brightness.clamp(0.0, 1.0), top.min(3)))
    }

    /// Vuelca las celdas encendidas a la capa. El color recorre la rampa según
    /// la altura del punto más alto de cada celda.
    pub fn render(&self, layer: &mut Layer, ctx: &VizCtx, threshold: u8, low: f32, high: f32) {
        if self.w != layer.w || self.h != layer.h {
            return;
        }
        let total = (self.h as f32 * 4.0).max(1.0);
        for cy in 0..self.h as i32 {
            for cx in 0..self.w as i32 {
                let Some((ch, brightness, top)) = self.cell(cx, cy, threshold) else {
                    continue;
                };
                let y = cy as f32 * 4.0 + top as f32;
                let height = (1.0 - y / total).clamp(0.0, 1.0);
                let color = ctx.shade(low + (high - low) * height);
                let alpha = (0.30 + 0.70 * brightness).clamp(0.0, 1.0);
                layer.put(cx, cy, ch, color, alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spote_core::artwork::Palette;
    use spote_core::audio::AudioFrame;

    #[test]
    fn full_cell_becomes_all_dots() {
        let mut dots = Dots::default();
        dots.resize(1, 1);
        for y in 0..4 {
            for x in 0..2 {
                dots.plot(x, y, 1.0);
            }
        }
        let (ch, brightness, top) = dots.cell(0, 0, 128).expect("celda llena");
        assert_eq!(ch, '\u{28ff}');
        assert_eq!(top, 0);
        assert!((brightness - 1.0).abs() < 0.01);
    }

    #[test]
    fn empty_and_clipped_dots_stay_empty() {
        let mut dots = Dots::default();
        dots.resize(1, 1);
        dots.plot(-1, 0, 1.0);
        dots.plot(2, 0, 1.0);
        dots.plot(0, 4, 1.0);
        assert!(dots.cell(0, 0, 128).is_none());
        assert!(dots.cell(1, 0, 128).is_none());
    }

    #[test]
    fn render_marks_every_full_cell() {
        let palette = Palette::dark();
        let audio = AudioFrame::default();
        let mut dots = Dots::default();
        dots.resize(4, 2);
        for y in 0..8 {
            for x in 0..8 {
                dots.plot(x, y, 1.0);
            }
        }
        let ctx = VizCtx {
            audio: &audio,
            palette: &palette,
            lut: super::super::shade_lut(&palette),
            w: 4,
            h: 2,
            dt: 0.016,
            time: 0.0,
        };
        let mut layer = Layer::new(4, 2);
        dots.render(&mut layer, &ctx, 128, 0.3, 0.9);
        assert_eq!(layer.cells.iter().filter(|cell| cell.ch != ' ').count(), 8);
    }
}
