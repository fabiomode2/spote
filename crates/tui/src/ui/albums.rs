//! Pantalla de álbumes: tarjetas de 3 filas y tracklist del álbum abierto.

use super::*;

pub(super) fn draw_albums(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    if layout.content.width < 10 {
        return;
    }
    if let Some(album) = app.album_open.clone() {
        draw_album_detail(app, buf, layout, pal, &album);
    } else {
        draw_album_list(app, buf, layout, pal);
    }
}

fn draw_album_list(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.content;
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.72,
        Some("discos"),
        app.audio.kick_pulse,
    );
    let inner = inset(rect, 2, 2);
    // Buscador de discos guardados, con el mismo lenguaje visual que el
    // buscador inline de la biblioteca.
    let search = Rect::new(inner.x, inner.y, inner.width, 1);
    fill(
        buf,
        search,
        if app.album_search_focus {
            pal.surface_alt
        } else {
            pal.surface
        },
    );
    let mut search_text = if app.album_query.is_empty() {
        "buscar discos…  ( / )".to_string()
    } else {
        format!("⌕ {}", app.album_query)
    };
    if app.album_search_focus {
        search_text.push('▏');
    }
    truncate_chars(&mut search_text, search.width.saturating_sub(1) as usize);
    text_at(
        buf,
        search,
        search_text,
        Style::new().fg(color(if app.album_search_focus {
            pal.text
        } else if app.album_query.is_empty() {
            pal.dim
        } else {
            pal.primary
        })),
        Alignment::Left,
    );
    app.hit.add(search, Action::AlbumSearch);
    let list = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    if app.albums.is_empty() {
        let message = if !app.album_query.is_empty() {
            "sin discos para la búsqueda · Esc limpia"
        } else if app.lib.count().unwrap_or(0) > 0 {
            "sin metadatos de álbum · usa clic derecho → portada de Spotify"
        } else {
            "biblioteca vacía · / para buscar o spote ingest <dir>"
        };
        text_at(
            buf,
            Rect::new(list.x, list.y + 1, list.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }
    // A partir de aquí se usan las coordenadas de la lista, no las del panel.
    let inner = list;
    let card_h = 3u16;
    let columns = if inner.width >= 100 { 2u16 } else { 1 };
    let card_w = (inner.width.saturating_sub(columns.saturating_sub(1) * 2)) / columns;
    let per_col = (inner.height / card_h).max(1) as usize;
    let visible = per_col * columns as usize;
    let start = app
        .album_selected
        .saturating_sub(visible / 2)
        .min(app.albums.len().saturating_sub(visible.min(app.albums.len())));
    for offset in 0..visible {
        let index = start + offset;
        let Some(album) = app.albums.get(index).cloned() else { break };
        let column = offset as u16 / per_col as u16;
        let row = offset % per_col;
        let x = inner.x + column * (card_w + 2);
        let y = inner.y + row as u16 * card_h;
        if y + card_h > inner.y + inner.height || x + card_w > inner.x + inner.width {
            break;
        }
        let card = Rect::new(x, y, card_w, card_h);
        let selected = index == app.album_selected;
        let hover = app.hover == Some(Action::AlbumOpen(index));
        fill_soft(buf, card, mix(pal.bg, pal.surface, 0.45), 1.0, pal.bg);
        if selected {
            fill(buf, card, mix(pal.surface, pal.primary, 0.18));
            put(buf, card.x, y, "▍", color(pal.primary), None);
        } else if hover {
            fill(buf, card, pal.surface_alt);
        }
        let thumb = Rect::new(card.x + 1, y, 6, card_h);
        app.ensure_album_art(&album);
        app.request_album_thumb(&album, 6, card_h);
        let track = App::album_thumb_track(&album);
        draw_thumb(app, buf, thumb, &track, pal);
        let text_x = thumb.x + thumb.width + 2;
        let text_w = (card.x + card.width).saturating_sub(text_x + 2);
        let mut title = album.album.clone();
        truncate_chars(&mut title, text_w as usize);
        put(
            buf,
            text_x,
            y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        let mut artist = album.artist.clone();
        truncate_chars(&mut artist, text_w as usize);
        put(buf, text_x, y + 1, &artist, color(pal.secondary), None);
        app.hit.add(
            Rect::new(text_x, y + 1, artist.chars().count() as u16, 1),
            Action::OpenArtist(album.artist.clone()),
        );
        let mut meta = format!(
            "{} {} · {}/{} descargadas · {}",
            album.track_count,
            if album.track_count == 1 { "pista" } else { "pistas" },
            album.ready_count,
            album.track_count,
            format_ms(album.duration_ms)
        );
        if album.favorite_count > 0 {
            meta.push_str(" · ♥");
        }
        truncate_chars(&mut meta, text_w as usize);
        put(buf, text_x, y + 2, &meta, color(pal.dim), None);
        app.hit.add(card, Action::AlbumOpen(index));
    }
    let footer = "enter abre · d baja el álbum · s guarda como lista · / busca";
    if rect.height > 4 {
        text_at(
            buf,
            Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
            footer,
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

fn draw_album_detail(
    app: &mut App,
    buf: &mut Buffer,
    layout: &Layout,
    pal: &Palette,
    album: &spote_core::library::AlbumRow,
) {
    let rect = layout.content;
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.78,
        Some("álbum"),
        app.audio.kick_pulse,
    );
    let inner = inset(rect, 2, 2);
    let header_h = 7u16.min(inner.height.saturating_sub(3));
    let thumb = Rect::new(inner.x, inner.y, 12.min(inner.width / 3), header_h);
    app.request_album_thumb(album, thumb.width, thumb.height);
    let track = App::album_thumb_track(album);
    draw_thumb(app, buf, thumb, &track, pal);
    let text_x = thumb.x + thumb.width + 2;
    let _text_w = (inner.x + inner.width).saturating_sub(text_x);
    put(
        buf,
        text_x,
        inner.y,
        &album.album,
        color(pal.text),
        None,
    );
    put(
        buf,
        text_x,
        inner.y + 1,
        &album.artist,
        color(pal.secondary),
        None,
    );
    app.hit.add(
        Rect::new(
            text_x,
            inner.y + 1,
            album.artist.chars().count() as u16,
            1,
        ),
        Action::OpenArtist(album.artist.clone()),
    );
    put(
        buf,
        text_x,
        inner.y + 2,
        &format!(
            "{} {} · {}/{} descargadas · {}",
            album.track_count,
            if album.track_count == 1 { "pista" } else { "pistas" },
            album.ready_count,
            album.track_count,
            format_ms(album.duration_ms)
        ),
        color(pal.dim),
        None,
    );
    // Acciones clicables.
    let mut x = text_x;
    for (label, action) in [
        ("d bajar", Action::AlbumDownload),
        ("s guardar lista", Action::AlbumSavePlaylist),
        ("esc volver", Action::AlbumBack),
    ] {
        let width = label.chars().count() as u16 + 2;
        if x + width > inner.x + inner.width {
            break;
        }
        let chip = Rect::new(x, inner.y + 4, width, 1);
        let hover = app.hover == Some(action.clone());
        fill(buf, chip, if hover { pal.surface_alt } else { pal.surface });
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if matches!(action, Action::AlbumDownload) {
                pal.warn
            } else {
                pal.muted
            })),
            Alignment::Center,
        );
        app.hit.add(chip, action);
        x += width + 1;
    }

    let list_y = inner.y + header_h + 1;
    let rows = (inner.y + inner.height).saturating_sub(list_y + 1) as usize;
    if app.view.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, list_y, inner.width, 1),
            "álbum vacío",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        return;
    }
    let start = app
        .selected
        .saturating_sub(rows / 2)
        .min(app.view.len().saturating_sub(rows.min(app.view.len())));
    for offset in 0..rows {
        let index = start + offset;
        let Some(row) = app.view.get(index) else { break };
        let y = list_y + offset as u16;
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == app.selected;
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.2));
            put(buf, row_rect.x, y, "▍", color(pal.primary), None);
        }
        let duration = format_ms(row.track.duration_ms);
        let d_w = duration.chars().count() as u16;
        let marker = if row.playable() { "·" } else { "↓" };
        put(
            buf,
            row_rect.x + 2,
            y,
            marker,
            color(if row.playable() { pal.dim } else { pal.warn }),
            None,
        );
        let mut title = row.track.title.clone();
        truncate_chars(&mut title, row_rect.width.saturating_sub(8 + d_w) as usize);
        put(
            buf,
            row_rect.x + 4,
            y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        put(
            buf,
            row_rect.x + row_rect.width - d_w,
            y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row_rect, Action::SelectRow(index));
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "enter reproduce · clic derecho acciones · esc vuelve",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}
