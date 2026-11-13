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
        "pega la URL de una playlist de Spotify o Deezer (o su id)",
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
    // Sombra.
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            put_bg(buf, x, y, pal.bg);
        }
    }
    panel(buf, rect, pal, true, 1.0, Some(title), 0.0);
    rect
}

pub(super) fn draw_help(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 78, 27, "ayuda · todas las teclas");
    let inner = Rect::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(4), rect.height.saturating_sub(3));
    let keys = [
        ("space", "play / pausa"),
        ("n / p", "siguiente / anterior"),
        ("← →", "seek ±5 s (shift ±30 s)"),
        ("↑ ↓", "mover selección"),
        (", .", "mover por la cola"),
        ("shift ↑↓", "reordenar cola"),
        ("enter", "reproducir / unirse a sala"),
        ("a", "añadir a la cola"),
        ("x", "quitar de la cola"),
        ("f", "favorita (corazón)"),
        ("c", "portada a todo el centro"),
        ("C", "letra a todo el centro"),
        ("/", "buscar (en buscar ya escribes)"),
        ("tab", "cambiar de vista"),
        ("t", "seguir spotify (mpris)"),
        ("v", "siguiente visualización"),
        ("V", "visualización a pantalla completa"),
        ("e", "editor de capas"),
        ("A", "siguiente modo de arte"),
        ("r", "rotación automática del arte"),
        ("z", "lista compacta / tarjetas"),
        ("b", "visualizador de fondo"),
        ("i", "zen: ocultar portada y letra"),
        ("N", "crear lista (nombre, descripción, portada)"),
        ("I", "importar lista de Spotify o Deezer"),
        ("l / d", "añadir a lista / borrar la lista abierta"),
        ("[ / ]", "ciclar filtros y listas"),
        ("m", "silenciar"),
        ("+ / -", "volumen"),
        ("y", "copiar letra"),
        ("F", "portada a pantalla completa"),
        ("L", "salir de la sala"),
        ("q", "salir"),
    ];
    let half = (keys.len() + 1) / 2;
    for (i, (key, label)) in keys.iter().enumerate() {
        let column = i / half;
        let row = i % half;
        let x = inner.x + column as u16 * (inner.width / 2);
        let y = inner.y + row as u16;
        if y >= inner.y + inner.height {
            continue;
        }
        let width = key.chars().count() as u16 + 2;
        let cap = Rect::new(x, y, width, 1);
        keycap(buf, cap, pal, key);
        put(buf, x + width + 1, y, label, color(pal.muted), None);
    }
    app.hit.add(rect, Action::CloseOverlay);
    let hint = "esc / ?  cerrar";
    text_at(
        buf,
        Rect::new(inner.x, inner.y + inner.height.saturating_sub(1), inner.width, 1),
        hint.to_string(),
        Style::new().fg(color(pal.dim)),
        Alignment::Right,
    );
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
