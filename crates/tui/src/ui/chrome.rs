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
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            Alignment::Center,
        );
        app.hit.add(tab_rect, Action::Tab(tab));
        x += width + 1;
    }

    // Chip del reproductor del sistema: puerta de entrada al modo MPRIS.
    let spotify_label = if app.follow_spotify {
        "● sistema"
    } else {
        "○ sistema"
    };
    let spotify_width = spotify_label.chars().count() as u16 + 2;
    if x + spotify_width + 1 < rect.x + rect.width.saturating_sub(24) {
        let chip_rect = Rect::new(x + 1, rect.y, spotify_width, 1);
        let hover = app.hover == Some(Action::ToggleFollow);
        fill(
            buf,
            chip_rect,
            if app.follow_spotify {
                mix(
                    pal.surface,
                    pal.secondary,
                    0.30 + app.audio.kick_pulse * 0.20,
                )
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
        Some(format!(
            "{:.0} fps · {:.1} ms",
            app.fps.value(),
            app.frame_ms
        ))
    } else {
        None
    };
    let clock = clock_now();
    let mut items: Vec<(String, Rgb)> = Vec::new();
    if app.update_run.is_some() {
        items.push(("↻ actualizando".into(), pal.warn));
    } else if app.update_finished == Some(false) {
        items.push(("↻ actualización falló".into(), pal.error));
    }
    if let Some(fps) = fps {
        items.push((fps, pal.dim));
    }
    if app.cfg.ui.fx.bloom_play {
        items.push((
            "✺".into(),
            mix_oklab(pal.secondary, pal.primary, app.audio.kick_pulse),
        ));
    }
    items.push((clock, pal.muted));
    if let Some(room) = &app.room {
        let label = app.room_label(&room.id);
        items.push((format!("◆ {label}"), pal.primary));
    } else if app.follow_spotify {
        items.push(("● sistema".into(), pal.secondary));
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
    panel(
        Some(app),
        buf,
        rect,
        pal,
        focused,
        0.55,
        Some("vistas"),
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Nav));
    // Clic derecho en la cabecera "vistas": crear/importar lista, recargar...
    app.hit
        .add(Rect::new(rect.x, rect.y, rect.width, 1), Action::NavMenu);

    // Las listas creadas con «guardar disco» son álbumes, no listas de
    // usuario: se gestionan en la pestaña Discos y no ensucian la navegación.
    let playlists: Vec<_> = app
        .lib
        .playlists()
        .unwrap_or_default()
        .into_iter()
        .filter(|playlist| !is_album_playlist(&playlist.description))
        .collect();
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
            fill(buf, row, mix(pal.surface, pal.primary, 0.22 + pulse * 0.10));
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
            Style::new().fg(color(fg)).add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
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

/// Barra inferior de canción actual, al estilo de Spotify: portada pequeña,
/// título/artista/álbum, corazón para favorita y estado de reproducción.
pub(super) fn draw_nowbar(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.nowbar;
    if rect.height == 0 || rect.width < 24 {
        return;
    }
    fill_soft(buf, rect, mix(pal.bg_deep, pal.surface, 0.62), 0.88, pal.bg_deep);
    for x in rect.x..rect.x + rect.width {
        put(buf, x, rect.y, "─", color(pal.border_dim), None);
    }
    let art = Rect::new(
        rect.x + rect.width.saturating_sub(6),
        rect.y,
        4,
        rect.height,
    );
    let right = rect.x + rect.width.saturating_sub(2);
    let Some(track) = app.now.track.clone() else {
        text_at(
            buf,
            Rect::new(rect.x + 2, rect.y, right.saturating_sub(rect.x + 2), rect.height),
            "nada sonando · enter reproduce lo seleccionado",
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
        app.hit.add(rect, Action::Tab(Tab::Now));
        return;
    };
    draw_thumb(app, buf, art, &track, pal);
    let text_right = art.x.saturating_sub(2);
    let status = if rect.width < 72 {
        String::new()
    } else if app.now.loading {
        "cargando…".to_string()
    } else if app.now.playing {
        format!(
            "▶ {} / {}",
            format_ms(app.display_position_ms()),
            format_ms(app.now.duration_ms)
        )
    } else {
        format!(
            "⏸ {} / {}",
            format_ms(app.now.position_ms),
            format_ms(app.now.duration_ms)
        )
    };
    // La canción actual vive en el extremo derecho, con la portada pegada al
    // borde. El estado de reproducción queda a la izquierda para no competir
    // con el título.
    if !status.is_empty() {
        let status_w = status.chars().count() as u16;
        text_at(
            buf,
            Rect::new(rect.x + 3, rect.y, status_w.min(rect.width), 1),
            status,
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
    let favorite = app
        .lib
        .track_by_key(&track.key)
        .ok()
        .flatten()
        .map(|row| row.favorite)
        .unwrap_or(false);
    let heart = if favorite { "✓" } else { "♡" };
    put(
        buf,
        text_right,
        rect.y,
        heart,
        color(if favorite { pal.ok } else { pal.muted }),
        None,
    );
    app.hit.add(
        Rect::new(text_right.saturating_sub(1), rect.y, 3, rect.height),
        Action::FavoriteNow,
    );
    let text_w = text_right.saturating_sub(rect.x + 2).saturating_sub(2);
    let title_right = text_right.saturating_sub(2);
    let mut title = track.title.clone();
    truncate_chars(&mut title, text_w as usize);
    let title_x = title_right.saturating_sub(title.chars().count() as u16);
    put(buf, title_x, rect.y, &title, color(pal.text), None);
    app.hit.add(rect, Action::Tab(Tab::Now));

    // Artista y álbum van por separado para que cada uno tenga su hitbox y su
    // resaltado animado. La línea se alinea a la derecha como antes.
    let mut artist = track.artist.clone();
    truncate_chars(&mut artist, text_w as usize);
    let album = track
        .album
        .as_deref()
        .filter(|album| !album.is_empty())
        .map(str::to_string);
    let artist_w = artist.chars().count() as u16;
    let sep = "  ·  ";
    let sep_w = sep.chars().count() as u16;
    let mut album = album;
    if album.as_ref().is_some_and(|name| {
        artist_w + sep_w + name.chars().count() as u16 > text_w
    }) {
        if let Some(name) = album.as_mut() {
            let room = text_w.saturating_sub(artist_w + sep_w);
            if room > 2 {
                truncate_chars(name, room as usize);
            } else {
                album = None;
            }
        }
    }
    let album_w = album
        .as_ref()
        .map(|name| name.chars().count() as u16)
        .unwrap_or(0);
    let line_w = artist_w + if album_w > 0 { sep_w + album_w } else { 0 };
    let line_x = title_right.saturating_sub(line_w);
    let hover_artist = app.hover == Some(Action::OpenArtist(track.artist.clone()));
    if artist_w > 1 {
        link_at(
            buf,
            line_x,
            rect.y + 1,
            artist_w,
            &artist,
            pal,
            pal.secondary,
            hover_artist,
            app.ui_time(),
        );
        app.hit.add(
            Rect::new(line_x, rect.y + 1, artist_w, 1),
            Action::OpenArtist(track.artist.clone()),
        );
    }
    if let Some(album) = album {
        let sep_x = line_x + artist_w;
        put(buf, sep_x, rect.y + 1, sep, color(pal.dim), None);
        let album_x = sep_x + sep_w;
        let hover_album =
            app.hover == Some(Action::OpenAlbum(album.clone(), track.artist.clone()));
        let drawn = link_at(
            buf,
            album_x,
            rect.y + 1,
            album_w,
            &album,
            pal,
            pal.muted,
            hover_album,
            app.ui_time(),
        );
        if drawn > 1 {
            app.hit.add(
                Rect::new(album_x, rect.y + 1, drawn, 1),
                Action::OpenAlbum(album, track.artist.clone()),
            );
        }
    }
}

pub(super) fn draw_transport(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.transport;
    if rect.height == 0 || rect.width < 12 {
        return;
    }
    fill_soft(buf, rect, pal.bg_deep, 0.82, pal.bg_deep);
    let progress = if app.now.duration_ms == 0 {
        0.0
    } else {
        let duration = app.now.duration_ms;
        let position = app.display_position_ms().min(duration);
        (position as f32 / duration as f32).clamp(0.0, 1.0)
    };
    let pulse = (app.pulse.eased() * 0.45 + app.audio.kick_pulse * 0.55).min(1.0);
    let progress_row = rect.height >= 2 && (!app.progress_in_viz || layout.viz.height < 2);
    if app.progress_in_viz && layout.viz.height >= 2 {
        let y = layout.viz.y + layout.viz.height / 2;
        let width = bar_width(rect.width, app.bar_width_pct);
        let bar_rect = centered(Rect::new(rect.x, y, rect.width, 1), width, 1);
        draw_progress_bar(app, buf, bar_rect, pal, progress, pulse, true);
    } else if progress_row {
        let width = bar_width(rect.width, app.bar_width_pct);
        let bar_rect = centered(Rect::new(rect.x, rect.y, rect.width, 1), width, 1);
        draw_progress_bar(app, buf, bar_rect, pal, progress, pulse, false);
    }

    let controls_y = rect.y + u16::from(progress_row);
    let controls_h = rect.height.saturating_sub(u16::from(progress_row)).max(1);
    let compact = rect.width < 82;
    let big = controls_h >= 2 && rect.width >= 100;
    let pad = u16::from(app.button_pad.clamp(1, 4));
    let (prev_label, play_label, next_label) =
        ("◀◀", if app.now.playing { PAUSE } else { PLAY }, "▶▶");
    let repeat_label = match app.repeat_mode {
        spote_core::playback::RepeatMode::Off => "⟲",
        spote_core::playback::RepeatMode::All => "⟲",
        spote_core::playback::RepeatMode::One => "⟲1",
    };
    let side_w = if big { 9 } else { 5 };
    let (prev_w, play_w, next_w, repeat_w) = if big {
        (7, 9, 9, side_w)
    } else {
        (
            prev_label.chars().count() as u16 + pad * 2,
            play_label.chars().count() as u16 + pad * 2,
            next_label.chars().count() as u16 + pad * 2,
            side_w,
        )
    };
    let show_modes = rect.width >= 72;
    // Con los modos ocultos no se reserva su hueco: play/next deben quedar
    // centrados aunque el ancho no permita enseñar shuffle y repeat.
    let total_w = prev_w + play_w + next_w + 2 + if show_modes { (side_w + 1) * 2 } else { 0 };
    let mut x = rect.x + rect.width.saturating_sub(total_w) / 2;
    let play_rect = Rect::new(
        x + if show_modes { side_w + 1 } else { 0 } + prev_w + 1,
        controls_y,
        play_w,
        controls_h,
    );
    if app.cfg.ui.fx.bloom_play {
        bloom_glow(buf, play_rect, pal, 0.35 + app.audio.kick_pulse * 0.45);
    }

    if show_modes {
        let shuffle = Rect::new(x, controls_y, side_w, controls_h);
        draw_transport_control(
            app,
            buf,
            shuffle,
            Action::ToggleShuffle,
            false,
            app.shuffle_on,
            ICON_SHUFFLE,
            "azar",
            "⇄",
            pal,
            big,
            controls_h,
            pulse,
        );
        x += side_w + 1;
    }
    let prev = Rect::new(x, controls_y, prev_w, controls_h);
    draw_transport_control(
        app,
        buf,
        prev,
        Action::Prev,
        false,
        false,
        ICON_PREV,
        "atrás",
        prev_label,
        pal,
        big,
        controls_h,
        pulse,
    );
    x += prev_w + 1;
    let play = Rect::new(x, controls_y, play_w, controls_h);
    draw_transport_control(
        app,
        buf,
        play,
        Action::PlayPause,
        true,
        false,
        if app.now.playing {
            ICON_PAUSE
        } else {
            ICON_PLAY
        },
        if app.now.playing { "pausa" } else { "play" },
        play_label,
        pal,
        big,
        controls_h,
        pulse,
    );
    x += play_w + 1;
    let next = Rect::new(x, controls_y, next_w, controls_h);
    draw_transport_control(
        app,
        buf,
        next,
        Action::Next,
        false,
        false,
        ICON_NEXT,
        "siguiente",
        next_label,
        pal,
        big,
        controls_h,
        pulse,
    );
    x += next_w + 1;
    if show_modes {
        let repeat = Rect::new(x, controls_y, repeat_w, controls_h);
        draw_transport_control(
            app,
            buf,
            repeat,
            Action::CycleRepeat,
            false,
            !matches!(app.repeat_mode, spote_core::playback::RepeatMode::Off),
            if matches!(app.repeat_mode, spote_core::playback::RepeatMode::One) {
                ICON_REPEAT_ONE
            } else {
                ICON_REPEAT
            },
            match app.repeat_mode {
                spote_core::playback::RepeatMode::One => "bucle 1",
                spote_core::playback::RepeatMode::All => "todo",
                spote_core::playback::RepeatMode::Off => "bucle",
            },
            repeat_label,
            pal,
            big,
            controls_h,
            pulse,
        );
    }

    // Slider de volumen: barra con degradado, knob y porcentaje. Se puede
    // arrastrar, clicar y mover con ↑↓ cuando el transporte tiene el foco.
    let vol_w = if compact {
        8
    } else {
        14u16.min(rect.width / 6).max(8)
    };
    let vol_x = rect.x + 2;
    let y = controls_y + controls_h / 2;
    let icon = if app.now.volume <= 0.001 {
        "🔇"
    } else {
        "♪"
    };
    put(
        buf,
        vol_x,
        y,
        icon,
        color(if app.now.volume <= 0.001 {
            pal.error
        } else {
            pal.secondary
        }),
        None,
    );
    let track_x = vol_x + 2;
    if app.focus == crate::app::Pane::Transport {
        put(
            buf,
            vol_x.saturating_sub(1),
            y,
            "▏",
            color(pal.primary),
            None,
        );
    }
    for i in 0..vol_w {
        let t = (i as f32 + 0.5) / vol_w as f32;
        let on = t <= app.now.volume;
        let color_fill = mix_oklab(pal.secondary, pal.primary, t);
        let (ch, fg) = if on {
            if i == ((app.now.volume * vol_w as f32) as u16).min(vol_w - 1) {
                ("◉", mix_oklab(color_fill, pal.text, 0.55))
            } else {
                ("█", color_fill)
            }
        } else {
            ("░", pal.border_dim)
        };
        put(buf, track_x + i, y, ch, color(fg), None);
    }
    // La zona clicable no puede subir a la fila de progreso: en un transporte
    // de dos filas se comía los clics de la barra de seek.
    let hit_h = if controls_h >= 3 { 2 } else { 1 };
    app.hit.add(
        Rect::new(track_x, y, vol_w, hit_h.min(rect.height)),
        Action::VolumeTrack,
    );
    let label = format!("{:>3.0}%", app.now.volume * 100.0);
    put(
        buf,
        track_x + vol_w + 1,
        y,
        &label,
        color(
            if app.dragging_volume || app.focus == crate::app::Pane::Transport {
                pal.primary
            } else {
                pal.dim
            },
        ),
        None,
    );

    if compact {
        return;
    }
    // Descargas activas: chip discreto, siempre visible.
    if let Some(download) = app.downloads.iter().find(|info| info.active()) {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        let spin = frames[(app.viz.time * 10.0) as usize % frames.len()];
        let text = format!("{spin} {} {:>3.0}%", download.stage, download.percent);
        let width = text.chars().count() as u16 + 2;
        if track_x + vol_w + 8 + width < rect.x + rect.width / 2 {
            let chip = Rect::new(track_x + vol_w + 7, y, width, 1);
            fill(buf, chip, mix(pal.surface, pal.primary, 0.12));
            text_at(
                buf,
                chip,
                text,
                Style::new().fg(color(pal.primary)),
                Alignment::Center,
            );
        }
    }

    // Derecha: viz, arte, spotify (separados del resto).
    let mut right = rect.x + rect.width.saturating_sub(2);
    let art_label = if app.art_mode == ArtMode::Plain {
        "arte".to_string()
    } else {
        format!("arte {}", app.art_mode.label())
    };
    let mut items: Vec<(String, Action, Rgb)> = vec![
        (
            app.viz.preset_name().to_string(),
            Action::VizNext,
            pal.primary,
        ),
        (art_label, Action::ArtMode, pal.dim),
    ];
    if app.follow_spotify {
        items.push(("sistema".into(), Action::ToggleFollow, pal.secondary));
    }
    for (label, action, fg) in items {
        let width = label.chars().count() as u16 + 2;
        if right < track_x + vol_w + 9 + width {
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

const ICON_PLAY: &[&str] = &["█    ", "██   ", "███  ", "██   ", "█    "];
const ICON_PAUSE: &[&str] = &["█  █", "█  █", "█  █", "█  █", "█  █"];

const ICON_PREV: &[&str] = &["█  █ ", "██ █ ", "███  ", "██ █ ", "█  █ "];
const ICON_NEXT: &[&str] = &[" █  █", " █ ██", "  ███", " █ ██", " █  █"];

const ICON_SHUFFLE: &[&str] = &["██   ██", " ██ ██ ", "  ███  ", " ██ ██ ", "██   ██"];
const ICON_REPEAT: &[&str] = &["███████", "█     █", "█     █", "█     █", "███████"];
const ICON_REPEAT_ONE: &[&str] = &["███████", "█     █", "█  █  █", "█  █  █", "███████"];

/// Pinta un icono de bloque centrado, escalando si el botón es más alto.
pub(super) fn draw_icon(buf: &mut Buffer, rect: Rect, icon: &[&str], fg: Color, glow: bool) {
    let w = icon
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as u16;
    if rect.width < w || rect.height == 0 {
        return;
    }
    // Si el botón es más bajo que el icono, se muestrean las filas centrales:
    // el play sigue leyéndose en 2 filas sin deformarse.
    let h = icon.len();
    let rows: Vec<&str> = if rect.height as usize >= h {
        icon.to_vec()
    } else {
        let keep = rect.height as usize;
        (0..keep)
            .map(|i| {
                let src = (i * h / keep).min(h - 1);
                icon[src]
            })
            .collect()
    };
    let x0 = rect.x + (rect.width - w) / 2;
    let y0 = rect.y + (rect.height.saturating_sub(rows.len() as u16)) / 2;
    for (dy, line) in rows.iter().enumerate() {
        for (dx, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            if glow {
                put(buf, x0 + dx as u16, y0 + dy as u16 + 1, "▁", fg, None);
            }
            put(buf, x0 + dx as u16, y0 + dy as u16, "█", fg, None);
        }
    }
}

/// Halo radial alrededor del play. Usa el verde/acento de la paleta y late con
/// el bombo; con el ajuste `bloom_play` apagado no se pinta.
pub(super) fn bloom_glow(buf: &mut Buffer, rect: Rect, pal: &Palette, amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    let area = Rect::new(
        rect.x.saturating_sub(2),
        rect.y.saturating_sub(1),
        rect.width + 4,
        rect.height + 2,
    );
    let cx = area.x as f32 + area.width as f32 / 2.0;
    let cy = area.y as f32 + area.height as f32 / 2.0;
    let radius = (area.width.max(area.height) as f32 / 2.0).max(1.0);
    let b = *buf.area();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if x >= b.x + b.width || y >= b.y + b.height {
                continue;
            }
            let dx = x as f32 - cx;
            let dy = (y as f32 - cy) * 2.0;
            let distance = (dx * dx + dy * dy).sqrt() / radius;
            if distance > 1.0 {
                continue;
            }
            let glow = (1.0 - distance).powf(2.2) * 0.22 * amount;
            if glow <= 0.005 {
                continue;
            }
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            let under = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => pal.bg_deep,
            };
            let mixed = mix(under, mix_oklab(pal.primary, pal.secondary, distance), glow);
            cell.set_bg(color(mixed));
            cell.set_fg(color(mixed));
            if cell.symbol() == " " {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Botón del transporte. Todos comparten altura, relleno, hover, borde activo
/// y el mismo par icono/etiqueta; así shuffle y bucle no parecen de otra TUI.
#[allow(clippy::too_many_arguments)]
fn draw_transport_control(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    action: Action,
    primary: bool,
    active: bool,
    icon: &[&str],
    big_label: &str,
    compact_label: &str,
    pal: &Palette,
    big: bool,
    controls_h: u16,
    pulse: f32,
) {
    let hover = app.hover == Some(action.clone());
    let bg = if hover {
        pal.surface_alt
    } else if primary {
        mix_oklab(pal.surface, pal.primary, 0.25 + pulse * 0.32)
    } else if active {
        mix_oklab(pal.surface, pal.primary, 0.30)
    } else {
        pal.surface
    };
    fill_soft(buf, rect, bg, 0.95, pal.bg_deep);
    if (primary || active) && controls_h == 2 {
        // Con dos filas el borde superior/inferior sustituye al fondo como
        // indicador. Con tres o más hay icono pixel-art: el fondo y el brillo
        // ya marcan el estado y no se pisa la primera fila del dibujo.
        let edge = if active {
            mix_oklab(pal.primary, pal.secondary, 0.35)
        } else {
            mix_oklab(pal.primary, pal.secondary, pulse)
        };
        for i in 1..rect.width.saturating_sub(1) {
            put(buf, rect.x + i, rect.y, "▔", color(edge), None);
            put(
                buf,
                rect.x + i,
                rect.y + rect.height - 1,
                "▁",
                color(edge),
                None,
            );
        }
    }
    if big {
        if controls_h >= 4 {
            // Cuatro filas: tres de icono y una de etiqueta.
            let icon_rect = Rect::new(rect.x, rect.y, rect.width, controls_h - 1);
            draw_icon(
                buf,
                icon_rect,
                icon,
                color(if primary || active {
                    pal.text
                } else {
                    pal.muted
                }),
                primary,
            );
            text_at(
                buf,
                Rect::new(rect.x, rect.y + controls_h - 1, rect.width, 1),
                big_label,
                Style::new().fg(color(if primary || active {
                    pal.primary
                } else {
                    pal.dim
                })),
                Alignment::Center,
            );
        } else {
            // Tres filas (el alto por defecto): el icono usa todo el botón y
            // se muestra a resolución completa. Las etiquetas volverían a
            // robar una fila y dejarían el repeat en dos píxeles ilegibles.
            draw_icon(
                buf,
                rect,
                icon,
                color(if primary || active {
                    pal.text
                } else {
                    pal.muted
                }),
                primary,
            );
        }
    } else {
        text_at(
            buf,
            rect,
            compact_label.to_string(),
            Style::new()
                .fg(color(if primary || active {
                    pal.primary
                } else {
                    pal.text
                }))
                .add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
    }
    app.hit.add(rect, action);
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
    // Marcas del recorte visibles siempre en la barra.
    let duration = app.now.duration_ms.max(1);
    for ms in [
        (app.trim_start_ms > 0).then_some(app.trim_start_ms),
        app.trim_end_ms,
    ]
    .into_iter()
    .flatten()
    {
        let pct = (ms as f32 / duration as f32).clamp(0.0, 1.0);
        let x = bar_rect.x + (pct * bar_rect.width.saturating_sub(1) as f32) as u16;
        put(
            buf,
            x.min(bar_rect.x + bar_rect.width.saturating_sub(1)),
            bar_rect.y,
            "┃",
            color(pal.warn),
            None,
        );
    }

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

/// `save_album_playlist` marca las listas de disco con «álbum · N pistas».
/// Esas listas no se pintan en la navegación de biblioteca.
fn is_album_playlist(description: &str) -> bool {
    let description = description.trim_start().to_lowercase();
    description.starts_with("álbum") || description.starts_with("album")
}

pub(super) fn draw_hint(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.hint;
    fill_soft(buf, rect, pal.bg_deep, 0.82, pal.bg_deep);
    let right_text = if app.update_run.is_some() {
        let secs = app
            .update_started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        format!("↻ actualizando spote… {secs}s · no cierres")
    } else if app.update_finished == Some(false) {
        "↻ actualización fallida · esc para cerrar".to_string()
    } else if let Some(download) = app.downloads.iter().find(|info| info.active()) {
        format!("↓ {:.0}% · {}", download.percent, download.stage)
    } else if app.pending_play.is_some() {
        "descargando…".to_string()
    } else if app.now.loading {
        "cargando…".to_string()
    } else if layout.nowbar.height == 0 {
        if let Some(track) = &app.now.track {
            format!(
                "♪ {}  ·  {}{}",
                track.title,
                track.artist,
                if app.follow_spotify {
                    "  ·  sistema"
                } else {
                    ""
                }
            )
        } else if app.follow_spotify {
            "sistema (MPRIS)".to_string()
        } else {
            // Sin estado que contar, la barra se queda solo con los atajos.
            // Los contadores de la biblioteca viven en su pestaña y no
            // necesitan ocupar el pie en todas las vistas.
            String::new()
        }
    } else if app.follow_spotify {
        "sistema (MPRIS)".to_string()
    } else {
        String::new()
    };
    let width = right_text.chars().count() as u16;
    if width > 0 && rect.width > width + 4 {
        let right_rect = Rect::new(rect.x + rect.width - width - 2, rect.y, width, 1);
        text_at(
            buf,
            right_rect,
            right_text,
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
        if layout.nowbar.height == 0 {
            if let Some(track) = &app.now.track {
                if !track.artist.is_empty() {
                    app.hit
                        .add(right_rect, Action::OpenArtist(track.artist.clone()));
                }
            }
        }
    }
    // Las leyendas salen de `keymap`: una sola fuente de verdad. Nada de
    // teclas que no existen o que hacen otra cosa.
    let hints = app.hints();
    let mut x = rect.x + 1;
    // Cinco atajos como máximo: el pie es una ayuda, no un manual. El resto
    // sigue disponible en `?`.
    for hint in hints.into_iter().take(5) {
        let key_width = hint.key.chars().count() as u16 + 2;
        let usable_right = rect.x + rect.width.saturating_sub((width + 4).min(rect.width));
        if x + key_width + hint.label.chars().count() as u16 + 3 > usable_right {
            break;
        }
        let cap = Rect::new(x, rect.y, key_width, 1);
        keycap(buf, cap, pal, hint.key);
        x += key_width + 1;
        put(buf, x, rect.y, hint.label, color(pal.dim), None);
        x += hint.label.chars().count() as u16 + 2;
    }
    // La barra de teclas cambia de longitud al abrir overlays. Forzamos su
    // difusión completa para que no queden restos de la leyenda anterior en
    // terminales que no limpian bien las celdas reemplazadas por espacios.
    for x in rect.x..rect.x + rect.width {
        if let Some(cell) = buf.cell_mut((x, rect.y)) {
            cell.diff_option = ratatui::buffer::CellDiffOption::AlwaysUpdate;
        }
    }
}

// ---------------------------------------------------------------------------
// Toasts y overlays
// ---------------------------------------------------------------------------
