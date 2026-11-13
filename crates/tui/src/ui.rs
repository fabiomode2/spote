//! Dibujo de la interfaz. Todo se pinta sobre `Buffer` para controlar color,
//! posición y glifo celda a celda.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use spote_core::artwork::{ArtMode, Palette, Rgb};
use spote_core::util::{format_ms, luma, mix, mix_oklab, scale};

use crate::app::{Action, App, Overlay, Tab, ToastKind};
use crate::theme::ease_out_cubic;
use crate::viz::Blend;

const PLAY: &str = "⏵";
const PAUSE: &str = "⏸";

#[derive(Debug, Clone, Copy, Default)]
pub struct Layout {
    pub full: Rect,
    pub top: Rect,
    pub main: Rect,
    pub nav: Rect,
    pub center: Rect,
    pub side: Rect,
    pub queue: Rect,
    pub info: Rect,
    pub viz: Rect,
    pub transport: Rect,
    pub hint: Rect,
    pub art: Rect,
    pub content: Rect,
}

impl Layout {
    pub fn compute(
        area: Rect,
        show_viz: bool,
        fullscreen: crate::app::Fullscreen,
        art_ratio: u16,
    ) -> Self {
        let mut out = Self {
            full: area,
            ..Default::default()
        };
        if area.width < 20 || area.height < 8 {
            out.top = Rect::new(area.x, area.y, area.width, 1);
            out.main = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
            out.center = out.main;
            out.hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            return out;
        }
        out.top = Rect::new(area.x, area.y, area.width, 1);
        out.hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        out.transport = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(3),
            area.width,
            2,
        );
        let mut viz_height = if show_viz {
            match area.height {
                0..=23 => 4,
                24..=31 => 6,
                32..=44 => 8,
                _ => 10,
            }
        } else {
            0
        };
        if fullscreen != crate::app::Fullscreen::None {
            viz_height = 0;
        }
        let main_height = area
            .height
            .saturating_sub(3)
            .saturating_sub(viz_height)
            .saturating_sub(1);
        out.main = Rect::new(area.x, area.y + 1, area.width, main_height);
        out.viz = Rect::new(
            area.x,
            area.y + 1 + main_height,
            area.width,
            viz_height,
        );

        let nav_width = if area.width >= 92 { 19 } else { 0 };
        // La cola lateral solo entra cuando sobra sitio de verdad: con 160
        // columnas se come el ancho de la letra.
        let side_width = if area.width >= 186 {
            (area.width / 4).clamp(34, 46)
        } else {
            0
        };
        out.nav = Rect::new(out.main.x, out.main.y, nav_width, out.main.height);
        out.center = Rect::new(
            out.main.x + nav_width,
            out.main.y,
            out.main.width.saturating_sub(nav_width + side_width),
            out.main.height,
        );
        out.side = Rect::new(
            out.main.x + out.main.width.saturating_sub(side_width),
            out.main.y,
            side_width,
            out.main.height,
        );
        if side_width > 0 {
            let queue_height = (out.side.height as f32 * 0.62) as u16;
            out.queue = Rect::new(out.side.x, out.side.y, out.side.width, queue_height);
            out.info = Rect::new(
                out.side.x,
                out.side.y + queue_height,
                out.side.width,
                out.side.height.saturating_sub(queue_height),
            );
        }
        // Portada: cuadrada de verdad (la celda es ~1:2, así que w = 2*h).
        let inner_h = out.center.height.saturating_sub(2);
        let (art, content) = if out.center.width >= 86 && inner_h >= 12 {
            let art_h = inner_h;
            let mut art_w = (art_h * 2).min(out.center.width * art_ratio / 10);
            // Nunca dejar el texto por debajo de ~44 columnas.
            let max_art = out.center.width.saturating_sub(47);
            if max_art > 24 {
                art_w = art_w.min(max_art);
            }
            let art = Rect::new(
                out.center.x + out.center.width.saturating_sub(art_w + 1),
                out.center.y + 1,
                art_w,
                art_h,
            );
            let content_w = out.center.width.saturating_sub(art_w + 3);
            let content = Rect::new(out.center.x + 1, out.center.y, content_w, out.center.height);
            (art, content)
        } else if out.center.width >= 46 && out.center.height >= 14 {
            // Estrecho: miniatura arriba a la derecha y el texto ocupa todo el
            // alto. Así la letra no desaparece en terminales pequeñas.
            let art_w = (out.center.width / 3).clamp(18, 30);
            let art_h = art_w / 2;
            let art = Rect::new(
                out.center.x + out.center.width.saturating_sub(art_w + 2),
                out.center.y + 2,
                art_w,
                art_h,
            );
            (art, out.center)
        } else {
            (Rect::new(out.center.x, out.center.y, 0, 0), out.center)
        };
        out.art = art;
        out.content = content;
        out
    }
}

pub fn draw(app: &mut App, buf: &mut Buffer, area: Rect) {
    // Frame limpio: con paneles translúcidos y textos que cambian de longitud,
    // sin esto quedan restos del frame anterior (placeholders, valores viejos).
    buf.reset();
    // El mismo reparto en todas las vistas: si cambia, la interfaz "salta" al
    // cambiar de pestaña.
    let art_ratio = 6;
    let layout = Layout::compute(area, app.show_viz, app.fullscreen, art_ratio);
    app.hit.clear();
    let pal = *app.palette.current();

    background(buf, area, &pal, app.intro);
    let zen = app.zen && app.show_viz;
    let viz_fullscreen = (app.viz_background || zen) && app.show_viz && layout.main.height > 2;
    if viz_fullscreen {
        let audio = &app.audio;
        app.viz.render(
            layout.full.width,
            layout.full.height,
            audio,
            &pal,
            1.0 / app.fps.value().max(1.0),
        );
        // En zen el visualizador es el protagonista: más opaco y sin paneles.
        if zen {
            blit_viz(app, buf, layout.full, 0, 0.9, 0.85);
        } else {
            blit_viz(app, buf, layout.full, 0, 0.32, 0.10);
        }
    }
    app.set_art_size_hint(layout.art.width, layout.art.height);
    app.ensure_art();

    if layout.main.width < 20 {
        draw_too_small(buf, area, &pal);
        return;
    }

    draw_top(app, buf, &layout, &pal);
    if !zen && layout.nav.width > 0 {
        draw_nav(app, buf, &layout, &pal);
    }
    match app.tab {
        Tab::Now => draw_now(app, buf, &layout, &pal),
        Tab::Search => draw_search(app, buf, &layout, &pal),
        Tab::Library => draw_library(app, buf, &layout, &pal),
        Tab::Room => draw_room(app, buf, &layout, &pal),
        Tab::Settings => draw_settings(app, buf, &layout, &pal),
    }
    if !zen && layout.side.width > 0 {
        draw_side(app, buf, &layout, &pal);
    }
    if layout.viz.height > 0 {
        draw_viz(app, buf, &layout, &pal);
    }
    draw_transport(app, buf, &layout, &pal);
    draw_hint(app, buf, &layout, &pal);
    draw_toasts(app, buf, &layout, &pal);
    draw_overlay(app, buf, &layout, &pal);
}

// ---------------------------------------------------------------------------
// Helpers de pintura
// ---------------------------------------------------------------------------

/// Recorta por caracteres, nunca por bytes: los títulos con acentos o
/// japoneses partían a mitad de un carácter y paniqueaban.
fn truncate_chars(text: &mut String, max: usize) {
    if text.chars().count() <= max {
        return;
    }
    let end = text
        .char_indices()
        .nth(max)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    text.truncate(end);
}

fn color(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// Rect interior con márgenes simétricos.
fn inset(rect: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        rect.x + x,
        rect.y + y,
        rect.width.saturating_sub(x * 2),
        rect.height.saturating_sub(y * 2),
    )
}

/// Coloca un bloque de `w x h` en el centro exacto de `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

fn put(buf: &mut Buffer, x: u16, y: u16, symbol: &str, fg: Color, bg: Option<Color>) {
    let area = buf.area();
    if x >= area.x + area.width || y >= area.y + area.height {
        return;
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_fg(fg);
        if let Some(bg) = bg {
            cell.set_bg(bg);
        }
    }
}

fn put_bg(buf: &mut Buffer, x: u16, y: u16, bg: Rgb) {
    let area = buf.area();
    if x >= area.x + area.width || y >= area.y + area.height {
        return;
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(" ");
        cell.set_bg(color(bg));
        cell.set_fg(color(bg));
    }
}

fn fill(buf: &mut Buffer, rect: Rect, bg: Rgb) {
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            put_bg(buf, x, y, bg);
        }
    }
}

/// Rellena mezclando con lo que ya hay debajo. `amount` 1.0 = opaco. Es lo que
/// deja ver el visualizador de fondo a través de los paneles.
fn fill_soft(buf: &mut Buffer, rect: Rect, target: Rgb, amount: f32, fallback: Rgb) {
    let amount = amount.clamp(0.0, 1.0);
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let under = match buf.cell((x, y)).map(|cell| cell.bg) {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                _ => fallback,
            };
            put_bg(buf, x, y, mix(under, target, amount));
        }
    }
}

fn line_at(buf: &mut Buffer, rect: Rect, spans: Vec<Span<'static>>, align: Alignment) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    Paragraph::new(Line::from(spans))
        .alignment(align)
        .render(rect, buf);
}

fn text_at(buf: &mut Buffer, rect: Rect, text: impl Into<String>, s: Style, align: Alignment) {
    line_at(buf, rect, vec![Span::styled(text.into(), s)], align);
}

fn background(buf: &mut Buffer, area: Rect, pal: &Palette, intro: f32) {
    let intro = ease_out_cubic(intro);
    for y in area.y..area.y + area.height {
        let t = (y - area.y) as f32 / area.height.max(1) as f32;
        let base = mix_oklab(pal.bg_deep, pal.bg, t * 0.8);
        let base = mix_oklab(pal.bg_deep, base, intro);
        for x in area.x..area.x + area.width {
            put_bg(buf, x, y, base);
        }
    }
}

fn draw_too_small(buf: &mut Buffer, area: Rect, pal: &Palette) {
    let text = "spote · terminal demasiado pequeña";
    let width = text.chars().count() as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height / 2;
    put(buf, x, y, text, color(pal.muted), None);
}

/// Panel con borde fino y relleno tintado. `active` enciende el borde.
/// Los paneles se vuelven ligeramente translúcidos cuando hay visualizador de
/// fondo, para que los efectos se lean "a través" de la UI.
fn soft_background(_pal: &Palette) -> bool {
    true
}

fn panel(buf: &mut Buffer, rect: Rect, pal: &Palette, active: bool, alpha: f32, title: Option<&str>, pulse: f32) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let fill_color = mix(pal.bg, pal.surface, alpha.clamp(0.0, 1.0));
    // alpha 0 = panel transparente: se usa sobre el arte para no taparlo.
    let opaque = alpha > 0.001;
    let bg = if opaque { Some(color(fill_color)) } else { None };
    if opaque {
        fill_soft(buf, rect, fill_color, if soft_background(pal) { 0.88 } else { 1.0 }, pal.bg);
    }
    let border = if active {
        // El borde del panel activo late con el bombo.
        mix_oklab(
            mix_oklab(pal.border, pal.primary, 0.55),
            pal.text,
            (pulse * 0.35).clamp(0.0, 0.35),
        )
    } else if alpha >= 0.6 {
        mix_oklab(pal.border, pal.primary, pulse * 0.12)
    } else {
        mix_oklab(pal.border_dim, pal.primary, pulse * 0.10)
    };
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width - 1, rect.y + rect.height - 1);
    for x in x0 + 1..x1 {
        put(buf, x, y0, "─", color(border), bg);
        put(buf, x, y1, "─", color(border), bg);
    }
    for y in y0 + 1..y1 {
        put(buf, x0, y, "│", color(border), bg);
        put(buf, x1, y, "│", color(border), bg);
    }
    for (x, y, glyph) in [
        (x0, y0, "╭"),
        (x1, y0, "╮"),
        (x0, y1, "╰"),
        (x1, y1, "╯"),
    ] {
        put(buf, x, y, glyph, color(border), bg);
    }
    if let Some(title) = title {
        if rect.width > 8 {
            let max = rect.width.saturating_sub(6) as usize;
            let mut label = title.to_uppercase();
            truncate_chars(&mut label, max);
            let fg = if active { pal.text } else { pal.muted };
            if opaque {
                put(buf, x0 + 2, y0, " ", color(fill_color), bg);
            }
            put(buf, x0 + 3, y0, &label, color(fg), bg);
        }
    }
}

fn chip(buf: &mut Buffer, rect: Rect, pal: &Palette, active: bool, hover: bool) {
    if rect.width < 2 {
        return;
    }
    let bg = if active {
        pal.primary
    } else if hover {
        pal.surface_alt
    } else {
        pal.surface
    };
    let fg = if active { pal.bg_deep } else { pal.text };
    fill(buf, rect, bg);
    let s = Style::new()
        .fg(color(fg))
        .bg(color(bg))
        .add_modifier(if active { Modifier::BOLD } else { Modifier::empty() });
    let _ = s;
}

fn keycap(buf: &mut Buffer, rect: Rect, pal: &Palette, label: &str) {
    fill(buf, rect, pal.surface_alt);
    text_at(
        buf,
        rect,
        label.to_string(),
        Style::new().fg(color(pal.text)).bg(color(pal.surface_alt)),
        Alignment::Center,
    );
}

// ---------------------------------------------------------------------------
// Barra superior
// ---------------------------------------------------------------------------

fn draw_top(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
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

fn clock_now() -> String {
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

fn local_offset_secs() -> i64 {
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

fn draw_nav(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
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
            format!("{ready}/{tracks} listas"),
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
}

// ---------------------------------------------------------------------------
// Ahora suena
// ---------------------------------------------------------------------------

fn draw_now(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    use crate::app::{CenterFocus, Fullscreen};

    if app.fullscreen == Fullscreen::Art {
        let h = layout.full.height.saturating_sub(4);
        let w = (h * 2).min(layout.full.width.saturating_sub(4));
        let art_rect = Rect::new(
            layout.full.x + (layout.full.width.saturating_sub(w)) / 2,
            layout.full.y + (layout.full.height.saturating_sub(h)) / 2,
            w,
            h,
        );
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
        app.hit.add(layout.full, Action::ArtFullscreen);
        return;
    }

    if app.zen {
        // Zen: solo visualización. La canción sigue sonando, pero el centro
        // queda limpio y la portada/letra no se pintan.
        if let Some(track) = &app.now.track {
            let label = format!("{}  ·  {}", track.title, track.artist);
            let rect = Rect::new(
                layout.center.x + 2,
                layout.center.y + 1,
                layout.center.width.saturating_sub(4),
                1,
            );
            text_at(
                buf,
                rect,
                format!("{label}   ·   i para salir de zen"),
                Style::new().fg(color(pal.dim)),
                Alignment::Right,
            );
        }
        return;
    }

    // Foco de portada: la portada ocupa TODO el centro, centrada de verdad.
    if app.center_focus == CenterFocus::Art {
        let area = layout.center;
        let inner = inset(area, 2, 1);
        let h = inner.height.saturating_sub(2);
        let w = (h * 2).min(inner.width);
        let art_rect = centered(inner, w, h);
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
        if let Some(track) = app.now.track.clone() {
            let caption = Rect::new(art_rect.x, art_rect.y + art_rect.height + 1, art_rect.width, 1);
            text_at(
                buf,
                caption,
                format!("{}  ·  {}", track.title, track.artist),
                Style::new().fg(color(pal.muted)),
                Alignment::Center,
            );
            app.hit.add(art_rect, Action::PlayPause);
        }
        text_at(
            buf,
            Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), 1),
            "c / esc volver  ·  ⇧F pantalla completa  ·  i zen",
            Style::new().fg(color(pal.dim)),
            Alignment::Right,
        );
        app.hit.add(area, Action::CenterArt);
        return;
    }

    let lyrics_focus = app.center_focus == CenterFocus::Lyrics;
    let art_rect = layout.art;
    let has_track = app.now.track.is_some();
    if !lyrics_focus && has_track {
        draw_art(app, buf, art_rect, pal, 1.0);
        frame_art(buf, art_rect, pal);
    }

    // Con la letra enfocada ocupa TODO el centro; si no, la columna de texto.
    let source = if lyrics_focus { layout.center } else { layout.content };
    let rect = Rect::new(
        source.x,
        source.y + 1,
        source.width,
        source.height.saturating_sub(1),
    );
    panel(
        buf,
        rect,
        pal,
        app.focus == crate::app::Pane::Center,
        0.34,
        None,
        app.audio.kick_pulse,
    );
    app.hit.add(rect, Action::Focus(crate::app::Pane::Center));
    if !lyrics_focus && has_track {
        app.hit.add(art_rect, Action::CenterArt);
    }

    let Some(track) = app.now.track.clone() else {
        draw_empty_now(app, buf, rect, pal);
        return;
    };

    let inner = Rect::new(
        rect.x + 3,
        rect.y + 1,
        rect.width.saturating_sub(6),
        rect.height.saturating_sub(2),
    );
    // El panel de texto ya está a la izquierda de la portada: no hay que
    // recortarlo (el recorte antiguo dejaba la cabecera a 16 columnas).
    let inner = Rect::new(inner.x, inner.y, inner.width, inner.height);

    // Cabecera compacta: badges arriba a la derecha, título y artista en una
    // sola línea. Antes cada cosa gastaba una fila y se comía la letra.
    let lines = app
        .lyrics
        .as_ref()
        .map(|doc| doc.lines.len() as u16)
        .unwrap_or(6)
        .min(16);
    let block = 3 + lines * 2 + 2;
    let offset = if app.vcenter {
        inner.height.saturating_sub(block) / 2
    } else {
        0
    };
    let mut y = inner.y + offset;

    text_at(
        buf,
        Rect::new(inner.x, y, inner.width, 1),
        if lyrics_focus { "LETRA" } else { "AHORA SUENA" },
        Style::new()
            .fg(color(pal.primary))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    // Badges + corazón alineados a la derecha de la primera fila.
    let favorite = app
        .lib
        .track_by_key(&track.key)
        .ok()
        .flatten()
        .map(|row| row.favorite)
        .unwrap_or(false);
    let mut items: Vec<(String, Rgb, Option<Action>)> = badges(app, pal)
        .into_iter()
        .map(|(label, fg)| (label, fg, None))
        .collect();
    items.push((
        if favorite { "♥".into() } else { "♡".into() },
        if favorite { pal.error } else { pal.dim },
        Some(Action::Favorite),
    ));
    let mut badge_x = rect.x + rect.width.saturating_sub(3);
    for (label, fg, action) in items.into_iter().rev() {
        let width = label.chars().count() as u16 + 2;
        if badge_x < inner.x + 16 || badge_x < width + inner.x {
            break;
        }
        badge_x -= width;
        let badge_rect = Rect::new(badge_x, y, width, 1);
        fill(buf, badge_rect, mix(pal.bg, pal.surface_alt, 0.7));
        text_at(
            buf,
            badge_rect,
            label,
            Style::new().fg(color(fg)),
            Alignment::Center,
        );
        if let Some(action) = action {
            app.hit.add(badge_rect, action);
        }
        badge_x = badge_x.saturating_sub(1);
    }
    // El ecualizador solo ocupa el hueco que dejan los badges.
    let eq_x = inner.x + 14;
    let eq_limit = badge_x.saturating_sub(1);
    for i in 0..10u16 {
        if eq_x + i >= eq_limit {
            break;
        }
        let value = app.audio.band(i as f32 / 10.0 * 48.0);
        let glyph = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][(value * 7.9) as usize];
        put(
            buf,
            eq_x + i,
            y,
            glyph,
            color(pal.ramp_at(0.5 + value * 0.5)),
            None,
        );
    }
    y += 1;

    // Título + artista/álbum en la misma línea (el título manda; lo demás se
    // recorta si no cabe).
    let title = track.title.clone();
    let meta = match &track.album {
        Some(album) if !album.is_empty() => format!("{}  ·  {}", track.artist, album),
        _ => track.artist.clone(),
    };
    let title_width = title.chars().count() as u16;
    let mut spans = vec![Span::styled(
        title,
        Style::new()
            .fg(color(mix_oklab(pal.text, pal.primary, 0.22)))
            .add_modifier(Modifier::BOLD),
    )];
    let room = inner.width.saturating_sub(title_width + 3);
    if room > 10 {
        let mut meta = meta;
        truncate_chars(&mut meta, room as usize);
        spans.push(Span::styled(
            format!("  ·  {meta}"),
            Style::new().fg(color(pal.muted)),
        ));
    }
    line_at(buf, Rect::new(inner.x, y, inner.width, 1), spans, Alignment::Left);
    y += 1;

    let rule = inner.width;
    for i in 0..rule {
        let t = i as f32 / rule.max(1) as f32;
        let rgb = mix_oklab(pal.primary, pal.border_dim, t.powf(0.6));
        put(buf, inner.x + i, y, "─", color(rgb), None);
    }
    y += 1;

    let available = (inner.y + inner.height).saturating_sub(y);
    let lyrics_h = if app.vcenter {
        available.min(lines * 2 + 2)
    } else {
        available
    };
    let lyrics_rect = Rect::new(inner.x, y, inner.width, lyrics_h);
    app.hit.add(lyrics_rect, Action::CenterLyrics);
    draw_lyrics(app, buf, lyrics_rect, pal);
}

/// Placeholder cuando no hay portada: degradado del tema + inicial del artista.
fn draw_art_placeholder(app: &App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    for y in 0..rect.height {
        let t = y as f32 / rect.height.max(1) as f32;
        for x in 0..rect.width {
            let nx = x as f32 / rect.width.max(1) as f32;
            let value = (0.16 + t * 0.22 + (nx * 2.0 + t).sin().abs() * 0.06).min(0.55);
            put_bg(buf, rect.x + x, rect.y + y, pal.ramp_at(value));
        }
    }
    let initial = app
        .now
        .track
        .as_ref()
        .and_then(|track| track.artist.chars().next())
        .unwrap_or('♪');
    let cy = rect.y + rect.height / 2;
    let cx = rect.x + rect.width / 2;
    let mut glyph = [0u8; 4];
    let symbol: &str = initial.to_uppercase().next().unwrap_or(initial).encode_utf8(&mut glyph);
    put(
        buf,
        cx.saturating_sub(1),
        cy,
        symbol,
        color(pal.text),
        Some(color(pal.bg_deep)),
    );
}

/// Marco fino alrededor de la portada: la separa del fondo sin recortarla.
fn frame_art(buf: &mut Buffer, rect: Rect, pal: &Palette) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width - 1, rect.y + rect.height - 1);
    for x in x0..=x1 {
        put(buf, x, y0, "─", color(pal.border), None);
        put(buf, x, y1, "─", color(pal.border), None);
    }
    for y in y0..=y1 {
        put(buf, x0, y, "│", color(pal.border), None);
        put(buf, x1, y, "│", color(pal.border), None);
    }
    for (x, y, glyph) in [
        (x0, y0, "╭"),
        (x1, y0, "╮"),
        (x0, y1, "╰"),
        (x1, y1, "╯"),
    ] {
        put(buf, x, y, glyph, color(pal.border), None);
    }
}

fn badges(app: &App, pal: &Palette) -> Vec<(String, Rgb)> {
    let mut out = Vec::new();
    if app.room.is_some() {
        out.push(("sala".into(), pal.primary));
    }
    if app.follow_spotify {
        out.push(("mpris".into(), pal.secondary));
    } else if let Some(provider) = app
        .now
        .track
        .as_ref()
        .and_then(|track| track.source_hint.as_ref())
        .map(|hint| hint.provider.clone())
    {
        out.push((provider, pal.secondary));
    } else {
        out.push(("local".into(), pal.secondary));
    }
    if let Some(doc) = &app.lyrics {
        if doc.synced {
            out.push(("sincronizada".into(), pal.primary));
        } else if !doc.is_empty() {
            out.push(("aprox".into(), pal.warn));
        }
    }
    if app.lyrics_offset_ms != 0 {
        out.push((format!("offset {:+}ms", app.lyrics_offset_ms), pal.warn));
    }
    if app.now.loading || app.pending_play.is_some() {
        out.push(("descargando".into(), pal.warn));
    }
    if app.now.playing {
        out.push(("●".into(), pal.ok));
    } else if app.now.track.is_some() {
        out.push(("⏸".into(), pal.dim));
    }
    out
}

fn draw_empty_now(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    let lines = [
        "nada sonando todavía",
        "",
        "enter  ›  reproduce lo seleccionado",
        "/      ›  busca en youtube y descarga",
        "t      ›  sigue lo que suena en spotify",
    ];
    let mut y = rect.y + rect.height / 2 - 2;
    for (i, line) in lines.iter().enumerate() {
        text_at(
            buf,
            Rect::new(rect.x + 3, y, rect.width.saturating_sub(6), 1),
            *line,
            Style::new().fg(color(if i == 0 { pal.muted } else { pal.dim })),
            Alignment::Center,
        );
        y += 1;
    }
    let _ = app;
}

fn draw_art(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette, strength: f32) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let mode = app.art_mode;
    if mode == ArtMode::Off || mode == ArtMode::Gradient || app.art.is_none() {
        draw_art_placeholder(app, buf, rect, pal);
        return;
    }
    let Some(art) = app.art.as_ref() else {
        draw_art_placeholder(app, buf, rect, pal);
        return;
    };
    let intro = ease_out_cubic(app.intro);
    // La portada respira con el bombo: integra el audio en toda la UI.
    let pulse = 1.0 + app.audio.kick * 0.06 + app.audio.bass * 0.035;
    let gain = if strength > 0.95 { 1.28 * pulse } else { 1.0 };
    let blend = if app.art_fade_active {
        app.art_fade
    } else {
        1.0
    };
    let previous = if app.art_fade_active {
        app.art_prev.as_ref()
    } else {
        None
    };
    // Dither y braille en grande se leen como ruido: se muestrean a 2x2 celdas
    // para que el patrón sea grueso y se vea como pixel art, no como estática.
    let chunky = matches!(mode, ArtMode::Dither | ArtMode::Braille) && rect.width >= 60;
    let factor: u32 = if chunky { 2 } else { 1 };
    for y in 0..rect.height {
        for x in 0..rect.width {
            let sx = ((x as u32 / factor) * art.w as u32 / (rect.width / factor as u16).max(1) as u32) as u16;
            let sy = ((y as u32 / factor) * art.h as u32 / (rect.height / factor as u16).max(1) as u32) as u16;
            let Some(cell) = art.cell(sx.min(art.w.saturating_sub(1)), sy.min(art.h.saturating_sub(1))) else {
                continue;
            };
            let mut ch = cell.ch;
            let mut fg = cell.fg;
            let mut bg = cell.bg;
            if let Some(previous) = previous {
                let psx = ((x as u32 / factor) * previous.w as u32
                    / (rect.width / factor as u16).max(1) as u32) as u16;
                let psy = ((y as u32 / factor) * previous.h as u32
                    / (rect.height / factor as u16).max(1) as u32) as u16;
                if let Some(old) = previous
                    .cell(psx.min(previous.w.saturating_sub(1)), psy.min(previous.h.saturating_sub(1)))
                {
                    // Disolución ordenada: el umbral de Bayer hace que el cambio
                    // se vea como una transición, no como un parpadeo.
                    const BAYER: [[f32; 4]; 4] = [
                        [0.0, 8.0, 2.0, 10.0],
                        [12.0, 4.0, 14.0, 6.0],
                        [3.0, 11.0, 1.0, 9.0],
                        [15.0, 7.0, 13.0, 5.0],
                    ];
                    let threshold = (BAYER[(y % 4) as usize][(x % 4) as usize] + 0.5) / 16.0;
                    if blend < threshold {
                        ch = old.ch;
                        fg = mix_oklab(old.fg, cell.fg, blend * 0.6);
                        bg = old.bg.or(cell.bg).map(|color| {
                            let other = old.bg.unwrap_or(color);
                            mix_oklab(other, color, blend)
                        });
                    }
                }
            }
            let fg = match mode {
                ArtMode::Half => fg,
                _ => pal.ramp_at(luma(fg)),
            };
            let bg = bg.map(|bg| match mode {
                ArtMode::Half => bg,
                _ => pal.ramp_at(luma(bg)),
            });
            let fg = scale(mix_oklab(pal.bg_deep, fg, intro * strength), gain);
            let ch = if ch == '\0' { ' ' } else { ch };
            let mut glyph = [0u8; 4];
            let symbol: &str = ch.encode_utf8(&mut glyph);
            put(
                buf,
                rect.x + x,
                rect.y + y,
                symbol,
                color(fg),
                bg.map(|bg| color(scale(mix_oklab(pal.bg_deep, bg, intro * strength), gain))),
            );
        }
    }
}

/// Marco fino alrededor de la portada: la separa del fondo sin recortarla.
fn draw_lyrics(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    if rect.height < 3 || rect.width < 10 {
        return;
    }
    let Some(doc) = &app.lyrics else {
        let message = if app.lyrics_status.is_empty() {
            "sin letra".to_string()
        } else {
            app.lyrics_status.clone()
        };
        text_at(
            buf,
            Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1),
            message,
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    };
    if doc.is_empty() {
        text_at(
            buf,
            Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1),
            "♪",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }
    let position = app.now.position_ms;
    let active = doc.index_at(position);
    let scroll = app.lyrics_scroll.value;
    let center = rect.y + rect.height / 2;
    let half = (rect.height as f32 / 2.0).max(1.0);

    for (index, line) in doc.lines.iter().enumerate() {
        let offset = index as f32 - scroll;
        let distance = offset.abs();
        if distance > half + 1.5 {
            continue;
        }
        let y = center as i32 + offset.round() as i32;
        if y < rect.y as i32 || y >= (rect.y + rect.height) as i32 {
            continue;
        }
        let fade = (1.0 - (distance / (half + 0.8)).powf(1.6)).clamp(0.0, 1.0);
        let is_active = Some(index) == active;
        let base = if is_active {
            mix_oklab(
                mix_oklab(pal.text, pal.primary, 0.18),
                pal.secondary,
                (app.audio.vocal_pulse * 0.35).clamp(0.0, 0.35),
            )
        } else {
            mix_oklab(pal.bg, pal.muted, fade)
        };
        let style = Style::new()
            .fg(color(base))
            .add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() });
        let prefix = if is_active { "▸ " } else { "  " };
        let suffix = if is_active { " ◂" } else { "" };
        let text = format!("{prefix}{}{suffix}", line.text);
        let line_rect = Rect::new(rect.x, y as u16, rect.width, 1);
        if is_active {
            // Banda de foco: la línea que se canta respira sobre el fondo.
            fill(
                buf,
                line_rect,
                mix(pal.bg, pal.surface_alt, 0.9 + app.audio.vocal_pulse * 0.1),
            );
            put(buf, line_rect.x, y as u16, "▎", color(pal.primary), None);
            // Karaoke: la parte cantada va en el acento, el resto en el texto.
            let progress = doc
                .index_at(position)
                .map(|_| {
                    let span = line.end_ms.saturating_sub(line.start_ms).max(1);
                    (position.saturating_sub(line.start_ms) as f32 / span as f32).clamp(0.0, 1.0)
                })
                .unwrap_or(0.0);
            karaoke_line(buf, line_rect, &text, progress, pal, prefix.chars().count());
        } else {
            text_at(buf, line_rect, text, style, Alignment::Center);
        }
    }

    // Progreso fino de la canción bajo la letra activa.
    if let Some((_, progress)) = doc.active(position) {
        let bar_width = (rect.width as f32 * 0.22) as u16;
        if bar_width > 4 && rect.height > 2 {
            let x = rect.x + (rect.width - bar_width) / 2;
            let y = rect.y + rect.height - 1;
            let filled = (bar_width as f32 * progress) as u16;
            for i in 0..bar_width {
                let ch = if i < filled { "━" } else { "─" };
                put(
                    buf,
                    x + i,
                    y,
                    ch,
                    color(if i < filled { pal.primary } else { pal.border_dim }),
                    None,
                );
            }
        }
    }
}

fn karaoke_line(
    buf: &mut Buffer,
    rect: Rect,
    text: &str,
    progress: f32,
    pal: &Palette,
    prefix_len: usize,
) {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len().saturating_sub(prefix_len * 2).max(1);
    let sung = progress * total as f32;
    let width = chars.len() as u16;
    if width > rect.width {
        // Si no cabe, se corta el final con puntos suspensivos: perder el
        // principio de la frase es peor que perder el final.
        let keep = rect.width.saturating_sub(1) as usize;
        let mut text: String = chars.into_iter().take(keep).collect();
        text.push('…');
        text_at(
            buf,
            rect,
            text,
            Style::new().fg(color(pal.text)).add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
        return;
    }
    let x = rect.x + (rect.width.saturating_sub(width)) / 2;
    for (i, ch) in chars.iter().enumerate() {
        let index = i.saturating_sub(prefix_len);
        let local = if i < prefix_len || i >= chars.len().saturating_sub(prefix_len) {
            0.0
        } else {
            (sung - index as f32).clamp(0.0, 1.0)
        };
        let fg = mix_oklab(pal.muted, mix_oklab(pal.text, pal.primary, 0.35), local);
        let mut glyph = [0u8; 4];
        let symbol: &str = ch.encode_utf8(&mut glyph);
        put(buf, x + i as u16, rect.y, symbol, color(fg), None);
    }
}

// ---------------------------------------------------------------------------
// Listas (librería / búsqueda)
// ---------------------------------------------------------------------------

fn draw_library(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.content;
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    let title = if app.lib_filter == crate::app::LibFilter::All {
        "librería".to_string()
    } else {
        format!("librería · {}", app.lib_filter.label())
    };
    panel(
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
    if app.view.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
            "librería vacía · / para buscar · spote ingest <dir> para importar",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    }
    if app.list_cards {
        draw_cards(app, buf, inner, pal);
        return;
    }
    let rows = inner.height as usize;
    let start = (app.list_scroll.value.max(0.0) as usize)
        .saturating_sub(rows / 2)
        .min(app.view.len().saturating_sub(rows.min(app.view.len())));
    let shown = app.view.len().saturating_sub(start).min(rows) as u16;
    let y_offset = if app.vcenter {
        inner.height.saturating_sub(shown) / 2
    } else {
        0
    };
    for (offset, row) in app.view.iter().skip(start).take(rows).enumerate() {
        let index = start + offset;
        let y = inner.y + y_offset + offset as u16;
        let selected = index == app.selected;
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        let hover = app.hover == Some(Action::SelectRow(index));
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.20));
            put(buf, row_rect.x, y, "▍", color(pal.primary), None);
        } else if hover {
            fill(buf, row_rect, pal.surface_alt);
        }
        let ready = row.playable();
        let duration = format_ms(row.track.duration_ms);
        let duration_width = duration.chars().count() as u16;
        let playing = app
            .now
            .track
            .as_ref()
            .is_some_and(|track| track.key == row.track.key);
        let marker = if playing {
            "▶"
        } else if ready {
            "·"
        } else {
            "↓"
        };
        let marker_color = if playing {
            pal.primary
        } else if ready {
            pal.dim
        } else {
            pal.warn
        };
        put(buf, row_rect.x + 2, y, marker, color(marker_color), None);
        // Corazón siempre visible si es favorita, con su propio hueco.
        let heart_x = row_rect.x + row_rect.width - duration_width - 2;
        if row.favorite {
            put(buf, heart_x, y, "♥", color(pal.error), None);
        }
        let title_width = row_rect.width.saturating_sub(6 + duration_width + 3) as usize;
        let style = Style::new()
            .fg(color(if selected { pal.text } else { pal.muted }))
            .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() });
        let mut title = row.track.title.clone();
        truncate_chars(&mut title, title_width);
        put(buf, row_rect.x + 4, y, &title, style.fg.unwrap_or(color(pal.text)), None);
        let title_len = title.chars().count() as u16;
        let artist_x = row_rect.x + 4 + title_len + 2;
        let max_artist =
            (heart_x).saturating_sub(artist_x + 1);
        if max_artist > 4 {
            let mut artist = row.track.artist.clone();
            truncate_chars(&mut artist, max_artist as usize);
            put(buf, artist_x, y, &artist, color(pal.dim), None);
        }
        put(
            buf,
            row_rect.x + row_rect.width - duration_width,
            y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row_rect, Action::SelectRow(index));
    }
}

/// Lista en tarjetas de 3 filas: miniatura, título, artista · álbum, badges.
fn draw_cards(app: &mut App, buf: &mut Buffer, inner: Rect, pal: &Palette) {
    let card_h = 3u16;
    let per_page = (inner.height / card_h).max(1) as usize;
    let start = app.selected.saturating_sub(per_page / 2);
    let start = start.min(app.view.len().saturating_sub(per_page.min(app.view.len())));
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
        let mut meta = format!("{}  ·  {}", format_ms(row.track.duration_ms), status_label(row));
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
        y += card_h;
    }
}

fn status_label(row: &spote_core::library::TrackRow) -> &'static str {
    use spote_core::library::SourceStatus;
    match row.status {
        SourceStatus::Ready => "local",
        SourceStatus::Pending => "pendiente",
        SourceStatus::Downloading => "descargando",
        SourceStatus::Failed => "error",
        SourceStatus::Missing => "perdida",
    }
}

fn draw_thumb(app: &mut App, buf: &mut Buffer, rect: Rect, track: &spote_protocol::TrackRef, pal: &Palette) {
    if rect.width < 4 || rect.height < 2 {
        return;
    }
    if let Some(art) = app.thumbs.get(&track.key) {
        for y in 0..rect.height {
            for x in 0..rect.width {
                let sx = (x as u32 * art.w as u32 / rect.width.max(1) as u32) as u16;
                let sy = (y as u32 * art.h as u32 / rect.height.max(1) as u32) as u16;
                let Some(cell) = art.cell(sx, sy) else { continue };
                let fg = pal.ramp_at(luma(cell.fg));
                let bg = cell
                    .bg
                    .map(|bg| pal.ramp_at(luma(bg)))
                    .unwrap_or(pal.bg_deep);
                let mut glyph = [0u8; 4];
                let symbol: &str = cell.ch.encode_utf8(&mut glyph);
                put(
                    buf,
                    rect.x + x,
                    rect.y + y,
                    symbol,
                    color(fg),
                    Some(color(bg)),
                );
            }
        }
    } else {
        // Placeholder de color estable por artista mientras llega el arte.
        let seed = spote_core::util::short_hash(&track.artist);
        let base = pal.ramp_at(0.45 + (u32::from_str_radix(&seed[..2], 16).unwrap_or(0) as f32 / 255.0) * 0.3);
        for y in 0..rect.height {
            for x in 0..rect.width {
                put_bg(buf, rect.x + x, rect.y + y, base);
            }
        }
        let ch = track
            .artist
            .chars()
            .next()
            .unwrap_or('♪')
            .to_uppercase()
            .next()
            .unwrap_or('♪');
        let mut glyph = [0u8; 4];
        let symbol: &str = ch.encode_utf8(&mut glyph);
        put(
            buf,
            rect.x + rect.width / 2 - 1,
            rect.y + rect.height / 2,
            symbol,
            color(pal.text),
            None,
        );
        // Pide la miniatura una sola vez por pista.
        if !app.thumb_requests.contains(&track.key) {
            app.thumb_requests.insert(track.key.clone());
            app.request_thumb(track);
        }
    }
}

fn draw_search(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.content;
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    panel(buf, rect, pal, true, 0.78, Some("buscar"), app.audio.kick_pulse);
    let inner = Rect::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(4), rect.height.saturating_sub(3));

    // Campo de búsqueda.
    let input_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    fill(buf, input_rect, pal.bg_deep);
    put(buf, input_rect.x + 1, input_rect.y, "/", color(pal.primary), None);
    let empty = app.search.query.is_empty();
    let query = if empty {
        "escribe y pulsa enter…".to_string()
    } else {
        app.search.query.clone()
    };
    let fg = if empty { pal.dim } else { pal.text };
    put(buf, input_rect.x + 3, input_rect.y, &query, color(fg), None);
    // Cursor solo con texto: con el placeholder se ve más limpio.
    let cursor_x = input_rect.x + 3 + app.search.query.chars().count() as u16;
    if !empty && cursor_x < input_rect.x + input_rect.width {
        put(buf, cursor_x, input_rect.y, "▌", color(pal.primary), None);
    }

    let mut y = inner.y + 2;
    if app.search.loading {
        text_at(
            buf,
            Rect::new(inner.x, y, inner.width, 1),
            "buscando en el catálogo…",
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

    let rows = (inner.y + inner.height).saturating_sub(y) as usize;
    for (index, item) in app.search.results.iter().enumerate().take(rows) {
        let row_y = y + index as u16;
        let selected = index == app.search.selected;
        let row_rect = Rect::new(inner.x, row_y, inner.width, 1);
        if selected {
            fill(buf, row_rect, mix(pal.surface, pal.primary, 0.22));
            put(buf, row_rect.x, row_y, "▍", color(pal.primary), None);
        }
        let duration = format_ms(item.duration_ms);
        let duration_width = duration.chars().count() as u16;
        let topic = item.channel.ends_with(" - Topic");
        let marker = if topic { "♪" } else { "▷" };
        put(
            buf,
            row_rect.x + 2,
            row_y,
            marker,
            color(if selected { pal.secondary } else { pal.dim }),
            None,
        );
        // Título y artista/álbum en la misma fila: con solo el título no se
        // sabía qué versión era.
        let title_width = row_rect.width.saturating_sub(6 + duration_width) as usize;
        let mut title = item.title.clone();
        let title_chars = title.chars().count();
        truncate_chars(&mut title, title_width);
        put(
            buf,
            row_rect.x + 4,
            row_y,
            &title,
            color(if selected { pal.text } else { pal.muted }),
            None,
        );
        let after = row_rect.x + 4 + title_chars.min(title_width) as u16 + 2;
        let meta = match &item.album {
            Some(album) if !album.is_empty() => format!("{}  ·  {}", item.artist, album),
            _ => item.artist.clone(),
        };
        let meta_room = (row_rect.x + row_rect.width - duration_width - 2)
            .saturating_sub(after);
        if meta_room > 6 && title_width > title_chars {
            let mut meta = meta;
            truncate_chars(&mut meta, meta_room as usize);
            put(buf, after, row_y, &meta, color(pal.dim), None);
        }
        put(
            buf,
            row_rect.x + row_rect.width - duration_width,
            row_y,
            &duration,
            color(pal.dim),
            None,
        );
        app.hit.add(row_rect, Action::SearchPlay(index));
    }
}

fn draw_settings(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.content;
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.85);
        frame_art(buf, layout.art, pal);
    }
    panel(
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
    let block = rows.len() as u16 * 2 + 2;
    let offset = if app.vcenter {
        inner.height.saturating_sub(block) / 2
    } else {
        0
    };
    let mut y = inner.y + offset;
    for (index, (label, value)) in rows.iter().enumerate() {
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
        if index == 1 || index == 2 {
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

fn draw_room(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.content;
    if app.now.track.is_some() {
        draw_art(app, buf, layout.art, pal, 0.82);
        frame_art(buf, layout.art, pal);
    }
    panel(
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
}

// ---------------------------------------------------------------------------
// Panel lateral
// ---------------------------------------------------------------------------

fn draw_side(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    // Cola.
    panel(
        buf,
        layout.queue,
        pal,
        app.focus == crate::app::Pane::Side,
        0.55,
        Some("cola"),
        app.audio.kick_pulse,
    );
    let inner = Rect::new(
        layout.queue.x + 2,
        layout.queue.y + 2,
        layout.queue.width.saturating_sub(4),
        layout.queue.height.saturating_sub(3),
    );
    let queue_len = if app.room_queue.is_empty() {
        app.queue.len()
    } else {
        app.room_queue.len()
    };
    let hint_row = inner.y + inner.height.saturating_sub(1);
    if queue_len == 0 {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1),
            "vacía · a para encolar",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    } else if app.list_cards {
        let card_h = 3u16;
        let visible = (inner.height / card_h).max(1) as usize;
        let start = app.queue_selected.saturating_sub(visible / 2);
        let start = start.min(queue_len.saturating_sub(visible.min(queue_len)));
        let shown = (queue_len - start).min(visible);
        let y_offset = if app.vcenter {
            inner.height.saturating_sub(shown as u16 * card_h) / 2
        } else {
            0
        };
        for offset in 0..visible {
            let index = start + offset;
            if index >= queue_len || inner.y + (offset as u16 + 1) * card_h > inner.y + inner.height {
                break;
            }
            let y = inner.y + y_offset + offset as u16 * card_h;
            let card = Rect::new(inner.x, y, inner.width, card_h);
            fill_soft(buf, card, mix(pal.bg, pal.surface, 0.5), 1.0, pal.bg);
            let selected = index == app.queue_selected;
            if selected {
                fill(buf, card, mix(pal.surface, pal.primary, 0.18));
            }
            let (title, artist, path_pending) = if app.room_queue.is_empty() {
                let entry = &app.queue[index];
                (
                    entry.track.title.clone(),
                    entry.track.artist.clone(),
                    entry.path.is_none(),
                )
            } else {
                let item = &app.room_queue[index];
                (item.track.title.clone(), item.track.artist.clone(), false)
            };
            let thumb = Rect::new(card.x + 1, y, 5, card_h);
            let track_ref = if app.room_queue.is_empty() {
                app.queue[index].track.clone()
            } else {
                app.room_queue[index].track.clone()
            };
            draw_thumb(app, buf, thumb, &track_ref, pal);
            let text_x = thumb.x + thumb.width + 1;
            let text_w = (card.x + card.width).saturating_sub(text_x + 3);
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
            let mut artist = artist;
            if path_pending {
                artist.push_str("  ·  ↓");
            }
            truncate_chars(&mut artist, text_w as usize);
            put(buf, text_x, y + 1, &artist, color(pal.dim), None);
            put(
                buf,
                text_x,
                y + 2,
                &format!("{:>2}  ·  {}", index + 1, if path_pending { "pendiente" } else { "lista" }),
                color(pal.dim),
                None,
            );
            let remove_rect = Rect::new(card.x + card.width - 2, y, 2, 1);
            put(buf, remove_rect.x, y, "×", color(pal.error), None);
            // El último registro gana: la tarjeta primero y el × encima, para
            // que un solo clic en la cruz quite la pista.
            app.hit.add(card, Action::QueueRow(index));
            app.hit.add(remove_rect, Action::QueueRemove(index));
        }
    } else {
        let visible = inner.height.saturating_sub(1) as usize;
        let start = app.queue_selected.saturating_sub(visible / 2);
        let start = start.min(queue_len.saturating_sub(visible.min(queue_len)));
        let shown = (queue_len - start).min(visible);
        let y_offset = if app.vcenter {
            inner.height.saturating_sub(1).saturating_sub(shown as u16) / 2
        } else {
            0
        };
        for offset in 0..visible {
            let index = start + offset;
            if index >= queue_len || inner.y + offset as u16 >= hint_row {
                break;
            }
            let y = inner.y + y_offset + offset as u16;
            let row = Rect::new(inner.x, y, inner.width, 1);
            let selected = index == app.queue_selected;
            let hover = app.hover == Some(Action::QueueRow(index));
            if selected {
                fill(buf, row, mix(pal.surface, pal.primary, 0.22));
            } else if hover {
                fill(buf, row, pal.surface_alt);
            }
            let (title, artist, pending) = if app.room_queue.is_empty() {
                let entry = &app.queue[index];
                (
                    entry.track.title.clone(),
                    entry.track.artist.clone(),
                    entry.path.is_none(),
                )
            } else {
                let item = &app.room_queue[index];
                (item.track.title.clone(), item.track.artist.clone(), false)
            };
            let remove_rect = Rect::new(row.x + row.width.saturating_sub(2), y, 2, 1);
            if app.hover == Some(Action::QueueRemove(index)) {
                fill(buf, remove_rect, pal.error);
            }
            put(buf, remove_rect.x, y, "×", color(pal.error), None);
            app.hit.add(row, Action::QueueRow(index));
            app.hit.add(remove_rect, Action::QueueRemove(index));
            put(
                buf,
                row.x,
                y,
                &format!("{:>2}", index + 1),
                color(if selected { pal.primary } else { pal.dim }),
                None,
            );
            let room_for_artist = row.width > 34;
            let artist_w = if room_for_artist {
                (row.width / 3).min(14)
            } else {
                0
            };
            let title_w = row
                .width
                .saturating_sub(4 + remove_rect.width + artist_w + 1)
                as usize;
            let mut title = title;
            truncate_chars(&mut title, title_w);
            put(
                buf,
                row.x + 3,
                y,
                &title,
                color(if selected { pal.text } else { pal.muted }),
                None,
            );
            if room_for_artist && artist_w > 4 {
                let mut artist = artist;
                truncate_chars(&mut artist, artist_w.saturating_sub(1) as usize);
                put(
                    buf,
                    row.x + 3 + title.chars().count() as u16 + 1,
                    y,
                    &artist,
                    color(pal.dim),
                    None,
                );
            }
            if pending {
                put(
                    buf,
                    remove_rect.x.saturating_sub(2),
                    y,
                    "↓",
                    color(pal.warn),
                    None,
                );
            }
        }
        // Ayuda de la cola, en la última fila del panel.
        if hint_row > inner.y {
            let text = if app.room_queue.is_empty() {
                "x quita · arrastra para ordenar"
            } else {
                "x quita · la sala la ordena el servidor"
            };
            text_at(
                buf,
                Rect::new(inner.x, hint_row, inner.width, 1),
                text.to_string(),
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
    }

    // Info: paleta + estado.
    panel(buf, layout.info, pal, false, 0.45, Some("sesión"), app.audio.kick_pulse * 0.8);
    let inner = Rect::new(
        layout.info.x + 2,
        layout.info.y + 2,
        layout.info.width.saturating_sub(4),
        layout.info.height.saturating_sub(3),
    );
    let mut y = inner.y;
    // Muestras de la paleta: 12 bloques.
    let swatch_width = inner.width.max(1);
    for i in 0..swatch_width {
        let t = i as f32 / swatch_width.max(1) as f32;
        put_bg(buf, inner.x + i, y, pal.ramp_at(t));
    }
    y += 2;
    let mut line = |text: String, fg: Rgb, y: &mut u16| {
        if *y < layout.info.y + layout.info.height.saturating_sub(1) {
            text_at(
                buf,
                Rect::new(inner.x, *y, inner.width, 1),
                text,
                Style::new().fg(color(fg)),
                Alignment::Left,
            );
        }
        *y += 1;
    };
    if let Some(room) = &app.room {
        line(
            format!("sala · {}", app.room_label(&room.id)),
            pal.primary,
            &mut y,
        );
    } else if app.follow_spotify {
        line("siguiendo spotify".into(), pal.secondary, &mut y);
    } else if app.social.is_some() {
        line("servidor social".into(), pal.secondary, &mut y);
    } else {
        line("modo local".into(), pal.secondary, &mut y);
    }
    line(
        format!("viz · {} ({})", app.viz.preset_name(), app.viz.layers.len()),
        pal.dim,
        &mut y,
    );
    line(format!("arte · {}", app.art_mode.label()), pal.dim, &mut y);
    if app.show_bpm && app.audio.bpm > 0.0 {
        line(
            format!(
                "{:.0} bpm · ♩ {:>3}  ▽ {:>3}  ♯ {:>3}",
                app.audio.bpm,
                (app.audio.kick * 100.0) as u32,
                (app.audio.snare * 100.0) as u32,
                (app.audio.hat * 100.0) as u32
            ),
            pal.dim,
            &mut y,
        );
    }
    let tools = &app.tools;
    if !tools.missing().is_empty() {
        line(
            format!("faltan: {}", tools.missing().join(", ")),
            pal.warn,
            &mut y,
        );
    } else {
        line("mpv · ffmpeg · yt-dlp ok".into(), pal.dim, &mut y);
    }
    if !app.status.is_empty() {
        y += 1;
        line(app.status.clone(), pal.muted, &mut y);
    }
}

// ---------------------------------------------------------------------------
// Visualizador
// ---------------------------------------------------------------------------

fn draw_viz(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.viz;
    if rect.width < 4 || rect.height < 2 {
        return;
    }
    let background_on = app.viz_background && app.show_viz;
    if !background_on {
        let audio = &app.audio;
        let dt = 1.0 / app.fps.value().max(1.0);
        app.viz.render(rect.width, rect.height, audio, pal, dt);
        blit_viz(app, buf, rect, 0, 1.0, 0.0);
    } else {
        // El fondo ya renderizó el canvas a pantalla completa: la franja es
        // una cosecha de sus últimas filas, así el efecto es continuo.
        let src_y = app.viz.output.h.saturating_sub(rect.height);
        blit_viz(app, buf, rect, src_y, 1.0, 0.0);
    }
    let label = format!("{} · {}", app.viz.preset_name(), app.viz.preset_description());
    let width = label.chars().count() as u16;
    if rect.width > width + 4 {
        let x = rect.x + rect.width - width - 2;
        fill_soft(buf, Rect::new(x, rect.y, width, 1), pal.bg_deep, 0.7, pal.bg_deep);
        put(buf, x, rect.y, &label, color(pal.dim), None);
    }
    app.hit.add(rect, Action::VizNext);
}

/// Copia el canvas del visualizador a la pantalla. `src_y0` permite dibujar
/// solo una franja; `amount` controla cuánto color se suma al fondo.
fn blit_viz(app: &App, buf: &mut Buffer, dst: Rect, src_y0: u16, fg_amount: f32, bg_amount: f32) {
    let layer = &app.viz.output;
    if layer.w == 0 || layer.h == 0 {
        return;
    }
    for y in 0..dst.height {
        for x in 0..dst.width {
            let sx = x.min(layer.w.saturating_sub(1));
            let sy = (src_y0 + y).min(layer.h.saturating_sub(1));
            let Some(cell) = layer.get(sx as i32, sy as i32) else {
                continue;
            };
            if cell.alpha <= 0.01 {
                continue;
            }
            let pal_hint = (11u8, 11u8, 16u8);
            let fg = mix(pal_hint, cell.fg, cell.alpha * fg_amount);
            let bg = mix(pal_hint, cell.fg, cell.alpha * bg_amount);
            let mut glyph = [0u8; 4];
            let symbol: &str = cell.ch.encode_utf8(&mut glyph);
            put(
                buf,
                dst.x + x,
                dst.y + y,
                symbol,
                color(fg),
                Some(color(bg)),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Transporte
// ---------------------------------------------------------------------------

fn draw_transport(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = layout.transport;
    if rect.height < 2 {
        return;
    }
    fill_soft(buf, rect, pal.bg_deep, 0.82, pal.bg_deep);
    let duration = app.now.duration_ms.max(1);
    let position = app.display_position_ms().min(duration);
    let progress = (position as f32 / duration as f32).clamp(0.0, 1.0);
    let pulse = (app.pulse.eased() * 0.45 + app.audio.kick_pulse * 0.55).min(1.0);

    // Barra centrada: es el elemento principal del transporte, no un adorno
    // pegado a la izquierda.
    let bar_width = rect.width.saturating_sub(28).min(104).max(20);
    let bar_rect = centered(Rect::new(rect.x, rect.y, rect.width, 1), bar_width, 1);
    for x in 0..bar_rect.width {
        let t = x as f32 / bar_rect.width.max(1) as f32;
        let filled = t <= progress;
        let ch = if filled { "━" } else { "─" };
        let fg = if filled {
            mix_oklab(pal.primary, pal.secondary, t)
        } else {
            pal.border_dim
        };
        put(buf, bar_rect.x + x, bar_rect.y, ch, color(fg), Some(color(pal.bg_deep)));
    }
    let head = bar_rect.x + (progress * bar_rect.width as f32) as u16;
    put(
        buf,
        head.min(bar_rect.x + bar_rect.width.saturating_sub(1)),
        bar_rect.y,
        if app.now.playing { "●" } else { "○" },
        color(mix_oklab(pal.text, pal.primary, 0.4 + pulse * 0.6)),
        Some(color(pal.bg_deep)),
    );
    app.hit.add(
        Rect::new(bar_rect.x, bar_rect.y, bar_rect.width, 1),
        Action::SeekTo(position),
    );

    // Tiempos a los lados de la barra.
    let time = format!("{} / {}", format_ms(position), format_ms(app.now.duration_ms));
    let played = format_ms(position);
    if bar_rect.x > rect.x + 8 {
        text_at(
            buf,
            Rect::new(bar_rect.x - 8, bar_rect.y, 7, 1),
            played,
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
    } else {
        text_at(buf, bar_rect, time, Style::new().fg(color(pal.muted)), Alignment::Center);
    }

    // Controles centrados en la fila inferior del transporte.
    let y = rect.y + 1;
    let buttons: [(&str, Action, bool); 3] = [
        ("◀◀", Action::Prev, false),
        (if app.now.playing { PAUSE } else { PLAY }, Action::PlayPause, true),
        ("▶▶", Action::Next, false),
    ];
    let widths: u16 = buttons.iter().map(|(label, _, _)| label.chars().count() as u16 + 2).sum();
    let mut x = rect.x + (rect.width.saturating_sub(widths + 2)) / 2;
    for (label, action, primary) in buttons {
        let width = label.chars().count() as u16 + 2;
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

fn draw_hint(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
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
            "{} pistas · {} listas",
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

fn draw_toasts(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let mut y = layout.transport.y.saturating_sub(1);
    for toast in app.toasts.iter().rev().take(3) {
        let age = toast.created.elapsed().as_secs_f32();
        let slide = ease_out_cubic((age / 0.22).clamp(0.0, 1.0));
        let fade = if age > 5.4 {
            ((6.0 - age) / 0.6).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let text = toast.text.clone();
        let width = text.chars().count() as u16 + 4;
        if width + 4 > layout.center.width {
            continue;
        }
        let x = layout.full.x + layout.full.width.saturating_sub(width + 2);
        let rect = Rect::new(
            x + ((1.0 - slide) * 6.0) as u16,
            y,
            width,
            1,
        );
        let accent = match toast.kind {
            ToastKind::Info => pal.primary,
            ToastKind::Ok => pal.ok,
            ToastKind::Warn => pal.warn,
            ToastKind::Error => pal.error,
        };
        let bg = mix(pal.bg_deep, accent, 0.16 * fade);
        fill(buf, rect, bg);
        put(buf, rect.x, y, "▎", color(accent), Some(color(bg)));
        text_at(
            buf,
            Rect::new(rect.x + 2, y, rect.width.saturating_sub(2), 1),
            text,
            Style::new().fg(color(mix(pal.bg_deep, pal.text, fade))),
            Alignment::Left,
        );
        y = y.saturating_sub(1);
        if y <= layout.main.y {
            break;
        }
    }
}

fn draw_overlay(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    match app.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(app, buf, layout, pal),
        Overlay::Effects => draw_effects(app, buf, layout, pal),
    }
}

fn overlay_frame(layout: &Layout, pal: &Palette, buf: &mut Buffer, width: u16, height: u16, title: &str) -> Rect {
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

fn draw_help(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 78, 24, "ayuda · todas las teclas");
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

fn draw_effects(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
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

fn blend_label(blend: Blend) -> &'static str {
    match blend {
        Blend::Normal => "normal",
        Blend::Add => "add",
        Blend::Screen => "screen",
        Blend::Multiply => "multiply",
        Blend::Overlay => "overlay",
    }
}

fn mask_label(mask: crate::viz::Mask) -> &'static str {
    match mask {
        crate::viz::Mask::Full => "full",
        crate::viz::Mask::Radial(_) => "radial",
        crate::viz::Mask::Bottom => "gradiente",
        crate::viz::Mask::Band(_, _) => "banda",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Fullscreen;

    #[test]
    fn truncate_respects_char_boundaries() {
        let mut text = "Río · 東京".to_string();
        truncate_chars(&mut text, 4);
        assert_eq!(text, "Río ");
        let mut short = "abc".to_string();
        truncate_chars(&mut short, 10);
        assert_eq!(short, "abc");
    }

    #[test]
    fn layout_is_sane_at_many_sizes() {
        for (w, h) in [(200, 60), (130, 40), (100, 30), (80, 24), (30, 10)] {
            let area = Rect::new(0, 0, w, h);
            let layout = Layout::compute(area, true, Fullscreen::None, 6);
            assert!(layout.center.width > 0);
            assert!(layout.center.height > 0);
            assert!(layout.center.x + layout.center.width <= area.x + area.width);
            assert!(layout.main.y + layout.main.height <= area.y + area.height);
        }
    }

    #[test]
    fn layout_collapses_nav_and_side_on_narrow() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = Layout::compute(area, true, Fullscreen::None, 6);
        assert_eq!(layout.nav.width, 0);
        assert_eq!(layout.side.width, 0);
        assert_eq!(layout.center.width, 80);
    }

    #[test]
    fn fullscreen_removes_viz_strip() {
        let area = Rect::new(0, 0, 160, 50);
        let layout = Layout::compute(area, true, Fullscreen::Viz, 6);
        assert_eq!(layout.viz.height, 0);
        assert!(layout.main.height >= area.height - 4);
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        for (w, h) in [(1, 1), (10, 4), (19, 7)] {
            let layout = Layout::compute(Rect::new(0, 0, w, h), true, Fullscreen::None, 6);
            assert!(layout.center.width <= w);
        }
    }
}
