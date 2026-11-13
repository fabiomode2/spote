//! Pestaña de ajustes.
use super::*;

pub(super) fn draw_settings(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    if app.now.track.is_some() && layout.art.width > 0 {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.85,
        Some("ajustes"),
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Center));
    let inner = inset(rect, 3, 2);
    let rows = app.settings_rows();
    // Con 13 filas y paneles centrados ya no siempre caben: se desplaza la
    // ventana para que la selección quede siempre visible.
    let visible = (inner.height / 2).max(3) as usize;
    let first = app
        .settings_selected
        .saturating_sub(visible.saturating_sub(1)/2)
        .min(rows.len().saturating_sub(visible));
    let shown = rows.len().saturating_sub(first).min(visible) as u16;
    let offset = if app.vcenter {
        inner.height.saturating_sub(shown * 2) / 2
    } else {
        0
    };
    let mut y = inner.y + offset;
    for (index, (label, value)) in rows.iter().enumerate().skip(first).take(visible) {
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = index == app.settings_selected;
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
        let value_width = value.chars().count() as u16;
        let mut value_x = row.x + row.width.saturating_sub(value_width + 2);
        let adjustable = app
            .setting_at(index)
            .is_some_and(|row| row.adjustable());
        if adjustable {
            // ◀ valor ▶ para los ajustes que se tocan con izquierda/derecha.
            let left = Rect::new(value_x.saturating_sub(3), y, 2, 1);
            let right = Rect::new(row.x + row.width - 2, y, 2, 1);
            put(buf, left.x, y, "◀", color(pal.dim), None);
            put(buf, right.x, y, "▶", color(pal.dim), None);
            app.hit.add(left, Action::SettingsAdjust(index, -1));
            app.hit.add(right, Action::SettingsAdjust(index, 1));
            value_x = value_x.min(right.x.saturating_sub(value_width + 1));
        }
        put(
            buf,
            value_x,
            y,
            value,
            color(if selected { pal.secondary } else { pal.dim }),
            None,
        );
        app.hit.add(row, Action::SettingsRow(index));
        y += 2;
    }
    let footer = "↑↓ mover · ←→ cambiar · enter alterna";
    if y < rect.y + rect.height.saturating_sub(2) {
        text_at(
            buf,
            Rect::new(inner.x, rect.y + rect.height - 2, inner.width, 1),
            footer,
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}
