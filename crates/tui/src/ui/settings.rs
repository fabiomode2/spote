//! Pestaña de ajustes: secciones, tipos de control y ayuda contextual.
use super::*;
use crate::app::SettingRow;

enum Item<'a> {
    Section(&'static str),
    Row(usize, &'a str, &'a str, SettingRow),
}

pub(super) fn draw_settings(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() && layout.art.width > 0 {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(app, buf, layout.art, pal);
    }
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.86,
        Some("ajustes"),
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Center));
    let inner = inset(rect, 2, 2);
    if inner.height < 6 || inner.width < 24 {
        return;
    }

    // Cabecera de la pestaña: deja claro que las filas se cambian, no solo se
    // seleccionan.
    text_at(
        buf,
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
        "CONFIGURACIÓN",
        Style::new()
            .fg(color(mix_oklab(pal.primary, pal.text, 0.22)))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    text_at(
        buf,
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
        if app.settings_search_focus {
            "escribe para filtrar · enter cierra"
        } else {
            "enter para cambiar · / buscar"
        },
        Style::new().fg(color(pal.dim)),
        Alignment::Right,
    );

    // Buscador inline: mismo lenguaje que discos/biblioteca.
    let search = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    fill(
        buf,
        search,
        if app.settings_search_focus {
            pal.surface_alt
        } else {
            pal.surface
        },
    );
    let mut search_text = if app.settings_query.is_empty() {
        "buscar ajustes…  ( / )".to_string()
    } else {
        format!("⌕ {}", app.settings_query)
    };
    if app.settings_search_focus {
        search_text.push('▏');
    }
    truncate_chars(&mut search_text, search.width.saturating_sub(1) as usize);
    text_at(
        buf,
        search,
        search_text,
        Style::new().fg(color(if app.settings_search_focus {
            pal.text
        } else if app.settings_query.is_empty() {
            pal.dim
        } else {
            pal.primary
        })),
        Alignment::Left,
    );
    app.hit.add(search, Action::SettingsSearch);

    let rows = app.settings_rows();
    let visible = app.settings_visible();
    let mut items: Vec<Item> = Vec::new();
    let mut last_section = "";
    for &index in &visible {
        let Some((label, value)) = rows.get(index) else {
            continue;
        };
        let Some(setting) = app.setting_at(index) else {
            continue;
        };
        let section = setting.section();
        if section != last_section {
            items.push(Item::Section(section));
            last_section = section;
        }
        items.push(Item::Row(index, label, value, setting));
    }
    // Con el filtro vacío no hay selección posible: mensaje y salida.
    if items.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x + 1, inner.y + 4, inner.width.saturating_sub(2), 1),
            "sin ajustes para esa búsqueda",
            Style::new().fg(color(pal.muted)),
            Alignment::Center,
        );
        text_at(
            buf,
            Rect::new(inner.x + 1, inner.y + 5, inner.width.saturating_sub(2), 1),
            "esc limpia · / busca otra cosa",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }

    // Altura de cada elemento: las secciones llevan aire arriba; las filas
    // ocupan una línea y dejan una de separación.
    let height = |item: &Item| matches!(item, Item::Section(_)) as u16 * 1 + 2;
    let selected_item = items
        .iter()
        .position(|item| matches!(item, Item::Row(index, _, _, _) if *index == app.settings_selected))
        .unwrap_or(0);
    let footer_h = 4u16;
    let list_top = inner.y + 3;
    let list_bottom = inner.y + inner.height.saturating_sub(footer_h);
    let list_h = list_bottom.saturating_sub(list_top);
    let mut first = 0usize;
    loop {
        let mut used = 0u16;
        let mut selected_visible = false;
        for (position, item) in items.iter().enumerate().skip(first) {
            let h = height(item);
            if used.saturating_add(h) > list_h {
                break;
            }
            used = used.saturating_add(h);
            if position == selected_item {
                selected_visible = true;
            }
        }
        if selected_visible || first >= selected_item {
            break;
        }
        first += 1;
    }

    let mut y = list_top;
    let mut rows_drawn = 0usize;
    for (position, item) in items.iter().enumerate().skip(first) {
        let _ = position;
        let h = height(item);
        if y + h > list_bottom {
            break;
        }
        match item {
            Item::Section(title) => {
                if y + 1 > list_bottom {
                    break;
                }
                let label = format!("◆ {}", title.to_uppercase());
                put(
                    buf,
                    inner.x + 1,
                    y,
                    &label,
                    color(pal.primary),
                    None,
                );
                let used = label.chars().count() as u16 + 2;
                for x in inner.x + used + 1..inner.x + inner.width - 1 {
                    put(buf, x, y, "─", color(pal.border_dim), None);
                }
                y += 2;
                continue;
            }
            Item::Row(index, label, value, setting) => {
                rows_drawn += 1;
                let selected = *index == app.settings_selected;
                let row = Rect::new(inner.x + 1, y, inner.width.saturating_sub(3), 1);
                if selected {
                    fill(buf, row, mix(pal.surface, pal.primary, 0.26));
                    put(buf, row.x, row.y, "▍", color(pal.primary), None);
                } else if rows_drawn % 2 == 0 {
                    fill_soft(buf, row, mix(pal.bg_deep, pal.surface, 0.18), 0.5, pal.bg_deep);
                }
                let is_toggle = setting.toggle();
                let is_adjustable = setting.adjustable();
                let is_on = *value == "sí" || *value == "on";
                let glyph = if is_toggle {
                    if is_on {
                        "◉"
                    } else {
                        "○"
                    }
                } else if is_adjustable {
                    "⇄"
                } else {
                    "↗"
                };
                put(
                    buf,
                    row.x + 2,
                    y,
                    glyph,
                    color(if selected {
                        pal.secondary
                    } else if is_toggle && is_on {
                        pal.primary
                    } else {
                        pal.dim
                    }),
                    None,
                );
                let mut label_text = (*label).to_string();
                truncate_chars(&mut label_text, row.width.saturating_sub(18) as usize);
                put(
                    buf,
                    row.x + 4,
                    y,
                    &label_text,
                    color(if selected { pal.text } else { pal.muted }),
                    None,
                );

                // La fila se registra antes que las flechas: el HitMap se
                // queda con la última acción y, si no, el clic en ◀/▶ caía en
                // SettingsRow en vez de ajustar.
                app.hit.add(row, Action::SettingsRow(*index));
                let value_width = value.chars().count() as u16;
                let arrows = is_adjustable && !is_toggle;
                let reserve = if arrows { 3 } else { 0 };
                let value_x = row.x + row.width.saturating_sub(value_width + reserve + 1);
                if arrows {
                    let left = Rect::new(value_x.saturating_sub(2), y, 2, 1);
                    let right = Rect::new(row.x + row.width - 2, y, 2, 1);
                    let color_arrow = if selected { pal.muted } else { pal.border_dim };
                    put(buf, left.x, y, "◀", color(color_arrow), None);
                    put(buf, right.x, y, "▶", color(color_arrow), None);
                    app.hit.add(left, Action::SettingsAdjust(*index, -1));
                    app.hit.add(right, Action::SettingsAdjust(*index, 1));
                }
                let chip = Rect::new(value_x, y, value_width + 1, 1);
                if is_toggle {
                    fill(
                        buf,
                        chip,
                        if is_on {
                            mix(pal.surface, pal.primary, 0.22)
                        } else {
                            pal.surface
                        },
                    );
                }
                put(
                    buf,
                    value_x,
                    y,
                    value,
                    color(if selected {
                        pal.secondary
                    } else if is_toggle && is_on {
                        pal.text
                    } else {
                        pal.dim
                    }),
                    None,
                );
                y += 2;
            }
        }
    }

    // Barra de scroll: pequeña, a la derecha, solo si no cabe todo.
    if first > 0 || rows_drawn < visible.len() {
        let track = Rect::new(
            inner.x + inner.width.saturating_sub(1),
            list_top,
            1,
            list_bottom.saturating_sub(list_top),
        );
        if track.height == 0 {
            return;
        }
        let total = visible.len().max(1);
        let thumb_h = ((track.height as usize * rows_drawn.max(1)) / total)
            .clamp(1, track.height as usize) as u16;
        let max_first = visible.len().saturating_sub(rows_drawn);
        let thumb_y = if max_first == 0 {
            0
        } else {
            (first as u16 * track.height.saturating_sub(thumb_h)) / max_first as u16
        };
        for row in 0..track.height {
            let active = row >= thumb_y && row < thumb_y + thumb_h;
            put(
                buf,
                track.x,
                track.y + row,
                if active { "▐" } else { "│" },
                color(if active { pal.primary } else { pal.border_dim }),
                None,
            );
        }
    }

    // Pie contextual: qué hace la opción seleccionada y qué teclas existen.
    let footer_y = inner.y + inner.height.saturating_sub(footer_h);
    let selected = app
        .setting_at(app.settings_selected)
        .unwrap_or(SettingRow::Save);
    let footer = Rect::new(inner.x + 1, footer_y, inner.width.saturating_sub(2), 3);
    fill_soft(buf, footer, mix(pal.bg_deep, pal.surface, 0.7), 0.55, pal.bg_deep);
    put(buf, footer.x, footer.y, "▎", color(pal.primary), None);
    put(
        buf,
        footer.x + 2,
        footer.y,
        selected.label(),
        color(pal.text),
        None,
    );
    let hint_x = footer.x + 2 + selected.label().chars().count() as u16 + 2;
    if hint_x < footer.x + footer.width {
        let mut hint = selected.hint().to_string();
        truncate_chars(&mut hint, footer.width.saturating_sub(hint_x - footer.x) as usize);
        put(buf, hint_x, footer.y, &hint, color(pal.muted), None);
    }
    put(
        buf,
        footer.x + 2,
        footer.y + 1,
        "↑↓ mover   ←→ cambiar   enter alternar   / buscar   E diseño   ? ayuda",
        color(pal.dim),
        None,
    );
    if selected == SettingRow::Audio {
        put(
            buf,
            footer.x + 2,
            footer.y + 2,
            "A abre la pestaña de audio con la nivelación y el ecualizador",
            color(pal.secondary),
            None,
        );
    }
}
