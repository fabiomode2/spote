//! Cola, amigos y visualizador.
use super::*;

pub(super) fn draw_side(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    if app.detail.is_some() {
        super::detail::draw_detail(app, buf, layout.side, pal);
        return;
    }
    if layout.queue.height > 2 {
        draw_queue(app, buf, layout.queue, pal);
    }
    if layout.info.height > 2 {
        draw_friends(app, buf, layout.info, pal);
    }
}

fn draw_queue(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Side,
        0.55,
        Some("cola"),
        app.audio.kick_pulse,
    );
    app.hit
        .add(Rect::new(rect.x, rect.y, rect.width, 1), Action::QueueMenu);
    app.hit.add(
        Rect::new(
            rect.x,
            rect.y + 1,
            rect.width,
            rect.height.saturating_sub(1),
        ),
        Action::Focus(crate::app::Pane::Side),
    );
    let mut inner = Rect::new(
        rect.x + 2,
        rect.y + 2,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(3),
    );
    // Lo que suena ahora, siempre arriba de la cola. Antes solo se veía en la
    // cabecera y en la lista había que adivinar qué entrada estaba sonando.
    if let Some(track) = app.now.track.clone() {
        if inner.height >= 4 {
            let two = inner.height >= 7;
            let now_h = if two { 2 } else { 1 };
            let now_rect = Rect::new(inner.x, inner.y, inner.width, now_h);
            fill_soft(buf, now_rect, mix(pal.surface, pal.primary, 0.16), 0.9, pal.bg);
            let mut title = track.title.clone();
            truncate_chars(&mut title, inner.width.saturating_sub(4) as usize);
            put(buf, inner.x, inner.y, "▶", color(pal.primary), None);
            put(buf, inner.x + 2, inner.y, &title, color(pal.text), None);
            if two {
                let mut artist = track.artist.clone();
                if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
                    artist.push_str(" · ");
                    artist.push_str(album);
                }
                truncate_chars(&mut artist, inner.width.saturating_sub(4) as usize);
                put(buf, inner.x + 2, inner.y + 1, &artist, color(pal.secondary), None);
            }
            app.hit.add(now_rect, Action::Tab(Tab::Now));
            inner.y += now_h + 1;
            inner.height = inner.height.saturating_sub(now_h + 1);
        }
    }
    let queue_len = if app.room_queue.is_empty() {
        app.queue.len()
    } else {
        app.room_queue.len()
    };
    if queue_len == 0 {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1),
            "vacía · a para encolar",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        return;
    }
    let hint_row = inner.y + inner.height.saturating_sub(1);
    if app.list_cards {
        let card_h = 3u16;
        let visible = (inner.height / card_h).max(1) as usize;
        let start = super::lists::keep_visible(
            app.queue_start,
            app.queue_selected,
            visible,
            queue_len,
        );
        app.queue_start = start;
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
            let (title, artist, path_pending, track_ref) = if app.room_queue.is_empty() {
                let entry = &app.queue[index];
                (
                    entry.track.title.clone(),
                    entry.track.artist.clone(),
                    entry.path.is_none(),
                    entry.track.clone(),
                )
            } else {
                let item = &app.room_queue[index];
                (
                    item.track.title.clone(),
                    item.track.artist.clone(),
                    false,
                    item.track.clone(),
                )
            };
            let thumb = Rect::new(card.x + 1, y, 6, card_h);
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
            put(
                buf,
                text_x,
                y + 2,
                &format!(
                    "{:>2}  ·  {}",
                    index + 1,
                    if path_pending { "pendiente" } else { "lista" }
                ),
                color(pal.dim),
                None,
            );
            let remove_rect = Rect::new(card.x + card.width - 2, y, 2, 1);
            put(buf, remove_rect.x, y, "×", color(pal.error), None);
            app.hit.add(card, Action::QueueRow(index));
            // El nombre del artista de la tarjeta abre su ficha; la × sigue
            // teniendo prioridad porque se registra después.
            let artist_w = (artist.chars().count() as u16).min(text_w);
            if artist_w > 0 {
                let hover = app.hover == Some(Action::OpenArtist(track_ref.artist.clone()));
                link_at(
                    buf,
                    text_x,
                    y + 1,
                    artist_w,
                    &artist,
                    pal,
                    pal.secondary,
                    hover,
                    app.ui_time(),
                );
                app.hit.add(
                    Rect::new(text_x, y + 1, artist_w, 1),
                    Action::OpenArtist(track_ref.artist.clone()),
                );
            }
            app.hit.add(remove_rect, Action::QueueRemove(index));
        }
        return;
    }

    let visible = inner.height.saturating_sub(1) as usize;
    let start = super::lists::keep_visible(
        app.queue_start,
        app.queue_selected,
        visible,
        queue_len,
    );
    app.queue_start = start;
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
        // Columnas fijas: el título y el artista nunca se pisan.
        let artist_w = if row.width > 40 {
            (row.width / 3).min(16)
        } else {
            0
        };
        let title_w = row
            .width
            .saturating_sub(6 + remove_rect.width + artist_w + 1) as usize;
        let mut title = title;
        truncate_chars(&mut title, title_w);
        let provider = if app.room_queue.is_empty() {
            app.queue
                .get(index)
                .and_then(|entry| app.lib.track_by_key(&entry.track.key).ok().flatten())
                .and_then(|row| row.provider)
        } else {
            None
        };
        put(
            buf,
            row.x + 3,
            y,
            source_tag(provider.as_deref()),
            color(pal.secondary),
            None,
        );
        put(
            buf,
            row.x + 5,
            y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        if artist_w > 4 {
            let artist_x = row.x + row.width - remove_rect.width - artist_w - 1;
            let mut artist = artist;
            truncate_chars(&mut artist, artist_w.saturating_sub(1) as usize);
            let drawn = artist.chars().count() as u16;
            if drawn > 0 {
                let action = Action::OpenArtist(if app.room_queue.is_empty() {
                    app.queue[index].track.artist.clone()
                } else {
                    app.room_queue[index].track.artist.clone()
                });
                let hover = app.hover == Some(action.clone());
                link_at(
                    buf,
                    artist_x,
                    y,
                    drawn,
                    &artist,
                    pal,
                    pal.dim,
                    hover,
                    app.ui_time(),
                );
                app.hit.add(Rect::new(artist_x, y, drawn, 1), action);
            }
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
    if hint_row > inner.y {
        let text = if app.room_queue.is_empty() {
            "↑↓ elige · enter suena · x quita"
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

/// Panel de amigos/presencia. Sustituye al viejo bloque de "sesión": la
/// información técnica baja a la vista de sala, donde tiene sentido.
fn draw_friends(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    panel(
        Some(app),
        buf,
        rect,
        pal,
        false,
        0.45,
        Some("amigos"),
        app.audio.snare_pulse * 0.7,
    );
    let inner = Rect::new(
        rect.x + 2,
        rect.y + 2,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(3),
    );
    // Geometría compartida: la lista de amigos no puede pisar ni el bloque
    // de Tunebat ni el pie.
    let tunebat_data_lines = if rect.height >= 14 {
        4
    } else if rect.height >= 11 {
        3
    } else if rect.height >= 8 {
        2
    } else {
        0
    };
    let tunebat_base =
        (tunebat_data_lines > 0).then(|| rect.y + rect.height - (tunebat_data_lines + 2));
    let list_bottom = tunebat_base
        .map(|base| base.saturating_sub(1))
        .unwrap_or_else(|| rect.y + rect.height - 3);
    let mut y = inner.y;
    if app.social.is_none() {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "modo local · sin servidor",
            Style::new().fg(color(pal.secondary)),
            Alignment::Left,
        );
        y += 1;
        if rect.height >= 12 {
            text_at(
                buf,
                Rect::new(inner.x, y, inner.width, 1),
                "configura [server] para ver amigos",
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
    } else {
        let online = app.presence.iter().filter(|p| p.online).count();
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            format!("{} online · {} en total", online, app.presence.len()),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        y += 1;
        let mut friends: Vec<_> = app.presence.iter().collect();
        friends.sort_by_key(|p| (!p.online, p.user.name.to_lowercase()));
        for presence in friends {
            let needed = if presence.now_playing.is_some() { 2 } else { 1 };
            if y + needed > list_bottom {
                break;
            }
            let dot = if presence.online { "●" } else { "○" };
            let dot_color = if presence.online { pal.ok } else { pal.dim };
            put(buf, inner.x, y, dot, color(dot_color), None);
            let name = if presence.user.name == app.cfg.server.name {
                format!("{} (tú)", presence.user.name)
            } else {
                presence.user.name.clone()
            };
            let mut label = name;
            truncate_chars(&mut label, inner.width.saturating_sub(2) as usize);
            put(
                buf,
                inner.x + 2,
                y,
                &label,
                color(if presence.online { pal.text } else { pal.muted }),
                None,
            );
            y += 1;
            if let Some(track) = &presence.now_playing {
                let mut what = format!("♪ {} · {}", track.title, track.artist);
                truncate_chars(&mut what, inner.width.saturating_sub(4) as usize);
                put(buf, inner.x + 3, y, &what, color(pal.dim), None);
                y += 1;
            }
        }
    }

    // Tunebat: BPM, tonalidad y "feeling" de la pista actual, sin cabecera.
    // En paneles bajos se recorta a las dos primeras líneas.
    if let Some(base) = tunebat_base {
        let mut y = base;
        if app.now.track.is_none() {
            put(buf, inner.x, y, "sin reproducción", color(pal.dim), None);
        } else if app.tunebat_loading {
            put(buf, inner.x, y, "buscando en Tunebat…", color(pal.dim), None);
        } else if let Some(info) = &app.tunebat {
            let mut bpm = info.bpm_line();
            truncate_chars(&mut bpm, inner.width as usize);
            put(buf, inner.x, y, &bpm, color(pal.text), None);
            y += 1;
            let mut feeling = format!(
                "energy {:.0}% · dance {:.0}%",
                info.energy * 100.0,
                info.danceability * 100.0
            );
            truncate_chars(&mut feeling, inner.width as usize);
            put(buf, inner.x, y, &feeling, color(pal.muted), None);
            if tunebat_data_lines >= 3 {
                y += 1;
                let mut mood = format!(
                    "happy {:.0}% · pop {}",
                    info.happiness * 100.0,
                    info.popularity
                );
                if let Some(year) = info.release_date.get(0..4) {
                    if year.chars().all(|ch| ch.is_ascii_digit()) {
                        mood.push_str(&format!(" · {year}"));
                    }
                }
                if info.loudness != 0.0 {
                    mood.push_str(&format!(" · {:.1} dB", info.loudness));
                }
                truncate_chars(&mut mood, inner.width as usize);
                put(buf, inner.x, y, &mood, color(pal.muted), None);
            }
            if tunebat_data_lines >= 4 {
                y += 1;
                let mut texture = format!(
                    "ac {:.0}% · instr {:.0}% · live {:.0}%",
                    info.acousticness * 100.0,
                    info.instrumentalness * 100.0,
                    info.liveness * 100.0
                );
                if !info.label.is_empty() {
                    texture.push_str(" · ");
                    texture.push_str(&info.label);
                }
                truncate_chars(&mut texture, inner.width as usize);
                put(buf, inner.x, y, &texture, color(pal.dim), None);
            }
        } else {
            put(buf, inner.x, y, "sin datos en Tunebat", color(pal.dim), None);
        }
    }

    // Pie: solo el número de pistas guardadas. El BPM local y el preset
    // sobraban aquí y el BPM del audio no es fiable.
    let footer = format!(
        "{}/{}",
        app.lib.ready_count().unwrap_or(0),
        app.lib.count().unwrap_or(0)
    );
    if rect.height > 5 {
        let mut footer = footer;
        truncate_chars(&mut footer, inner.width as usize);
        put(
            buf,
            inner.x,
            rect.y + rect.height - 2,
            &footer,
            color(pal.dim),
            None,
        );
    }
    // Descargas activas: el título nunca se come el porcentaje. Título arriba,
    // barra + porcentaje abajo y un spinner ASCII animado.
    if let Some(download) = app.downloads.iter().find(|info| info.active()) {
        if rect.height > 8 {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
            let spin = frames[(app.viz.time * 10.0) as usize % frames.len()];
            let base = rect.y + rect.height - 4;
            let mut title = download.title.clone();
            truncate_chars(&mut title, inner.width.saturating_sub(2) as usize);
            put(
                buf,
                inner.x,
                base,
                &format!("{spin} {title}"),
                color(pal.warn),
                None,
            );
            let bar_w = inner.width.saturating_sub(7).max(4) as usize;
            let filled = ((download.percent / 100.0) * bar_w as f32).round() as usize;
            let bar = format!(
                "{}{} {:>3.0}%",
                "█".repeat(filled.min(bar_w)),
                "░".repeat(bar_w.saturating_sub(filled)),
                download.percent
            );
            put(
                buf,
                inner.x,
                base + 1,
                &bar,
                color(if download.percent > 0.0 { pal.primary } else { pal.dim }),
                None,
            );
        }
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
        // El canvas se regenera como mucho `fx_fps` veces por segundo; entre
        // medias se reutiliza el anterior. La franja no cambia de tamaño, así
        // que el reloj es el único que manda.
        let hz = app.strip_effects_hz();
        app.pump_viz(rect.width, rect.height, hz, pal);
        blit_viz(app, buf, rect, 0, 1.0, 0.0, 1);
    } else {
        // El fondo ya renderizó el canvas a pantalla completa: la franja es
        // una cosecha de sus últimas filas, así el efecto es continuo.
        let src_y = app
            .viz
            .output
            .h
            .saturating_sub((rect.height / VIZ_SCALE).max(1));
        blit_viz(app, buf, rect, src_y, 1.0, 0.0, VIZ_SCALE);
    }
    // `clean` puede subir por encima de los paneles: la cresta de la onda se
    // derrama unas filas hacia arriba con poca opacidad.
    if app.cfg.ui.fx.clean_spill && app.viz.preset_name() == "clean" && !background_on {
        clean_spill(app, buf, rect, pal);
    }
    // Brasa en el borde inferior de la franja.
    if app.cfg.ui.fx.edge_flame {
        edge_flame(app, buf, rect, pal);
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
/// Cresta de la onda de `clean` derramada sobre los paneles de arriba.
fn clean_spill(app: &App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    let spill = 6u16.min(rect.y.saturating_sub(1));
    if spill == 0 {
        return;
    }
    let t = app.viz.time;
    for x in 0..rect.width {
        let band = app.audio.band(x as f32 / rect.width.max(1) as f32 * 48.0);
        let wave = (x as f32 * 0.09 + t * 2.1).sin() * 0.5 + 0.5;
        let level = (band * 0.75 + wave * 0.25) * (0.55 + app.song_intensity * 0.8) * spill as f32;
        let height = level.round() as u16;
        for row in 0..height.min(spill) {
            let y = rect.y - 1 - row;
            let alpha = 0.74 * (1.0 - row as f32 / spill as f32);
            let rgb = mix_oklab(pal.bg_deep, pal.primary, alpha);
            put(buf, x, y, "▁", color(rgb), None);
        }
    }
}

/// Brasa continua en el borde inferior del visualizador. Antes el parpadeo
/// usaba una seno de frecuencia alta y dibujaba bloques de `▒` con huecos, que
/// parecían una portada rota; ahora es una línea de color que respira con el
/// bombo, sin patrones de bloques.
fn edge_flame(app: &App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    let y = rect.y + rect.height.saturating_sub(1);
    let t = app.viz.time;
    let energy = (app.audio.bass * 0.9
        + app.audio.kick_pulse * 0.75
        + app.audio.mid * 0.25)
        * (0.7 + app.song_intensity * 1.1);
    for x in 0..rect.width {
        let nx = x as f32 / rect.width.max(1) as f32;
        let envelope = (1.0 - (nx * 2.0 - 1.0).abs()).max(0.0);
        let breathe = ((x as f32 * 0.018 + t * 2.2).sin() * 0.5 + 0.5) * 0.18;
        let value = (energy * envelope + breathe).clamp(0.0, 1.0);
        let ch = if value > 0.55 { "▂" } else { "▁" };
        let rgb = mix_oklab(pal.warn, pal.error, value);
        put(buf, x, y, ch, color(rgb), None);
    }
}

/// Escala del canvas de fondo respecto a la pantalla.
pub(super) const VIZ_SCALE: u16 = 1;

pub(super) fn blit_viz(
    app: &App,
    buf: &mut Buffer,
    dst: Rect,
    src_y0: u16,
    fg_amount: f32,
    bg_amount: f32,
    scale: u16,
) {
    blit_layer(&app.viz.output, buf, dst, src_y0, fg_amount, bg_amount, scale);
}

/// Copia una capa ya renderizada. La usa el fondo ambiental (que vive en un
/// canvas aparte) y el visualizador normal.
pub(super) fn blit_layer(
    layer: &crate::viz::Layer,
    buf: &mut Buffer,
    dst: Rect,
    src_y0: u16,
    fg_amount: f32,
    bg_amount: f32,
    scale: u16,
) {
    if layer.w == 0 || layer.h == 0 {
        return;
    }
    let area = *buf.area();
    let clip = dst.intersection(area);
    if clip.width == 0 || clip.height == 0 {
        return;
    }
    let width = area.width as usize;
    for y in clip.y..clip.y + clip.height {
        let start = (y - area.y) as usize * width + (clip.x - area.x) as usize;
        let sy = (src_y0 + (y - dst.y) / scale).min(layer.h.saturating_sub(1));
        for (offset, cell) in buf.content[start..start + clip.width as usize]
            .iter_mut()
            .enumerate()
        {
            let x = (clip.x + offset as u16 - dst.x) / scale;
            let sx = x.min(layer.w.saturating_sub(1));
            let Some(src) = layer.get(sx as i32, sy as i32) else {
                continue;
            };
            if src.alpha <= 0.01 {
                continue;
            }
            let pal_hint = (11u8, 11u8, 16u8);
            let fg = mix(pal_hint, src.fg, src.alpha * fg_amount);
            let bg = if bg_amount <= 0.001 {
                pal_hint
            } else {
                mix(pal_hint, src.fg, src.alpha * bg_amount)
            };
            let mut glyph = [0u8; 4];
            cell.set_symbol(src.ch.encode_utf8(&mut glyph));
            cell.set_fg(color(fg));
            cell.set_bg(color(bg));
        }
    }
}

// ---------------------------------------------------------------------------
// Transporte
// ---------------------------------------------------------------------------
