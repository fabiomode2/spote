//! Portada grande, marco y miniaturas.
use super::*;

/// Placeholder cuando no hay portada: degradado del tema + inicial del artista.
pub(super) fn draw_art_placeholder(app: &App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    for y in 0..rect.height {
        let t = y as f32 / rect.height.max(1) as f32;
        for x in 0..rect.width {
            let nx = x as f32 / rect.width.max(1) as f32;
            let value = (0.16 + t * 0.22 + (nx * 2.0 + t).sin().abs() * 0.06).min(0.55);
            put_bg(buf, rect.x + x, rect.y + y, pal.ramp_at(value));
        }
    }
    let initial = app
        .now
        .track
        .as_ref()
        .and_then(|track| track.artist.chars().next())
        .unwrap_or('♪');
    let cy = rect.y + rect.height / 2;
    let cx = rect.x + rect.width / 2;
    let mut glyph = [0u8; 4];
    let symbol: &str = initial
        .to_uppercase()
        .next()
        .unwrap_or(initial)
        .encode_utf8(&mut glyph);
    put(
        buf,
        cx.saturating_sub(1),
        cy,
        symbol,
        color(pal.text),
        Some(color(pal.bg_deep)),
    );
}

/// Marco fino alrededor de la portada: la separa del fondo sin recortarla.
/// Color real del borde de la portada. `t` recorre el perímetro 0..1; se
/// muestrea la celda más cercana del `CoverArt` actual, no la paleta generada.
pub(super) fn cover_edge_color(app: &App, t: f32) -> Option<Rgb> {
    let art = app
        .art
        .as_ref()
        .filter(|art| !art.is_empty())
        .or_else(|| app.ambient_art.as_ref().filter(|art| !art.is_empty()))?;
    let w = art.w as f32;
    let h = art.h as f32;
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let perimeter = 2.0 * (w + h);
    let mut p = t.rem_euclid(1.0) * perimeter;
    let (x, y, bottom) = if p < w {
        (p, 0.0, false)
    } else if p < w + h {
        p -= w;
        (w - 1.0, p, false)
    } else if p < 2.0 * w + h {
        p -= w + h;
        (w - 1.0 - p, h - 1.0, true)
    } else {
        p -= 2.0 * w + h;
        (0.0, h - 1.0 - p, false)
    };
    let cell = art.cell(
        (x.round() as u16).min(art.w.saturating_sub(1)),
        (y.round() as u16).min(art.h.saturating_sub(1)),
    )?;
    Some(if bottom {
        cell.bg.unwrap_or(cell.fg)
    } else {
        cell.fg
    })
}

pub(super) fn frame_art(app: &App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width - 1, rect.y + rect.height - 1);
    if app.cfg.ui.panel_border == "none" {
        return;
    }
    if matches!(app.cfg.ui.panel_border.as_str(), "rim" | "color") {
        let time = app.mosaic_time;
        let strength = app.cfg.ui.rim_intensity.clamp(0, 100) as f32 / 100.0;
        let reactive = app.cfg.ui.rim_reactive;
        let amount = if reactive {
            (0.35 + app.audio.kick_pulse * 0.65).clamp(0.0, 1.0)
        } else {
            0.55
        };
        let border_opacity = app.cfg.ui.border_opacity.clamp(0, 100) as f32 / 100.0;
        let perimeter = 2 * (rect.width as usize + rect.height as usize).max(1);
        let mut index = 0usize;
        let mut paint = |x: u16, y: u16| {
            let t = index as f32 / perimeter as f32;
            let wave = (t * std::f32::consts::TAU * 3.0 + time * 1.9).sin() * 0.5 + 0.5;
            let base = cover_edge_color(app, t)
                .unwrap_or_else(|| mix(pal.primary, pal.secondary, wave));
            let lit = mix(
                pal.bg_deep,
                base,
                (0.65 + amount * 0.35) * strength.clamp(0.15, 1.0),
            );
            let lit = mix(pal.bg_deep, lit, border_opacity);
            put(buf, x, y, "█", color(lit), None);
            index += 1;
        };
        for x in x0..=x1 {
            paint(x, y0);
        }
        for y in (y0 + 1)..=y1 {
            paint(x1, y);
        }
        if y1 > y0 {
            for x in (x0..x1).rev() {
                paint(x, y1);
            }
        }
        if x1 > x0 {
            for y in ((y0 + 1)..y1).rev() {
                paint(x0, y);
            }
        }
        return;
    }
    let fg = color(mix(
        pal.bg_deep,
        pal.border,
        app.cfg.ui.border_opacity.clamp(0, 100) as f32 / 100.0,
    ));
    for x in x0..=x1 {
        put(buf, x, y0, "─", fg, None);
        put(buf, x, y1, "─", fg, None);
    }
    for y in y0..=y1 {
        put(buf, x0, y, "│", fg, None);
        put(buf, x1, y, "│", fg, None);
    }
    for (x, y, glyph) in [(x0, y0, "╭"), (x1, y0, "╮"), (x0, y1, "╰"), (x1, y1, "╯")] {
        put(buf, x, y, glyph, fg, None);
    }
}

pub(super) fn badges(app: &App, pal: &Palette) -> Vec<(String, Rgb)> {
    let mut out = Vec::new();
    if app.room.is_some() {
        out.push(("sala".into(), pal.primary));
    }
    if app.follow_spotify {
        out.push(("mpris".into(), pal.secondary));
    } else if let Some(provider) = app
        .now
        .track
        .as_ref()
        .and_then(|track| track.source_hint.as_ref())
        .map(|hint| hint.provider.clone())
    {
        out.push((provider, pal.secondary));
    } else {
        out.push(("local".into(), pal.secondary));
    }
    if let Some(doc) = &app.lyrics {
        if doc.source.is_generated() {
            out.push(("letra inventada".into(), pal.warn));
        } else if doc.synced {
            out.push(("sincronizada".into(), pal.primary));
        } else if !doc.is_empty() {
            out.push(("aprox".into(), pal.warn));
        }
    }
    if app.lyrics_offset_ms != 0 {
        out.push((format!("offset {:+}ms", app.lyrics_offset_ms), pal.warn));
    }
    if !app.lyrics_follow {
        out.push(("letra manual".into(), pal.warn));
    }
    if app.trim_start_ms > 0 || app.trim_end_ms.is_some() {
        let start = if app.trim_start_ms > 0 {
            spote_core::util::format_ms(app.trim_start_ms)
        } else {
            "0:00".to_string()
        };
        let end = app
            .trim_end_ms
            .map(spote_core::util::format_ms)
            .unwrap_or_else(|| "fin".to_string());
        out.push((format!("recorte {start}–{end}"), pal.warn));
    }
    if app.now.loading || app.pending_play.is_some() {
        out.push(("descargando".into(), pal.warn));
    }
    if app.now.playing {
        out.push(("●".into(), pal.ok));
    } else if app.now.track.is_some() {
        out.push(("⏸".into(), pal.dim));
    }
    out
}

pub(super) fn draw_art(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette, strength: f32) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    // El tamaño real manda: en foco de portada o pantalla completa el rect es
    // más grande que la miniatura del layout. Sin esto se estiraba una portada
    // pequeña y el braille se veía como estática.
    app.set_art_size_hint(rect.width, rect.height);
    app.ensure_art();
    let mode = app.art_mode;
    if mode == ArtMode::Plain && app.art.is_some() {
        // El backend de imágenes (Kitty) coloca la foto real encima del
        // half-block; en terminales sin soporte se queda el fallback.
        app.art_visible_rect = Some(rect);
    }
    if mode == ArtMode::Off || mode == ArtMode::Gradient || app.art.is_none() {
        draw_art_placeholder(app, buf, rect, pal);
        return;
    }
    let Some(art) = app.art.as_ref() else {
        draw_art_placeholder(app, buf, rect, pal);
        return;
    };
    let intro = ease_out_cubic(app.intro);
    let plain = mode == ArtMode::Plain;
    // La portada respira con el bombo: integra el audio en toda la UI. El modo
    // `plain` es la foto literal: ni pulso ni tinte de la paleta.
    let pulse = if plain {
        1.0
    } else {
        1.0 + app.audio.kick * 0.06 + app.audio.bass * 0.035
    };
    let gain = if plain {
        1.0
    } else if strength > 0.95 {
        1.28 * pulse
    } else {
        1.0
    };
    let blend = if app.art_fade_active {
        app.art_fade
    } else {
        1.0
    };
    let previous = if app.art_fade_active {
        app.art_prev.as_ref()
    } else {
        None
    };
    // Dither y braille en grande se leen como ruido: se muestrean a 2x2 celdas
    // para que el patrón sea grueso y se vea como pixel art, no como estática.
    // El pixelado grueso solo tiene sentido en dither. Braille necesita cada
    // subpíxel: muestrearlo a 2x2 lo convierte en ruido.
    let chunky = matches!(mode, ArtMode::Dither) && rect.width >= 60;
    let factor: u32 = if chunky { 2 } else { 1 };
    for y in 0..rect.height {
        for x in 0..rect.width {
            let sx = ((x as u32 / factor) * art.w as u32
                / (rect.width / factor as u16).max(1) as u32) as u16;
            let sy = ((y as u32 / factor) * art.h as u32
                / (rect.height / factor as u16).max(1) as u32) as u16;
            let Some(cell) = art.cell(
                sx.min(art.w.saturating_sub(1)),
                sy.min(art.h.saturating_sub(1)),
            ) else {
                continue;
            };
            let mut ch = cell.ch;
            let mut fg = cell.fg;
            let mut bg = cell.bg;
            if let Some(previous) = previous {
                let psx = ((x as u32 / factor) * previous.w as u32
                    / (rect.width / factor as u16).max(1) as u32) as u16;
                let psy = ((y as u32 / factor) * previous.h as u32
                    / (rect.height / factor as u16).max(1) as u32) as u16;
                if let Some(old) = previous.cell(
                    psx.min(previous.w.saturating_sub(1)),
                    psy.min(previous.h.saturating_sub(1)),
                ) {
                    // Disolución ordenada: el umbral de Bayer hace que el cambio
                    // se vea como una transición, no como un parpadeo.
                    const BAYER: [[f32; 4]; 4] = [
                        [0.0, 8.0, 2.0, 10.0],
                        [12.0, 4.0, 14.0, 6.0],
                        [3.0, 11.0, 1.0, 9.0],
                        [15.0, 7.0, 13.0, 5.0],
                    ];
                    let threshold = (BAYER[(y % 4) as usize][(x % 4) as usize] + 0.5) / 16.0;
                    if blend < threshold {
                        ch = old.ch;
                        fg = mix_oklab(old.fg, cell.fg, blend * 0.6);
                        bg = old.bg.or(cell.bg).map(|color| {
                            let other = old.bg.unwrap_or(color);
                            mix_oklab(other, color, blend)
                        });
                    }
                }
            }
            let fg = match mode {
                ArtMode::Half | ArtMode::Plain => fg,
                _ => pal.ramp_at(luma(fg)),
            };
            let bg = bg.map(|bg| match mode {
                ArtMode::Half | ArtMode::Plain => bg,
                _ => pal.ramp_at(luma(bg)),
            });
            let fg = if plain {
                scale(fg, strength)
            } else {
                scale(mix_oklab(pal.bg_deep, fg, intro * strength), gain)
            };
            let ch = if ch == '\0' { ' ' } else { ch };
            let mut glyph = [0u8; 4];
            let symbol: &str = ch.encode_utf8(&mut glyph);
            put(
                buf,
                rect.x + x,
                rect.y + y,
                symbol,
                color(fg),
                bg.map(|bg| {
                    color(if plain {
                        scale(bg, strength)
                    } else {
                        scale(mix_oklab(pal.bg_deep, bg, intro * strength), gain)
                    })
                }),
            );
        }
    }
}

pub(super) fn draw_thumb(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    track: &spote_protocol::TrackRef,
    pal: &Palette,
) {
    if rect.width < 4 || rect.height < 2 {
        return;
    }
    let mode = app.thumb_mode;
    if mode == ThumbMode::Off {
        // Sin miniatura: un bloque con la inicial, como el placeholder.
        fill(buf, rect, mix(pal.surface, pal.bg_deep, 0.4));
        let ch = track
            .artist
            .chars()
            .next()
            .unwrap_or('♪')
            .to_uppercase()
            .next()
            .unwrap_or('♪');
        put(
            buf,
            rect.x + rect.width / 2,
            rect.y + rect.height / 2,
            &ch.to_string(),
            color(pal.dim),
            None,
        );
        return;
    }

    // La miniatura exacta manda; si aún no llegó, se estira la mayor que
    // haya mientras se pide a la resolución del hueco.
    let exact = app.thumb_for(&track.key, rect.width, rect.height);
    let sufficient = exact.is_some();
    let art = exact.or_else(|| app.thumb_any(&track.key));
    if let Some(art) = art {
        for y in 0..rect.height {
            for x in 0..rect.width {
                let sx = (x as u32 * art.w as u32 / rect.width.max(1) as u32) as u16;
                let sy = (y as u32 * art.h as u32 / rect.height.max(1) as u32) as u16;
                let Some(cell) = art.cell(sx, sy) else {
                    continue;
                };
                let (fg, bg) = if mode == ThumbMode::Plain {
                    (cell.fg, cell.bg.unwrap_or(pal.bg_deep))
                } else {
                    (
                        pal.ramp_at(luma(cell.fg)),
                        cell.bg
                            .map(|bg| pal.ramp_at(luma(bg)))
                            .unwrap_or(pal.bg_deep),
                    )
                };
                let mut glyph = [0u8; 4];
                let symbol: &str = cell.ch.encode_utf8(&mut glyph);
                put(
                    buf,
                    rect.x + x,
                    rect.y + y,
                    symbol,
                    color(fg),
                    Some(color(bg)),
                );
            }
        }
    } else {
        // Placeholder de color estable por artista mientras llega el arte.
        let seed = spote_core::util::short_hash(&track.artist);
        let base = pal.ramp_at(
            0.45 + (u32::from_str_radix(&seed[..2], 16).unwrap_or(0) as f32 / 255.0) * 0.3,
        );
        for y in 0..rect.height {
            for x in 0..rect.width {
                put_bg(buf, rect.x + x, rect.y + y, base);
            }
        }
        let ch = track
            .artist
            .chars()
            .next()
            .unwrap_or('♪')
            .to_uppercase()
            .next()
            .unwrap_or('♪');
        let mut glyph = [0u8; 4];
        let symbol: &str = ch.encode_utf8(&mut glyph);
        put(
            buf,
            rect.x + rect.width / 2 - 1,
            rect.y + rect.height / 2,
            symbol,
            color(pal.text),
            None,
        );
    }

    // En Kitty la miniatura se coloca como imagen real; el half-block queda
    // de respaldo debajo. La clave incluye la posición para distinguir la
    // misma pista repetida en pantalla.
    app.hi_thumbs.push((
        format!("{}@{}:{}", track.key, rect.x, rect.y),
        track.key.clone(),
        rect,
    ));

    // Las portadas de álbum las pide su vista, que conoce la URL aunque la
    // pista no esté en la librería. El resto se pide aquí, una vez por hueco.
    let album = track.key.starts_with("album:") || track.key.starts_with("albumpage:");
    if !sufficient && !album {
        app.request_thumb(track, rect.width, rect.height);
    }
}

/// Pinta un `CoverArt` ya cacheado (perfil de artista, cabeceras...).
pub(super) fn draw_cached_art(
    buf: &mut Buffer,
    rect: Rect,
    art: &CoverArt,
    pal: &Palette,
    plain: bool,
) {
    if rect.width < 2 || rect.height < 2 || art.is_empty() {
        return;
    }
    for y in 0..rect.height {
        for x in 0..rect.width {
            let sx = (x as u32 * art.w as u32 / rect.width.max(1) as u32) as u16;
            let sy = (y as u32 * art.h as u32 / rect.height.max(1) as u32) as u16;
            let Some(cell) = art.cell(
                sx.min(art.w.saturating_sub(1)),
                sy.min(art.h.saturating_sub(1)),
            ) else {
                continue;
            };
            let (fg, bg) = if plain {
                (cell.fg, cell.bg.unwrap_or(pal.bg_deep))
            } else {
                (
                    pal.ramp_at(luma(cell.fg)),
                    cell.bg
                        .map(|bg| pal.ramp_at(luma(bg)))
                        .unwrap_or(pal.bg_deep),
                )
            };
            let mut glyph = [0u8; 4];
            let symbol: &str = cell.ch.encode_utf8(&mut glyph);
            put(
                buf,
                rect.x + x,
                rect.y + y,
                symbol,
                color(fg),
                Some(color(bg)),
            );
        }
    }
}
