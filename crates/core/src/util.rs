//! Utilidades compartidas: rutas, formato de tiempo, normalización y claves.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha1::{Digest, Sha1};

/// `mm:ss` (o `h:mm:ss` si hace falta).
pub fn format_ms(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `mm:ss` con décimas, para el tiempo bajo el cursor al hacer seek.
pub fn format_ms_precise(ms: u64) -> String {
    let total = ms / 100;
    let (m, s) = (total / 600, (total % 600) / 10);
    let d = total % 10;
    format!("{m}:{s:02}.{d}")
}

/// Normaliza texto para comparar canciones: minúsculas, sin acentos, sin
/// sufijos de vídeo/remaster y sin puntuación.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        // Genius devuelve a veces los acentos descompuestos (n + tilde
        // combinante). Si no se descartan, "Karin\u{303}osa" no casa con
        // "Kariñosa": el título se convierte en "karin osa".
        if ('\u{0300}'..='\u{036f}').contains(&ch) {
            continue;
        }
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            _ => {
                let ch = deaccent(ch);
                if ch.is_alphanumeric() || ch == ' ' {
                    out.extend(ch.to_lowercase());
                } else if ch == '&' {
                    out.push_str(" and ");
                } else {
                    out.push(' ');
                }
            }
        }
    }
    let lower = out.to_lowercase();
    let mut cleaned: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    for noise in [
        " official video",
        " official music video",
        " official audio",
        " lyric video",
        " lyrics",
        " audio",
        " video",
        " remaster",
        " remastered",
        " hd",
        " hq",
        " m v",
    ] {
        cleaned = cleaned.replace(noise, "");
    }
    cleaned.trim().to_string()
}

fn deaccent(ch: char) -> char {
    match ch {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

/// Clave canónica de respaldo cuando no hay ISRC: `sha1:artist|title|segundos`.
pub fn fallback_key(artist: &str, title: &str, duration_ms: u64) -> String {
    let seconds = duration_ms / 1000;
    let raw = format!(
        "{}|{}|{}",
        normalize(artist),
        normalize(title),
        seconds
    );
    format!("sha1:{}", hex_sha1(raw.as_bytes()))
}

pub fn hex_sha1(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(40);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn short_hash(text: &str) -> String {
    hex_sha1(text.as_bytes())[..8].to_string()
}

/// `true` si el binario existe en PATH.
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

/// Devuelve una ruta segura (sin `..` ni separadores) para usar como nombre.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= max {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            // Cualquier separador (espacio, /, &, puntuación) colapsa a '-'.
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Extensión de audio reconocida (minúsculas, sin punto).
pub fn audio_ext(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ["opus", "ogg", "flac", "mp3", "m4a", "aac", "wav", "webm"]
        .into_iter()
        .find(|known| *known == ext)
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn duration_hms(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}h {:02}m", total / 3600, (total % 3600) / 60)
}

pub fn elapsed(start: std::time::Instant) -> Duration {
    start.elapsed()
}

// ---------------------------------------------------------------------------
// Color helpers (compartidos entre artwork y TUI)
// ---------------------------------------------------------------------------

pub fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

/// Mezcla dos colores RGB.
pub fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        lerp_u8(a.0, b.0, t),
        lerp_u8(a.1, b.1, t),
        lerp_u8(a.2, b.2, t),
    )
}

pub fn scale(c: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    (
        (c.0 as f32 * factor).clamp(0.0, 255.0) as u8,
        (c.1 as f32 * factor).clamp(0.0, 255.0) as u8,
        (c.2 as f32 * factor).clamp(0.0, 255.0) as u8,
    )
}

pub fn luma((r, g, b): (u8, u8, u8)) -> f32 {
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

/// Sube/baja el brillo conservando el tono.
pub fn with_value(c: (u8, u8, u8), value: f32) -> (u8, u8, u8) {
    let (h, s, _) = rgb_to_hsv(c);
    hsv_to_rgb((h, s, value.clamp(0.0, 1.0)))
}

pub fn rgb_to_hsv((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let sat = if max <= f32::EPSILON { 0.0 } else { delta / max };
    (hue, sat, max)
}

pub fn hsv_to_rgb((h, s, v): (f32, f32, f32)) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).clamp(0.0, 255.0) as u8,
    )
}

/// Interpolación perceptual aproximada (OKLab). Mucho mejor que RGB para
/// degradados: evita el gris sucio en los medios.
pub fn mix_oklab(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let la = rgb_to_oklab(a);
    let lb = rgb_to_oklab(b);
    let t = t.clamp(0.0, 1.0);
    oklab_to_rgb((
        la.0 + (lb.0 - la.0) * t,
        la.1 + (lb.1 - la.1) * t,
        la.2 + (lb.2 - la.2) * t,
    ))
}

fn rgb_to_oklab((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

fn oklab_to_rgb((l, a, b): (f32, f32, f32)) -> (u8, u8, u8) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;
    let gamma = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    };
    (
        (gamma(r) * 255.0).round().clamp(0.0, 255.0) as u8,
        (gamma(g) * 255.0).round().clamp(0.0, 255.0) as u8,
        (gamma(b) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(format_ms(0), "0:00");
        assert_eq!(format_ms(83_000), "1:23");
        assert_eq!(format_ms(3_723_000), "1:02:03");
    }

    #[test]
    fn normalizes_decomposed_accents() {
        assert_eq!(normalize("Karin\u{303}osa"), normalize("Kariñosa"));
    }

    #[test]
    fn normalizes_noise_and_accents() {
        assert_eq!(
            normalize("Déjà Vu (Official Video) [HD]"),
            "deja vu"
        );
        assert_eq!(normalize("AC/DC"), "ac dc");
    }

    #[test]
    fn fallback_key_is_stable() {
        let a = fallback_key("Arctic Monkeys", "Do I Wanna Know?", 272_000);
        let b = fallback_key("arctic monkeys", "do i wanna know", 272_500);
        assert_eq!(a, b);
        assert!(a.starts_with("sha1:"));
    }

    #[test]
    fn oklab_roundtrip_is_close() {
        for color in [(0, 0, 0), (255, 255, 255), (200, 30, 40), (18, 120, 210)] {
            let (l, a, b) = rgb_to_oklab(color);
            let back = oklab_to_rgb((l, a, b));
            assert!(
                back.0.abs_diff(color.0) <= 2
                    && back.1.abs_diff(color.1) <= 2
                    && back.2.abs_diff(color.2) <= 2,
                "{color:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn slug_is_path_safe() {
        assert_eq!(slug("AC/DC & Friends", 20), "ac-dc-friends");
        assert_eq!(slug("../../etc/passwd", 40), "etc-passwd");
    }
}
