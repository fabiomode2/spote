//! Barra superior, navegación, transporte y leyenda.
use super::*;

pub(super) fn draw_top(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.top;
    fill_soft(buf, rect, pal.bg_deep, 0.84, pal.bg_deep);
    // Logo.
    let logo_style = Style::new()
        .fg(color(mix_oklab(pal.primary, pal.text, 0.25)))
        .add_modifier(Modifier::BOLD);
    let kick = app.audio.kick_pulse;
    put(
        buf,
        rect.x + 1,
        rect.y,
        "◈",
        color(mix_oklab(pal.primary, pal.text, kick * 0.45)),
        None,
    );
    let mut logo = app.name.clone();
    truncate_chars(&mut logo, rect.width.saturating_sub(12) as usize);
    put(buf, rect.x + 3, rect.y, &logo, color(pal.text), None);
    let _ = logo_style;

    // Tabs.
    let mut x = rect.x + 11;
    for tab in Tab::ALL {
        let label = format!("{} {}", tab.icon(), tab.label());
        let width = label.chars().count() as u16 + 2;
        if x + width > rect.x + rect.width.saturating_sub(24) {
            break;
        }
        let tab_rect = Rect::new(x, rect.y, width, 1);
        let active = app.tab == tab;
        let hover = app.hover == Some(Action::Tab(tab));
        chip(buf, tab_rect, pal, active, hover);
        if active {
            let glow = mix_oklab(pal.primary, pal.secondary, kick * 0.35);
            fill(buf, tab_rect, glow);
            let _ = glow;
        }
        text_at(
            buf,
            tab_rect,
            label,
            Style::new()
                .fg(color(if active { pal.bg_deep } else { pal.muted }))
                .bg(color(if active {
                    pal.primary
                } else if hover {
                    pal.surface_alt
                } else {
                    pal.surface
                }))
                .add_modifier(if active { Modifier::BOLD } else { Modifier::empty() }),
            Alignment::Center,
        );
        app.hit.add(tab_rect, Action::Tab(tab));
        x += width + 1;
    }

    // Chip de Spotify: la puerta de entrada al modo acompañante.
    let spotify_label = if app.follow_spotify {
        "● spotify"
    } else {
        "○ spotify"
    };
    let spotify_width = spotify_label.chars().count() as u16 + 2;
    if x + spotify_width + 1 < rect.x + rect.width.saturating_sub(24) {
        let chip_rect = Rect::new(x + 1, rect.y, spotify_width, 1);
        let hover = app.hover == Some(Action::ToggleFollow);
        fill(
            buf,
            chip_rect,
            if app.follow_spotify {
                mix(pal.surface, pal.secondary, 0.30 + app.audio.kick_pulse * 0.20)
            } else if hover {
                pal.surface_alt
            } else {
                pal.surface
            },
        );
        text_at(
            buf,
            chip_rect,
            spotify_label,
            Style::new().fg(color(if app.follow_spotify {
                pal.secondary
            } else {
                pal.muted
            })),
            Alignment::Center,
        );
        app.hit.add(chip_rect, Action::ToggleFollow);
    }

    // Estado de la derecha.
    let mut right = rect.x + rect.width.saturating_sub(1);
    let fps = if app.cfg.ui.show_fps {
        Some(format!("{:.0} fps · {:.1} ms", app.fps.value(), app.frame_ms))
    } else {
        None
    };
    let clock = clock_now();
    let mut items: Vec<(String, Rgb)> = Vec::new();
    if let Some(fps) = fps {
        items.push((fps, pal.dim));
    }
    items.push((clock, pal.muted));
    if let Some(room) = &app.room {
        let label = app.room_label(&room.id);
        items.push((format!("◆ {label}"), pal.primary));
    } else if app.follow_spotify {
        items.push(("● spotify".into(), pal.secondary));
    } else if app.social.is_some() {
        items.push((
            if app.social_connected {
                "● servidor".to_string()
            } else {
                "○ servidor".to_string()
            },
            if app.social_connected {
                pal.secondary
            } else {
                pal.dim
            },
        ));
    } else {
        items.push(("● local".into(), pal.dim));
    }
    for (text, fg) in items {
        let width = text.chars().count() as u16;
        if right < rect.x + width + 2 {
            break;
        }
        right -= width;
        text_at(
            buf,
            Rect::new(right, rect.y, width, 1),
            text,
            Style::new().fg(color(fg)),
            Alignment::Right,
        );
        right = right.saturating_sub(2);
    }
}

pub(super) fn clock_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now % 86_400;
    // Hora local aproximada vía offset del sistema (sin chrono).
    let offset = local_offset_secs();
    let local = (seconds as i64 + offset).rem_euclid(86_400) as u64;
    format!("{:02}:{:02}", local / 3600, (local % 3600) / 60)
}

pub(super) fn local_offset_secs() -> i64 {
    // Una sola vez por proceso: el TZ no cambia a mitad de sesión.
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let output = std::process::Command::new("date").arg("+%z").output();
        match output {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if text.len() == 5 {
                    let sign = if text.starts_with('-') { -1 } else { 1 };
                    let hours: i64 = text[1..3].parse().unwrap_or(0);
                    let minutes: i64 = text[3..5].parse().unwrap_or(0);
                    sign * (hours * 3600 + minutes * 60)
                } else {
                    0
                }
            }
            Err(_) => 0,
        }
    })
}

// ---------------------------------------------------------------------------
// Navegación
// ---------------------------------------------------------------------------

pub(super) fn draw_nav(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    use crate::app::LibFilter;
    let rect = layout.nav;
    let focused = app.focus == crate::app::Pane::Nav;
    panel(buf, rect, pal, focused, 0.55, Some("vistas"), app.audio.kick_pulse);
    app.hit.add(rect, Action::Focus(crate::app::Pane::Nav));

    let playlists = app.lib.playlists().unwrap_or_default();
    let block = (Tab::ALL.len() + 3 + playlists.len().min(6) + 3) as u16;
    let offset = if app.vcenter {
        rect.height.saturating_sub(block) / 2
    } else {
        0
    };
    let mut y = rect.y + 2 + offset;
    let pulse = app.audio.kick_pulse;

    for tab in Tab::ALL {
        if y + 1 >= rect.y + rect.height {
            break;
        }
        let active = app.tab == tab;
        let row = Rect::new(rect.x + 1, y, rect.width - 2, 1);
        if active {
            fill(
                buf,
                row,
                mix(pal.surface, pal.primary, 0.22 + pulse * 0.10),
            );
            put(buf, row.x, y, "▍", color(pal.primary), None);
        }
        let fg = if active {
            pal.text
        } else if app.hover == Some(Action::Tab(tab)) {
            pal.text
        } else {
            pal.muted
        };
        text_at(
            buf,
            Rect::new(row.x + 2, y, row.width.saturating_sub(2), 1),
            format!("{}  {}", tab.icon(), tab.label()),
            Style::new()
                .fg(color(fg))
                .add_modifier(if active { Modifier::BOLD } else { Modifier::empty() }),
            Alignment::Left,
        );
        app.hit.add(row, Action::Tab(tab));
        y += 1;
    }

    y += 1;
    if y < rect.y + rect.height.saturating_sub(2) {
        text_at(
            buf,
            Rect::new(rect.x + 2, y, rect.width.saturating_sub(3), 1),
            "biblioteca".to_string(),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
        y += 1;
    }
    for playlist in playlists.iter().take(6) {
        if y + 1 >= rect.y + rect.height {
            break;
        }
        let row = Rect::new(rect.x + 1, y, rect.width - 2, 1);
        let active = app.lib_filter == LibFilter::Playlist(playlist.id);
        if active {
            fill(buf, row, mix(pal.surface, pal.primary, 0.18));
        }
        let mut label = format!("· {}", playlist.name);
        truncate_chars(&mut label, row.width.saturating_sub(4) as usize);
        text_at(
            buf,
            Rect::new(row.x + 2, y, row.width.saturating_sub(2), 1),
            label,
            Style::new().fg(color(if active { pal.text } else { pal.muted })),
            Alignment::Left,
        );
        app.hit.add(row, Action::PlaylistOpen(playlist.id));
        y += 1;
    }
    for (label, filter) in [
        ("favoritas", LibFilter::Favorites),
        ("descargadas", LibFilter::Downloaded),
        ("recientes", LibFilter::Recent),
    ] {
        if y + 1 >= rect.y + rect.height {
            break;
        }
        let row = Rect::new(rect.x + 1, y, rect.width - 2, 1);
        let active = app.lib_filter == filter;
        if active {
            fill(buf, row, mix(pal.surface, pal.primary, 0.18));
        }
        text_at(
            buf,
            Rect::new(row.x + 2, y, row.width.saturating_sub(2), 1),
            format!("{} {label}", if active { "♥" } else { "♪" }),
            Style::new().fg(color(if active { pal.text } else { pal.dim })),
            Alignment::Left,
        );
        app.hit.add(row, Action::LibraryFilter(filter));
        y += 1;
    }

    // Pie: contadores.
    if rect.height > 6 {
        let footer_y = rect.y + rect.height - 2;
        let tracks = app.lib.count().unwrap_or(0);
        let ready = app.lib.ready_count().unwrap_or(0);
        text_at(
            buf,
            Rect::new(rect.x + 2, footer_y, rect.width.saturating_sub(3), 1),
            format!("{ready}/{tracks} pistas"),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

// ---------------------------------------------------------------------------
// Ahora suena
// ---------------------------------------------------------------------------

pub(super) fn draw_transport(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.transport;
    if rect.height < 2 {
        return;
    }
    fill_soft(buf, rect, pal.bg_deep, 0.82, pal.bg_deep);
    let progress = {
        let duration = app.now.duration_ms.max(1);
        let position = app.display_position_ms().min(duration);
        (position as f32 / duration as f32).clamp(0.0, 1.0)
    };
    let pulse = (app.pulse.eased() * 0.45 + app.audio.kick_pulse * 0.55).min(1.0);

    if app.progress_in_viz && layout.viz.height >= 2 {
        // La barra vive sobre el visualizador, centrada en su franja. La
        // franja de fondo es opaca: sobre el efecto la barra se perdía.
        let y = layout.viz.y + layout.viz.height / 2;
        let width = bar_width(rect.width, app.bar_width_pct);
        let bar_rect = centered(Rect::new(rect.x, y, rect.width, 1), width, 1);
        draw_progress_bar(app, buf, bar_rect, pal, progress, pulse, true);
    } else {
        let width = bar_width(rect.width, app.bar_width_pct);
        let bar_rect = centered(Rect::new(rect.x, rect.y, rect.width, 1), width, 1);
        draw_progress_bar(app, buf, bar_rect, pal, progress, pulse, false);
    }

    // Controles centrados en la fila inferior del transporte (o en la única
    // fila que queda libre cuando la barra se fue al visualizador).
    let y = if app.progress_in_viz {
        rect.y
    } else {
        rect.y + 1
    };
    let pad = u16::from(app.button_pad.clamp(1, 4));
    let buttons: [(&str, Action, bool); 3] = [
        ("◀◀", Action::Prev, false),
        (if app.now.playing { PAUSE } else { PLAY }, Action::PlayPause, true),
        ("▶▶", Action::Next, false),
    ];
    let widths: u16 = buttons
        .iter()
        .map(|(label, _, _)| label.chars().count() as u16 + pad * 2)
        .sum();
    let mut x = rect.x + (rect.width.saturating_sub(widths + 2)) / 2;
    for (label, action, primary) in buttons {
        let width = label.chars().count() as u16 + pad * 2;
        let button = Rect::new(x, y, width, 1);
        let hover = app.hover == Some(action.clone());
        let bg = if hover {
            pal.surface_alt
        } else if primary {
            mix_oklab(pal.surface, pal.primary, 0.22 + pulse * 0.28)
        } else {
            pal.surface
        };
        fill(buf, button, bg);
        text_at(
            buf,
            button,
            label.to_string(),
            Style::new()
                .fg(color(if primary { pal.primary } else { pal.text }))
                .add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
        app.hit.add(button, action);
        x += width + 1;
    }

    // Volumen a la izquierda.
    let mut vx = rect.x + 2;
    let vol_label = if app.now.volume <= 0.001 { "🔇" } else { "♪" };
    put(buf, vx, y, vol_label, color(pal.muted), None);
    vx += 2;
    let cells = 8u16;
    for i in 0..cells {
        let threshold = i as f32 / cells as f32;
        let on = app.now.volume > threshold;
        put(
            buf,
            vx + i,
            y,
            if on { "▮" } else { "▯" },
            color(if on { pal.secondary } else { pal.border_dim }),
            None,
        );
        app.hit.add(
            Rect::new(vx + i, y, 1, 1),
            Action::Volume((i + 1) as f32 / cells as f32),
        );
    }

    // Descargas activas: un chip discreto, siempre visible.
    if let Some(download) = app.downloads.iter().find(|info| info.active()) {
        let text = format!("↓ {} {:>3.0}%", download.stage, download.percent);
        let width = text.chars().count() as u16 + 2;
        if vx + cells + 3 + width < rect.x + rect.width / 2 {
            let chip = Rect::new(vx + cells + 3, y, width, 1);
            fill(buf, chip, mix(pal.surface, pal.primary, 0.12));
            text_at(buf, chip, text, Style::new().fg(color(pal.primary)), Alignment::Center);
        }
    }

    // Derecha: viz, arte, spotify (separados del resto).
    let mut right = rect.x + rect.width.saturating_sub(2);
    let mut items: Vec<(String, Action, Rgb)> = vec![
        (app.viz.preset_name().to_string(), Action::VizNext, pal.primary),
        (format!("arte {}", app.art_mode.label()), Action::ArtMode, pal.dim),
    ];
    if app.follow_spotify {
        items.push(("spotify".into(), Action::ToggleFollow, pal.secondary));
    }
    for (label, action, fg) in items {
        let width = label.chars().count() as u16 + 2;
        if right < vx + cells + 4 + width {
            break;
        }
        right -= width;
        let chip_rect = Rect::new(right, y, width, 1);
        let hover = app.hover == Some(action.clone());
        fill(
            buf,
            chip_rect,
            if hover { pal.surface_alt } else { pal.surface },
        );
        text_at(
            buf,
            chip_rect,
            label,
            Style::new().fg(color(fg)),
            Alignment::Center,
        );
        app.hit.add(chip_rect, action);
        right = right.saturating_sub(1);
    }
}

/// Ancho de la barra según el ajuste, sin comerse los tiempos laterales.
pub(super) fn bar_width(total: u16, pct: u8) -> u16 {
    let target = (u32::from(total) * u32::from(pct.clamp(55, 100)) / 100) as u16;
    let max = total.saturating_sub(20).max(20.min(total));
    let min = 10.min(total);
    target.clamp(min, max.max(min))
}

/// Barra de progreso + tiempos. `overlay` le pone fondo para que se lea sobre
/// el visualizador.
pub(super) fn draw_progress_bar(
    app: &mut App,
    buf: &mut Buffer,
    bar_rect: Rect,
    pal: &Palette,
    progress: f32,
    pulse: f32,
    overlay: bool,
) {
    if overlay {
        let strip = Rect::new(
            bar_rect.x.saturating_sub(9),
            bar_rect.y,
            bar_rect.width + 18,
            1,
        );
        fill(buf, strip, pal.bg_deep);
        for x in strip.x..strip.x + strip.width {
            put_bg(buf, x, strip.y, mix(pal.bg_deep, pal.bg, 0.35));
        }
    }
    let position = app.display_position_ms().min(app.now.duration_ms.max(1));
    for x in 0..bar_rect.width {
        let t = x as f32 / bar_rect.width.max(1) as f32;
        let filled = t <= progress;
        let ch = if filled { "━" } else { "─" };
        let fg = if filled {
            mix_oklab(pal.primary, pal.secondary, t)
        } else {
            pal.border_dim
        };
        put(buf, bar_rect.x + x, bar_rect.y, ch, color(fg), None);
    }
    let head = bar_rect.x + (progress * bar_rect.width as f32) as u16;
    put(
        buf,
        head.min(bar_rect.x + bar_rect.width.saturating_sub(1)),
        bar_rect.y,
        if app.now.playing { "●" } else { "○" },
        color(mix_oklab(pal.text, pal.primary, 0.4 + pulse * 0.6)),
        None,
    );
    app.hit.add(
        Rect::new(bar_rect.x, bar_rect.y, bar_rect.width, 1),
        Action::SeekTo(position),
    );

    if bar_rect.x > 8 {
        text_at(
            buf,
            Rect::new(bar_rect.x - 8, bar_rect.y, 7, 1),
            format_ms(position),
            Style::new().fg(color(pal.muted)),
            Alignment::Right,
        );
        text_at(
            buf,
            Rect::new(bar_rect.x + bar_rect.width + 1, bar_rect.y, 7, 1),
            format_ms(app.now.duration_ms),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

pub(super) fn draw_hint(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.hint;
    fill_soft(buf, rect, pal.bg_deep, 0.82, pal.bg_deep);
    let hints: [(&str, &str); 9] = [
        ("space", "play"),
        ("←→", "seek"),
        ("/", "buscar"),
        ("a", "cola"),
        ("x", "quitar"),
        ("c", "portada"),
        ("v", "visual"),
        ("A", "arte"),
        ("?", "ayuda"),
    ];
    let mut x = rect.x + 1;
    for (key, label) in hints {
        let key_width = key.chars().count() as u16 + 2;
        if x + key_width + label.len() as u16 + 3 > rect.x + rect.width {
            break;
        }
        let cap = Rect::new(x, rect.y, key_width, 1);
        keycap(buf, cap, pal, key);
        x += key_width + 1;
        put(buf, x, rect.y, label, color(pal.dim), None);
        x += label.chars().count() as u16 + 2;
    }
    let right_text = if app.pending_play.is_some() {
        "descargando…".to_string()
    } else if app.now.loading {
        "cargando…".to_string()
    } else if let Some(track) = &app.now.track {
        format!(
            "♪ {}  ·  {}{}",
            track.title,
            track.artist,
            if app.follow_spotify { "  ·  spotify" } else { "" }
        )
    } else if app.follow_spotify {
        "modo acompañante de spotify".to_string()
    } else {
        format!(
            "{} pistas · {} descargadas",
            app.lib.count().unwrap_or(0),
            app.lib.ready_count().unwrap_or(0)
        )
    };
    let width = right_text.chars().count() as u16;
    if rect.width > width + 4 {
        text_at(
            buf,
            Rect::new(rect.x + rect.width - width - 2, rect.y, width, 1),
            right_text,
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
    }
}

// ---------------------------------------------------------------------------
// Toasts y overlays
// ---------------------------------------------------------------------------
