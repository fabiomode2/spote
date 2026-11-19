//! Búsqueda, biblioteca y salas.
use super::*;

pub(super) fn draw_library(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() && layout.art.width > 0 {
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
        Some(app),
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

    // Cabecera de la lista: portada y descripción, si hay una abierta. Los
    // chips de gestión ocupan la primera fila; la lista empieza debajo para
    // que las canciones no se pinten encima de «N lista / I importar».
    let mut list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
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

    if !app.view.is_empty() {
        // Backfill silencioso: las canciones viejas sin portada la piden solas
        // a Spotify (máximo 3 a la vez, una sola vez por pista).
        let missing: Vec<_> = app
            .view
            .iter()
            .filter(|row| row.art_path.is_none() && row.track.art_url.is_none())
            .take(3)
            .cloned()
            .collect();
        for row in missing {
            app.ensure_missing_art(&row);
        }
    }
    if app.view.is_empty() {
        let message = if in_playlist {
            "lista vacía · l añade desde la biblioteca · d borra la lista"
        } else if app.lib.count().unwrap_or(0) > 0 {
            "no hay pistas en este filtro · [ ] cambia de filtro"
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
    let row_h = 2u16;
    let rows = (inner.height / row_h).max(1) as usize;
    let start = keep_visible(app.list_start, app.selected, rows, app.view.len());
    app.list_start = start;
    let shown = app.view.len().saturating_sub(start).min(rows) as u16;
    let y_offset = if app.vcenter {
        inner.height.saturating_sub(shown * row_h) / 2
    } else {
        0
    };
    let g = list_geometry(inner);
    // Cabecera de columnas en la fila libre bajo los chips.
    let header_y = list_area.y.saturating_sub(1);
    if header_y > 0 && inner.width >= 40 {
        let h = header_y;
        put(buf, g.num_x, h, "#", color(pal.dim), None);
        put(buf, g.text_x, h, "TÍTULO", color(pal.dim), None);
        if g.album_w > 6 {
            put(buf, g.album_x, h, "ÁLBUM", color(pal.dim), None);
        }
        if g.date_w > 0 {
            put(buf, g.date_x, h, "AÑADIDA", color(pal.dim), None);
        }
        let right = inner.x + inner.width;
        put(buf, right.saturating_sub(1), h, "⏱", color(pal.dim), None);
    }
    // Columnas al estilo de Spotify: número, miniatura, título + artista
    // debajo, álbum, fecha de alta y duración. El ancho decide qué columnas
    // caben; nunca se pisan. Se copian las filas visibles porque el dibujo de
    // miniaturas pide prestado `App` entero.
    let visible: Vec<spote_core::library::TrackRow> = app
        .view
        .iter()
        .skip(start)
        .take(rows)
        .cloned()
        .collect();
    for (offset, row) in visible.iter().enumerate() {
        let index = start + offset;
        let y = inner.y + y_offset + offset as u16 * row_h;
        if y + row_h > inner.y + inner.height {
            break;
        }
        let row_rect = Rect::new(inner.x, y, inner.width, row_h);
        let selected = index == app.selected;
        let hover = app.hover == Some(Action::SelectRow(index));
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.20));
        } else if hover {
            fill(buf, row_rect, pal.surface_alt);
        }
        // La fila se registra primero: los enlaces de artista/álbum van
        // después y ganan al clic (antes era al revés y no se podían pulsar).
        app.hit.add(row_rect, Action::SelectRow(index));
        let playing = app
            .now
            .track
            .as_ref()
            .is_some_and(|track| track.key == row.track.key);
        let ready = row.playable();
        let duration = format_ms(row.track.duration_ms);

        if playing {
            put(buf, g.num_x, y, "▶", color(pal.primary), None);
        } else {
            let number = format!("{:>2}", index + 1);
            put(buf, g.num_x, y, &number, color(pal.dim), None);
        }
        if let Some(thumb_x) = g.thumb_x {
            draw_thumb(app, buf, Rect::new(thumb_x, y, 4, row_h), &row.track, pal);
        }
        let mut title = row.track.title.clone();
        truncate_chars(&mut title, g.title_w as usize);
        let title_color = if playing {
            pal.primary
        } else if selected {
            pal.text
        } else {
            pal.muted
        };
        put(buf, g.text_x, y, &title, color(title_color), None);
        let mut artist = row.track.artist.clone();
        truncate_chars(&mut artist, g.title_w as usize);
        let artist_draw_w = artist.chars().count() as u16;
        put(buf, g.text_x, y + 1, &artist, color(pal.dim), None);
        if !artist.is_empty() {
            app.hit.add(
                Rect::new(g.text_x, y + 1, artist_draw_w.min(g.title_w), 1),
                Action::OpenArtist(row.track.artist.clone()),
            );
        }
        if g.album_w > 6 {
            if let Some(album) = row.track.album.as_deref().filter(|album| !album.is_empty()) {
                let mut shown = album.to_string();
                truncate_chars(&mut shown, g.album_w as usize);
                put(
                    buf,
                    g.album_x,
                    y,
                    &shown,
                    color(if selected { pal.muted } else { pal.dim }),
                    None,
                );
                let album_draw_w = shown.chars().count() as u16;
                if album_draw_w > 0 {
                    app.hit.add(
                        Rect::new(g.album_x, y, album_draw_w.min(g.album_w), 1),
                        Action::OpenAlbum(album.to_string(), row.track.artist.clone()),
                    );
                }
            }
        }
        if g.date_w > 0 {
            let mut added = added_label(row.added_at);
            truncate_chars(&mut added, g.date_w as usize);
            put(buf, g.date_x, y, &added, color(pal.dim), None);
        }
        // Estado: favorita, descarga o pendiente, antes de la duración.
        if let Some(download) = app.download_for(&row.track.key) {
            let label = format!("↓{:.0}%", download.percent);
            let label_w = label.chars().count() as u16;
            put(
                buf,
                g.status_x.saturating_sub(label_w),
                y,
                &label,
                color(pal.warn),
                None,
            );
        } else if row.favorite {
            put(buf, g.status_x, y, "♥", color(pal.error), None);
        } else if !ready {
            put(buf, g.status_x, y, "↓", color(pal.warn), None);
        }
        let duration_w = duration.chars().count() as u16;
        put(
            buf,
            (inner.x + inner.width).saturating_sub(duration_w),
            y,
            &duration,
            color(pal.dim),
            None,
        );
    }
}

#[derive(Clone, Copy)]
struct ListGeometry {
    num_x: u16,
    thumb_x: Option<u16>,
    text_x: u16,
    title_w: u16,
    album_x: u16,
    album_w: u16,
    date_x: u16,
    date_w: u16,
    status_x: u16,
}

/// Reparte las columnas de la lista. Se reserva un ancho fijo para duración y
/// estado, así todas las filas alinean aunque una dure 10 minutos.
fn list_geometry(inner: Rect) -> ListGeometry {
    let right_edge = inner.x + inner.width;
    // 5 celdas para «10:00» y 2 de aire antes del icono de estado.
    let status_x = right_edge.saturating_sub(5).saturating_sub(2);
    let num_w = 3u16;
    let thumb_w = if inner.width >= 34 { 5u16 } else { 0 };
    let text_x = inner.x + num_w + thumb_w;
    let usable = status_x.saturating_sub(text_x + 2);
    let gap = 2u16;
    let (mut title_w, mut album_w, mut date_w) = (usable, 0, 0);
    if usable >= 40 {
        date_w = 7;
        let rest = usable - date_w - gap;
        album_w = rest * 30 / 100;
        title_w = rest - album_w - gap;
    } else if usable >= 34 {
        album_w = usable * 32 / 100;
        title_w = usable - album_w - gap;
    }
    if title_w < 10 {
        title_w = usable;
        album_w = 0;
        date_w = 0;
    }
    let album_x = text_x + title_w + gap;
    let date_x = album_x + album_w + if album_w > 0 { gap } else { 0 };
    ListGeometry {
        num_x: inner.x + 1,
        thumb_x: (thumb_w > 0).then_some(inner.x + num_w),
        text_x,
        title_w,
        album_x,
        album_w,
        date_x,
        date_w,
        status_x,
    }
}

/// «hace 3 h», «hace 2 d»… para la columna de fecha de alta.
fn added_label(added_at: i64) -> String {
    if added_at <= 0 {
        return String::new();
    }
    let secs = (crate::app::now_ms() - added_at).max(0) as u64 / 1000;
    match secs {
        0..=59 => "ahora".into(),
        60..=3_599 => format!("{} min", secs / 60),
        3_600..=86_399 => format!("{} h", secs / 3_600),
        86_400..=604_799 => format!("{} d", secs / 86_400),
        604_800..=2_591_999 => format!("{} sem", secs / 604_800),
        2_592_000..=31_535_999 => format!("{} mes", secs / 2_592_000),
        _ => format!("{} a", secs / 31_536_000),
    }
}

/// Lista en tarjetas de 3 filas: miniatura, título, artista · álbum, badges.
pub(super) fn draw_cards(app: &mut App, buf: &mut Buffer, inner: Rect, pal: &Palette) {
    let card_h = 3u16;
    let per_page = (inner.height / card_h).max(1) as usize;
    let start = keep_visible(app.list_start, app.selected, per_page, app.view.len());
    app.list_start = start;
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
        let mut meta = format!(
            "{}  ·  {}  ·  {}",
            format_ms(row.track.duration_ms),
            status_label(row),
            source_tag(row.provider.as_deref())
        );
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
        // El artista de la tarjeta gana al clic de fila: abre su ficha.
        let artist_w = (row.track.artist.chars().count() as u16).min(text_w);
        if artist_w > 0 {
            app.hit.add(
                Rect::new(text_x, y + 1, artist_w, 1),
                Action::OpenArtist(row.track.artist.clone()),
            );
        }
        if let Some(album) = row.track.album.as_deref().filter(|album| !album.is_empty()) {
            let album_x = text_x + artist_w + 3;
            let album_w = (text_x + text_w).saturating_sub(album_x);
            if album_w > 3 {
                app.hit.add(
                    Rect::new(album_x, y + 1, album_w, 1),
                    Action::OpenAlbum(album.to_string(), row.track.artist.clone()),
                );
            }
        }
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
    if app.now.track.is_some() && layout.art.width > 0 {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    panel(Some(app), buf, rect, pal, true, 0.78, Some("buscar"), app.audio.kick_pulse);
    let inner = Rect::new(
        rect.x + 2,
        rect.y + 2,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(3),
    );

    // Campo de búsqueda.
    let input_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    // Limpieza explícita celda a celda: este campo se redibuja mientras se
    // escribe y no puede quedarse el placeholder de la consulta anterior.
    for x in input_rect.x..input_rect.x + input_rect.width {
        put_bg(buf, x, input_rect.y, pal.bg_deep);
    }
    put(buf, input_rect.x + 1, input_rect.y, "/", color(pal.primary), None);
    let empty = app.search.query.is_empty();
    let query = if empty {
        "artistas, álbumes y canciones…".to_string()
    } else {
        app.search.query.clone()
    };
    let fg = if empty { pal.dim } else { pal.text };
    put(buf, input_rect.x + 3, input_rect.y, &query, color(fg), None);
    let cursor_x = input_rect.x + 3 + app.search.query.chars().count() as u16;
    if !empty && cursor_x < input_rect.x + input_rect.width {
        put(buf, cursor_x, input_rect.y, "▌", color(pal.primary), None);
    }

    let mut y = inner.y + 2;
    if app.search.loading {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "buscando artistas, álbumes y canciones…",
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

    // Lista mixta: artistas, álbumes y canciones, como Spotify.
    #[derive(Clone, Copy)]
    enum Row {
        Section(&'static str),
        Entry(crate::app::SearchEntry),
    }
    let mut rows: Vec<Row> = Vec::new();
    if !app.search.artists.is_empty() {
        rows.push(Row::Section("ARTISTAS"));
        for index in 0..app.search.artists.len() {
            rows.push(Row::Entry(crate::app::SearchEntry::Artist(index)));
        }
    }
    if !app.search.albums.is_empty() {
        rows.push(Row::Section("ÁLBUMES"));
        for index in 0..app.search.albums.len() {
            rows.push(Row::Entry(crate::app::SearchEntry::Album(index)));
        }
    }
    if !app.search.results.is_empty() {
        rows.push(Row::Section("CANCIONES"));
        for index in 0..app.search.results.len() {
            rows.push(Row::Entry(crate::app::SearchEntry::Track(index)));
        }
    }
    if rows.is_empty() {
        if !app.search.loading && !empty {
            text_at(
                buf,
                Rect::new(inner.x, y + 1, inner.width, 1),
                "sin resultados · prueba otro título o artista",
                Style::new().fg(color(pal.dim)),
                Alignment::Center,
            );
        }
        return;
    }

    // Cada entrada ocupa 2 filas; las cabeceras 1. Se desplaza la selección.
    let selected_flat = app.search.selected;
    let selected_row = rows
        .iter()
        .position(|row| {
            matches!(row, Row::Entry(entry) if entry_flat(app, *entry) == selected_flat)
        })
        .unwrap_or(0);
    let available = (inner.y + inner.height).saturating_sub(y) as usize;
    let row_h = if app.list_cards { 2 } else { 1 };
    // Scroll por bordes también en la búsqueda: la selección se mueve dentro
    // de la página y la lista solo avanza cuando hace falta. Antes la lista
    // perseguía al cursor y parecía que se movían dos cosas a la vez.
    let height_of = |row: &Row| -> usize {
        match row {
            Row::Section(_) => 1,
            Row::Entry(_) => row_h as usize,
        }
    };
    let mut start = app.search_start.min(rows.len().saturating_sub(1));
    if selected_row < start {
        start = selected_row;
    } else {
        let mut used: usize = rows[start..=selected_row]
            .iter()
            .map(|row| height_of(row))
            .sum();
        while used > available && start < selected_row {
            used = used.saturating_sub(height_of(&rows[start]));
            start += 1;
        }
    }
    app.search_start = start;
    let mut row_y = y;
    for row in rows.iter().skip(start) {
        if row_y + 1 >= inner.y + inner.height {
            break;
        }
        match row {
            Row::Section(label) => {
                text_at(
                    buf,
                    Rect::new(inner.x, row_y, inner.width, 1),
                    *label,
                    Style::new()
                        .fg(color(pal.primary))
                        .add_modifier(Modifier::BOLD),
                    Alignment::Left,
                );
                row_y += 1;
            }
            Row::Entry(entry) if !app.list_cards => {
                let flat = entry_flat(app, *entry);
                let selected = flat == selected_flat;
                let row_rect = Rect::new(inner.x, row_y, inner.width, 1);
                if selected {
                    fill(buf, row_rect, mix(pal.surface, pal.primary, 0.22));
                    put(buf, row_rect.x, row_y, "▍", color(pal.primary), None);
                }
                let (title, meta, right, provider, _) = search_entry_view(app, *entry);
                let right_w = right.chars().count() as u16 + provider.chars().count() as u16 + 3;
                let mut title = format!("{title}  ·  {meta}");
                truncate_chars(&mut title, row_rect.width.saturating_sub(right_w + 4) as usize);
                put(
                    buf,
                    row_rect.x + 2,
                    row_y,
                    &title,
                    color(if selected { pal.text } else { pal.muted }),
                    None,
                );
                if !right.is_empty() {
                    put(
                        buf,
                        row_rect.x + row_rect.width - right_w,
                        row_y,
                        &format!("{right} [{provider}]"),
                        color(pal.dim),
                        None,
                    );
                }
                app.hit.add(row_rect, Action::SearchPlay(flat));
                row_y += 1;
            }
            Row::Entry(entry) => {
                let flat = entry_flat(app, *entry);
                let selected = flat == selected_flat;
                let row_rect = Rect::new(inner.x, row_y, inner.width, 2);
                if selected {
                    fill(buf, row_rect, mix(pal.surface, pal.primary, 0.22));
                    put(buf, row_rect.x, row_y, "▍", color(pal.primary), None);
                }
                let thumb = Rect::new(row_rect.x + 2, row_y, 4, 2);
                let (title, meta, right, provider, thumb_track) = search_entry_view(app, *entry);
                draw_thumb(app, buf, thumb, &thumb_track, pal);
                let text_x = thumb.x + thumb.width + 1;
                let right_w = right.chars().count() as u16;
                let provider_w = provider.chars().count() as u16 + 2;
                let text_w = (row_rect.x + row_rect.width)
                    .saturating_sub(text_x + right_w + provider_w + 3);
                let mut title = title;
                truncate_chars(&mut title, text_w as usize);
                put(
                    buf,
                    text_x,
                    row_y,
                    &title,
                    color(if selected { pal.text } else { pal.muted }),
                    None,
                );
                let mut meta = meta;
                truncate_chars(&mut meta, text_w as usize);
                put(buf, text_x, row_y + 1, &meta, color(pal.dim), None);
                if !right.is_empty() {
                    put(
                        buf,
                        row_rect.x + row_rect.width - right_w,
                        row_y,
                        &right,
                        color(pal.dim),
                        None,
                    );
                }
                // Etiqueta de proveedor, pequeña, a la derecha de la meta.
                if !provider.is_empty() {
                    let tag = format!("[{provider}]");
                    let tw = tag.chars().count() as u16;
                    if text_x + text_w + 1 + tw <= row_rect.x + row_rect.width {
                        put(
                            buf,
                            row_rect.x + row_rect.width - tw,
                            row_y + 1,
                            &tag,
                            color(pal.secondary),
                            None,
                        );
                    }
                }
                app.hit.add(row_rect, Action::SearchPlay(flat));
                if let Some(artist) = search_entry_artist(app, *entry) {
                    let width = artist.chars().count() as u16;
                    app.hit.add(
                        Rect::new(text_x, row_y + 1, width.min(text_w), 1),
                        Action::OpenArtist(artist.clone()),
                    );
                    if let Some(album) = search_entry_album(app, *entry) {
                        let album_x = text_x + width.min(text_w) + 3;
                        let album_w = (row_rect.x + row_rect.width).saturating_sub(album_x);
                        if album_w > 3 {
                            app.hit.add(
                                Rect::new(album_x, row_y + 1, album_w, 1),
                                Action::OpenAlbum(album, artist),
                            );
                        }
                    }
                }
                row_y += 2;
            }
        }
    }
}

fn entry_flat(app: &App, entry: crate::app::SearchEntry) -> usize {
    match entry {
        crate::app::SearchEntry::Artist(i) => i,
        crate::app::SearchEntry::Album(i) => app.search.artists.len() + i,
        crate::app::SearchEntry::Track(i) => {
            app.search.artists.len() + app.search.albums.len() + i
        }
    }
}

fn search_entry_view(
    app: &App,
    entry: crate::app::SearchEntry,
) -> (String, String, String, String, spote_protocol::TrackRef) {
    match entry {
        crate::app::SearchEntry::Artist(index) => {
            let Some(artist) = app.search.artists.get(index) else {
                return (String::new(), String::new(), String::new(), String::new(), empty_track());
            };
            let meta = if artist.followers > 0 {
                format!("artista · {} fans", artist.followers)
            } else {
                "artista".to_string()
            };
            (
                artist.name.clone(),
                meta,
                String::new(),
                artist.provider.clone(),
                spote_protocol::TrackRef {
                    key: format!("searchartist:{}:{}", artist.provider, artist.id),
                    title: artist.name.clone(),
                    artist: artist.name.clone(),
                    album: None,
                    duration_ms: 0,
                    art_url: artist.image_url.clone(),
                    source_hint: None,
                },
            )
        }
        crate::app::SearchEntry::Album(index) => {
            let Some(album) = app.search.albums.get(index) else {
                return (String::new(), String::new(), String::new(), String::new(), empty_track());
            };
            let mut meta = album.artist.clone();
            if let Some(year) = album.release_year {
                meta.push_str(&format!(" · {year}"));
            }
            if album.track_count > 0 {
                meta.push_str(&format!(" · {} pistas", album.track_count));
            }
            (
                album.title.clone(),
                meta,
                "álbum".to_string(),
                album.provider.clone(),
                spote_protocol::TrackRef {
                    key: format!("searchalbum:{}:{}", album.provider, album.id),
                    title: album.title.clone(),
                    artist: album.artist.clone(),
                    album: Some(album.title.clone()),
                    duration_ms: 0,
                    art_url: album.cover_url.clone(),
                    source_hint: None,
                },
            )
        }
        crate::app::SearchEntry::Track(index) => {
            let Some(item) = app.search.results.get(index) else {
                return (String::new(), String::new(), String::new(), String::new(), empty_track());
            };
            let mut meta = item.artist.clone();
            if let Some(album) = item.album.as_deref().filter(|album| !album.is_empty()) {
                meta.push_str(" · ");
                meta.push_str(album);
            }
            (
                item.title.clone(),
                meta,
                format_ms(item.duration_ms),
                item.provider.clone(),
                item.to_track(),
            )
        }
    }
}

fn search_entry_album(app: &App, entry: crate::app::SearchEntry) -> Option<String> {
    match entry {
        crate::app::SearchEntry::Artist(_) => None,
        crate::app::SearchEntry::Album(index) => app.search.albums.get(index).map(|album| album.title.clone()),
        crate::app::SearchEntry::Track(index) => app
            .search
            .results
            .get(index)
            .and_then(|item| item.album.clone()),
    }
}

fn search_entry_artist(app: &App, entry: crate::app::SearchEntry) -> Option<String> {
    match entry {
        crate::app::SearchEntry::Artist(_) => None,
        crate::app::SearchEntry::Album(index) => {
            app.search.albums.get(index).map(|album| album.artist.clone())
        }
        crate::app::SearchEntry::Track(index) => app
            .search
            .results
            .get(index)
            .map(|item| item.artist.clone()),
    }
}

fn empty_track() -> spote_protocol::TrackRef {
    spote_protocol::TrackRef::new("", "", "")
}

pub(super) fn draw_room(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.82);
        frame_art(buf, layout.art, pal);
    }
    panel(
        Some(app),
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

    // La antigua "sesión" cabe aquí en dos líneas: modo, viz, arte y tools.
    if inner.height > 4 {
        let tools = if app.tools.missing().is_empty() {
            "mpv · ffmpeg · yt-dlp ok".to_string()
        } else {
            format!("faltan: {}", app.tools.missing().join(", "))
        };
        let mode = if app.room.is_some() {
            "sala"
        } else if app.follow_spotify {
            "spotify"
        } else if app.social.is_some() {
            "servidor"
        } else {
            "local"
        };
        let sesion = format!(
            "sesión · {mode} · viz {} · arte {} · {:.0} bpm",
            app.viz.preset_name(),
            app.art_mode.label(),
            app.audio.bpm
        );
        let mut sesion = sesion;
        truncate_chars(&mut sesion, inner.width as usize);
        put(
            buf,
            inner.x,
            rect.y + rect.height - 3,
            &sesion,
            color(pal.dim),
            None,
        );
        let mut tools = tools;
        truncate_chars(&mut tools, inner.width as usize);
        put(
            buf,
            inner.x,
            rect.y + rect.height - 2,
            &tools,
            color(if app.tools.missing().is_empty() { pal.dim } else { pal.warn }),
            None,
        );
    }
}

// ---------------------------------------------------------------------------
// Panel lateral
// ---------------------------------------------------------------------------


/// Ajusta la primera fila visible para que la selección esté dentro de la
/// página. Solo desplaza cuando el cursor cruza un borde: así no hay dos
/// movimientos (fila y cámara) por cada pulsación.
pub(super) fn keep_visible(start: usize, selected: usize, rows: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let rows = rows.max(1).min(len);
    let mut start = start.min(len.saturating_sub(rows));
    if selected < start {
        start = selected;
    } else if selected >= start + rows {
        start = selected + 1 - rows;
    }
    start.min(len.saturating_sub(rows))
}
