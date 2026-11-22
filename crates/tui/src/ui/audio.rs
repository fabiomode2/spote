//! Overlay de audio: EQ de 10 bandas, preamp, normalizador y limitador.
use super::*;

pub(super) fn draw_audio(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
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
        format!("{} EQ", if cfg.enabled { "● on" } else { "○ off" }),
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
    let right_w = inner.x + inner.width - right_x;
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

    let chain = cfg.filter_chain();
    text_at(
        buf,
        Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        if chain.is_some() {
            "↑↓ mover · ←→ ajustar · 0 reset banda · p preset · e on/off · r reset todo · esc cerrar"
        } else {
            "EQ apagado · e activa · esc cerrar"
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
