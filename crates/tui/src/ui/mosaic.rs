//! Fondo de portada en mosaico.
//!
//! Se pinta antes que cualquier panel y se compone con alfa sobre la celda:
//! así el mosaico sigue viéndose a través de la UI, de la franja de efectos y
//! de los paneles translúcidos. La baldosa se cachea en el propio `CoverArt`
//! del modo elegido; el frame solo hace lookup y mezcla lineal.

use super::*;

pub(super) fn draw(app: &mut App, buf: &mut Buffer, area: Rect, pal: &Palette) {
    let fx = app.cfg.ui.fx.clone();
    if fx.backdrop == "off" || fx.backdrop == "gradient" {
        return;
    }
    let opacity = (fx.mosaic_opacity.clamp(5, 50) as f32 / 100.0).clamp(0.05, 0.50);
    if app.art_size_hint.0 < 4 || app.art_size_hint.1 < 2 {
        // En vistas sin portada grande (biblioteca, ajustes) el mosaico pide
        // su propia miniatura; no debe depender de que otra vista la pida.
        app.set_art_size_hint(48, 24);
    }
    app.ensure_art();
    app.ensure_ambient_art();
    let Some(art) = app.ambient_art.as_ref().or(app.art.as_ref()) else {
        return;
    };
    if art.is_empty() || art.w == 0 || art.h == 0 {
        return;
    }
    let area_buf = *buf.area();
    let clip = area.intersection(area_buf);
    if clip.width == 0 || clip.height == 0 {
        return;
    }

    let motion = fx.mosaic_motion.as_str();
    let auto = fx.mosaic_auto && motion == "auto";
    let energy = ((app.song_intensity * 0.72 + app.audio.bass * 0.28).clamp(0.0, 1.0)
        * 10.0)
        .round()
        / 10.0;
    // Auto: deriva en calma, filas/columnas al subir y diagonal en el drop.
    let motion = if auto {
        if energy > 0.62 || app.audio.kick_pulse > 0.78 {
            "diagonal"
        } else if energy > 0.28 {
            "rows"
        } else {
            "drift"
        }
    } else {
        motion
    };

    let speed =
        fx.mosaic_speed.clamp(1, 10) as f32 * 0.11 * if auto { 1.0 + energy * 1.8 } else { 1.0 };
    // 30 fps: el mosaico cambia todas las celdas; 60 fps solo hace que la
    // terminal no pueda seguir el ritmo.
    let step = 1.0 / 30.0;
    let t = ((app.mosaic_time / step).floor() * step) * speed;

    // Salto al bombo con histéresis: solo en el ataque, no durante todo el
    // pulso. Evita el parpadeo cuando el valor oscila alrededor del umbral.
    let kick = app.audio.kick_pulse;
    if fx.mosaic_beat {
        if kick > 0.70 && !app.mosaic_beat_hit {
            app.mosaic_beat_hit = true;
            app.mosaic_beat_env = 1.0;
        } else if kick < 0.35 {
            app.mosaic_beat_hit = false;
        }
    } else {
        app.mosaic_beat_hit = false;
        app.mosaic_beat_env = 0.0;
    }
    // Sin desplazamiento de celdas en el beat: era lo que se sentía como
    // terremoto. El golpe se nota en un pulso de brillo muy leve.
    let jump_x = 0.0;
    let jump_y = 0.0;

    let tile_h = match fx.mosaic_tile.as_str() {
        "small" => clip.height as f32 / 6.0,
        "large" => clip.height as f32 / 3.0,
        _ => clip.height as f32 / 4.5,
    }
    .clamp(4.0, 16.0);
    let tile_w = tile_h * 2.0;

    let cx = clip.x as f32 + clip.width as f32 / 2.0;
    let cy = clip.y as f32 + clip.height as f32 / 2.0;
    let angle = if motion == "rotate" {
        0.02 + t * 0.05
    } else {
        0.0
    };
    let (sin, cos) = angle.sin_cos();
    let alpha = opacity;
    let tint = 0.18;
    let width = area_buf.width as usize;
    let tinted_primary = pal.primary;

    for y in clip.y..clip.y + clip.height {
        let start = (y - area_buf.y) as usize * width + (clip.x - area_buf.x) as usize;
        for (offset, cell) in buf.content[start..start + clip.width as usize]
            .iter_mut()
            .enumerate()
        {
            let x = clip.x + offset as u16;
            let mut gx = x as f32;
            let mut gy = y as f32;
            if angle != 0.0 {
                let dx = gx - cx;
                let dy = gy - cy;
                gx = cx + dx * cos - dy * sin;
                gy = cy + dx * sin + dy * cos;
            }
            let mut u = gx / tile_w;
            let mut v = gy / tile_h;
            let row = (gy / tile_h).floor() as i32;
            let column = (gx / tile_w).floor() as i32;
            match motion {
                "rows" => {
                    u += t
                        * (0.45 + (row.rem_euclid(3) as f32) * 0.35)
                        * if row.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
                    v += t * 0.08;
                }
                "columns" => {
                    v += t
                        * (0.45 + (column.rem_euclid(3) as f32) * 0.30)
                        * if column.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
                    u += t * 0.06;
                }
                "diagonal" => {
                    u += t
                        * (0.55 + (row.rem_euclid(3) as f32) * 0.20)
                        * if row.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
                    v += t
                        * (0.38 + (column.rem_euclid(3) as f32) * 0.16)
                        * if column.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
                }
                "rotate" => {
                    u += t * 0.10;
                    v += t * 0.06;
                }
                _ => {
                    u += t * 0.34;
                    v += t * 0.13;
                }
            }
            u += jump_x / tile_w;
            v += jump_y / tile_h;
            let sx = ((u.rem_euclid(1.0)) * art.w as f32) as u16;
            let sy = ((v.rem_euclid(1.0)) * art.h as f32) as u16;
            let Some(src) = art.cell(
                sx.min(art.w.saturating_sub(1)),
                sy.min(art.h.saturating_sub(1)),
            ) else {
                continue;
            };
            let under = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => pal.bg_deep,
            };
            // `half`/`plain` llevan dos colores por celda. El mosaico usa
            // ambos: el half-block de arriba y el de abajo. `dither`/`braille`
            // usan su glifo y un color de tinta.
            let (ch, fg, bg) = if matches!(src.ch, '▀' | '▄') || src.bg.is_some() {
                (
                    '▀',
                    mix(src.fg, tinted_primary, tint),
                    mix(src.bg.unwrap_or(src.fg), tinted_primary, tint),
                )
            } else {
                (src.ch, mix(src.fg, tinted_primary, tint), under)
            };
            let fg = mix(under, fg, alpha);
            let bg = mix(under, bg, alpha);
            let mut glyph = [0u8; 4];
            cell.set_symbol(ch.encode_utf8(&mut glyph));
            cell.set_fg(color(fg));
            cell.set_bg(color(bg));
        }
    }
}
