//! Cola, sesión y visualizador.
use super::*;

pub(super) fn draw_side(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    // Cola.
    panel(
        buf,
        layout.queue,
        pal,
        app.focus == crate::app::Pane::Side,
        0.55,
        Some("cola"),
        app.audio.kick_pulse,
    );
    let inner = Rect::new(
        layout.queue.x + 2,
        layout.queue.y + 2,
        layout.queue.width.saturating_sub(4),
        layout.queue.height.saturating_sub(3),
    );
    let queue_len = if app.room_queue.is_empty() {
        app.queue.len()
    } else {
        app.room_queue.len()
    };
    let hint_row = inner.y + inner.height.saturating_sub(1);
    if queue_len == 0 {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1),
            "vacía · a para encolar",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    } else if app.list_cards {
        let card_h = 3u16;
        let visible = (inner.height / card_h).max(1) as usize;
        let start = app.queue_selected.saturating_sub(visible / 2);
        let start = start.min(queue_len.saturating_sub(visible.min(queue_len)));
        let shown = (queue_len - start).min(visible);
        let y_offset = if app.vcenter {
            inner.height.saturating_sub(shown as u16 * card_h) / 2
        } else {
            0
        };
        for offset in 0..visible {
            let index = start + offset;
            if index >= queue_len || inner.y + (offset as u16 + 1) * card_h > inner.y + inner.height {
                break;
            }
            let y = inner.y + y_offset + offset as u16 * card_h;
            let card = Rect::new(inner.x, y, inner.width, card_h);
            fill_soft(buf, card, mix(pal.bg, pal.surface, 0.5), 1.0, pal.bg);
            let selected = index == app.queue_selected;
            if selected {
                fill(buf, card, mix(pal.surface, pal.primary, 0.18));
            }
            let (title, artist, path_pending) = if app.room_queue.is_empty() {
                let entry = &app.queue[index];
                (
                    entry.track.title.clone(),
                    entry.track.artist.clone(),
                    entry.path.is_none(),
                )
            } else {
                let item = &app.room_queue[index];
                (item.track.title.clone(), item.track.artist.clone(), false)
            };
            let thumb = Rect::new(card.x + 1, y, 5, card_h);
            let track_ref = if app.room_queue.is_empty() {
                app.queue[index].track.clone()
            } else {
                app.room_queue[index].track.clone()
            };
            draw_thumb(app, buf, thumb, &track_ref, pal);
            let text_x = thumb.x + thumb.width + 1;
            let text_w = (card.x + card.width).saturating_sub(text_x + 3);
            let mut title = title;
            truncate_chars(&mut title, text_w as usize);
            put(
                buf,
                text_x,
                y,
                &title,
                color(if selected { pal.text } else { pal.muted }),
                None,
            );
            let mut artist = artist;
            if path_pending {
                artist.push_str("  ·  ↓");
            }
            truncate_chars(&mut artist, text_w as usize);
            put(buf, text_x, y + 1, &artist, color(pal.dim), None);
            put(
                buf,
                text_x,
                y + 2,
                &format!("{:>2}  ·  {}", index + 1, if path_pending { "pendiente" } else { "lista" }),
                color(pal.dim),
                None,
            );
            let remove_rect = Rect::new(card.x + card.width - 2, y, 2, 1);
            put(buf, remove_rect.x, y, "×", color(pal.error), None);
            // El último registro gana: la tarjeta primero y el × encima, para
            // que un solo clic en la cruz quite la pista.
            app.hit.add(card, Action::QueueRow(index));
            app.hit.add(remove_rect, Action::QueueRemove(index));
        }
    } else {
        let visible = inner.height.saturating_sub(1) as usize;
        let start = app.queue_selected.saturating_sub(visible / 2);
        let start = start.min(queue_len.saturating_sub(visible.min(queue_len)));
        let shown = (queue_len - start).min(visible);
        let y_offset = if app.vcenter {
            inner.height.saturating_sub(1).saturating_sub(shown as u16) / 2
        } else {
            0
        };
        for offset in 0..visible {
            let index = start + offset;
            if index >= queue_len || inner.y + offset as u16 >= hint_row {
                break;
            }
            let y = inner.y + y_offset + offset as u16;
            let row = Rect::new(inner.x, y, inner.width, 1);
            let selected = index == app.queue_selected;
            let hover = app.hover == Some(Action::QueueRow(index));
            if selected {
                fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            } else if hover {
                fill(buf, row, pal.surface_alt);
            }
            let (title, artist, pending) = if app.room_queue.is_empty() {
                let entry = &app.queue[index];
                (
                    entry.track.title.clone(),
                    entry.track.artist.clone(),
                    entry.path.is_none(),
                )
            } else {
                let item = &app.room_queue[index];
                (item.track.title.clone(), item.track.artist.clone(), false)
            };
            let remove_rect = Rect::new(row.x + row.width.saturating_sub(2), y, 2, 1);
            if app.hover == Some(Action::QueueRemove(index)) {
                fill(buf, remove_rect, pal.error);
            }
            put(buf, remove_rect.x, y, "×", color(pal.error), None);
            app.hit.add(row, Action::QueueRow(index));
            app.hit.add(remove_rect, Action::QueueRemove(index));
            put(
                buf,
                row.x,
                y,
                &format!("{:>2}", index + 1),
                color(if selected { pal.primary } else { pal.dim }),
                None,
            );
            let room_for_artist = row.width > 34;
            let artist_w = if room_for_artist {
                (row.width / 3).min(14)
            } else {
                0
            };
            let title_w = row
                .width
                .saturating_sub(4 + remove_rect.width + artist_w + 1)
                as usize;
            let mut title = title;
            truncate_chars(&mut title, title_w);
            put(
                buf,
                row.x + 3,
                y,
                &title,
                color(if selected { pal.text } else { pal.muted }),
                None,
            );
            if room_for_artist && artist_w > 4 {
                let mut artist = artist;
                truncate_chars(&mut artist, artist_w.saturating_sub(1) as usize);
                put(
                    buf,
                    row.x + 3 + title.chars().count() as u16 + 1,
                    y,
                    &artist,
                    color(pal.dim),
                    None,
                );
            }
            if pending {
                put(
                    buf,
                    remove_rect.x.saturating_sub(2),
                    y,
                    "↓",
                    color(pal.warn),
                    None,
                );
            }
        }
        // Ayuda de la cola, en la última fila del panel.
        if hint_row > inner.y {
            let text = if app.room_queue.is_empty() {
                "x quita · arrastra para ordenar"
            } else {
                "x quita · la sala la ordena el servidor"
            };
            text_at(
                buf,
                Rect::new(inner.x, hint_row, inner.width, 1),
                text.to_string(),
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
    }

    // Info: paleta + estado.
    panel(buf, layout.info, pal, false, 0.45, Some("sesión"), app.audio.kick_pulse * 0.8);
    let inner = Rect::new(
        layout.info.x + 2,
        layout.info.y + 2,
        layout.info.width.saturating_sub(4),
        layout.info.height.saturating_sub(3),
    );
    let mut y = inner.y;
    // Muestras de la paleta: 12 bloques.
    let swatch_width = inner.width.max(1);
    for i in 0..swatch_width {
        let t = i as f32 / swatch_width.max(1) as f32;
        put_bg(buf, inner.x + i, y, pal.ramp_at(t));
    }
    y += 2;
    let bottom = layout.info.y + layout.info.height;
    let line = |buf: &mut Buffer, text: String, fg: Rgb, y: &mut u16| {
        if *y < bottom.saturating_sub(1) {
            text_at(
                buf,
                Rect::new(inner.x, *y, inner.width, 1),
                text,
                Style::new().fg(color(fg)),
                Alignment::Left,
            );
        }
        *y += 1;
    };
    if let Some(room) = &app.room {
        line(buf,
            format!("sala · {}", app.room_label(&room.id)),
            pal.primary,
            &mut y,
        );
    } else if app.follow_spotify {
        line(buf, "siguiendo spotify".into(), pal.secondary, &mut y);
    } else if app.social.is_some() {
        line(buf, "servidor social".into(), pal.secondary, &mut y);
    } else {
        line(buf, "modo local".into(), pal.secondary, &mut y);
    }
    // Descargas: paso y barra, aunque estés en otra pestaña.
    let active: Vec<_> = app.downloads.iter().filter(|info| info.active()).take(2).collect();
    for download in active {
        let mut title = download.title.clone();
        truncate_chars(&mut title, inner.width as usize);
        line(buf, format!("↓ {title}"), pal.warn, &mut y);
        if y < layout.info.y + layout.info.height.saturating_sub(1) {
            let width = inner.width.saturating_sub(1);
            let filled = (width as f32 * download.percent / 100.0).round() as u16;
            for i in 0..width {
                put(
                    buf,
                    inner.x + i,
                    y,
                    if i < filled { "━" } else { "─" },
                    color(if i < filled { pal.warn } else { pal.border_dim }),
                    None,
                );
            }
            y += 1;
        }
        line(buf,
            format!("{} {:.0}%", download.stage, download.percent),
            pal.dim,
            &mut y,
        );
    }
    line(buf,
        format!("viz · {} ({})", app.viz.preset_name(), app.viz.layers.len()),
        pal.dim,
        &mut y,
    );
    line(buf, format!("arte · {}", app.art_mode.label()), pal.dim, &mut y);
    if app.show_bpm && app.audio.bpm > 0.0 {
        line(buf,
            format!(
                "{:.0} bpm · ♩ {:>3}  ▽ {:>3}  ♯ {:>3}",
                app.audio.bpm,
                (app.audio.kick * 100.0) as u32,
                (app.audio.snare * 100.0) as u32,
                (app.audio.hat * 100.0) as u32
            ),
            pal.dim,
            &mut y,
        );
    }
    let tools = &app.tools;
    if !tools.missing().is_empty() {
        line(buf,
            format!("faltan: {}", tools.missing().join(", ")),
            pal.warn,
            &mut y,
        );
    } else {
        line(buf, "mpv · ffmpeg · yt-dlp ok".into(), pal.dim, &mut y);
    }
    if !app.status.is_empty() {
        y += 1;
        line(buf, app.status.clone(), pal.muted, &mut y);
    }
}

// ---------------------------------------------------------------------------
// Visualizador
// ---------------------------------------------------------------------------

pub(super) fn draw_viz(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.viz;
    if rect.width < 4 || rect.height < 2 {
        return;
    }
    let background_on = app.viz_background && app.show_viz;
    if !background_on {
        let audio = &app.audio;
        let dt = 1.0 / app.fps.value().max(1.0);
        app.viz.render(rect.width, rect.height, audio, pal, dt);
        blit_viz(app, buf, rect, 0, 1.0, 0.0);
    } else {
        // El fondo ya renderizó el canvas a pantalla completa: la franja es
        // una cosecha de sus últimas filas, así el efecto es continuo.
        let src_y = app.viz.output.h.saturating_sub(rect.height);
        blit_viz(app, buf, rect, src_y, 1.0, 0.0);
    }
    let label = format!("{} · {}", app.viz.preset_name(), app.viz.preset_description());
    let width = label.chars().count() as u16;
    if rect.width > width + 4 {
        let x = rect.x + rect.width - width - 2;
        fill_soft(buf, Rect::new(x, rect.y, width, 1), pal.bg_deep, 0.7, pal.bg_deep);
        put(buf, x, rect.y, &label, color(pal.dim), None);
    }
    app.hit.add(rect, Action::VizNext);
}

/// Copia el canvas del visualizador a la pantalla. `src_y0` permite dibujar
/// solo una franja; `amount` controla cuánto color se suma al fondo.
pub(super) fn blit_viz(app: &App, buf: &mut Buffer, dst: Rect, src_y0: u16, fg_amount: f32, bg_amount: f32) {
    let layer = &app.viz.output;
    if layer.w == 0 || layer.h == 0 {
        return;
    }
    for y in 0..dst.height {
        for x in 0..dst.width {
            let sx = x.min(layer.w.saturating_sub(1));
            let sy = (src_y0 + y).min(layer.h.saturating_sub(1));
            let Some(cell) = layer.get(sx as i32, sy as i32) else {
                continue;
            };
            if cell.alpha <= 0.01 {
                continue;
            }
            let pal_hint = (11u8, 11u8, 16u8);
            let fg = mix(pal_hint, cell.fg, cell.alpha * fg_amount);
            let bg = mix(pal_hint, cell.fg, cell.alpha * bg_amount);
            let mut glyph = [0u8; 4];
            let symbol: &str = cell.ch.encode_utf8(&mut glyph);
            put(
                buf,
                dst.x + x,
                dst.y + y,
                symbol,
                color(fg),
                Some(color(bg)),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Transporte
// ---------------------------------------------------------------------------
