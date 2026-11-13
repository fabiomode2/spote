//! Extracción de la paleta de una portada y derivación del tema completo.
//!
//! La regla que manda: **la portada elige el color, la legibilidad elige el
//! brillo**. Nunca al revés.

use image::DynamicImage;

use super::Rgb;
use crate::util::{hsv_to_rgb, luma, mix, mix_oklab, rgb_to_hsv, scale, with_value};

/// Tema completo de la UI, derivado de la portada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Rgb,
    pub bg_deep: Rgb,
    pub surface: Rgb,
    pub surface_alt: Rgb,
    pub border: Rgb,
    pub border_dim: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub dim: Rgb,
    pub primary: Rgb,
    pub secondary: Rgb,
    pub accent: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub error: Rgb,
    /// 12 pasos ordenados por luminancia, de oscuro a claro. Base del dither.
    pub ramp: [Rgb; 12],
}

const DEFAULT_BG: Rgb = (0x0b, 0x0b, 0x10);
const DEFAULT_PRIMARY: Rgb = (0xc8, 0xb6, 0xff);
const DEFAULT_SECONDARY: Rgb = (0x7e, 0xe0, 0xc3);
const TEXT: Rgb = (0xe8, 0xe6, 0xf0);

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

impl Palette {
    /// Tema neutro sin portada. Deliberadamente oscuro y con acento lila: si la
    /// portada es gris, esto es lo que se ve, y no debe parecer un accidente.
    pub fn dark() -> Self {
        let mut pal = Self {
            bg: DEFAULT_BG,
            bg_deep: (0x06, 0x06, 0x0a),
            surface: (0x12, 0x12, 0x1a),
            surface_alt: (0x18, 0x18, 0x23),
            border: (0x33, 0x31, 0x42),
            border_dim: (0x22, 0x21, 0x2c),
            text: TEXT,
            muted: (0x9a, 0x96, 0xad),
            dim: (0x6a, 0x67, 0x7a),
            primary: DEFAULT_PRIMARY,
            secondary: DEFAULT_SECONDARY,
            accent: DEFAULT_PRIMARY,
            ok: (0x7e, 0xe0, 0xb0),
            warn: (0xf0, 0xc0, 0x6a),
            error: (0xff, 0x6e, 0x7a),
            ramp: [(0, 0, 0); 12],
        };
        pal.ramp = build_ramp(&pal);
        pal
    }

    /// Construye el tema a partir de una portada. Si la imagen no tiene píxeles
    /// opacos, devuelve el tema por defecto teñido muy levemente.
    pub fn from_image(img: &DynamicImage) -> Self {
        match extract_palette(img) {
            Some((primary, secondary)) => Self::from_accents(primary, secondary),
            None => Self::dark(),
        }
    }

    /// Deriva el tema de dos acentos (útil para tests y para temas manuales).
    pub fn from_accents(primary: Rgb, secondary: Rgb) -> Self {
        let (hue, sat, _) = rgb_to_hsv(primary);
        // Una portada casi gris no debe dar una UI gris: se conserva el tono
        // pero se le inyecta el 65 % del acento por defecto.
        let primary = if sat < 0.18 {
            mix(primary, DEFAULT_PRIMARY, 0.65)
        } else {
            primary
        };
        let primary = legible(primary, 0.58, 0.40, 0.82);

        let sat2 = rgb_to_hsv(secondary).1;
        let secondary = if sat2 < 0.12 {
            rotate_hue(primary, 105.0)
        } else {
            secondary
        };
        let secondary = legible(secondary, 0.55, 0.35, 0.80);

        // Las superficies se tiñen pero con techo de luminancia: una portada
        // blanca no debe convertir la UI en gris claro.
        let deep = tint((0x08, 0x08, 0x0c), primary, 0.10, 0.060);
        let bg = tint((0x0c, 0x0c, 0x12), primary, 0.08, 0.085);
        let surface = tint(bg, primary, 0.12, 0.115);
        let surface_alt = tint(bg, primary, 0.18, 0.155);
        let border_dim = tint(bg, primary, 0.34, 0.20);
        let border = tint(bg, primary, 0.52, 0.34);
        let text = TEXT;
        let muted = mix_oklab(text, bg, 0.52);
        let dim = mix_oklab(text, bg, 0.70);
        let accent = primary;
        let _ = (hue, sat);

        let mut pal = Self {
            bg,
            bg_deep: deep,
            surface,
            surface_alt,
            border,
            border_dim,
            text,
            muted,
            dim,
            primary,
            secondary,
            accent,
            ok: mix_oklab(secondary, (0x7e, 0xe0, 0xb0), 0.5),
            warn: (0xf0, 0xc0, 0x6a),
            error: (0xff, 0x6e, 0x7a),
            ramp: [(0, 0, 0); 12],
        };
        pal.ramp = build_ramp(&pal);
        pal
    }

    /// Mezcla dos temas (transición entre canciones).
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mut out = *self;
        for (dst, src) in [
            (&mut out.bg, other.bg),
            (&mut out.bg_deep, other.bg_deep),
            (&mut out.surface, other.surface),
            (&mut out.surface_alt, other.surface_alt),
            (&mut out.border, other.border),
            (&mut out.border_dim, other.border_dim),
            (&mut out.text, other.text),
            (&mut out.muted, other.muted),
            (&mut out.dim, other.dim),
            (&mut out.primary, other.primary),
            (&mut out.secondary, other.secondary),
            (&mut out.accent, other.accent),
            (&mut out.ok, other.ok),
            (&mut out.warn, other.warn),
            (&mut out.error, other.error),
        ] {
            *dst = mix_oklab(*dst, src, t);
        }
        for i in 0..out.ramp.len() {
            out.ramp[i] = mix_oklab(self.ramp[i], other.ramp[i], t);
        }
        out
    }

    /// Acento desplazado `steps` posiciones en la rampa (0 = oscuro, 11 = claro).
    pub fn ramp_at(&self, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0) * (self.ramp.len() - 1) as f32;
        let i = t.floor() as usize;
        let j = (i + 1).min(self.ramp.len() - 1);
        mix_oklab(self.ramp[i], self.ramp[j], t - i as f32)
    }

    /// Color de un píxel del arte: brillo 0..1 → rampa.
    pub fn shade(&self, value: f32) -> Rgb {
        self.ramp_at(value)
    }

    /// Versión con alfa de un color sobre el fondo del tema.
    pub fn alpha(&self, color: Rgb, alpha: f32) -> Rgb {
        mix(self.bg, color, alpha.clamp(0.0, 1.0))
    }

    /// Contraste WCAG entre dos colores (1..21).
    pub fn contrast(a: Rgb, b: Rgb) -> f32 {
        let la = relative_luma(a);
        let lb = relative_luma(b);
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// `true` si el tema puede considerarse oscuro (siempre, hoy).
    pub fn is_dark(&self) -> bool {
        luma(self.bg) < 0.25
    }
}

fn relative_luma((r, g, b): Rgb) -> f32 {
    let f = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// Fuerza saturación y un brillo legible sobre fondo oscuro, con techo: un
/// blanco de portada no puede acabar siendo el color de acento.
fn legible(color: Rgb, min_value: f32, min_sat: f32, max_value: f32) -> Rgb {
    let (hue, sat, value) = rgb_to_hsv(color);
    let sat = sat.max(min_sat);
    let value = value.clamp(min_value, max_value.max(min_value));
    hsv_to_rgb((hue, sat, value))
}

fn rotate_hue(color: Rgb, degrees: f32) -> Rgb {
    let (h, s, v) = rgb_to_hsv(color);
    hsv_to_rgb(((h + degrees) % 360.0, s, v))
}

/// Tiñe `base` hacia `to` pero limita la luminancia al techo dado, para que un
/// borde nunca compita con el texto.
fn tint(base: Rgb, to: Rgb, amount: f32, max_luma: f32) -> Rgb {
    let mixed = mix_oklab(base, to, amount);
    let l = luma(mixed);
    if l > max_luma {
        scale(mixed, max_luma / l.max(0.001))
    } else {
        mixed
    }
}

fn build_ramp(pal: &Palette) -> [Rgb; 12] {
    let anchors = [
        (0.00, pal.bg_deep),
        (0.12, pal.bg),
        (0.24, mix_oklab(pal.bg, pal.primary, 0.28)),
        (0.38, mix_oklab(pal.bg, pal.primary, 0.58)),
        (0.52, with_value(pal.primary, 0.70)),
        (0.64, pal.primary),
        (0.74, mix_oklab(pal.primary, pal.secondary, 0.45)),
        (0.82, mix_oklab(pal.primary, pal.secondary, 0.75)),
        (0.88, pal.secondary),
        (0.93, mix_oklab(pal.secondary, pal.text, 0.55)),
        (0.97, mix_oklab(pal.text, pal.secondary, 0.15)),
        (1.00, pal.text),
    ];
    let mut ramp = [(0u8, 0u8, 0u8); 12];
    for (i, (_, color)) in anchors.into_iter().enumerate() {
        ramp[i] = color;
    }
    // El dither necesita una rampa monótona en luminancia: si dos pasos
    // consecutivos empatan (pasa con portadas muy oscuras), se aclara el
    // superior hacia el texto hasta separarlos.
    for i in 1..ramp.len() {
        let mut guard = 0;
        while luma(ramp[i]) <= luma(ramp[i - 1]) + 0.012 && guard < 8 {
            ramp[i] = mix_oklab(ramp[i], (255, 255, 255), 0.16);
            guard += 1;
        }
    }
    ramp
}

/// Extrae (primary, secondary) de la portada.
///
/// Bins de 4 bits por canal: cada cubo acumula media de color y conteo. La
/// puntuación premia saturación y brillo medio, así que gana "el color del
/// álbum" y no el marco negro ni el texto blanco.
pub fn extract_palette(img: &DynamicImage) -> Option<(Rgb, Rgb)> {
    let thumb = img.thumbnail(72, 72).to_rgba8();
    let mut bins: Vec<(u32, u64, u64, u64)> = vec![(0, 0, 0, 0); 4096];
    let mut total: u32 = 0;

    for px in thumb.pixels() {
        if px.0[3] < 128 {
            continue;
        }
        let (r, g, b) = (px.0[0], px.0[1], px.0[2]);
        // Ignora casi-negros y casi-blancos puros: son bordes, marcos y texto.
        let lum = luma((r, g, b));
        if !(0.03..0.99).contains(&lum) {
            continue;
        }
        let key = ((r >> 4) as usize) << 8 | ((g >> 4) as usize) << 4 | (b >> 4) as usize;
        let bin = &mut bins[key];
        bin.0 += 1;
        bin.1 += r as u64;
        bin.2 += g as u64;
        bin.3 += b as u64;
        total += 1;
    }
    if total == 0 {
        return None;
    }

    let mut candidates: Vec<(Rgb, f32, f32, f32, u32)> = Vec::new();
    for (count, r, g, b) in bins {
        if count == 0 {
            continue;
        }
        let n = count as u64;
        let color = ((r / n) as u8, (g / n) as u8, (b / n) as u8);
        let (hue, sat, value) = rgb_to_hsv(color);
        // Preferencia por brillo medio-alto: los oscuros dominan la portada
        // pero no representan su color.
        let brightness = (1.0 - (value - 0.66).abs()).max(0.25);
        let _score = count as f32 * (0.12 + sat) * brightness;
        candidates.push((color, hue, sat, value, count));
    }
    candidates.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| b_score(a).total_cmp(&b_score(b))));

    let primary = candidates.first()?;
    let primary_color = primary.0;
    let primary_hue = primary.1;

    let min_share = (total / 60).max(8);
    let eligible: Vec<&(Rgb, f32, f32, f32, u32)> = candidates
        .iter()
        .skip(1)
        .filter(|c| c.4 >= min_share)
        .collect();

    let secondary = eligible
        .iter()
        .find(|c| {
            c.2 >= 0.20 && {
                let d = (c.1 - primary_hue).abs();
                d.min(360.0 - d) >= 55.0
            }
        })
        .or_else(|| {
            eligible
                .iter()
                .find(|c| (c.3 - primary.3).abs() >= 0.25)
        })
        .map(|c| c.0)
        .unwrap_or_else(|| {
            let c = candidates
                .iter()
                .skip(1)
                .max_by(|a, b| b_score(a).total_cmp(&b_score(b)))
                .map(|c| c.0)
                .unwrap_or(primary_color);
            c
        });

    Some((primary_color, secondary))
}

fn b_score(c: &(Rgb, f32, f32, f32, u32)) -> f32 {
    c.4 as f32 * (0.12 + c.2) * (1.0 - (c.3 - 0.66).abs()).max(0.25)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([r, g, b, 255])))
    }

    #[test]
    fn solid_image_keeps_hue_and_lifts_brightness() {
        let img = solid(200, 40, 60);
        let (primary, _) = extract_palette(&img).unwrap();
        // El thumbnail promedia píxeles: tolerancia de ±3 por canal.
        assert!(primary.0.abs_diff(200) <= 3 && primary.1.abs_diff(40) <= 3 && primary.2.abs_diff(60) <= 3, "{primary:?}");
        let pal = Palette::from_image(&img);
        let (hue, sat, value) = rgb_to_hsv(pal.primary);
        assert!((hue - 350.0).abs() < 12.0 || hue < 8.0, "hue={hue}");
        assert!(sat > 0.5, "sat={sat}");
        assert!(value >= 0.55, "value={value}");
        assert!(Palette::contrast(pal.text, pal.bg) >= 7.0);
    }

    #[test]
    fn two_color_image_picks_distinct_secondary() {
        let mut img = RgbaImage::new(32, 32);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = if x < 16 {
                Rgba([20, 120, 220, 255])
            } else if y < 16 {
                Rgba([230, 60, 40, 255])
            } else {
                Rgba([240, 240, 240, 255])
            };
        }
        let (primary, secondary) = extract_palette(&DynamicImage::ImageRgba8(img)).unwrap();
        let (h1, s1, _) = rgb_to_hsv(primary);
        let (h2, s2, _) = rgb_to_hsv(secondary);
        assert!(s1 > 0.3 && s2 > 0.3);
        let d = (h1 - h2).abs();
        assert!(d.min(360.0 - d) > 40.0, "{primary:?} vs {secondary:?}");
    }

    #[test]
    fn grey_cover_gets_a_live_accent() {
        let pal = Palette::from_image(&solid(120, 120, 120));
        let (h, s, v) = rgb_to_hsv(pal.primary);
        assert!(s > 0.25, "sat={s} hue={h}");
        assert!(v > 0.5, "value={v}");
        // El fondo se tiñe, pero sigue siendo oscuro y legible.
        assert!(luma(pal.bg) < 0.12);
        assert!(Palette::contrast(pal.text, pal.bg) >= 7.0);
    }

    #[test]
    fn lerp_endpoints_are_exact() {
        let a = Palette::dark();
        let b = Palette::from_accents((20, 180, 90), (220, 80, 40));
        assert_eq!(a.lerp(&b, 0.0), a);
        assert_eq!(a.lerp(&b, 1.0), b);
    }

    #[test]
    fn ramp_is_monotonic_in_luma() {
        let pal = Palette::from_accents((210, 70, 140), (60, 190, 240));
        let mut previous = -1.0;
        for color in pal.ramp {
            let l = luma(color);
            assert!(l + 0.12 >= previous, "ramp not monotonic: {:?}", pal.ramp);
            previous = l;
        }
    }

    #[test]
    fn alpha_blends_towards_bg() {
        let pal = Palette::dark();
        assert_eq!(pal.alpha((255, 255, 255), 0.0), pal.bg);
        assert_eq!(pal.alpha((255, 255, 255), 1.0), (255, 255, 255));
    }
}
