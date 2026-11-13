//! Portada → celdas de terminal.
//!
//! El arte siempre pasa por el mismo oscurecido (viñeta radial suave +
//! gradiente vertical, más el `dim` del llamante) *antes* de cuantizar: así el
//! texto de la UI que se dibuja encima se lee sin tocar el brillo del tema.
//! Una sola muestra redimensionada por render, sin allocs por celda.

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};

use super::palette::Palette;
use super::{ArtCell, ArtMode, CoverArt, Rgb};
use crate::util::{luma, mix, scale};

/// Niveles de color: los de la rampa de la paleta.
const LEVELS: usize = 12;

/// Glifos del modo dither, de casi vacío a lleno. Los bloques dan la textura
/// de grabado; el color de la rampa da el tono real de la portada. Ningún paso
/// es un espacio: una portada oscura sigue dejando trama en vez de un hueco.
const DENSITY: [char; LEVELS] = [
    '·', '░', '░', '▒', '▒', '▒', '▓', '▓', '▓', '█', '█', '█',
];

/// Matriz de Bayer 8x8 estándar (0..63). El dither ordenado rompe las bandas
/// que deja la cuantización a 12 niveles sin meter ruido temporal: dos frames
/// de la misma portada salen idénticos.
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// (bit del patrón braille, columna, fila) de cada punto en la celda 2x4.
const BRAILLE_DOTS: [(u32, u32, u32); 8] = [
    (0x01, 0, 0),
    (0x02, 0, 1),
    (0x04, 0, 2),
    (0x08, 1, 0),
    (0x10, 1, 1),
    (0x20, 1, 2),
    (0x40, 0, 3),
    (0x80, 1, 3),
];

const BRAILLE_BASE: u32 = 0x2800;

/// Rasteriza `img` como arte de terminal de `w` columnas por `h` filas.
///
/// `dim` va de 0.0 (sin oscurecer) a 1.0 (negro). Con imagen válida nunca
/// devuelve celdas en blanco: como mínimo, un degradado del tema.
pub fn render_cover(
    img: &DynamicImage,
    w: u16,
    h: u16,
    mode: ArtMode,
    pal: &Palette,
    dim: f32,
) -> CoverArt {
    if w == 0 || h == 0 {
        return CoverArt::new(w, h);
    }
    // NaN o infinitos no deben teñir la portada de negro por accidente.
    let dim = if dim.is_finite() {
        dim.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if img.width() == 0 || img.height() == 0 {
        return gradient_panel(w, h, pal, dim);
    }
    match mode {
        ArtMode::Off => blank_panel(w, h, pal),
        ArtMode::Gradient => gradient_panel(w, h, pal, dim),
        ArtMode::Half => render_half(img, w, h, pal, dim),
        ArtMode::Plain => render_plain(img, w, h),
        ArtMode::Dither => render_dither(img, w, h, pal, dim),
        ArtMode::Braille => render_braille(img, w, h, pal, dim),
    }
}

// ---------------------------------------------------------------------------
// Muestreo y oscurecido
// ---------------------------------------------------------------------------

/// Recorta el centro al aspecto del destino (object-fit: cover) y redimensiona
/// una sola vez. Upscale incluido: el kernel se ocupa del suavizado.
fn sample_cover(img: &DynamicImage, tw: u32, th: u32, filter: FilterType) -> RgbaImage {
    let (iw, ih) = (img.width(), img.height());
    let target = tw as f32 / th as f32;
    let source = iw as f32 / ih as f32;
    let (cw, ch) = if source > target {
        let cw = ((ih as f32 * target).round() as u32).clamp(1, iw);
        (cw, ih)
    } else {
        let ch = ((iw as f32 / target).round() as u32).clamp(1, ih);
        (iw, ch)
    };
    let cx = (iw - cw) / 2;
    let cy = (ih - ch) / 2;
    img.crop_imm(cx, cy, cw, ch)
        .resize_exact(tw, th, filter)
        .to_rgba8()
}

/// Factor de brillo (0..1) para un punto normalizado del panel.
///
/// Viñeta radial cuadrática (el centro respira, las esquinas se apagan) y
/// gradiente vertical cuadrático, que deja la zona baja más oscura porque es
/// donde la UI dibuja la letra y los controles.
fn shade_factor(u: f32, v: f32, dim: f32) -> f32 {
    let dx = (u - 0.5) * 2.0;
    let dy = (v - 0.5) * 2.0;
    let d = (dx * dx + dy * dy).sqrt().min(1.0);
    let vignette = 1.0 - 0.36 * d * d;
    let vertical = 1.0 - 0.30 * v * v;
    vignette * vertical * (1.0 - dim)
}

/// Oscurece un color real acercándolo al fondo del tema en las zonas apagadas,
/// para que la viñeta no rompa la armonía cromática de la paleta.
fn shade_color(c: Rgb, f: f32, pal: &Palette) -> Rgb {
    mix(scale(c, f), pal.bg, (1.0 - f) * 0.18)
}

fn bayer8(x: u32, y: u32) -> f32 {
    (BAYER[(y & 7) as usize][(x & 7) as usize] as f32 + 0.5) / 64.0
}

fn px_luma(px: &image::Rgba<u8>) -> f32 {
    luma((px[0], px[1], px[2]))
}

/// Curva de tono de la portada.
///
/// Una portada oscura (la mayoría) se abre para no perderse contra el fondo del
/// tema, y una lavada se cierra un poco. La mediana cae cerca de media tinta,
/// que es justo el punto donde el dither reparte glifos al 50 %. Conserva 0→0 y
/// 1→1, así que `dim` sigue apagando el arte hasta negro.
fn tone_gamma(median: f32) -> f32 {
    if median <= 0.01 {
        return 1.0;
    }
    (0.5f32.ln() / median.ln()).clamp(0.62, 1.16)
}

/// Mediana de la luminancia del muestreo, por histograma de 64 cubos.
fn median_luma(pixels: &RgbaImage) -> f32 {
    let mut hist = [0u32; 64];
    let mut total = 0u32;
    for px in pixels.pixels() {
        let value = px_luma(px).clamp(0.0, 1.0);
        hist[((value * 63.0).round() as usize).min(63)] += 1;
        total += 1;
    }
    percentile(&hist, total, 0.5)
}

fn percentile(hist: &[u32; 64], total: u32, p: f32) -> f32 {
    if total == 0 {
        return 0.5;
    }
    let target = ((total as f32 * p.clamp(0.0, 1.0)).round() as u32).max(1);
    let mut acc = 0u32;
    for (i, &count) in hist.iter().enumerate() {
        acc += count;
        if acc >= target {
            return (i as f32 + 0.5) / 64.0;
        }
    }
    1.0
}

/// Luminancia real de cada paso de la rampa.
fn ramp_lumas(pal: &Palette) -> [f32; LEVELS] {
    let mut out = [0.0f32; LEVELS];
    for (slot, color) in out.iter_mut().zip(pal.ramp) {
        *slot = luma(color);
    }
    out
}

/// Paso de la rampa cuyo brillo encaja mejor con el tono pedido.
///
/// La rampa no reparte su luminancia de forma lineal (los pasos medios suelen
/// quedar pegados), así que indexarla directamente por el valor apaga el arte:
/// el gris 0.7 acabaría pintado con un color de brillo 0.4. Buscando por brillo
/// el panel conserva el contraste de la portada con los colores del tema.
fn ramp_step(lumas: &[f32; LEVELS], value: f32) -> usize {
    let mut best = (f32::MAX, 0usize);
    for (i, &l) in lumas.iter().enumerate() {
        let d = (l - value).abs();
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

// ---------------------------------------------------------------------------
// Modos
// ---------------------------------------------------------------------------

/// Sin arte: solo una celda vacía. `pal.bg` como fg deja el fondo coherente si
/// el llamante pinta el panel de todas formas.
fn blank_panel(w: u16, h: u16, pal: &Palette) -> CoverArt {
    let mut art = CoverArt::new(w, h);
    art.cells.fill(ArtCell {
        ch: ' ',
        fg: pal.bg,
        bg: None,
    });
    art
}

/// Degradado del tema: el plan B cuando no hay imagen válida. Sigue siendo un
/// panel legible en vez de un hueco negro.
fn gradient_panel(w: u16, h: u16, pal: &Palette, dim: f32) -> CoverArt {
    let mut art = CoverArt::new(w, h);
    let top = mix(pal.bg_deep, pal.primary, 0.22);
    for y in 0..h as u32 {
        let v = (y as f32 + 0.5) / h as f32;
        let color = scale(mix(top, pal.bg_deep, v * 0.85), 1.0 - dim);
        let cell = ArtCell {
            ch: ' ',
            fg: pal.dim,
            bg: Some(color),
        };
        let start = y as usize * w as usize;
        art.cells[start..start + w as usize].fill(cell);
    }
    art
}

/// Media celda: `▀` con el píxel de arriba en fg y el de abajo en bg. Es la
/// representación más fiel y la única que necesita dos colores por celda.
fn render_half(img: &DynamicImage, w: u16, h: u16, pal: &Palette, dim: f32) -> CoverArt {
    let (cw, rows) = (w as u32, h as u32 * 2);
    let pixels = sample_cover(img, cw, rows, FilterType::Lanczos3);
    let mut art = CoverArt::new(w, h);

    for y in 0..h as u32 {
        let top_v = (y as f32 * 2.0 + 0.5) / rows as f32;
        let bottom_v = (y as f32 * 2.0 + 1.5) / rows as f32;
        for x in 0..cw {
            let u = (x as f32 + 0.5) / cw as f32;
            let top = pixels.get_pixel(x, y * 2);
            let bottom = pixels.get_pixel(x, y * 2 + 1);
            let cell = ArtCell {
                ch: '▀',
                fg: shade_color((top[0], top[1], top[2]), shade_factor(u, top_v, dim), pal),
                bg: Some(shade_color(
                    (bottom[0], bottom[1], bottom[2]),
                    shade_factor(u, bottom_v, dim),
                    pal,
                )),
            };
            art.cells[y as usize * w as usize + x as usize] = cell;
        }
    }
    art
}

/// Dither ordenado: luminancia perceptual → curva de tono → Bayer 8x8 → rampa
/// de la paleta. El color lo pone la rampa (respeta la portada) y el glifo la
/// densidad, así que el contraste tinta/fondo se mantiene alto con `bg = None`.
/// La portada literal: media celda con los píxeles originales, sin paleta,
/// viñeta ni tinte. Es la opción "foto" para quien no quiere el look TUI.
fn render_plain(img: &DynamicImage, w: u16, h: u16) -> CoverArt {
    let pixels = sample_cover(img, w as u32, h as u32 * 2, FilterType::Lanczos3);
    let mut art = CoverArt::new(w, h);
    for y in 0..h as u32 {
        for x in 0..w as u32 {
            let top = pixels.get_pixel(x, y * 2);
            let bottom = pixels.get_pixel(x, y * 2 + 1);
            art.cells[y as usize * w as usize + x as usize] = ArtCell {
                ch: '▀',
                fg: (top[0], top[1], top[2]),
                bg: Some((bottom[0], bottom[1], bottom[2])),
            };
        }
    }
    art
}

fn render_dither(img: &DynamicImage, w: u16, h: u16, pal: &Palette, dim: f32) -> CoverArt {
    let (cw, ch) = (w as u32, h as u32);
    let pixels = sample_cover(img, cw, ch, FilterType::Triangle);
    let gamma = tone_gamma(median_luma(&pixels));
    let lumas = ramp_lumas(pal);
    let mut art = CoverArt::new(w, h);
    let span = (LEVELS - 1) as f32;

    for y in 0..ch {
        let v = (y as f32 + 0.5) / ch as f32;
        for x in 0..cw {
            let u = (x as f32 + 0.5) / cw as f32;
            let base = px_luma(pixels.get_pixel(x, y));
            // El tono se corrige antes del oscurecido para que `dim` no se
            // deshaga al normalizar.
            let lit = base.powf(gamma) * shade_factor(u, v, dim);
            // Medio escalón de Bayer: rompe la banda sin cambiar el tono medio.
            let value = (lit + (bayer8(x, y) - 0.5) / LEVELS as f32).clamp(0.0, 1.0);
            let idx = ramp_step(&lumas, value);
            // El glifo sigue la densidad cuantizada del valor, no el color: así
            // una rampa de colores planos mantiene la textura del grabado.
            let glyph = (value * span).round().clamp(0.0, span) as usize;
            art.cells[y as usize * w as usize + x as usize] = ArtCell {
                ch: DENSITY[glyph],
                fg: pal.ramp[idx],
                bg: None,
            };
        }
    }
    art
}

/// Braille: 2x4 muestras por celda. Detecta si la portada es tinta sobre
/// papel claro (line art, logos) y la invierte para que el fondo blanco no se
/// llene de puntos: en ese caso dibuja la tinta, no el papel. En fotos oscuras
/// mantiene la polaridad normal. El umbral se normaliza por percentiles y se
/// guía por el contraste local para no convertir el ruido de fondo en estática.
fn render_braille(img: &DynamicImage, w: u16, h: u16, pal: &Palette, dim: f32) -> CoverArt {
    let (cols, rows) = (w as u32 * 2, h as u32 * 4);
    let pixels = sample_cover(img, cols, rows, FilterType::Triangle);
    let mut luma = vec![0.0f32; (cols * rows) as usize];
    let mut hist = [0u32; 64];
    let mut border_sum = 0.0f32;
    let mut border_n = 0u32;
    for y in 0..rows {
        for x in 0..cols {
            let value = px_luma(pixels.get_pixel(x, y)).clamp(0.0, 1.0);
            luma[(y * cols + x) as usize] = value;
            hist[((value * 63.0).round() as usize).min(63)] += 1;
            if x == 0 || y == 0 || x + 1 == cols || y + 1 == rows {
                border_sum += value;
                border_n += 1;
            }
        }
    }
    let total = (cols * rows).max(1);
    let border = border_sum / border_n.max(1) as f32;
    let median = percentile(&hist, total, 0.5);
    // Papel claro (cómic, logo, portada escaneada): se dibuja la tinta oscura.
    let invert = border > 0.55 || median > 0.58;
    if invert {
        for value in &mut luma {
            *value = 1.0 - *value;
        }
        let mut inverted_hist = [0u32; 64];
        for value in &luma {
            inverted_hist[((value * 63.0).round() as usize).min(63)] += 1;
        }
        hist = inverted_hist;
    }
    let low = percentile(&hist, total, 0.18).min(0.35);
    let high = percentile(&hist, total, 0.94).max(low + 0.18).min(1.0);
    let range = (high - low).max(0.12);
    let at = |x: u32, y: u32| -> f32 {
        if x >= cols || y >= rows {
            0.0
        } else {
            luma[(y * cols + x) as usize]
        }
    };
    let mut art = CoverArt::new(w, h);
    for y in 0..h as u32 {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w as u32 {
            let mut bits = 0u32;
            let mut tone = 0.0f32;
            for &(bit, dx, dy) in &BRAILLE_DOTS {
                let (sx, sy) = (x * 2 + dx, y * 4 + dy);
                let base = at(sx, sy);
                // Contraste local 3x3: el papel liso no genera puntos, las
                // líneas y los bordes sí. Así una portada clara no es estática.
                let mut local = 0.0f32;
                let mut n = 0.0f32;
                for oy in -1i32..=1 {
                    for ox in -1i32..=1 {
                        local += at((sx as i32 + ox).max(0) as u32, (sy as i32 + oy).max(0) as u32);
                        n += 1.0;
                    }
                }
                local /= n.max(1.0);
                let contrast = (base - local).abs();
                let gx = (at(sx.saturating_add(1), sy) - at(sx.saturating_sub(1), sy)).abs();
                let gy = (at(sx, sy.saturating_add(1)) - at(sx, sy.saturating_sub(1))).abs();
                let edge = gx + gy;
                let fill = ((base - low) / range).clamp(0.0, 1.0);
                let mut value = (edge * 2.2 + contrast * 0.5 + fill * fill * 0.18).min(1.0);
                value = value.powf(if invert { 1.18 } else { 1.0 });
                value *= 1.0 - dim;
                if invert {
                    // Tinta sobre papel: se deja respirar la trama para que las
                    // zonas negras no se conviertan en bloques macizos.
                    value = (value * 0.78).min(0.82);
                }
                tone += value;
                if value > bayer8(sx, sy) * 0.98 {
                    bits |= bit;
                }
            }
            tone /= BRAILLE_DOTS.len() as f32;
            let ink = mix(mix(pal.secondary, pal.primary, tone), pal.text, 0.36);
            let glow = (0.45 + 0.55 * (1.0 - v * 0.4)) * (1.0 - dim * 0.7);
            art.cells[y as usize * w as usize + x as usize] = ArtCell {
                ch: char::from_u32(BRAILLE_BASE + bits).unwrap_or(' '),
                fg: scale(ink, glow.clamp(0.2, 1.0)),
                bg: None,
            };
        }
    }
    art
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    const MODES: [ArtMode; 4] = [
        ArtMode::Dither,
        ArtMode::Half,
        ArtMode::Braille,
        ArtMode::Off,
    ];

    /// Degradado horizontal + vertical: garantiza tonos distintos en 2D.
    fn gradient(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            *px = Rgba([r, g, 180, 255]);
        }
        DynamicImage::ImageRgba8(img)
    }

    fn mean_luma(art: &CoverArt) -> f32 {
        let mut sum = 0.0;
        let mut n = 0usize;
        for cell in &art.cells {
            sum += luma(cell.fg);
            n += 1;
            if let Some(bg) = cell.bg {
                sum += luma(bg);
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            sum / n as f32
        }
    }

    #[test]
    fn every_mode_fills_the_panel() {
        let img = gradient(64, 64);
        let pal = Palette::from_image(&img);
        for mode in MODES {
            let art = render_cover(&img, 32, 12, mode, &pal, 0.0);
            assert_eq!(art.w, 32);
            assert_eq!(art.h, 12);
            assert_eq!(art.cells.len(), 32 * 12, "{mode:?}");
            assert!(!art.is_empty(), "{mode:?}");
            if mode == ArtMode::Off {
                continue; // Off es plano por definición.
            }
            assert!(
                art.cells.windows(2).any(|pair| pair[0] != pair[1]),
                "{mode:?} devolvió un panel plano"
            );
            assert!(
                art.cells.iter().any(|c| c.ch != ' '),
                "{mode:?} no pintó nada"
            );
        }
    }

    #[test]
    fn half_uses_fg_and_bg() {
        let img = gradient(48, 48);
        let pal = Palette::dark();
        let art = render_cover(&img, 24, 10, ArtMode::Half, &pal, 0.0);
        assert_eq!(art.cells.len(), 240);
        for cell in &art.cells {
            assert_eq!(cell.ch, '▀');
            assert!(cell.bg.is_some(), "half debe rellenar bg con el píxel de abajo");
        }
        // Arriba y abajo no pueden ser el mismo color en todo el panel.
        assert!(art.cells.iter().any(|c| c.fg != c.bg.unwrap()));
    }

    #[test]
    fn dither_mixes_several_glyphs() {
        let img = gradient(64, 64);
        let pal = Palette::from_accents((210, 70, 140), (60, 190, 240));
        let art = render_cover(&img, 40, 12, ArtMode::Dither, &pal, 0.0);
        let mut chars: Vec<char> = art.cells.iter().map(|c| c.ch).collect();
        chars.sort_unstable();
        chars.dedup();
        assert!(
            chars.len() >= 3,
            "el dither debería escalonar glifos, salió {chars:?}"
        );
        // El color del glifo tiene que salir de la rampa de la paleta.
        assert!(art.cells.iter().all(|c| pal.ramp.contains(&c.fg)));
        assert!(art.cells.iter().all(|c| c.bg.is_none()));
    }

    #[test]
    fn off_returns_only_spaces() {
        let img = gradient(16, 16);
        let pal = Palette::dark();
        let art = render_cover(&img, 8, 4, ArtMode::Off, &pal, 0.5);
        assert_eq!(art.cells.len(), 32);
        assert!(art.cells.iter().all(|c| c.ch == ' ' && c.bg.is_none()));
        assert!(art.cells.iter().all(|c| c.fg == pal.bg));
    }

    #[test]
    fn full_dim_darkens_without_panicking() {
        let img = gradient(64, 64);
        let pal = Palette::from_image(&img);
        for mode in MODES {
            let bright = render_cover(&img, 20, 8, mode, &pal, 0.0);
            let dark = render_cover(&img, 20, 8, mode, &pal, 1.0);
            assert_eq!(dark.cells.len(), bright.cells.len(), "{mode:?}");
            if mode == ArtMode::Off {
                continue; // Off no depende de dim: no pinta arte.
            }
            assert!(
                mean_luma(&dark) < mean_luma(&bright),
                "{mode:?}: dim=1.0 no oscureció ({} vs {})",
                mean_luma(&dark),
                mean_luma(&bright)
            );
        }
    }

    #[test]
    fn zero_dimensions_do_not_panic() {
        let img = gradient(16, 16);
        let pal = Palette::dark();
        for mode in MODES {
            for (w, h) in [(0, 0), (0, 8), (8, 0)] {
                let art = render_cover(&img, w, h, mode, &pal, 0.3);
                assert_eq!(art.w, w);
                assert_eq!(art.h, h);
                assert!(art.cells.is_empty());
                assert!(art.is_empty());
            }
        }
    }

    #[test]
    fn broken_image_still_paints() {
        // Imagen de 0x0: no hay píxeles, pero el panel no puede quedar muerto.
        let img = DynamicImage::ImageRgba8(RgbaImage::new(0, 0));
        let pal = Palette::dark();
        for mode in MODES {
            let art = render_cover(&img, 10, 4, mode, &pal, 0.0);
            assert_eq!(art.cells.len(), 40);
            if mode != ArtMode::Off {
                assert!(art.cells.iter().all(|c| c.bg.is_some()), "{mode:?}");
            }
        }
    }

    #[test]
    fn braille_inverts_light_paper_and_leaves_background_empty() {
        // Papel blanco con una línea negra: el fondo no debe llenarse de puntos.
        let mut img = RgbaImage::new(64, 64);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let ink = (20..44).contains(&x) && (20..44).contains(&y);
            *px = if ink {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            };
        }
        let pal = Palette::dark();
        let art = render_cover(
            &DynamicImage::ImageRgba8(img),
            32,
            12,
            ArtMode::Braille,
            &pal,
            0.0,
        );
        let top = (0..32).filter(|x| art.cell(*x, 0).unwrap().ch != '\u{2800}').count();
        assert!(top < 8, "el papel blanco se llenó de puntos: {top}");
        let center = (0..32).filter(|x| art.cell(*x, 6).unwrap().ch != '\u{2800}').count();
        assert!(center > 2, "la tinta negra no se dibujó");
    }

    #[test]
    fn braille_draws_dots() {
        let img = gradient(64, 64);
        let pal = Palette::dark();
        let art = render_cover(&img, 24, 10, ArtMode::Braille, &pal, 0.0);
        let dotted = art
            .cells
            .iter()
            .filter(|c| c.ch != '\u{2800}')
            .count();
        assert!(dotted > 0, "el modo braille no pintó ningún punto");
        assert!(
            art.cells
                .iter()
                .all(|c| ('\u{2800}'..='\u{28ff}').contains(&c.ch)),
            "todos los glifos deben ser patrones braille"
        );
    }
}
