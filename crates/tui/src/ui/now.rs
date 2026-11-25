//! Vista "ahora": cabecera, letra y karaoke.
use super::*;

pub(super) fn draw_now(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    use crate::app::{CenterFocus, Fullscreen};

    if app.fullscreen == Fullscreen::Art {
        // Pantalla completa de verdad, pero por encima del transporte: si la
        // portada ocupa también esas filas, la imagen Kitty tapa los botones.
        let area = layout.main;
        let inner = inset(area, 2, 1);
        let h = inner.height.saturating_sub(1);
        let w = (h * 2).min(inner.width);
        let art_rect = centered(inner, w, h);
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
            let caption = Rect::new(
                art_rect.x,
                art_rect.y + art_rect.height + 1,
                art_rect.width,
                1,
            );
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
        let width = layout
            .content
            .width
            .min(64)
            .max(layout.center.width.min(40));
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
        Some(app),
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
    let trim_label = if app.trim_start_ms > 0 || app.trim_end_ms.is_some() {
        let start = if app.trim_start_ms > 0 {
            spote_core::util::format_ms(app.trim_start_ms)
        } else {
            "0:00".to_string()
        };
        let end = app
            .trim_end_ms
            .map(spote_core::util::format_ms)
            .unwrap_or_else(|| "fin".to_string());
        format!("✂ {start}–{end}")
    } else {
        "✂ recorte".to_string()
    };
    items.push((trim_label, pal.warn, Some(Action::OpenTrim)));
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
    // recorta si no cabe). Artista y álbum se pintan por separado para que
    // cada uno tenga su hitbox y su resaltado animado.
    let title = track.title.clone();
    let title_width = title.chars().count() as u16;
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        title,
        Style::new()
            .fg(color(mix_oklab(pal.text, pal.primary, 0.22)))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    let room = inner.width.saturating_sub(title_width + 3);
    if room > 6 {
        let meta_x = inner.x + title_width + 3;
        let mut artist = track.artist.clone();
        truncate_chars(&mut artist, room as usize);
        let artist_w = artist.chars().count() as u16;
        let mut album = track
            .album
            .as_deref()
            .filter(|album| !album.is_empty())
            .map(str::to_string);
        let sep = "  ·  ";
        let sep_w = sep.chars().count() as u16;
        let album_room = room.saturating_sub(artist_w + sep_w);
        if album
            .as_ref()
            .is_some_and(|name| name.chars().count() as u16 > album_room)
        {
            if album_room > 2 {
                if let Some(name) = album.as_mut() {
                    truncate_chars(name, album_room as usize);
                }
            } else {
                album = None;
            }
        }
        if artist_w > 1 {
            let hover = app.hover == Some(Action::OpenArtist(track.artist.clone()));
            link_at(
                buf,
                meta_x,
                y,
                artist_w,
                &artist,
                pal,
                pal.secondary,
                hover,
                app.ui_time(),
            );
            app.hit.add(
                Rect::new(meta_x, y, artist_w, 1),
                Action::OpenArtist(track.artist.clone()),
            );
        }
        if let Some(album) = album {
            let sep_x = meta_x + artist_w;
            put(buf, sep_x, y, sep, color(pal.dim), None);
            let album_x = sep_x + sep_w;
            let album_w = album.chars().count() as u16;
            let hover =
                app.hover == Some(Action::OpenAlbum(album.clone(), track.artist.clone()));
            let drawn = link_at(
                buf,
                album_x,
                y,
                album_w,
                &album,
                pal,
                pal.muted,
                hover,
                app.ui_time(),
            );
            if drawn > 1 {
                app.hit.add(
                    Rect::new(album_x, y, drawn, 1),
                    Action::OpenAlbum(album, track.artist.clone()),
                );
            }
        }
    }
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
    app.lyrics_area = rect;
    if app.lyrics_search.is_some() {
        draw_lyrics_search(app, buf, rect, pal);
        return;
    }
    let Some(doc) = &app.lyrics else {
        let message = if app.lyrics_status.is_empty() || app.lyrics_status == "sin letra" {
            if app.cfg.lyrics.allow_generated {
                "sin letra".to_string()
            } else {
                "sin letra verificada".to_string()
            }
        } else {
            app.lyrics_status.clone()
        };
        text_at(
            buf,
            Rect::new(rect.x, rect.y + rect.height / 2 - 1, rect.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        let label = "⌕ buscar letra a mano";
        let width = label.chars().count() as u16 + 2;
        if rect.width > width + 4 && rect.height > 4 {
            let chip = Rect::new(
                rect.x + (rect.width.saturating_sub(width)) / 2,
                rect.y + rect.height / 2 + 1,
                width,
                1,
            );
            let hover = app.hover == Some(Action::LyricsSearchOpen);
            fill(
                buf,
                chip,
                if hover {
                    pal.surface_alt
                } else {
                    mix(pal.surface, pal.primary, 0.22)
                },
            );
            text_at(
                buf,
                chip,
                label,
                Style::new().fg(color(pal.text)),
                Alignment::Center,
            );
            app.hit.add(chip, Action::LyricsSearchOpen);
        }
        // El toggle también debe estar disponible cuando no hay letra: es
        // justo el momento en el que se quiere probar a generarla.
        if rect.height > 5 {
            let inv_width = 5u16;
            let inv = Rect::new(
                rect.x + (rect.width.saturating_sub(inv_width)) / 2,
                rect.y + rect.height / 2 + 2,
                inv_width,
                1,
            );
            let hover = app.hover == Some(Action::LyricsToggleGenerated);
            let active = app.cfg.lyrics.allow_generated;
            fill(
                buf,
                inv,
                if hover {
                    pal.surface_alt
                } else if active {
                    mix(pal.surface, pal.primary, 0.32)
                } else {
                    pal.surface
                },
            );
            text_at(
                buf,
                inv,
                "inv",
                Style::new()
                    .fg(color(if active { pal.text } else { pal.muted }))
                    .add_modifier(Modifier::BOLD),
                Alignment::Center,
            );
            app.hit.add(inv, Action::LyricsToggleGenerated);
        }
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

    // Barra de controles de la letra: zoom, ajuste fino del offset (motor de
    // sync real, persistido por pista), scroll manual y vuelta a la línea
    // actual. Solo ocupa una fila.
    let toolbar_h = if rect.height >= 8 && rect.width >= 34 {
        1
    } else {
        0
    };
    if toolbar_h > 0 {
        let bar = Rect::new(rect.x, rect.y, rect.width, 1);
        draw_lyrics_toolbar(app, buf, bar, pal);
    }
    let rect = Rect::new(
        rect.x,
        rect.y + toolbar_h,
        rect.width,
        rect.height.saturating_sub(toolbar_h),
    );
    let doc = app.lyrics.as_ref().expect("la letra estaba cargada");
    // Tamaño: no se puede cambiar la fuente de la terminal, pero sí el ancho
    // de columna y el aire entre líneas. "Grande" = menos palabras por línea,
    // más altura y negrita, que es lo que se percibe como letra grande.
    let size = app.cfg.ui.lyrics_size.min(2);
    let line_h = 1 + size as u16;
    let pct = match size {
        2 => 74,
        1 => 88,
        _ => 100,
    };
    let text_w = (rect.width * pct / 100).max(24).min(rect.width);
    let x0 = rect.x + (rect.width.saturating_sub(text_w)) / 2;
    // Un único reloj para todo el dibujo de letras: la posición de canción ya
    // corregida por el offset. Mezclar `position` para el karaoke con
    // `index_at` (que suma el offset) hacía cambiar de línea a mitad de frase.
    let lyric_ms = doc.aligned_ms(app.display_position_ms());
    let active = doc.index_at_target(lyric_ms);
    let scroll = app.lyrics_scroll.value;
    let center = rect.y as f32 + rect.height as f32 / 2.0;
    let half = (rect.height as f32 / (line_h as f32 * 2.0)).max(1.0);

    // Las frases largas ocupan hasta dos filas. Se reservan dos columnas a
    // cada lado para el prefijo/sufijo del karaoke; así el alto de cada línea
    // es estable y el resto del bloque se coloca contando filas, no índices.
    let wrap_width = text_w.saturating_sub(4).max(8) as usize;
    let wrapped: Vec<Vec<String>> = doc
        .lines
        .iter()
        .map(|line| wrap_text(&line.text, wrap_width))
        .collect();
    let heights: Vec<u16> = wrapped
        .iter()
        .map(|rows| rows.len().max(1) as u16 * line_h)
        .collect();
    let active_index = active.unwrap_or_else(|| {
        (scroll.round().max(0.0) as usize).min(doc.lines.len().saturating_sub(1))
    });
    let active_h = heights.get(active_index).copied().unwrap_or(line_h);
    // El bloque activo se centra alrededor del mismo punto que antes: una
    // línea normal queda donde estaba y una de dos filas crece hacia arriba.
    let anchor = center - (active_h as f32 - line_h as f32) / 2.0;
    let bottom = rect.y as i32 + rect.height as i32;

    for (index, line) in doc.lines.iter().enumerate() {
        let distance = (index as f32 - scroll).abs();
        if distance > half + 2.0 {
            continue;
        }
        let y = anchor + relative_lyric_top(index, scroll, &heights);
        let top = y.round() as i32;
        let height = heights.get(index).copied().unwrap_or(line_h).max(line_h);
        if top < rect.y as i32 || top >= bottom {
            continue;
        }
        let fade = (1.0 - (distance / (half + 0.8)).powf(1.6)).clamp(0.0, 1.0);
        let is_active = Some(index) == active;
        let visible_h = height.min((bottom - top) as u16).max(1);
        let line_rect = Rect::new(x0, top as u16, text_w, visible_h);
        if is_active {
            fill(
                buf,
                line_rect,
                mix(pal.bg, pal.surface_alt, 0.92 + app.audio.vocal_pulse * 0.08),
            );
            for row in 0..(visible_h / line_h).max(1) {
                put(
                    buf,
                    line_rect.x,
                    line_rect.y + row * line_h,
                    "▎",
                    color(pal.primary),
                    None,
                );
            }
            draw_active_line(buf, line_rect, line, lyric_ms, pal, line_h);
        } else {
            let base = mix_oklab(pal.bg, pal.muted, fade);
            let style = Style::new().fg(color(base));
            for (row, text) in wrapped[index].iter().enumerate() {
                let row_y = top + row as i32 * line_h as i32;
                if row_y < rect.y as i32 || row_y >= bottom {
                    continue;
                }
                text_at(
                    buf,
                    Rect::new(x0, row_y as u16, text_w, 1),
                    text.clone(),
                    style,
                    Alignment::Center,
                );
            }
        }
        // Clic en una frase: salta a ese momento. El muelle de scroll anima el
        // salto; no hay corte seco.
        // El seek es tiempo de canción; la marca es tiempo de documento.
        let seek_ms = (line.start_ms as i64 - doc.offset_ms).max(0) as u64;
        app.hit.add(line_rect, Action::SeekLyrics(seek_ms));
        // El clic derecho también sirve para sincronizar; no abre el menú
        // contextual de la pista desde una línea de letra.
    }

    // Progreso de la línea activa, discreto, bajo la letra.
    if let Some((_, progress)) = doc.active_at_target(lyric_ms) {
        let bar_width = (rect.width as f32 * 0.18).max(6.0) as u16;
        if bar_width > 4 && rect.height > 2 {
            let x = rect.x + (rect.width - bar_width) / 2;
            let y = rect.y + rect.height - 1;
            let filled = (bar_width as f32 * progress) as u16;
            for i in 0..bar_width {
                put(
                    buf,
                    x + i,
                    y,
                    if i < filled { "━" } else { "─" },
                    color(if i < filled {
                        pal.primary
                    } else {
                        pal.border_dim
                    }),
                    None,
                );
            }
        }
    }

    // `D`: inspeccionar la base temporal. Si el índice cambia con la frase
    // saliente por debajo del 85 %, avisa: es exactamente el fallo reportado.
    if app.lyrics_debug {
        let position = app.display_position_ms();
        let target = doc.aligned_ms(position);
        let index = doc.index_at_target(target);
        let line_progress = doc
            .active_at_target(target)
            .map(|(_, progress)| progress)
            .unwrap_or(0.0);
        let word_progress = index.map_or(0.0, |i| doc.word_progress_at(position, i));
        let info = format!(
            "pos {}ms  off {:+}ms  lyric {}ms  idx {:?}  línea {:.2}  palabra {:.2}",
            position, doc.offset_ms, target, index, line_progress, word_progress
        );
        text_at(
            buf,
            Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(2), 1),
            info,
            Style::new().fg(color(pal.warn)),
            Alignment::Left,
        );
        if let Some((from, previous_word)) = app.lyrics_debug_last {
            if Some(from) != index && previous_word < 0.85 {
                text_at(
                    buf,
                    Rect::new(rect.x + 1, rect.y + 1, rect.width.saturating_sub(2), 1),
                    format!(
                        "⚠ cambio prematuro {from} → {:?} (palabra {:.2})",
                        index, previous_word
                    ),
                    Style::new().fg(color(pal.error)),
                    Alignment::Left,
                );
            }
        }
        app.lyrics_debug_last = index.map(|i| (i, word_progress));
    }
}

/// Barra de mini-controles de la letra. Todo pasa por el estado real de la app:
/// el zoom cambia `lyrics_size`, los ±ms llaman al offset persistido por pista
/// y el scroll solo mueve la vista, sin inventar tiempos.
fn draw_lyrics_toolbar(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    fill_soft(buf, rect, pal.bg_deep, 0.62, pal.bg_deep);
    let mut x = rect.x + 1;
    let right = (rect.x + rect.width).saturating_sub(1);

    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "A−",
        Action::LyricsZoom(-1),
    );
    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "A+",
        Action::LyricsZoom(1),
    );
    if x + 1 < right {
        put(buf, x, rect.y, "│", color(pal.border_dim), None);
        x += 2;
    }
    // Modo "letras inventadas" y volver a la línea actual. Van antes de los
    // offsets para que sigan visibles en columnas estrechas.
    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "inv",
        Action::LyricsToggleGenerated,
    );
    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "⟲",
        Action::LyricsFollow,
    );
    if x + 1 < right {
        put(buf, x, rect.y, "│", color(pal.border_dim), None);
        x += 2;
    }
    for (label, delta) in [
        ("−250", -250i64),
        ("−100", -100),
        ("+100", 100),
        ("+250", 250),
    ] {
        if x >= right {
            break;
        }
        lyric_chip(
            app,
            buf,
            pal,
            &mut x,
            right,
            rect.y,
            label,
            Action::LyricsSync(delta),
        );
    }
    if x + 1 < right {
        put(buf, x, rect.y, "│", color(pal.border_dim), None);
        x += 2;
    }
    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "↑",
        Action::LyricsScroll(-1),
    );
    lyric_chip(
        app,
        buf,
        pal,
        &mut x,
        right,
        rect.y,
        "↓",
        Action::LyricsScroll(1),
    );
    if app.now.track.is_some() {
        lyric_chip(
            app,
            buf,
            pal,
            &mut x,
            right,
            rect.y,
            "⟳",
            Action::LyricsRefetch,
        );
        lyric_chip(
            app,
            buf,
            pal,
            &mut x,
            right,
            rect.y,
            "⌕",
            Action::LyricsSearchOpen,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn lyric_chip(
    app: &mut App,
    buf: &mut Buffer,
    pal: &Palette,
    x: &mut u16,
    right: u16,
    y: u16,
    label: &str,
    action: Action,
) {
    let width = label.chars().count() as u16 + 2;
    if *x + width > right {
        return;
    }
    let rect = Rect::new(*x, y, width, 1);
    let hover = app.hover == Some(action.clone());
    let active = match &action {
        Action::LyricsSyncReset => app.lyrics_offset_ms != 0,
        Action::LyricsToggleGenerated => app.cfg.lyrics.allow_generated,
        // Resaltado cuando estás en manual y el botón sirve para volver.
        Action::LyricsFollow => !app.lyrics_follow,
        _ => false,
    };
    fill(
        buf,
        rect,
        if active {
            mix(pal.surface, pal.warn, 0.32)
        } else if hover {
            pal.surface_alt
        } else {
            pal.surface
        },
    );
    text_at(
        buf,
        rect,
        label.to_string(),
        Style::new()
            .fg(color(if active {
                pal.warn
            } else if hover {
                pal.text
            } else {
                pal.muted
            }))
            .add_modifier(Modifier::BOLD),
        Alignment::Center,
    );
    app.hit.add(rect, action);
    *x += width + 1;
}

/// Búsqueda manual de letras dentro del hueco de la letra. Es una pantalla
/// persistente, no un popup: el usuario puede leer la previsualización y
/// aceptar la versión que sea de verdad.
fn draw_lyrics_search(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    let Some(state) = app.lyrics_search.clone() else {
        return;
    };
    fill_soft(
        buf,
        rect,
        mix(pal.bg_deep, pal.surface, 0.55),
        0.55,
        pal.bg_deep,
    );
    let inner = Rect::new(
        rect.x + 1,
        rect.y,
        rect.width.saturating_sub(2),
        rect.height,
    );
    // Cabecera.
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(16), 1),
        "BUSCAR LETRA",
        Style::new()
            .fg(color(mix_oklab(pal.primary, pal.text, 0.2)))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    let back = "esc volver";
    let back_w = back.chars().count() as u16;
    if inner.width > back_w {
        let back_rect = Rect::new(inner.x + inner.width - back_w, inner.y, back_w, 1);
        put(buf, back_rect.x, back_rect.y, back, color(pal.dim), None);
        app.hit.add(back_rect, Action::LyricsSearchClose);
    }
    // Consulta + estado.
    let query_y = inner.y + 1;
    let query_rect = Rect::new(inner.x, query_y, inner.width, 1);
    fill(buf, query_rect, mix(pal.surface, pal.primary, 0.10));
    put(
        buf,
        query_rect.x + 1,
        query_y,
        "⌕",
        color(pal.primary),
        None,
    );
    let mut query = state.query.clone();
    truncate_chars(&mut query, inner.width.saturating_sub(6) as usize);
    put(
        buf,
        query_rect.x + 3,
        query_y,
        &query,
        color(pal.text),
        None,
    );
    let cursor_x =
        (query_rect.x + 3 + query.chars().count() as u16).min(query_rect.x + query_rect.width - 1);
    put(buf, cursor_x, query_y, "▏", color(pal.primary), None);
    if !state.status.is_empty() && inner.width > 30 {
        let mut status = state.status.clone();
        truncate_chars(&mut status, inner.width.saturating_sub(2) as usize);
        text_at(
            buf,
            Rect::new(inner.x + 1, query_y + 1, inner.width.saturating_sub(2), 1),
            status,
            Style::new().fg(color(if state.loading { pal.warn } else { pal.dim })),
            Alignment::Left,
        );
    }

    let body_y = query_y + 2;
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let body_h = footer_y.saturating_sub(body_y);
    let two_cols = inner.width >= 52 && body_h >= 7;
    let results_w = if two_cols {
        ((inner.width as u32 * 46 / 100) as u16).max(24)
    } else {
        inner.width
    };
    let preview_w = if two_cols {
        inner.width.saturating_sub(results_w + 2)
    } else {
        0
    };
    let results = state.results.clone();
    let selected = state.selected.min(results.len().saturating_sub(1));
    if results.is_empty() {
        let message = if state.loading {
            "buscando…".to_string()
        } else if !state.status.is_empty() {
            state.status.clone()
        } else {
            "escribe una búsqueda y pulsa enter".to_string()
        };
        text_at(
            buf,
            Rect::new(inner.x, body_y + body_h / 2, inner.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
    } else {
        // Lista de candidatos.
        let row_h = 2u16;
        let visible = (body_h / row_h).max(1) as usize;
        let start = selected
            .saturating_sub(visible / 2)
            .min(results.len().saturating_sub(visible.min(results.len())));
        for (offset, candidate) in results.iter().skip(start).take(visible).enumerate() {
            let index = start + offset;
            let y = body_y + offset as u16 * row_h;
            let row = Rect::new(inner.x, y, results_w, row_h);
            if index == selected {
                fill(buf, row, mix(pal.surface, pal.primary, 0.24));
                put(buf, row.x, y, "▍", color(pal.primary), None);
            } else if app.hover == Some(Action::LyricsSearchSelect(index)) {
                fill(buf, row, pal.surface_alt);
            }
            let sync = if candidate.synced {
                "♪ sync"
            } else {
                "· texto"
            };
            let header = format!("{}  ·  {}", candidate.title.trim(), candidate.artist.trim());
            let mut header = header;
            truncate_chars(&mut header, results_w.saturating_sub(10) as usize);
            put(
                buf,
                row.x + 2,
                y,
                &header,
                color(if index == selected {
                    pal.text
                } else {
                    pal.muted
                }),
                None,
            );
            let meta = format!(
                "{}  ·  {}",
                candidate.artist.trim(),
                format_ms(candidate.duration_ms)
            );
            let mut meta = meta;
            truncate_chars(&mut meta, results_w.saturating_sub(12) as usize);
            put(buf, row.x + 2, y + 1, &meta, color(pal.dim), None);
            put(
                buf,
                row.x + results_w.saturating_sub(sync.chars().count() as u16 + 1),
                y,
                sync,
                color(if candidate.synced {
                    pal.primary
                } else {
                    pal.dim
                }),
                None,
            );
            app.hit.add(row, Action::LyricsSearchSelect(index));
        }

        // Previsualización de la candidata seleccionada.
        if two_cols {
            if let Some(candidate) = results.get(selected) {
                let preview = Rect::new(inner.x + results_w + 2, body_y, preview_w, body_h);
                fill_soft(
                    buf,
                    preview,
                    mix(pal.bg_deep, pal.surface, 0.7),
                    0.5,
                    pal.bg_deep,
                );
                let source = format!(
                    "VISTA PREVIA · {} · {}",
                    candidate.source.to_uppercase(),
                    if candidate.synced {
                        "sincronizada"
                    } else {
                        "texto"
                    }
                );
                text_at(
                    buf,
                    Rect::new(preview.x + 1, preview.y, preview.width.saturating_sub(2), 1),
                    source,
                    Style::new().fg(color(pal.secondary)),
                    Alignment::Left,
                );
                let lines = candidate.preview_lines();
                let wrap_w = preview.width.saturating_sub(4).max(8) as usize;
                let mut wrapped: Vec<String> = Vec::new();
                for line in lines.iter() {
                    for row in wrap_text(line, wrap_w) {
                        wrapped.push(row);
                    }
                }
                let max_rows = preview.height.saturating_sub(2) as usize;
                let start = state.preview_scroll.min(wrapped.len().saturating_sub(1));
                for (row, line) in wrapped.iter().skip(start).take(max_rows).enumerate() {
                    put(
                        buf,
                        preview.x + 2,
                        preview.y + 1 + row as u16,
                        line,
                        color(if row == 0 { pal.text } else { pal.muted }),
                        None,
                    );
                }
                if wrapped.len() > start + max_rows && max_rows > 0 {
                    put(
                        buf,
                        preview.x + 2,
                        preview.y + preview.height.saturating_sub(1),
                        "…",
                        color(pal.dim),
                        None,
                    );
                }
            }
        }
    }

    // Acciones y ayuda.
    if body_h > 0 {
        let mut x = inner.x;
        let accept = if results.is_empty() {
            "enter buscar"
        } else {
            "enter usar esta"
        };
        let chips = [(accept, Action::LyricsAcceptCandidate)];
        for (label, action) in chips {
            let w = label.chars().count() as u16 + 2;
            if x + w > inner.x + inner.width {
                break;
            }
            let chip = Rect::new(x, footer_y, w, 1);
            let hover = app.hover == Some(action.clone());
            fill(buf, chip, if hover { pal.surface_alt } else { pal.surface });
            text_at(
                buf,
                chip,
                label,
                Style::new().fg(color(if matches!(&action, Action::LyricsAcceptCandidate) {
                    pal.primary
                } else {
                    pal.muted
                })),
                Alignment::Center,
            );
            app.hit.add(chip, action);
            x += w + 1;
        }
        let hint = "↑↓ elegir · PgUp/PgDn letra · esc volver";
        if x + hint.chars().count() as u16 + 2 < inner.x + inner.width {
            text_at(
                buf,
                Rect::new(x, footer_y, inner.width.saturating_sub(x - inner.x), 1),
                hint,
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
    }
}

/// Pinta la línea activa palabra a palabra. Los tiempos salen de
/// `line.words` (LRC mejorado o refinado con el mapa de audio), nunca de un
/// reparto global de la línea: por eso cada palabra se enciende cuando toca.
///
/// La frase se reparte en hasta dos filas; el estado de encendido viaja con
/// cada carácter, así que el karaoke sigue siendo correcto al saltar de fila.
fn draw_active_line(
    buf: &mut Buffer,
    rect: Rect,
    line: &spote_core::lyrics::LyricLine,
    lyric_ms: u64,
    pal: &Palette,
    line_h: u16,
) {
    type Cell = (char, f32, bool, bool);
    // `lyric_ms` ya tiene el offset aplicado y coincide con la base de
    // `Word.start_ms` / `Word.end_ms`; no volver a leer el reloj aquí.
    let position = lyric_ms;
    let max_rows = (rect.height / line_h).max(1) as usize;
    let mut cells: Vec<Cell> = Vec::new();
    if line.words.len() >= 2 {
        for (i, word) in line.words.iter().enumerate() {
            if i > 0 {
                cells.push((' ', 1.0, false, false));
            }
            let span = word.end_ms.saturating_sub(word.start_ms).max(1);
            let word_progress =
                (position.saturating_sub(word.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
            let started = position >= word.start_ms;
            let chars: Vec<char> = word.text.chars().collect();
            let total = chars.len().max(1) as f32;
            for (j, ch) in chars.into_iter().enumerate() {
                let lit = if started {
                    ((word_progress * total) - j as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                cells.push((ch, lit, started && word_progress < 1.0, word.optional));
            }
        }
    } else {
        // Sin palabras por sílaba, se llena también con un pelín de margen:
        // si el último carácter se enciende justo al cambiar de línea, no se
        // llega a ver nunca.
        let span = line
            .end_ms
            .saturating_sub(150)
            .saturating_sub(line.start_ms)
            .max(1);
        let progress =
            (position.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0);
        let chars: Vec<char> = line.text.chars().collect();
        let total = chars.len().max(1) as f32;
        for (j, ch) in chars.into_iter().enumerate() {
            cells.push((
                ch,
                ((progress * total) - j as f32).clamp(0.0, 1.0),
                false,
                false,
            ));
        }
    }

    let mut rows = wrap_cells(cells, rect.width.saturating_sub(4) as usize, max_rows);
    let prefix = [('▸', 0.0, false, false), (' ', 0.0, false, false)];
    if let Some(first) = rows.first_mut() {
        first.splice(0..0, prefix);
    }
    let suffix = [(' ', 0.0, false, false), ('◂', 0.0, false, false)];
    if let Some(last) = rows.last_mut() {
        last.extend(suffix);
    }

    for (row_index, row) in rows.into_iter().enumerate() {
        let y = rect.y + row_index as u16 * line_h;
        if y >= rect.y + rect.height {
            break;
        }
        let visible = row.len().min(rect.width as usize);
        let start_x = rect.x + (rect.width.saturating_sub(visible as u16)) / 2;
        for (i, (ch, lit, current, optional)) in row.into_iter().take(visible).enumerate() {
            let base = if optional {
                // Adlib de fondo: se ve, pero no compite con la voz principal.
                mix_oklab(pal.dim, pal.secondary, 0.22 + lit * 0.18)
            } else if lit <= 0.001 {
                mix_oklab(pal.dim, pal.muted, 0.4)
            } else {
                mix_oklab(pal.text, pal.primary, 0.28 + lit * 0.45)
            };
            let fg = if current && !optional {
                mix_oklab(base, pal.text, 0.45)
            } else {
                base
            };
            let mut glyph = [0u8; 4];
            let symbol: &str = ch.encode_utf8(&mut glyph);
            put(buf, start_x + i as u16, y, symbol, color(fg), None);
            if current && !optional && i > 0 {
                if let Some(cell) = buf.cell_mut((start_x + i as u16, y)) {
                    cell.modifier.insert(Modifier::BOLD);
                }
            }
        }
    }
}

/// Altura relativa (en celdas) del alto de la línea `index` respecto al alto
/// de la línea anclada por el muelle de scroll. Contar filas reales es lo que
/// permite que una frase de dos filas no pise a la siguiente.
fn relative_lyric_top(index: usize, scroll: f32, heights: &[u16]) -> f32 {
    if heights.is_empty() {
        return 0.0;
    }
    let scroll = scroll.max(0.0);
    let base = (scroll.floor() as usize).min(heights.len() - 1);
    let frac = scroll - scroll.floor();
    let base_h = heights[base] as f32;
    let mut y = 0.0;
    if index >= base {
        for height in heights.iter().take(index.min(heights.len())).skip(base) {
            y += *height as f32;
        }
        y -= frac * base_h;
    } else {
        for height in heights.iter().take(base).skip(index) {
            y -= *height as f32;
        }
        y -= frac * base_h;
    }
    y
}

/// Reparte el texto en como mucho dos filas por palabras. Si una palabra no
/// cabe se parte; si hay más de dos filas se corta con `…`.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            let head: String = word.chars().take(width).collect();
            word = word.chars().skip(width).collect();
            if !current.is_empty() {
                rows.push(std::mem::take(&mut current));
            }
            rows.push(head);
        }
        if current.is_empty() {
            current = word;
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(&word);
        } else {
            rows.push(std::mem::take(&mut current));
            current = word;
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    if rows.len() > 2 {
        rows.truncate(2);
        if let Some(last) = rows.last_mut() {
            let keep = width.saturating_sub(1);
            *last = last.chars().take(keep).collect();
            last.push('…');
        }
    }
    rows
}

/// Igual que [`wrap_text`] pero con las celdas del karaoke, conservando el
/// estado de encendido de cada carácter.
fn wrap_cells(
    cells: Vec<(char, f32, bool, bool)>,
    width: usize,
    max_rows: usize,
) -> Vec<Vec<(char, f32, bool, bool)>> {
    type Cell = (char, f32, bool, bool);
    fn flush(rows: &mut Vec<Vec<Cell>>, current: &mut Vec<Cell>, word: Vec<Cell>, width: usize) {
        if word.is_empty() {
            return;
        }
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            rows.push(std::mem::take(current));
        }
        if !current.is_empty() {
            current.push((' ', 1.0, false, false));
        }
        if word.len() > width {
            let mut rest = word;
            while rest.len() > width {
                rows.push(rest.drain(..width).collect());
            }
            current.extend(rest);
        } else {
            current.extend(word);
        }
    }

    let width = width.max(1);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut word: Vec<Cell> = Vec::new();
    for cell in cells {
        if cell.0 == ' ' {
            flush(&mut rows, &mut current, std::mem::take(&mut word), width);
        } else {
            word.push(cell);
        }
    }
    flush(&mut rows, &mut current, word, width);
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    if rows.len() > max_rows {
        rows.truncate(max_rows);
        if let Some(last) = rows.last_mut() {
            if last.len() >= width {
                last.pop();
            }
            last.push(('…', 0.0, false, false));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_in_two_rows() {
        let rows = wrap_text(
            "Te vo'a compra' una rosa, pero solo te la doy si viene' a verme",
            24,
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[1].chars().count() <= 24);
    }

    #[test]
    fn short_lines_stay_in_one_row() {
        let rows = wrap_text("Na-na-na-na-ni", 40);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], "Na-na-na-na-ni");
    }

    #[test]
    fn more_than_two_rows_are_ellipsized() {
        let rows = wrap_text("uno dos tres cuatro cinco seis siete ocho nueve diez", 8);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].ends_with('…'));
    }

    #[test]
    fn relative_top_counts_real_rows() {
        let heights = [1u16, 2, 1];
        assert_eq!(relative_lyric_top(1, 1.0, &heights), 0.0);
        assert_eq!(relative_lyric_top(2, 1.0, &heights), 2.0);
        assert_eq!(relative_lyric_top(0, 1.0, &heights), -1.0);
    }
}
