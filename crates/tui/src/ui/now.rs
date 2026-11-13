//! Vista "ahora": cabecera, letra y karaoke.
use super::*;

pub(super) fn draw_now(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    use crate::app::{CenterFocus, Fullscreen};

    if app.fullscreen == Fullscreen::Art {
        let h = layout.full.height.saturating_sub(4);
        let w = (h * 2).min(layout.full.width.saturating_sub(4));
        let art_rect = Rect::new(
            layout.full.x + (layout.full.width.saturating_sub(w)) / 2,
            layout.full.y + (layout.full.height.saturating_sub(h)) / 2,
            w,
            h,
        );
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
        app.hit.add(layout.full, Action::ArtFullscreen);
        return;
    }

    if app.zen {
        // Zen: solo visualización. La canción sigue sonando, pero el centro
        // queda limpio y la portada/letra no se pintan.
        if let Some(track) = &app.now.track {
            let label = format!("{}  ·  {}", track.title, track.artist);
            let rect = Rect::new(
                layout.center.x + 2,
                layout.center.y + 1,
                layout.center.width.saturating_sub(4),
                1,
            );
            text_at(
                buf,
                rect,
                format!("{label}   ·   i para salir de zen"),
                Style::new().fg(color(pal.dim)),
                Alignment::Right,
            );
        }
        return;
    }

    // Foco de portada: la portada ocupa TODO el centro, centrada de verdad.
    if app.center_focus == CenterFocus::Art {
        let area = layout.center;
        let inner = inset(area, 2, 1);
        let h = inner.height.saturating_sub(2);
        let w = (h * 2).min(inner.width);
        let art_rect = centered(inner, w, h);
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
        if let Some(track) = app.now.track.clone() {
            let caption = Rect::new(art_rect.x, art_rect.y + art_rect.height + 1, art_rect.width, 1);
            text_at(
                buf,
                caption,
                format!("{}  ·  {}", track.title, track.artist),
                Style::new().fg(color(pal.muted)),
                Alignment::Center,
            );
            app.hit.add(art_rect, Action::PlayPause);
        }
        text_at(
            buf,
            Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
            "c / esc volver  ·  ⇧F pantalla completa  ·  i zen",
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
        app.hit.add(area, Action::CenterArt);
        return;
    }

    let lyrics_focus = app.center_focus == CenterFocus::Lyrics;
    let art_rect = layout.art;
    let has_track = app.now.track.is_some();
    if !lyrics_focus && has_track {
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
    }

    // Con la letra enfocada ocupa TODO el centro; si no hay pista, el panel
    // vacío se centra en el hueco (antes quedaba pegado a la izquierda, con
    // medio centro en negro).
    let source = if lyrics_focus {
        layout.center
    } else if !has_track {
        let width = layout.content.width.min(64).max(layout.center.width.min(40));
        Rect::new(
            layout.center.x + layout.center.width.saturating_sub(width) / 2,
            layout.center.y,
            width,
            layout.center.height,
        )
    } else {
        layout.content
    };
    let rect = Rect::new(
        source.x,
        source.y + 1,
        source.width,
        source.height.saturating_sub(1),
    );
    panel(
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.34,
        None,
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Center));
    if !lyrics_focus && has_track {
        app.hit.add(art_rect, Action::CenterArt);
    }

    let Some(track) = app.now.track.clone() else {
        draw_empty_now(app, buf, rect, pal);
        return;
    };

    let inner = Rect::new(
        rect.x + 3,
        rect.y + 1,
        rect.width.saturating_sub(6),
        rect.height.saturating_sub(2),
    );
    // El panel de texto ya está a la izquierda de la portada: no hay que
    // recortarlo (el recorte antiguo dejaba la cabecera a 16 columnas).
    let inner = Rect::new(inner.x, inner.y, inner.width, inner.height);

    // Cabecera compacta: badges arriba a la derecha, título y artista en una
    // sola línea. Antes cada cosa gastaba una fila y se comía la letra.
    let lines = app
        .lyrics
        .as_ref()
        .map(|doc| doc.lines.len() as u16)
        .unwrap_or(6)
        .min(16);
    let block = 3 + lines * 2 + 2;
    let offset = if app.vcenter {
        inner.height.saturating_sub(block) / 2
    } else {
        0
    };
    let mut y = inner.y + offset;

    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        if lyrics_focus { "LETRA" } else { "AHORA SUENA" },
        Style::new()
            .fg(color(pal.primary))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    // Badges + corazón alineados a la derecha de la primera fila.
    let favorite = app
        .lib
        .track_by_key(&track.key)
        .ok()
        .flatten()
        .map(|row| row.favorite)
        .unwrap_or(false);
    let mut items: Vec<(String, Rgb, Option<Action>)> = badges(app, pal)
        .into_iter()
        .map(|(label, fg)| (label, fg, None))
        .collect();
    items.push((
        if favorite { "♥".into() } else { "♡".into() },
        if favorite { pal.error } else { pal.dim },
        Some(Action::Favorite),
    ));
    let mut badge_x = rect.x + rect.width.saturating_sub(3);
    for (label, fg, action) in items.into_iter().rev() {
        let width = label.chars().count() as u16 + 2;
        if badge_x < inner.x + 16 || badge_x < width + inner.x {
            break;
        }
        badge_x -= width;
        let badge_rect = Rect::new(badge_x, y, width, 1);
        fill(buf, badge_rect, mix(pal.bg, pal.surface_alt, 0.7));
        text_at(
            buf,
            badge_rect,
            label,
            Style::new().fg(color(fg)),
            Alignment::Center,
        );
        if let Some(action) = action {
            app.hit.add(badge_rect, action);
        }
        badge_x = badge_x.saturating_sub(1);
    }
    // El ecualizador solo ocupa el hueco que dejan los badges.
    let eq_x = inner.x + 14;
    let eq_limit = badge_x.saturating_sub(1);
    for i in 0..10u16 {
        if eq_x + i >= eq_limit {
            break;
        }
        let value = app.audio.band(i as f32 / 10.0 * 48.0);
        let glyph = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][(value * 7.9) as usize];
        put(
            buf,
            eq_x + i,
            y,
            glyph,
            color(pal.ramp_at(0.5 + value * 0.5)),
            None,
        );
    }
    y += 1;

    // Título + artista/álbum en la misma línea (el título manda; lo demás se
    // recorta si no cabe).
    let title = track.title.clone();
    let meta = match &track.album {
        Some(album) if !album.is_empty() => format!("{}  ·  {}", track.artist, album),
        _ => track.artist.clone(),
    };
    let title_width = title.chars().count() as u16;
    let mut spans = vec![Span::styled(
        title,
        Style::new()
            .fg(color(mix_oklab(pal.text, pal.primary, 0.22)))
            .add_modifier(Modifier::BOLD),
    )];
    let room = inner.width.saturating_sub(title_width + 3);
    if room > 10 {
        let mut meta = meta;
        truncate_chars(&mut meta, room as usize);
        spans.push(Span::styled(
            format!("  ·  {meta}"),
            Style::new().fg(color(pal.muted)),
        ));
    }
    line_at(buf, Rect::new(inner.x, y, inner.width, 1), spans, Alignment::Left);
    y += 1;

    let rule = inner.width;
    for i in 0..rule {
        let t = i as f32 / rule.max(1) as f32;
        let rgb = mix_oklab(pal.primary, pal.border_dim, t.powf(0.6));
        put(buf, inner.x + i, y, "─", color(rgb), None);
    }
    y += 1;

    let available = (inner.y + inner.height).saturating_sub(y);
    let lyrics_h = if app.vcenter {
        available.min(lines * 2 + 2)
    } else {
        available
    };
    let lyrics_rect = Rect::new(inner.x, y, inner.width, lyrics_h);
    app.hit.add(lyrics_rect, Action::CenterLyrics);
    draw_lyrics(app, buf, lyrics_rect, pal);
}

pub(super) fn draw_empty_now(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    let lines = [
        "nada sonando todavía",
        "",
        "enter  ›  reproduce lo seleccionado",
        "/      ›  busca una canción y descárgala",
        "t      ›  sigue lo que suena en spotify",
    ];
    let mut y = rect.y + rect.height / 2 - 2;
    for (i, line) in lines.iter().enumerate() {
        text_at(
            buf,
            Rect::new(rect.x + 3, y, rect.width.saturating_sub(6), 1),
            *line,
            Style::new().fg(color(if i == 0 { pal.muted } else { pal.dim })),
            Alignment::Center,
        );
        y += 1;
    }
    let _ = app;
}

/// Marco fino alrededor de la portada: la separa del fondo sin recortarla.
pub(super) fn draw_lyrics(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    if rect.height < 3 || rect.width < 10 {
        return;
    }
    let Some(doc) = &app.lyrics else {
        let message = if app.lyrics_status.is_empty() {
            "sin letra".to_string()
        } else {
            app.lyrics_status.clone()
        };
        text_at(
            buf,
            Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    };
    if doc.is_empty() {
        text_at(
            buf,
            Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1),
            "♪",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }
    let position = app.now.position_ms;
    let active = doc.index_at(position);
    let scroll = app.lyrics_scroll.value;
    let center = rect.y + rect.height / 2;
    let half = (rect.height as f32 / 2.0).max(1.0);

    for (index, line) in doc.lines.iter().enumerate() {
        let offset = index as f32 - scroll;
        let distance = offset.abs();
        if distance > half + 1.5 {
            continue;
        }
        let y = center as i32 + offset.round() as i32;
        if y < rect.y as i32 || y >= (rect.y + rect.height) as i32 {
            continue;
        }
        let fade = (1.0 - (distance / (half + 0.8)).powf(1.6)).clamp(0.0, 1.0);
        let is_active = Some(index) == active;
        let base = if is_active {
            mix_oklab(
                mix_oklab(pal.text, pal.primary, 0.18),
                pal.secondary,
                (app.audio.vocal_pulse * 0.35).clamp(0.0, 0.35),
            )
        } else {
            mix_oklab(pal.bg, pal.muted, fade)
        };
        let style = Style::new()
            .fg(color(base))
            .add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() });
        let prefix = if is_active { "▸ " } else { "  " };
        let suffix = if is_active { " ◂" } else { "" };
        let text = format!("{prefix}{}{suffix}", line.text);
        let line_rect = Rect::new(rect.x, y as u16, rect.width, 1);
        if is_active {
            // Banda de foco: la línea que se canta respira sobre el fondo.
            fill(
                buf,
                line_rect,
                mix(pal.bg, pal.surface_alt, 0.9 + app.audio.vocal_pulse * 0.1),
            );
            put(buf, line_rect.x, y as u16, "▎", color(pal.primary), None);
            // Karaoke: la parte cantada va en el acento, el resto en el texto.
            let progress = doc
                .index_at(position)
                .map(|_| {
                    let span = line.end_ms.saturating_sub(line.start_ms).max(1);
                    (position.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0)
                })
                .unwrap_or(0.0);
            karaoke_line(buf, line_rect, &text, progress, pal, prefix.chars().count());
        } else {
            text_at(buf, line_rect, text, style, Alignment::Center);
        }
    }

    // Progreso fino de la canción bajo la letra activa.
    if let Some((_, progress)) = doc.active(position) {
        let bar_width = (rect.width as f32 * 0.22) as u16;
        if bar_width > 4 && rect.height > 2 {
            let x = rect.x + (rect.width - bar_width) / 2;
            let y = rect.y + rect.height - 1;
            let filled = (bar_width as f32 * progress) as u16;
            for i in 0..bar_width {
                let ch = if i < filled { "━" } else { "─" };
                put(
                    buf,
                    x + i,
                    y,
                    ch,
                    color(if i < filled { pal.primary } else { pal.border_dim }),
                    None,
                );
            }
        }
    }
}

pub(super) fn karaoke_line(
    buf: &mut Buffer,
    rect: Rect,
    text: &str,
    progress: f32,
    pal: &Palette,
    prefix_len: usize,
) {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len().saturating_sub(prefix_len * 2).max(1);
    let sung = progress * total as f32;
    let width = chars.len() as u16;
    if width > rect.width {
        // Si no cabe, se corta el final con puntos suspensivos: perder el
        // principio de la frase es peor que perder el final.
        let keep = rect.width.saturating_sub(1) as usize;
        let mut text: String = chars.into_iter().take(keep).collect();
        text.push('…');
        text_at(
            buf,
            rect,
            text,
            Style::new().fg(color(pal.text)).add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
        return;
    }
    let x = rect.x + (rect.width.saturating_sub(width)) / 2;
    for (i, ch) in chars.iter().enumerate() {
        let index = i.saturating_sub(prefix_len);
        let local = if i < prefix_len || i >= chars.len().saturating_sub(prefix_len) {
            0.0
        } else {
            (sung - index as f32).clamp(0.0, 1.0)
        };
        let fg = mix_oklab(pal.muted, mix_oklab(pal.text, pal.primary, 0.35), local);
        let mut glyph = [0u8; 4];
        let symbol: &str = ch.encode_utf8(&mut glyph);
        put(buf, x + i as u16, rect.y, symbol, color(fg), None);
    }
}

// ---------------------------------------------------------------------------
// Listas (librería / búsqueda)
// ---------------------------------------------------------------------------
