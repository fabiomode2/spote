//! Audio: pestaña de ecualizador/dinámica y overlay rápido.
use super::*;

/// Pestaña completa de audio: ecualizador gráfico, nivelación automática y
/// dinámica. Comparte acciones con el overlay antiguo, pero aquí hay espacio
/// para ver de un vistazo qué está haciendo el motor.
pub(super) fn draw_audio_tab(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = content_rect(app, layout);
    panel(
        Some(app),
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.88,
        Some("audio · ecualizador y dinámica"),
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Center));
    let inner = inset(rect, 3, 2);
    if inner.height < 10 || inner.width < 34 {
        text_at(
            buf,
            Rect::new(inner.x, inner.y, inner.width, 1),
            "haz la ventana más grande para ver el ecualizador",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }

    let cfg = app.cfg.audio.clone();
    let selected = app.audio_selected;
    let mut y = inner.y;

    // Cabecera.
    text_at(
        buf,
        Rect::new(inner.x, y, inner.width.saturating_sub(30), 1),
        "ECUALIZADOR",
        Style::new()
            .fg(color(mix_oklab(pal.primary, pal.text, 0.25)))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    let mut right = inner.x + inner.width;
    let master = format!(
        "{} proceso",
        if cfg.enabled { "●" } else { "○" }
    );
    right = audio_toggle_chip(
        app,
        buf,
        pal,
        right,
        y,
        &master,
        Action::AudioToggle,
        cfg.enabled,
    );
    let eq = format!("{} EQ", if cfg.eq_enabled { "●" } else { "○" });
    let _ = audio_toggle_chip(
        app,
        buf,
        pal,
        right,
        y,
        &eq,
        Action::AudioEqToggle,
        cfg.eq_enabled,
    );
    y += 2;

    // Presets en una fila.
    let presets = spote_core::audio_fx::EQ_PRESETS;
    let mut px = inner.x;
    for (index, (name, bands)) in presets.iter().enumerate() {
        let label = format!(" {name} ");
        let width = label.chars().count() as u16;
        if px + width > inner.x + inner.width {
            break;
        }
        let chip = Rect::new(px, y, width, 1);
        let active = cfg.bands == *bands;
        fill(
            buf,
            chip,
            if active {
                mix(pal.surface, pal.primary, 0.30)
            } else if app.hover == Some(Action::AudioPreset(index)) {
                pal.surface_alt
            } else {
                pal.surface
            },
        );
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.muted })),
            Alignment::Center,
        );
        app.hit.add(chip, Action::AudioPreset(index));
        px += width + 1;
    }
    y += 2;

    // Tarjeta de nivelación automática entre pistas.
    let card_h = (inner.y + inner.height)
        .saturating_sub(y)
        .min(3)
        .max(2);
    let card = Rect::new(inner.x, y, inner.width, card_h);
    let on = cfg.normalize_tracks;
    fill_soft(
        buf,
        card,
        if on {
            mix(pal.bg_deep, pal.primary, 0.10)
        } else {
            mix(pal.bg_deep, pal.surface, 0.65)
        },
        0.55,
        pal.bg_deep,
    );
    put(
        buf,
        card.x + 1,
        card.y,
        if on { "▎" } else { "▏" },
        color(if on { pal.primary } else { pal.dim }),
        None,
    );
    text_at(
        buf,
        Rect::new(card.x + 2, card.y, card.width.saturating_sub(22), 1),
        "NIVELACIÓN AUTOMÁTICA",
        Style::new()
            .fg(color(if on { pal.text } else { pal.muted }))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    let state = if on { "● ON" } else { "○ OFF" };
    let state_w = state.chars().count() as u16 + 2;
    let state_rect = Rect::new(card.x + card.width - state_w - 1, card.y, state_w, 1);
    fill(
        buf,
        state_rect,
        if on {
            mix(pal.surface, pal.primary, 0.42)
        } else {
            pal.surface
        },
    );
    text_at(
        buf,
        state_rect,
        state,
        Style::new().fg(color(if on { pal.bg_deep } else { pal.dim })),
        Alignment::Center,
    );
    if card_h >= 2 {
        let measured = if app.follow_spotify {
            "modo sistema: la nivelación la decide el reproductor externo".to_string()
        } else if let Some(gain) = app.track_gain_db {
            format!(
                "esta pista {:+.1} dB · objetivo {:.0} dB RMS",
                gain,
                spote_core::audio_map::TARGET_RMS_DB
            )
        } else if app.song_map.is_some() {
            "sin medida fiable para esta pista · nivelador en directo".to_string()
        } else {
            "midiendo la pista… nivelador en directo mientras tanto".to_string()
        };
        text_at(
            buf,
            Rect::new(card.x + 2, card.y + 1, card.width.saturating_sub(3), 1),
            measured,
            Style::new().fg(color(if on { pal.muted } else { pal.dim })),
            Alignment::Left,
        );
    }
    if card_h >= 3 {
        text_at(
            buf,
            Rect::new(card.x + 2, card.y + 2, card.width.saturating_sub(3), 1),
            "todas las pistas al mismo nivel · clic para cambiar",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
    app.hit.add(card, Action::AudioNormalize);
    y += card_h + 1;

    // Zona gráfica + dinámica.
    let footer = inner.y + inner.height.saturating_sub(1);
    let body_h = footer.saturating_sub(y);
    if body_h < 6 {
        return;
    }
    let two_cols = inner.width >= 82;
    let dynamics_w = if two_cols {
        (inner.width * 34 / 100).clamp(28, 42)
    } else {
        0
    };
    let graph_w = if two_cols {
        inner.width.saturating_sub(dynamics_w + 2)
    } else {
        inner.width
    };
    let compact_h = if two_cols { 0 } else { 4 };
    let graph_h = body_h.saturating_sub(if compact_h > 0 {
        compact_h + 1
    } else {
        0
    });
    if graph_h < 6 {
        return;
    }
    let graph_rect = Rect::new(inner.x, y, graph_w, graph_h);
    draw_audio_graph(app, buf, graph_rect, pal, &cfg, selected);
    if two_cols {
        let dyn_rect = Rect::new(
            inner.x + graph_w + 2,
            y,
            inner.width.saturating_sub(graph_w + 2),
            body_h,
        );
        draw_audio_dynamics(app, buf, dyn_rect, pal, &cfg);
    } else {
        let compact = Rect::new(
            inner.x,
            y + graph_h + 1,
            inner.width,
            compact_h.min(footer.saturating_sub(y + graph_h + 1)),
        );
        draw_audio_dynamics_compact(app, buf, compact, pal, &cfg);
    }

    let hints = "↑↓ mover · ←→ ±1 dB · clic en una banda · p preset · n nivelación · e proceso · esc vuelve";
    text_at(
        buf,
        Rect::new(inner.x, footer, inner.width, 1),
        hints,
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

/// Chip de estado alineado a la derecha. Devuelve la nueva coordenada derecha.
fn audio_toggle_chip(
    app: &mut App,
    buf: &mut Buffer,
    pal: &Palette,
    right: u16,
    y: u16,
    label: &str,
    action: Action,
    active: bool,
) -> u16 {
    let width = label.chars().count() as u16 + 2;
    if right < width + 1 {
        return right;
    }
    let rect = Rect::new(right - width, y, width, 1);
    let hover = app.hover == Some(action.clone());
    fill(
        buf,
        rect,
        if active {
            mix(pal.surface, pal.primary, 0.32)
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
        Style::new().fg(color(if active {
            pal.text
        } else {
            pal.muted
        })),
        Alignment::Center,
    );
    app.hit.add(rect, action);
    rect.x.saturating_sub(1)
}

fn draw_audio_graph(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    cfg: &spote_core::audio_fx::AudioCfg,
    selected: usize,
) {
    panel(
        Some(app),
        buf,
        rect,
        pal,
        false,
        0.22,
        Some("gráfico"),
        app.audio.kick_pulse,
    );
    let inner = Rect::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), rect.height.saturating_sub(2));
    if inner.width < 22 || inner.height < 6 {
        return;
    }
    let axis_w = 5u16;
    let plot = Rect::new(
        inner.x + axis_w,
        inner.y,
        inner.width.saturating_sub(axis_w),
        inner.height.saturating_sub(2),
    );
    let zero_y = plot.y + plot.height / 2;
    // Rejilla de dB.
    for step in 0..5 {
        let row = plot.y + plot.height.saturating_sub(1) * step / 4;
        let db = 12.0 - step as f32 * 6.0;
        let label = format!("{db:+.0}");
        text_at(
            buf,
            Rect::new(inner.x, row, axis_w.saturating_sub(1), 1),
            label,
            Style::new().fg(color(if row == zero_y {
                pal.muted
            } else {
                pal.border_dim
            })),
            Alignment::Right,
        );
        for x in plot.x..plot.x + plot.width {
            put(
                buf,
                x,
                row,
                if row == zero_y { "─" } else { "·" },
                color(if row == zero_y {
                    pal.border
                } else {
                    mix(pal.bg_deep, pal.border_dim, 0.55)
                }),
                None,
            );
        }
    }

    let bands = spote_core::audio_fx::BAND_LABELS;
    let col_w = (plot.width / 10).max(3);
    for (index, label) in bands.iter().enumerate() {
        let x0 = plot.x + index as u16 * col_w;
        if x0 + col_w > plot.x + plot.width + 1 {
            break;
        }
        let is_selected = selected == index;
        let column = Rect::new(
            x0,
            plot.y,
            col_w.min(plot.x + plot.width - x0),
            plot.height,
        );
        if is_selected {
            fill(
                buf,
                column,
                mix(pal.bg_deep, pal.primary, 0.12),
            );
        }
        let center = x0 + col_w / 2;
        for x in x0..column.x + column.width {
            put(
                buf,
                x,
                zero_y,
                "─",
                color(mix(pal.bg_deep, pal.border, 0.55)),
                None,
            );
        }

        // Nivel en directo: mancha tenue que sube desde el cero, al lado de la
        // barra de ganancia para que se vea la música aunque el EQ esté plano.
        let level = app.audio.band(index as f32 / 9.0 * 48.0).clamp(0.0, 1.0);
        let live_rows = (level * (plot.height / 2) as f32) as u16;
        let live_x = if column.width >= 4 {
            (center + 1).min(column.x + column.width - 1)
        } else {
            center
        };
        for row in 0..live_rows.min(plot.height / 2) {
            put(
                buf,
                live_x,
                zero_y.saturating_sub(1 + row),
                "░",
                color(mix(pal.bg_deep, pal.secondary, 0.42)),
                None,
            );
        }

        let gain = cfg.bands[index].clamp(-12.0, 12.0);
        let half = (plot.height / 2).max(1) as f32;
        let rows = (gain.abs() / 12.0 * half).round() as u16;
        let gain_color = if is_selected { pal.primary } else { pal.secondary };
        if gain > 0.0 {
            for row in 0..rows.min(plot.height / 2) {
                put(
                    buf,
                    center,
                    zero_y.saturating_sub(1 + row),
                    "█",
                    color(gain_color),
                    None,
                );
            }
            let knob_y = zero_y.saturating_sub(rows);
            put(buf, center, knob_y, "◆", color(mix_oklab(gain_color, pal.text, 0.5)), None);
        } else if gain < 0.0 {
            for row in 0..rows.min(plot.height / 2) {
                put(
                    buf,
                    center,
                    zero_y + 1 + row,
                    "█",
                    color(if is_selected { pal.warn } else { pal.muted }),
                    None,
                );
            }
            let knob_y = (zero_y + rows).min(plot.y + plot.height - 1);
            put(buf, center, knob_y, "◆", color(pal.warn), None);
        } else {
            put(buf, center, zero_y, "◆", color(pal.border), None);
        }

        let value = format!("{gain:+.0}");
        text_at(
            buf,
            Rect::new(
                x0.saturating_sub(1),
                inner.y + inner.height.saturating_sub(2),
                col_w + 2,
                1,
            ),
            value,
            Style::new().fg(color(if is_selected { pal.text } else { pal.dim })),
            Alignment::Center,
        );
        text_at(
            buf,
            Rect::new(
                x0.saturating_sub(1),
                inner.y + inner.height.saturating_sub(1),
                col_w + 2,
                1,
            ),
            *label,
            Style::new().fg(color(if is_selected { pal.secondary } else { pal.dim })),
            Alignment::Center,
        );
        app.hit.add(column, Action::AudioBandSet(index));
    }
}

fn draw_audio_dynamics_compact(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    cfg: &spote_core::audio_fx::AudioCfg,
) {
    if rect.height < 3 || rect.width < 24 {
        return;
    }
    // Fila 1: preamp + limitador.
    let pre = Rect::new(rect.x, rect.y, rect.width.saturating_sub(12), 1);
    text_at(
        buf,
        pre,
        format!("PREAMP {:+.1} dB", cfg.preamp_db),
        Style::new().fg(color(pal.muted)),
        Alignment::Left,
    );
    audio_gauge(
        app,
        buf,
        pal,
        Rect::new(pre.x + 16, pre.y, pre.width.saturating_sub(17), 1),
        (cfg.preamp_db / 12.0 + 1.0) / 2.0,
        Action::AudioPreamp,
    );
    let lim_label = if cfg.limiter {
        "● limitador"
    } else {
        "○ limitador"
    };
    let lim_w = lim_label.chars().count() as u16 + 2;
    let lim = Rect::new(rect.x + rect.width.saturating_sub(lim_w), rect.y, lim_w, 1);
    fill(
        buf,
        lim,
        if cfg.limiter {
            mix(pal.surface, pal.primary, 0.30)
        } else {
            pal.surface
        },
    );
    text_at(
        buf,
        lim,
        lim_label,
        Style::new().fg(color(if cfg.limiter { pal.text } else { pal.dim })),
        Alignment::Center,
    );
    app.hit.add(lim, Action::AudioLimiter);

    // Fila 2: nivelador en directo.
    text_at(
        buf,
        Rect::new(rect.x, rect.y + 1, 12, 1),
        "EN DIRECTO",
        Style::new().fg(color(pal.muted)),
        Alignment::Left,
    );
    let mut nx = rect.x + 12;
    for mode in spote_core::audio_fx::NormalizerMode::ALL {
        let label = format!(" {} ", normalizer_label(mode));
        let w = label.chars().count() as u16;
        if nx + w > rect.x + rect.width {
            break;
        }
        let chip = Rect::new(nx, rect.y + 1, w, 1);
        let active = cfg.normalizer == mode;
        fill(
            buf,
            chip,
            if active {
                mix(pal.surface, pal.primary, 0.28)
            } else {
                pal.surface
            },
        );
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.dim })),
            Alignment::Center,
        );
        app.hit.add(chip, Action::AudioNormalizer);
        nx += w + 1;
    }
    if rect.height >= 3 {
        text_at(
            buf,
            Rect::new(rect.x, rect.y + 2, rect.width, 1),
            format!(
                "intensidad {:.0}% · {} bandas activas · {}",
                cfg.normalizer_strength * 100.0,
                cfg.bands.iter().filter(|gain| **gain != 0.0).count(),
                if cfg.normalize_tracks {
                    "nivelación entre pistas on"
                } else {
                    "nivelación entre pistas off"
                }
            ),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

fn draw_audio_dynamics(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    cfg: &spote_core::audio_fx::AudioCfg,
) {
    if rect.height < 18 || rect.width < 26 {
        return;
    }
    let mut y = rect.y;
    // Preamp.
    let pre = dynamics_card(buf, pal, rect, &mut y, 4, "PREAMP", app.audio_selected == 10);
    text_at(
        buf,
        Rect::new(pre.x + 2, pre.y + 1, pre.width.saturating_sub(3), 1),
        format!("{:+.1} dB", cfg.preamp_db),
        Style::new().fg(color(pal.secondary)),
        Alignment::Left,
    );
    audio_gauge(
        app,
        buf,
        pal,
        Rect::new(pre.x + 2, pre.y + 2, pre.width.saturating_sub(4), 1),
        (cfg.preamp_db / 12.0 + 1.0) / 2.0,
        Action::AudioPreamp,
    );
    text_at(
        buf,
        Rect::new(pre.x + 2, pre.y + 3, pre.width.saturating_sub(3), 1),
        "-12 dB                 +12",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );

    // Nivelador en directo.
    let norm = dynamics_card(
        buf,
        pal,
        rect,
        &mut y,
        6,
        "EN DIRECTO",
        app.audio_selected == 11,
    );
    let mut nx = norm.x + 2;
    for mode in spote_core::audio_fx::NormalizerMode::ALL {
        let label = format!(" {} ", normalizer_label(mode));
        let w = label.chars().count() as u16;
        if nx + w > norm.x + norm.width {
            break;
        }
        let chip = Rect::new(nx, norm.y + 1, w, 1);
        let active = cfg.normalizer == mode;
        fill(
            buf,
            chip,
            if active {
                mix(pal.surface, pal.primary, 0.28)
            } else {
                pal.surface
            },
        );
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.dim })),
            Alignment::Center,
        );
        app.hit.add(chip, Action::AudioNormalizer);
        nx += w + 1;
    }
    text_at(
        buf,
        Rect::new(norm.x + 2, norm.y + 2, norm.width.saturating_sub(3), 1),
        "respaldo mientras no hay medida de la pista",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    text_at(
        buf,
        Rect::new(norm.x + 2, norm.y + 3, norm.width.saturating_sub(3), 1),
        format!("intensidad {:.0}%", cfg.normalizer_strength * 100.0),
        Style::new().fg(color(pal.muted)),
        Alignment::Left,
    );
    audio_gauge(
        app,
        buf,
        pal,
        Rect::new(norm.x + 2, norm.y + 4, norm.width.saturating_sub(4), 1),
        cfg.normalizer_strength,
        Action::AudioStrength,
    );

    // Limitador.
    let lim = dynamics_card(
        buf,
        pal,
        rect,
        &mut y,
        2,
        "LIMITADOR",
        app.audio_selected == 13,
    );
    let lim_label = if cfg.limiter { "● on" } else { "○ off" };
    let w = lim_label.chars().count() as u16 + 2;
    let chip = Rect::new(lim.x + lim.width.saturating_sub(w + 1), lim.y, w, 1);
    fill(
        buf,
        chip,
        if cfg.limiter {
            mix(pal.surface, pal.primary, 0.30)
        } else {
            pal.surface
        },
    );
    text_at(
        buf,
        chip,
        lim_label,
        Style::new().fg(color(if cfg.limiter { pal.text } else { pal.dim })),
        Alignment::Center,
    );
    app.hit.add(chip, Action::AudioLimiter);
    text_at(
        buf,
        Rect::new(lim.x + 2, lim.y + 1, lim.width.saturating_sub(3), 1),
        "recorta picos al final de la cadena",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );

    // Resumen del motor.
    if y + 2 < rect.y + rect.height {
        let summary = format!(
            "EQ {} · {} bandas activas · {}",
            if cfg.eq_enabled { "on" } else { "off" },
            cfg.bands.iter().filter(|gain| **gain != 0.0).count(),
            if cfg.normalize_tracks {
                "nivelación entre pistas"
            } else {
                "sin nivelación"
            }
        );
        text_at(
            buf,
            Rect::new(rect.x, y, rect.width, 1),
            summary,
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

fn normalizer_label(mode: spote_core::audio_fx::NormalizerMode) -> &'static str {
    if mode == spote_core::audio_fx::NormalizerMode::Off {
        "auto"
    } else {
        mode.label()
    }
}

fn dynamics_card(
    buf: &mut Buffer,
    pal: &Palette,
    bounds: Rect,
    y: &mut u16,
    height: u16,
    title: &str,
    selected: bool,
) -> Rect {
    let available = (bounds.y + bounds.height).saturating_sub(*y);
    let r = Rect::new(bounds.x, *y, bounds.width, height.min(available));
    fill_soft(buf, r, mix(pal.bg_deep, pal.surface, 0.72), 0.58, pal.bg_deep);
    put(
        buf,
        r.x,
        r.y,
        if selected { "▎" } else { "▏" },
        color(if selected { pal.primary } else { pal.border_dim }),
        None,
    );
    text_at(
        buf,
        Rect::new(r.x + 2, r.y, r.width.saturating_sub(3), 1),
        title,
        Style::new()
            .fg(color(if selected { pal.text } else { pal.muted }))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    *y += height + 1;
    r
}

fn audio_gauge(
    app: &mut App,
    buf: &mut Buffer,
    pal: &Palette,
    rect: Rect,
    ratio: f32,
    action: Action,
) {
    let ratio = ratio.clamp(0.0, 1.0);
    let fill = (ratio * rect.width as f32).round() as u16;
    for x in 0..rect.width {
        put(
            buf,
            rect.x + x,
            rect.y,
            if x < fill { "━" } else { "─" },
            color(if x < fill {
                pal.primary
            } else {
                pal.border_dim
            }),
            None,
        );
    }
    app.hit.add(rect, action);
}

pub(super) fn draw_audio_overlay(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 88, 24, "audio · ecualizador y dinámica");
    let inner = inset(rect, 2, 2);
    app.hit.add(rect, Action::CloseOverlay);

    let cfg = app.cfg.audio.clone();
    let selected = app.audio_selected;
    let mut y = inner.y;

    // Fila maestra.
    let master = Rect::new(inner.x, y, 16, 1);
    text_at(
        buf,
        master,
        format!(
            "{} {} EQ",
            if selected == 15 { "▸" } else { " " },
            if cfg.enabled { "● on" } else { "○ off" }
        ),
        Style::new()
            .fg(color(if cfg.enabled { pal.primary } else { pal.dim }))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    app.hit.add(master, Action::AudioToggle);

    let presets = spote_core::audio_fx::EQ_PRESETS;
    let mut px = inner.x + 20;
    for (index, (name, _)) in presets.iter().enumerate() {
        let label = format!(" {name} ");
        let w = label.chars().count() as u16;
        let chip = Rect::new(px, y, w, 1);
        let active = cfg.bands == presets[index].1;
        if active {
            fill(buf, chip, mix(pal.surface, pal.primary, 0.24));
        }
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.muted })),
            Alignment::Left,
        );
        app.hit.add(chip, Action::AudioPreset(index));
        px += w + 1;
    }
    y += 2;

    // Diez columnas de banda. Cada una ocupa 6 celdas: barra + separación.
    let graph_y = y;
    let graph_h = 10u16;
    let zero_y = graph_y + graph_h / 2;
    for (index, label) in spote_core::audio_fx::BAND_LABELS.iter().enumerate() {
        let x = inner.x + 1 + index as u16 * 6;
        let gain = cfg.bands[index];
        let is_selected = selected == index;
        if is_selected {
            fill(
                buf,
                Rect::new(x.saturating_sub(1), graph_y, 6, graph_h + 2),
                mix(pal.surface, pal.primary, 0.13),
            );
        }
        for row in 0..graph_h {
            let t = row as f32 / (graph_h - 1) as f32;
            let db = 12.0 - t * 24.0;
            let ch = if (db - gain).abs() < 0.55 {
                "─"
            } else if db.abs() < 1.2 && (gain < 0.0) == (db < 0.0) {
                "·"
            } else {
                " "
            };
            if ch != " " {
                put(buf, x, graph_y + row, ch, color(pal.border_dim), None);
            }
        }
        let fill_ch = "█";
        if gain > 0.0 {
            let rows = (gain / 12.0 * (graph_h / 2) as f32).round() as u16;
            for row in 0..rows.min(graph_h / 2) {
                put(
                    buf,
                    x,
                    zero_y.saturating_sub(1 + row),
                    fill_ch,
                    color(if is_selected {
                        pal.primary
                    } else {
                        pal.secondary
                    }),
                    None,
                );
            }
        } else if gain < 0.0 {
            let rows = (-gain / 12.0 * (graph_h / 2) as f32).round() as u16;
            for row in 0..rows.min(graph_h / 2) {
                put(
                    buf,
                    x,
                    zero_y + 1 + row,
                    fill_ch,
                    color(if is_selected { pal.primary } else { pal.muted }),
                    None,
                );
            }
        }
        let value = format!("{gain:+.0}");
        text_at(
            buf,
            Rect::new(x.saturating_sub(1), graph_y + graph_h, 5, 1),
            value,
            Style::new().fg(color(if is_selected { pal.text } else { pal.dim })),
            Alignment::Center,
        );
        text_at(
            buf,
            Rect::new(x.saturating_sub(1), graph_y + graph_h + 1, 5, 1),
            *label,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        app.hit.add(
            Rect::new(x.saturating_sub(1), graph_y, 6, graph_h + 2),
            Action::AudioBand(index),
        );
    }

    // Columna derecha: preamp, normalizador, intensidad y limitador.
    let right_x = inner.x + 1 + 10 * 6 + 2;
    let right_w = (inner.x + inner.width).saturating_sub(right_x);
    let mut ry = graph_y;
    let preamp_selected = selected == 10;
    text_at(
        buf,
        Rect::new(right_x, ry, right_w, 1),
        format!(
            "{} preamp  {:+.1} dB",
            if preamp_selected { "▸" } else { " " },
            cfg.preamp_db
        ),
        Style::new().fg(color(if preamp_selected { pal.text } else { pal.muted })),
        Alignment::Left,
    );
    ry += 1;
    draw_hbar(
        app,
        buf,
        Rect::new(right_x + 1, ry, right_w.saturating_sub(2).min(20), 1),
        cfg.preamp_db / 12.0,
        pal,
        Action::AudioPreamp,
    );
    ry += 2;

    let norm_selected = selected == 11;
    text_at(
        buf,
        Rect::new(right_x, ry, right_w, 1),
        format!("{} normalizador", if norm_selected { "▸" } else { " " }),
        Style::new().fg(color(if norm_selected { pal.text } else { pal.muted })),
        Alignment::Left,
    );
    ry += 1;
    let mut nx = right_x + 1;
    for mode in spote_core::audio_fx::NormalizerMode::ALL {
        let label = format!(" {} ", mode.label());
        let w = label.chars().count() as u16;
        let chip = Rect::new(nx, ry, w, 1);
        let active = cfg.normalizer == mode;
        if active {
            fill(buf, chip, mix(pal.surface, pal.primary, 0.24));
        }
        text_at(
            buf,
            chip,
            label,
            Style::new().fg(color(if active { pal.text } else { pal.dim })),
            Alignment::Left,
        );
        app.hit.add(chip, Action::AudioNormalizer);
        nx += w + 1;
    }
    ry += 2;

    let strength_selected = selected == 12;
    text_at(
        buf,
        Rect::new(right_x, ry, right_w, 1),
        format!(
            "{} intensidad  {:.0}%",
            if strength_selected { "▸" } else { " " },
            cfg.normalizer_strength * 100.0
        ),
        Style::new().fg(color(if strength_selected {
            pal.text
        } else {
            pal.muted
        })),
        Alignment::Left,
    );
    ry += 1;
    draw_hbar(
        app,
        buf,
        Rect::new(right_x + 1, ry, right_w.saturating_sub(2).min(20), 1),
        cfg.normalizer_strength,
        pal,
        Action::AudioStrength,
    );
    ry += 2;

    let limiter_selected = selected == 13;
    let limiter = Rect::new(right_x, ry, right_w, 1);
    text_at(
        buf,
        limiter,
        format!(
            "{} limitador  {}",
            if limiter_selected { "▸" } else { " " },
            if cfg.limiter { "on" } else { "off" }
        ),
        Style::new().fg(color(if cfg.limiter { pal.primary } else { pal.dim })),
        Alignment::Left,
    );
    app.hit.add(limiter, Action::AudioLimiter);
    ry += 2;

    let preset_selected = selected == 14;
    text_at(
        buf,
        Rect::new(right_x, ry, right_w, 1),
        format!(
            "{} preset  {}",
            if preset_selected { "▸" } else { " " },
            preset_name(&cfg.bands)
        ),
        Style::new().fg(color(if preset_selected { pal.text } else { pal.muted })),
        Alignment::Left,
    );

    let active = cfg.enabled;
    text_at(
        buf,
        Rect::new(inner.x, inner.y + inner.height.saturating_sub(1), inner.width, 1),
        if active {
            "↑↓ mover · ←→ ±1 dB · 0 reset · p preset · e master · d bandas · r reset · esc"
        } else {
            "procesado apagado · e activa · d bandas · esc"
        },
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

fn preset_name(bands: &[f32; 10]) -> &'static str {
    spote_core::audio_fx::EQ_PRESETS
        .iter()
        .find(|(_, values)| values == bands)
        .map(|(name, _)| *name)
        .unwrap_or("custom")
}

fn draw_hbar(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    ratio: f32,
    pal: &Palette,
    action: Action,
) {
    let ratio = ratio.clamp(0.0, 1.0);
    let fill = (ratio * rect.width as f32).round() as u16;
    for x in 0..rect.width {
        let ch = if x < fill { "━" } else { "─" };
        put(
            buf,
            rect.x + x,
            rect.y,
            ch,
            color(if x < fill {
                pal.primary
            } else {
                pal.border_dim
            }),
            None,
        );
    }
    app.hit.add(rect, action);
}
