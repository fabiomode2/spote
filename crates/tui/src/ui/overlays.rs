//! Ayuda, efectos y formularios modales.
use super::*;

pub(super) fn draw_overlay(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let overlay = app.overlay.clone();
    match overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(app, buf, layout, pal),
        Overlay::Effects => draw_effects(app, buf, layout, pal),
        Overlay::PlaylistForm => draw_playlist_form(app, buf, layout, pal),
        Overlay::ImportPlaylist => draw_import_playlist(app, buf, layout, pal),
        Overlay::ConfirmDeletePlaylist { name, .. } => {
            draw_confirm_delete(app, buf, layout, pal, &name)
        }
        Overlay::AddToPlaylist { key, selected } => {
            draw_add_to_playlist(app, buf, layout, pal, &key, selected)
        }
        Overlay::Layout => draw_layout(app, buf, layout, pal),
        Overlay::Context => draw_context(app, buf, layout, pal),
        Overlay::Artist => draw_artist(app, buf, layout, pal),
        Overlay::Album => draw_album_page(app, buf, layout, pal),
    }
}

pub(super) fn draw_playlist_form(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 66, 14, "nueva lista");
    let inner = inset(rect, 3, 2);
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        "nombre, descripción y portada. vacío = portada automática",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    let mut y = inner.y + 2;
    for (index, (label, value)) in app.playlist_form.fields().iter().enumerate() {
        let active = app.playlist_form.field == index;
        let field = Rect::new(inner.x, y, inner.width, 1);
        if active {
            fill(buf, field, mix(pal.surface, pal.primary, 0.22));
            put(buf, field.x, y, "▍", color(pal.primary), None);
        }
        put(
            buf,
            field.x + 2,
            y,
            &format!("{label}:"),
            color(if active { pal.text } else { pal.dim }),
            None,
        );
        let text_x = field.x + 2 + label.chars().count() as u16 + 2;
        let shown = if value.is_empty() && !active {
            "—".to_string()
        } else {
            value.to_string()
        };
        put(
            buf,
            text_x,
            y,
            &shown,
            color(if active { pal.text } else { pal.muted }),
            None,
        );
        if active {
            let cursor = text_x + value.chars().count() as u16;
            if cursor < field.x + field.width {
                put(buf, cursor, y, "▌", color(pal.primary), None);
            }
        }
        y += 2;
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "tab cambia de campo · enter crea · esc cancela",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

pub(super) fn draw_import_playlist(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 72, 10, "importar lista");
    let inner = inset(rect, 3, 2);
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        "pega la URL de una playlist o álbum de Spotify (o su id)",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    let field = Rect::new(inner.x, inner.y + 2, inner.width, 1);
    fill(buf, field, pal.bg_deep);
    put(buf, field.x + 1, field.y, "↳", color(pal.primary), None);
    let shown = if app.import_input.is_empty() {
        "https://open.spotify.com/playlist/…".to_string()
    } else {
        app.import_input.clone()
    };
    put(
        buf,
        field.x + 3,
        field.y,
        &shown,
        color(if app.import_input.is_empty() { pal.dim } else { pal.text }),
        None,
    );
    let cursor = field.x + 3 + app.import_input.chars().count() as u16;
    if cursor < field.x + field.width {
        put(buf, cursor, field.y, "▌", color(pal.primary), None);
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "enter importa · esc cancela",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

pub(super) fn draw_confirm_delete(
    _app: &mut App,
    buf: &mut Buffer,
    layout: &Layout,
    pal: &Palette,
    name: &str,
) {
    let rect = overlay_frame(layout, pal, buf, 56, 8, "borrar lista");
    let inner = inset(rect, 3, 2);
    text_at(
        buf,
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
        format!("¿borrar '{name}'?"),
        Style::new().fg(color(pal.text)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    text_at(
        buf,
        Rect::new(inner.x, inner.y + 3, inner.width, 1),
        "enter sí · esc no (las pistas se quedan en la biblioteca)",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

pub(super) fn draw_add_to_playlist(
    app: &mut App,
    buf: &mut Buffer,
    layout: &Layout,
    pal: &Palette,
    key: &str,
    selected: usize,
) {
    let lists = app.lib.playlists().unwrap_or_default();
    let height = (lists.len() as u16 + 6).clamp(8, 20);
    let rect = overlay_frame(layout, pal, buf, 58, height, "añadir a lista");
    let inner = inset(rect, 3, 2);
    let title = app.title_for_key(key);
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        title,
        Style::new().fg(color(pal.muted)),
        Alignment::Left,
    );
    let mut y = inner.y + 2;
    for (index, list) in lists.iter().enumerate() {
        let row = Rect::new(inner.x, y, inner.width, 1);
        let is_selected = index == selected;
        if is_selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        put(
            buf,
            row.x + 2,
            y,
            &list.name,
            color(if is_selected { pal.text } else { pal.muted }),
            None,
        );
        let count = format!("{}", list.count);
        text_at(
            buf,
            Rect::new(row.x + row.width - count.len() as u16, y, count.len() as u16, 1),
            count,
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
        y += 1;
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "↑↓ elegir · enter añade · esc cancela",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

pub(super) fn overlay_frame(layout: &Layout, pal: &Palette, buf: &mut Buffer, width: u16, height: u16, title: &str) -> Rect {
    let width = width.min(layout.full.width.saturating_sub(4));
    let height = height.min(layout.full.height.saturating_sub(2));
    let rect = Rect::new(
        layout.full.x + layout.full.width.saturating_sub(width) / 2,
        layout.full.y + layout.full.height.saturating_sub(height) / 2,
        width,
        height,
    );
    // Sombra exterior de una celda y relleno totalmente opaco. El panel se
    // pinta luego con alpha 0: solo aporta el borde, así el menú contextual no
    // se mezcla con lo que hay detrás (el bug de la captura).
    let shadow = Rect::new(
        rect.x.saturating_sub(1),
        rect.y.saturating_sub(1),
        rect.width + 2,
        rect.height + 2,
    );
    fill(buf, shadow, pal.bg_deep);
    fill(buf, rect, mix(pal.bg, pal.surface, 1.0));
    panel(None, buf, rect, pal, true, 0.0, Some(title), 0.0);
    // La capa interior vuelve a rellenarse por si el borde dejó huecos.
    let inner = Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    );
    fill(buf, inner, mix(pal.bg, pal.surface, 1.0));
    rect
}

pub(super) fn draw_help(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let width = layout.full.width.saturating_sub(8).min(104);
    let height = layout.full.height.saturating_sub(4).min(44);
    let rect = overlay_frame(layout, pal, buf, width, height, "ayuda · todas las teclas");
    let inner = inset(rect, 3, 2);
    // Aplanamos secciones en filas; dos columnas para que quepa todo.
    let mut rows: Vec<(Option<&'static str>, &'static str, &'static str)> = Vec::new();
    for (section, keys) in crate::app::help_sections() {
        rows.push((Some(section), "", ""));
        for (key, label) in keys {
            rows.push((None, key, label));
        }
    }
    let per_col = rows.len().div_ceil(2);
    let col_w = inner.width / 2;
    for (index, (section, key, label)) in rows.into_iter().enumerate() {
        let column = index / per_col;
        let row = index % per_col;
        let x = inner.x + column as u16 * col_w;
        let y = inner.y + row as u16;
        if y >= inner.y + inner.height.saturating_sub(1) {
            continue;
        }
        if let Some(section) = section {
            put(
                buf,
                x,
                y,
                section,
                color(pal.primary),
                None,
            );
            continue;
        }
        let key_w = key.chars().count() as u16 + 2;
        let cap = Rect::new(x, y, key_w, 1);
        keycap(buf, cap, pal, key);
        put(
            buf,
            x + key_w + 1,
            y,
            label,
            color(pal.muted),
            None,
        );
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "esc / ?  cerrar · E diseño · e capas",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    app.hit.add(rect, Action::CloseOverlay);
}

pub(super) fn draw_effects(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 74, 22, "visualizaciones · capas y presets");
    let inner = Rect::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(4), rect.height.saturating_sub(3));

    // Presets.
    let mut y = inner.y;
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        "presets  ·  v cicla  ·  V pantalla completa",
        Style::new().fg(color(pal.primary)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    y += 1;
    let per_row = 3usize;
    let presets = crate::viz::PRESETS;
    for (i, preset) in presets.iter().enumerate() {
        let column = i % per_row;
        let row = i / per_row;
        let x = inner.x + column as u16 * (inner.width / per_row as u16);
        let py = y + row as u16;
        let active = i == app.viz.preset_index;
        let label = format!("{} {}", if active { "▸" } else { "·" }, preset.name);
        let width = label.chars().count() as u16 + 2;
        let chip_rect = Rect::new(x, py, width.min(inner.width), 1);
        if active {
            fill(buf, chip_rect, mix(pal.surface, pal.primary, 0.25));
        }
        text_at(
            buf,
            chip_rect,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.muted })),
            Alignment::Left,
        );
        app.hit.add(chip_rect, Action::VizNext);
    }
    y += ((presets.len() + per_row - 1) / per_row + 1) as u16;

    // Capas.
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        format!("capas de «{}»  ·  1-9 alterna", app.viz.preset_name()),
        Style::new().fg(color(pal.primary)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    y += 1;
    for (index, layer) in app.viz.layers.iter().enumerate() {
        if y >= inner.y + inner.height {
            break;
        }
        let on = layer.enabled;
        let label = format!(
            "{} {}  ·  {}  ·  {:.0}%  ·  {}",
            if on { "●" } else { "○" },
            layer.effect.name(),
            blend_label(layer.blend),
            layer.opacity * 100.0,
            mask_label(layer.mask)
        );
        let row = Rect::new(inner.x, y, inner.width, 1);
        if app.hover == Some(Action::VizLayer(index)) {
            fill(buf, row, pal.surface_alt);
        }
        text_at(
            buf,
            row,
            format!("{} {label}", index + 1),
            Style::new().fg(color(if on { pal.text } else { pal.dim })),
            Alignment::Left,
        );
        app.hit.add(row, Action::VizLayer(index));
        y += 1;
    }

    // Efectos disponibles.
    y += 1;
    if y < inner.y + inner.height {
        let keys: Vec<&str> = crate::viz::EffectKind::ALL.iter().map(|kind| kind.key()).collect();
        let mut available = format!("disponibles: {}", keys.join(" · "));
        available.truncate(inner.width.saturating_sub(2) as usize);
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            available,
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
    app.hit.add(rect, Action::CloseOverlay);
}

pub(super) fn blend_label(blend: Blend) -> &'static str {
    match blend {
        Blend::Normal => "normal",
        Blend::Add => "add",
        Blend::Screen => "screen",
        Blend::Multiply => "multiply",
        Blend::Overlay => "overlay",
    }
}

pub(super) fn mask_label(mask: crate::viz::Mask) -> &'static str {
    match mask {
        crate::viz::Mask::Full => "full",
        crate::viz::Mask::Radial(_) => "radial",
        crate::viz::Mask::Bottom => "gradiente",
        crate::viz::Mask::Band(_, _) => "banda",
    }
}

// ---------------------------------------------------------------------------
// Editor de diseño
// ---------------------------------------------------------------------------

pub(super) fn draw_layout(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rows = app.layout_rows();
    let height = (rows.len() as u16 + 6).min(layout.full.height.saturating_sub(2));
    let rect = overlay_frame(layout, pal, buf, 64, height, "diseño · E para cerrar");
    let inner = inset(rect, 3, 2);
    let visible = inner.height.saturating_sub(2) as usize;
    let first = app
        .layout_selected
        .saturating_sub(visible / 2)
        .min(rows.len().saturating_sub(visible));
    let mut y = inner.y;
    for (index, (label, value)) in rows.iter().enumerate().skip(first).take(visible) {
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == app.layout_selected;
        if selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.24));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        put(
            buf,
            row.x + 2,
            y,
            label,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        let value_w = value.chars().count() as u16;
        put(
            buf,
            row.x + row.width.saturating_sub(value_w + 2),
            y,
            value,
            color(if selected { pal.secondary } else { pal.dim }),
            None,
        );
        y += 1;
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "↑↓ elegir · ←→ tamaño · enter on/off · r restaurar · se guarda solo",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    app.hit.add(rect, Action::CloseOverlay);
}

// ---------------------------------------------------------------------------
// Menú contextual
// ---------------------------------------------------------------------------

pub(super) fn draw_context(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let Some(menu) = app.context_menu().cloned() else {
        return;
    };
    let actions = menu.actions();
    let mut headline = format!("{}  ·  {}", menu.title, menu.artist);
    truncate_chars(&mut headline, 72);
    let width = (headline.chars().count() as u16 + 10).max(58);
    let rect = overlay_frame(
        layout,
        pal,
        buf,
        width,
        actions.len() as u16 + 6,
        "acciones · clic derecho",
    );
    let inner = inset(rect, 3, 2);
    let mut title = format!("{}  ·  {}", menu.title, menu.artist);
    truncate_chars(&mut title, inner.width as usize);
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        title,
        Style::new().fg(color(pal.text)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    app.hit.add(rect, Action::CloseOverlay);
    let mut y = inner.y + 2;
    for (index, action) in actions.iter().enumerate() {
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == menu.selected;
        if selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.24));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        let danger = matches!(
            action,
            crate::app::ContextAction::RemoveLibrary
                | crate::app::ContextAction::DeleteFile
                | crate::app::ContextAction::RemoveQueue
        );
        put(
            buf,
            row.x + 2,
            y,
            context_label(*action),
            color(if danger {
                pal.error
            } else if selected {
                pal.text
            } else {
                pal.muted
            }),
            None,
        );
        app.hit.add(row, Action::ContextSelect(index));
        y += 1;
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "↑↓ elegir · enter aplicar · esc cerrar",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

fn context_label(action: crate::app::ContextAction) -> &'static str {
    use crate::app::ContextAction::*;
    match action {
        Play => "reproducir ahora",
        Queue => "añadir a la cola",
        Download => "descargar audio",
        Favorite => "favorita / quitar",
        Artist => "ver artista",
        Cover => "traer portada de Spotify",
        RemoveLibrary => "quitar de la biblioteca",
        DeleteFile => "borrar audio del disco",
        RemoveQueue => "quitar de la cola",
        Status => "ver estado",
        NewPlaylist => "crear lista nueva",
        ImportPlaylist => "importar de Spotify",
        RefreshLibrary => "recargar biblioteca",
        Layout => "editar diseño (E)",
        ClearQueue => "vaciar la cola",
        Open => "abrir",
    }
}

// ---------------------------------------------------------------------------
// Perfil de artista
// ---------------------------------------------------------------------------

pub(super) fn draw_artist(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let width = layout.full.width.saturating_sub(8).min(104);
    let height = layout.full.height.saturating_sub(4).min(34);
    let rect = overlay_frame(layout, pal, buf, width, height, "artista");
    let inner = inset(rect, 3, 2);
    let Some(profile) = app.artist.clone() else {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
            if app.artist_loading {
                "cargando perfil de Spotify…"
            } else {
                "sin perfil"
            },
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        if !app.artist_status.is_empty() {
            let mut status = app.artist_status.clone();
            truncate_chars(&mut status, inner.width as usize);
            text_at(
                buf,
                Rect::new(inner.x, inner.y + 3, inner.width, 1),
                status,
                Style::new().fg(color(pal.warn)),
                Alignment::Left,
            );
        }
        return;
    };

    let image_w = if inner.width >= 70 { 24 } else { 0 };
    let image_h = if image_w > 0 { 12 } else { 0 };
    if image_w > 0 {
        if let Some(art) = &app.artist_art {
            draw_cached_art(buf, Rect::new(inner.x, inner.y, image_w, image_h), art, pal, true);
        } else {
            fill(
                buf,
                Rect::new(inner.x, inner.y, image_w, image_h),
                mix(pal.surface, pal.primary, 0.25),
            );
            text_at(
                buf,
                Rect::new(inner.x, inner.y + image_h / 2, image_w, 1),
                "♪",
                Style::new().fg(color(pal.primary)),
                Alignment::Center,
            );
        }
    }
    let text_x = inner.x + image_w + u16::from(image_w > 0) * 2;
    let text_w = (inner.x + inner.width).saturating_sub(text_x);
    let mut name = profile.name.clone();
    truncate_chars(&mut name, text_w as usize);
    put(
        buf,
        text_x,
        inner.y,
        &name,
        color(pal.text),
        None,
    );
    let meta = format!(
        "{} · {} fans{}",
        profile.provider,
        profile.followers,
        if profile.popularity > 0 {
            format!(" · popularidad {}/100", profile.popularity)
        } else {
            String::new()
        }
    );
    let mut meta = meta;
    truncate_chars(&mut meta, text_w as usize);
    put(buf, text_x, inner.y + 1, &meta, color(pal.muted), None);
    if !profile.genres.is_empty() {
        let mut genres = profile.genres.join("  ·  ");
        truncate_chars(&mut genres, text_w as usize);
        put(buf, text_x, inner.y + 2, &genres, color(pal.secondary), None);
    }
    if profile.popularity > 0 {
        let bar_w = text_w.min(30);
        let filled = (bar_w as f32 * profile.popularity as f32 / 100.0) as u16;
        for i in 0..bar_w {
            put(
                buf,
                text_x + i,
                inner.y + 3,
                if i < filled { "━" } else { "─" },
                color(if i < filled { pal.primary } else { pal.border_dim }),
                None,
            );
        }
    }

    let mut y = inner.y + image_h.max(5) + 1;
    // Discografía: cada álbum abre su ficha como en Spotify.
    if !profile.albums.is_empty() && y + 2 < inner.y + inner.height {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "ÁLBUMES",
            Style::new()
                .fg(color(pal.primary))
                .add_modifier(Modifier::BOLD),
            Alignment::Left,
        );
        y += 1;
        for (index, album) in profile.albums.iter().take(5).enumerate() {
            if y + 1 >= inner.y + inner.height {
                break;
            }
            let row = Rect::new(inner.x, y, inner.width, 1);
            let hover = app.hover == Some(Action::ArtistAlbum(index));
            if hover {
                fill(buf, row, pal.surface_alt);
            }
            let mut label = format!(
                "▸ {}  ·  {}",
                album.title,
                album
                    .release_year
                    .map(|year| year.to_string())
                    .unwrap_or_else(|| "—".into())
            );
            truncate_chars(&mut label, row.width.saturating_sub(4) as usize);
            put(buf, row.x + 2, y, &label, color(pal.muted), None);
            app.hit.add(row, Action::ArtistAlbum(index));
            y += 1;
        }
        y += 1;
    }
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        "TOP · enter descarga y reproduce · a encola",
        Style::new()
            .fg(color(pal.primary))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    y += 1;
    let rows = (inner.y + inner.height).saturating_sub(y + 1) as usize;
    let start = app.artist_selected.saturating_sub(rows / 2);
    let start = start.min(profile.top_tracks.len().saturating_sub(rows.min(profile.top_tracks.len())));
    for (offset, track) in profile.top_tracks.iter().skip(start).take(rows).enumerate() {
        let index = start + offset;
        let row = Rect::new(inner.x, y + offset as u16, inner.width, 1);
        let selected = index == app.artist_selected;
        if selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            put(buf, row.x, y + offset as u16, "▍", color(pal.primary), None);
        }
        let duration = format_ms(track.duration_ms);
        let d_w = duration.chars().count() as u16;
        let mut label = format!("{:>2}  {}  ·  {}", index + 1, track.title, track.artists);
        truncate_chars(&mut label, row.width.saturating_sub(d_w + 4) as usize);
        put(
            buf,
            row.x + 2,
            y + offset as u16,
            &label,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        put(
            buf,
            row.x + row.width - d_w,
            y + offset as u16,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row, Action::ArtistTrack(index));
    }
    if !app.artist_status.is_empty() {
        let mut status = app.artist_status.clone();
        truncate_chars(&mut status, inner.width as usize);
        put(
            buf,
            inner.x,
            rect.y + rect.height - 2,
            &status,
            color(pal.dim),
            None,
        );
    }
}

/// Álbum abierto como overlay: portada, ficha y tracklist. Sustituye a la
/// pestaña independiente de álbumes, que el usuario no quería.
pub(super) fn draw_album_page(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let Some(page) = app.album_page.clone() else { return };
    let width = layout.full.width.saturating_sub(8).min(106);
    let height = layout.full.height.saturating_sub(4).min(36);
    let rect = overlay_frame(layout, pal, buf, width, height, "álbum");
    let inner = inset(rect, 3, 2);
    if app.album_page_loading {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
            app.album_page_status.clone(),
            Style::new().fg(color(pal.warn)),
            Alignment::Left,
        );
        app.hit.add(rect, Action::AlbumPageClose);
        return;
    }
    let cover_w = 18u16.min(inner.width / 3);
    let cover_h = 9u16.min(inner.height.saturating_sub(2));
    let cover = Rect::new(inner.x, inner.y, cover_w, cover_h);
    app.request_album_page_art(&page);
    let thumb_track = spote_protocol::TrackRef {
        key: format!("albumpage:{}:{}", page.provider, page.id),
        title: page.title.clone(),
        artist: page.artist.clone(),
        album: Some(page.title.clone()),
        duration_ms: 0,
        art_url: page.cover_url.clone(),
        source_hint: None,
    };
    draw_thumb(app, buf, cover, &thumb_track, pal);
    let text_x = cover.x + cover.width + 2;
    let text_w = (inner.x + inner.width).saturating_sub(text_x);
    let mut title = page.title.clone();
    truncate_chars(&mut title, text_w as usize);
    put(
        buf,
        text_x,
        inner.y,
        &title,
        color(pal.text),
        None,
    );
    let mut artist = page.artist.clone();
    truncate_chars(&mut artist, text_w as usize);
    put(buf, text_x, inner.y + 1, &artist, color(pal.secondary), None);
    if !artist.is_empty() {
        app.hit.add(
            Rect::new(text_x, inner.y + 1, artist.chars().count() as u16, 1),
            Action::OpenArtist(page.artist.clone()),
        );
    }
    let meta = format!(
        "{} · {} pistas{}",
        page.release_date.clone().unwrap_or_else(|| "fecha desconocida".into()),
        page.tracks.len(),
        if page.provider == "local" { " · en biblioteca" } else { "" }
    );
    put(buf, text_x, inner.y + 2, &meta, color(pal.dim), None);
    let mut x = text_x;
    for (label, action) in [
        ("d bajar", Action::AlbumPageDownload),
        ("s guardar lista", Action::AlbumPageSave),
        ("esc cerrar", Action::AlbumPageClose),
    ] {
        let w = label.chars().count() as u16 + 2;
        if x + w > inner.x + inner.width {
            break;
        }
        let chip = Rect::new(x, inner.y + 4, w, 1);
        let hover = app.hover == Some(action.clone());
        fill(buf, chip, if hover { pal.surface_alt } else { pal.surface });
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if matches!(action, Action::AlbumPageDownload) {
                pal.warn
            } else {
                pal.muted
            })),
            Alignment::Center,
        );
        app.hit.add(chip, action);
        x += w + 1;
    }
    let list_y = inner.y + cover_h + 2;
    let rows = (inner.y + inner.height).saturating_sub(list_y + 1) as usize;
    if page.tracks.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, list_y, inner.width, 1),
            "este álbum no tiene pistas",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        return;
    }
    let start = page
        .selected
        .saturating_sub(rows / 2)
        .min(page.tracks.len().saturating_sub(rows.min(page.tracks.len())));
    for offset in 0..rows {
        let index = start + offset;
        let Some(track) = page.tracks.get(index) else { break };
        let y = list_y + offset as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == page.selected;
        if selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        let duration = format_ms(track.duration_ms);
        let d_w = duration.chars().count() as u16;
        let ready = app
            .lib
            .local_path(&track.key)
            .is_some_and(|path| path.exists());
        put(
            buf,
            row.x + 2,
            y,
            if ready { "·" } else { "↓" },
            color(if ready { pal.dim } else { pal.warn }),
            None,
        );
        put(
            buf,
            row.x + 4,
            y,
            &format!("{:>2}", index + 1),
            color(pal.dim),
            None,
        );
        let mut title = track.title.clone();
        truncate_chars(&mut title, row.width.saturating_sub(10 + d_w) as usize);
        put(
            buf,
            row.x + 7,
            y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        put(
            buf,
            row.x + row.width - d_w,
            y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row, Action::AlbumTrackPlay(index));
    }
    text_at(
        buf,
        Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
        "enter reproduce · a encola · d baja · s guarda lista · esc cierra",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}
