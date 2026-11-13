//! Búsqueda, biblioteca y salas.
use super::*;

pub(super) fn draw_library(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    let title = match app.lib_filter {
        crate::app::LibFilter::All => "librería".to_string(),
        crate::app::LibFilter::Playlist(id) => app
            .lib
            .playlist(id)
            .ok()
            .flatten()
            .map(|list| format!("librería · {}", list.name))
            .unwrap_or_else(|| "librería · lista".to_string()),
        filter => format!("librería · {}", filter.label()),
    };
    panel(
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.72,
        Some(title.as_str()),
        app.audio.kick_pulse,
    );
    let inner = Rect::new(
        rect.x + 2,
        rect.y + 2,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(3),
    );
    // Chips de gestión de listas, arriba a la derecha del panel.
    let mut chip_right = inner.x + inner.width;
    let in_playlist = matches!(app.lib_filter, crate::app::LibFilter::Playlist(_));
    let mut chips: Vec<(&str, Action, bool)> = vec![
        ("N lista", Action::NewPlaylist, false),
        ("I importar", Action::ImportPlaylist, false),
        ("l añadir", Action::AddToPlaylist, false),
    ];
    if let crate::app::LibFilter::Playlist(id) = app.lib_filter {
        chips.push(("d borrar", Action::DeletePlaylist(id), true));
    }
    for (label, action, danger) in chips {
        let width = label.chars().count() as u16 + 2;
        if chip_right < inner.x + width + 4 {
            break;
        }
        chip_right -= width;
        let chip = Rect::new(chip_right, inner.y, width, 1);
        let hover = app.hover == Some(action.clone());
        fill(
            buf,
            chip,
            if hover { pal.surface_alt } else { pal.surface },
        );
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if danger { pal.error } else { pal.dim })),
            Alignment::Center,
        );
        app.hit.add(chip, action);
        chip_right = chip_right.saturating_sub(1);
    }

    // Cabecera de la lista: portada y descripción, si hay una abierta.
    let mut list_area = inner;
    if let crate::app::LibFilter::Playlist(id) = app.lib_filter {
        if let Ok(Some(list)) = app.lib.playlist(id) {
            let header = Rect::new(inner.x, inner.y + 2, inner.width, 3);
            let cover = app.lib.playlist_cover(id);
            if let Some(cover) = cover {
                let track = if cover.starts_with("http") {
                    spote_protocol::TrackRef {
                        key: format!("playlist:{id}"),
                        title: list.name.clone(),
                        artist: String::new(),
                        album: None,
                        duration_ms: 0,
                        art_url: Some(cover),
                        source_hint: None,
                    }
                } else {
                    app.lib
                        .track_by_key(&cover)
                        .ok()
                        .flatten()
                        .map(|row| row.track)
                        .unwrap_or_else(|| spote_protocol::TrackRef::new("", "", ""))
                };
                draw_thumb(app, buf, Rect::new(header.x, header.y, 7, 3), &track, pal);
            }
            let text_x = header.x + 8;
            let text_w = (header.x + header.width).saturating_sub(text_x);
            let desc = if list.description.is_empty() {
                "portada automática".to_string()
            } else {
                list.description.clone()
            };
            let mut desc = desc;
            truncate_chars(&mut desc, text_w as usize);
            put(
                buf,
                text_x,
                header.y,
                &desc,
                color(if list.description.is_empty() { pal.dim } else { pal.muted }),
                None,
            );
            put(
                buf,
                text_x,
                header.y + 1,
                &format!("{} pistas", list.count),
                color(pal.dim),
                None,
            );
            list_area = Rect::new(
                inner.x,
                inner.y + 5,
                inner.width,
                inner.height.saturating_sub(5),
            );
        }
    }

    if app.view.is_empty() {
        let message = if in_playlist {
            "lista vacía · l añade desde la biblioteca · d borra la lista"
        } else {
            "librería vacía · / para buscar · spote ingest <dir> para importar"
        };
        text_at(
            buf,
            Rect::new(list_area.x, list_area.y + 1, list_area.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }
    if app.list_cards {
        draw_cards(app, buf, list_area, pal);
        return;
    }
    let inner = list_area;
    let rows = inner.height as usize;
    let start = (app.list_scroll.value.max(0.0) as usize)
        .saturating_sub(rows / 2)
        .min(app.view.len().saturating_sub(rows.min(app.view.len())));
    let shown = app.view.len().saturating_sub(start).min(rows) as u16;
    let y_offset = if app.vcenter {
        inner.height.saturating_sub(shown) / 2
    } else {
        0
    };
    for (offset, row) in app.view.iter().skip(start).take(rows).enumerate() {
        let index = start + offset;
        let y = inner.y + y_offset + offset as u16;
        let selected = index == app.selected;
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        let hover = app.hover == Some(Action::SelectRow(index));
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.20));
            put(buf, row_rect.x, y, "▍", color(pal.primary), None);
        } else if hover {
            fill(buf, row_rect, pal.surface_alt);
        }
        let ready = row.playable();
        let duration = format_ms(row.track.duration_ms);
        let duration_width = duration.chars().count() as u16;
        let playing = app
            .now
            .track
            .as_ref()
            .is_some_and(|track| track.key == row.track.key);
        let marker = if playing {
            "▶"
        } else if ready {
            "·"
        } else {
            "↓"
        };
        let marker_color = if playing {
            pal.primary
        } else if ready {
            pal.dim
        } else {
            pal.warn
        };
        put(buf, row_rect.x + 2, y, marker, color(marker_color), None);
        // Corazón siempre visible si es favorita, con su propio hueco. Si la
        // pista se está descargando, ese hueco lo ocupa el progreso.
        let heart_x = row_rect.x + row_rect.width - duration_width - 2;
        if let Some(download) = app.download_for(&row.track.key) {
            let label = format!("↓{:.0}%", download.percent);
            let label_x = heart_x.saturating_sub(label.len() as u16);
            put(buf, label_x, y, &label, color(pal.warn), None);
        } else if row.favorite {
            put(buf, heart_x, y, "♥", color(pal.error), None);
        }
        let title_width = row_rect.width.saturating_sub(6 + duration_width + 3) as usize;
        let style = Style::new()
            .fg(color(if selected { pal.text } else { pal.muted }))
            .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() });
        let mut title = row.track.title.clone();
        truncate_chars(&mut title, title_width);
        put(buf, row_rect.x + 4, y, &title, style.fg.unwrap_or(color(pal.text)), None);
        let title_len = title.chars().count() as u16;
        let artist_x = row_rect.x + 4 + title_len + 2;
        let max_artist =
            (heart_x).saturating_sub(artist_x + 1);
        if max_artist > 4 {
            let mut artist = row.track.artist.clone();
            truncate_chars(&mut artist, max_artist as usize);
            put(buf, artist_x, y, &artist, color(pal.dim), None);
        }
        put(
            buf,
            row_rect.x + row_rect.width - duration_width,
            y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row_rect, Action::SelectRow(index));
    }
}

/// Lista en tarjetas de 3 filas: miniatura, título, artista · álbum, badges.
pub(super) fn draw_cards(app: &mut App, buf: &mut Buffer, inner: Rect, pal: &Palette) {
    let card_h = 3u16;
    let per_page = (inner.height / card_h).max(1) as usize;
    let start = app.selected.saturating_sub(per_page / 2);
    let start = start.min(app.view.len().saturating_sub(per_page.min(app.view.len())));
    let rows: Vec<spote_core::library::TrackRow> = app
        .view
        .iter()
        .skip(start)
        .take(per_page)
        .cloned()
        .collect();
    let mut y = inner.y
        + if app.vcenter {
            inner.height.saturating_sub(rows.len() as u16 * card_h) / 2
        } else {
            0
        };
    for (offset, row) in rows.iter().enumerate() {
        let index = start + offset;
        if y + card_h > inner.y + inner.height {
            break;
        }
        let selected = index == app.selected;
        let hover = app.hover == Some(Action::SelectRow(index));
        let card = Rect::new(inner.x, y, inner.width, card_h);
        fill_soft(buf, card, mix(pal.bg, pal.surface, 0.45), 1.0, pal.bg);
        if selected {
            fill(buf, card, mix(pal.surface, pal.primary, 0.18));
        } else if hover {
            fill(buf, card, pal.surface_alt);
        }
        let thumb = Rect::new(inner.x + 1, y, 6, card_h);
        draw_thumb(app, buf, thumb, &row.track, pal);
        let text_x = thumb.x + thumb.width + 2;
        let text_w = (inner.x + inner.width).saturating_sub(text_x + 1);
        let playing = app
            .now
            .track
            .as_ref()
            .is_some_and(|track| track.key == row.track.key);
        let title = if playing {
            format!("▶ {}", row.track.title)
        } else {
            row.track.title.clone()
        };
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
        let mut artist = row.track.artist.clone();
        if let Some(album) = &row.track.album {
            artist.push_str("  ·  ");
            artist.push_str(album);
        }
        truncate_chars(&mut artist, text_w as usize);
        put(buf, text_x, y + 1, &artist, color(pal.dim), None);
        let mut meta = format!("{}  ·  {}", format_ms(row.track.duration_ms), status_label(row));
        if row.favorite {
            meta.push_str("  ·  ♥");
        }
        if !row.playable() {
            meta.push_str("  ·  ↓ pendiente");
        }
        truncate_chars(&mut meta, text_w as usize);
        put(
            buf,
            text_x,
            y + 2,
            &meta,
            color(if row.favorite { pal.error } else { pal.dim }),
            None,
        );
        app.hit.add(card, Action::SelectRow(index));
        y += card_h;
    }
}

pub(super) fn status_label(row: &spote_core::library::TrackRow) -> &'static str {
    use spote_core::library::SourceStatus;
    match row.status {
        SourceStatus::Ready => "local",
        SourceStatus::Pending => "pendiente",
        SourceStatus::Downloading => "descargando",
        SourceStatus::Failed => "error",
        SourceStatus::Missing => "perdida",
    }
}

pub(super) fn draw_search(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    panel(buf, rect, pal, true, 0.78, Some("buscar"), app.audio.kick_pulse);
    let inner = Rect::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(4), rect.height.saturating_sub(3));

    // Campo de búsqueda.
    let input_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    fill(buf, input_rect, pal.bg_deep);
    put(buf, input_rect.x + 1, input_rect.y, "/", color(pal.primary), None);
    let empty = app.search.query.is_empty();
    let query = if empty {
        "escribe y pulsa enter…".to_string()
    } else {
        app.search.query.clone()
    };
    let fg = if empty { pal.dim } else { pal.text };
    put(buf, input_rect.x + 3, input_rect.y, &query, color(fg), None);
    // Cursor solo con texto: con el placeholder se ve más limpio.
    let cursor_x = input_rect.x + 3 + app.search.query.chars().count() as u16;
    if !empty && cursor_x < input_rect.x + input_rect.width {
        put(buf, cursor_x, input_rect.y, "▌", color(pal.primary), None);
    }

    let mut y = inner.y + 2;
    if app.search.loading {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "buscando en el catálogo…",
            Style::new().fg(color(pal.warn)),
            Alignment::Left,
        );
        y += 1;
    } else if !app.search.message.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            app.search.message.clone(),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        y += 1;
    }

    let rows = (inner.y + inner.height).saturating_sub(y) as usize;
    for (index, item) in app.search.results.iter().enumerate().take(rows) {
        let row_y = y + index as u16;
        let selected = index == app.search.selected;
        let row_rect = Rect::new(inner.x, row_y, inner.width, 1);
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.22));
            put(buf, row_rect.x, row_y, "▍", color(pal.primary), None);
        }
        let duration = format_ms(item.duration_ms);
        let duration_width = duration.chars().count() as u16;
        let topic = item.channel.ends_with(" - Topic");
        let marker = if topic { "♪" } else { "▷" };
        put(
            buf,
            row_rect.x + 2,
            row_y,
            marker,
            color(if selected { pal.secondary } else { pal.dim }),
            None,
        );
        // Título y artista/álbum en la misma fila: con solo el título no se
        // sabía qué versión era.
        let title_width = row_rect.width.saturating_sub(6 + duration_width) as usize;
        let mut title = item.title.clone();
        let title_chars = title.chars().count();
        truncate_chars(&mut title, title_width);
        put(
            buf,
            row_rect.x + 4,
            row_y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        let after = row_rect.x + 4 + title_chars.min(title_width) as u16 + 2;
        let meta = match &item.album {
            Some(album) if !album.is_empty() => format!("{}  ·  {}", item.artist, album),
            _ => item.artist.clone(),
        };
        let meta_room = (row_rect.x + row_rect.width - duration_width - 2)
            .saturating_sub(after);
        if meta_room > 6 && title_width > title_chars {
            let mut meta = meta;
            truncate_chars(&mut meta, meta_room as usize);
            put(buf, after, row_y, &meta, color(pal.dim), None);
        }
        put(
            buf,
            row_rect.x + row_rect.width - duration_width,
            row_y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row_rect, Action::SearchPlay(index));
    }
}

pub(super) fn draw_room(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.82);
        frame_art(buf, layout.art, pal);
    }
    panel(
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.85,
        Some("salas"),
        app.audio.kick_pulse,
    );
    let inner = Rect::new(
        rect.x + 2,
        rect.y + 2,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(3),
    );
    let mut y = inner.y;

    if app.social.is_none() {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "sin servidor · configura [server] url",
            Style::new().fg(color(pal.warn)),
            Alignment::Left,
        );
        return;
    }
    let status = if app.social_status.is_empty() {
        app.cfg.server.url.clone()
    } else {
        format!("{}  ·  {}", app.cfg.server.url, app.social_status)
    };
    let mut status = status;
    truncate_chars(&mut status, inner.width as usize);
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        status,
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    y += 2;

    // Salas disponibles.
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        "SALAS",
        Style::new()
            .fg(color(pal.primary))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    y += 1;
    if app.rooms.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "ninguna sala · crea una o espera a que te inviten",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        y += 1;
    }
    let max_rooms = 5usize;
    for (index, room) in app.rooms.iter().take(max_rooms).enumerate() {
        let row = Rect::new(inner.x, y, inner.width, 1);
        let active = index == app.room_selected;
        let joined = app.room.as_ref().is_some_and(|current| current.id == room.id);
        if active {
            fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        let label = format!(
            "{} {}  ·  {} miembros{}",
            if joined { "◉" } else { "○" },
            app.room_label(&room.id),
            room.members.len(),
            if joined { "  ·  dentro" } else { "" }
        );
        let mut label = label;
        truncate_chars(&mut label, row.width.saturating_sub(3) as usize);
        put(
            buf,
            row.x + 2,
            y,
            &label,
            color(if active { pal.text } else { pal.muted }),
            None,
        );
        app.hit.add(row, Action::JoinRoom(index));
        y += 1;
    }
    y += 1;

    // Sala actual.
    if let Some(state) = &app.room_state {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "AHORA EN LA SALA",
            Style::new()
                .fg(color(pal.primary))
                .add_modifier(Modifier::BOLD),
            Alignment::Left,
        );
        y += 1;
        let track = state
            .track
            .as_ref()
            .map(|track| track.label())
            .unwrap_or_else(|| "nada sonando".into());
        let mut line = format!(
            "{}  {}  ·  {}",
            if state.playing { "▶" } else { "⏸" },
            track,
            format_ms(state.position_ms)
        );
        truncate_chars(&mut line, inner.width as usize);
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            line,
            Style::new().fg(color(pal.text)),
            Alignment::Left,
        );
        y += 1;
        if let Some(room) = &app.room {
            for member in room.members.iter().take(6) {
                let presence = app
                    .presence
                    .iter()
                    .find(|presence| presence.user.id == member.user.id);
                let playing = presence
                    .and_then(|presence| presence.now_playing.as_ref())
                    .map(|track| track.label())
                    .unwrap_or_else(|| "—".into());
                let mut label = format!(
                    "{} {}{}  ·  {}",
                    if presence.is_some_and(|presence| presence.online) {
                        "●"
                    } else {
                        "○"
                    },
                    member.user.name,
                    if member.role == spote_protocol::Role::Host {
                        " (host)"
                    } else {
                        ""
                    },
                    playing
                );
                truncate_chars(&mut label, inner.width as usize);
                text_at(
                    buf,
                    Rect::new(inner.x, y, inner.width, 1),
                    label,
                    Style::new().fg(color(pal.muted)),
                    Alignment::Left,
                );
                y += 1;
            }
        }
        y += 1;
        if !app.room_queue.is_empty() {
            text_at(
                buf,
                Rect::new(inner.x, y, inner.width, 1),
                "COLA COMPARTIDA",
                Style::new()
                    .fg(color(pal.primary))
                    .add_modifier(Modifier::BOLD),
                Alignment::Left,
            );
            y += 1;
            for item in app.room_queue.iter().take(4) {
                let mut label = format!("{:>2} · {}  ·  {}", item.position + 1, item.track.title, item.track.artist);
                truncate_chars(&mut label, inner.width as usize);
                text_at(
                    buf,
                    Rect::new(inner.x, y, inner.width, 1),
                    label,
                    Style::new().fg(color(pal.muted)),
                    Alignment::Left,
                );
                y += 1;
            }
        }
    } else {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "enter sobre una sala para entrar · L para salir",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

// ---------------------------------------------------------------------------
// Panel lateral
// ---------------------------------------------------------------------------
