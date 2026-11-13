//! Dibujo de la interfaz. Todo se pinta sobre `Buffer` para controlar color,
//! posición y glifo celda a celda.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use spote_core::artwork::{ArtMode, CoverArt, Palette, Rgb};
use spote_core::util::{format_ms, luma, mix, mix_oklab, scale};

use crate::app::{Action, App, Overlay, Pane, Tab, ThumbMode, ToastKind};
use crate::theme::ease_out_cubic;
use crate::viz::Blend;

mod albums;
mod art;
mod layout;
mod now;
mod lists;
mod settings;
mod side;
mod chrome;
mod overlays;
pub use layout::Layout;
use albums::*;
use art::*;
use now::*;
use lists::*;
use settings::*;
use side::*;
use chrome::*;
use overlays::*;

const PLAY: &str = "⏵";
const PAUSE: &str = "⏸";

pub fn draw(app: &mut App, buf: &mut Buffer, area: Rect) {
    app.art_visible_rect = None;
    // El fondo limpia y rellena todo el buffer de una pasada: `reset` + fondo
    // era recorrer la pantalla dos veces cada frame.
    // El mismo reparto en todas las vistas: si cambia, la interfaz "salta" al
    // cambiar de pestaña.
    // La portada grande es para "ahora"; en listas y ajustes ocupa menos para
    // que quepan título, artista y duración sin comerse el panel.
    let art_ratio = if app.now.track.is_none() {
        0
    } else {
        match app.tab {
            Tab::Now => 6,
            Tab::Library | Tab::Search | Tab::Room => 3,
            Tab::Albums | Tab::Settings => 0,
        }
    };
    let layout = Layout::compute(area, &app.cfg.ui.layout, app.show_viz, app.fullscreen, art_ratio);
    app.last_main = layout.main;
    app.hit.clear();
    let pal = *app.palette.current();

    background(buf, area, &pal, app.intro);
    // Fondo ambiental: un efecto ruidoso (void/neural/glitch/aurora) a baja
    // opacidad detrás de los paneles. Los paneles son translúcidos, así que se
    // lee a través de ellos sin quitar legibilidad.
    let fx_opacity = app.cfg.ui.fx.ambient_opacity.min(40) as f32 / 100.0;
    if fx_opacity > 0.001 {
        match app.cfg.ui.fx.ambient.as_str() {
            "off" | "cover" | "lyrics" => {}
            _ => {
                let w = (area.width / 2).max(12);
                let h = (area.height / 2).max(6);
                let dt = 1.0 / app.fps.value().max(1.0);
                let audio = app.audio.clone();
                app.ambient.render(w, h, &audio, &pal, dt);
                blit_layer(
                    &app.ambient.output,
                    buf,
                    area,
                    0,
                    fx_opacity * 0.55,
                    fx_opacity * 0.28,
                    2,
                );
            }
        }
    }
    let zen = app.zen && app.show_viz;
    let viz_fullscreen = (app.viz_background || zen) && app.show_viz && layout.main.height > 2;
    if viz_fullscreen {
        let audio = &app.audio;
        // A resolución completa: en zen el efecto es el protagonista y el
        // centro no puede verse más basto que la franja inferior.
        app.viz.render(
            layout.full.width.max(20),
            layout.full.height.max(6),
            audio,
            &pal,
            1.0 / app.fps.value().max(1.0),
        );
        // En zen el visualizador es el protagonista: más opaco y sin paneles.
        if zen {
            // Solo glifos sobre el fondo oscuro: antes el `bg_amount` alto
            // lavaba la pantalla con un velo blanquecino en presets claros.
            blit_viz(app, buf, layout.full, 0, 0.95, 0.0, VIZ_SCALE);
        } else {
            blit_viz(app, buf, layout.full, 0, 0.32, 0.10, VIZ_SCALE);
        }
    }
    if layout.main.width < 20 {
        draw_too_small(buf, area, &pal);
        return;
    }

    if layout.top.height > 0 {
        draw_top(app, buf, &layout, &pal);
    }
    if !zen && layout.nav.width > 0 {
        draw_nav(app, buf, &layout, &pal);
    }
    match app.tab {
        Tab::Now => draw_now(app, buf, &layout, &pal),
        Tab::Search => draw_search(app, buf, &layout, &pal),
        Tab::Library => draw_library(app, buf, &layout, &pal),
        Tab::Albums => draw_albums(app, buf, &layout, &pal),
        Tab::Room => draw_room(app, buf, &layout, &pal),
        Tab::Settings => draw_settings(app, buf, &layout, &pal),
    }
    if !zen && layout.side.width > 0 {
        draw_side(app, buf, &layout, &pal);
    }
    if layout.viz.height > 0 {
        draw_viz(app, buf, &layout, &pal);
    }
    if layout.transport.height > 0 {
        draw_transport(app, buf, &layout, &pal);
    }
    if layout.hint.height > 0 {
        draw_hint(app, buf, &layout, &pal);
    }
    // Portada/letra ambiental: se mezcla solo el fondo de las celdas ya
    // pintadas, preservando el texto. Así se ve a través de los paneles sin
    // taparlos ni ensuciar la lectura.
    if fx_opacity > 0.001 {
        match app.cfg.ui.fx.ambient.as_str() {
            "cover" => draw_ambient_cover_tint(app, buf, area, fx_opacity),
            "lyrics" => draw_ambient_lyrics_tint(app, buf, area, fx_opacity),
            _ => {}
        }
    }
    // Las asas van al final: el borde del panel registra su foco antes y, si
    // no, el clic de redimensionar nunca llegaría.
    if let Some(split) = layout.nav_split {
        app.hit.add(split, Action::Resize(Pane::Nav));
    }
    if let Some(split) = layout.side_split {
        app.hit.add(split, Action::Resize(Pane::Side));
    }
    draw_toasts(app, buf, &layout, &pal);
    draw_overlay(app, buf, &layout, &pal);
}

// ---------------------------------------------------------------------------
// Helpers de pintura
// ---------------------------------------------------------------------------

/// Portada ambiental a pantalla completa, sin bloques ni costuras: mezcla el
/// color de la portada en el fondo de cada celda y deja intactos los glifos.
fn draw_ambient_cover_tint(app: &App, buf: &mut Buffer, area: Rect, opacity: f32) {
    let Some(art) = app.ambient_art.as_ref().or(app.art.as_ref()) else {
        return;
    };
    if art.is_empty() || area.width < 20 || area.height < 8 {
        return;
    }
    let alpha = (opacity * 0.95).clamp(0.02, 0.20);
    let area_buf = *buf.area();
    let width = area_buf.width as usize;
    let src_aspect = art.w as f32 / (art.h as f32 * 2.0).max(1.0);
    let dst_aspect = area.width as f32 / (area.height as f32 * 2.0).max(1.0);
    for y in area.y..area.y + area.height {
        if y >= area_buf.y + area_buf.height {
            break;
        }
        let start = (y - area_buf.y) as usize * width + (area.x - area_buf.x) as usize;
        for (offset, cell) in buf.content[start..start + area.width as usize]
            .iter_mut()
            .enumerate()
        {
            let u = offset as f32 / area.width.max(1) as f32;
            let v = (y - area.y) as f32 / area.height.max(1) as f32;
            let (u, v) = if src_aspect > dst_aspect {
                let span = dst_aspect / src_aspect;
                ((1.0 - span) / 2.0 + u * span, v)
            } else {
                let span = src_aspect / dst_aspect;
                (u, (1.0 - span) / 2.0 + v * span)
            };
            let sx = (u * art.w as f32) as u16;
            let sy = (v * art.h as f32) as u16;
            let Some(src) = art.cell(
                sx.min(art.w.saturating_sub(1)),
                sy.min(art.h.saturating_sub(1)),
            ) else {
                continue;
            };
            let under = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => continue,
            };
            let mixed = mix(under, src.fg, alpha);
            cell.set_bg(color(mixed));
            if cell.symbol() == " " {
                cell.set_fg(color(mixed));
            }
        }
    }
}

/// Letra actual repetida en bandas, pero mezclando solo el fondo de la celda:
/// se lee como textura detrás de la UI y no se come el texto de los paneles.
fn draw_ambient_lyrics_tint(app: &App, buf: &mut Buffer, area: Rect, opacity: f32) {
    let text = app
        .lyrics
        .as_ref()
        .and_then(|doc| doc.index_at(app.now.position_ms))
        .and_then(|index| app.lyrics.as_ref().and_then(|doc| doc.lines.get(index)))
        .map(|line| line.text.clone())
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            app.now
                .track
                .as_ref()
                .map(|track| format!("{} · {}", track.title, track.artist))
        });
    let Some(text) = text else { return };
    let alpha = (opacity * 1.4).clamp(0.04, 0.22);
    let tint = mix_oklab(palette_bg(app), app.palette.current().secondary, 0.55);
    let drift = (app.now.position_ms / 90) as i32;
    let area_buf = *buf.area();
    let width = area_buf.width as usize;
    for band in 0..3u16 {
        let y = area.y + (area.height as u32 * (band as u32 * 32 + 12) / 100) as u16;
        if y >= area.y + area.height || y >= area_buf.y + area_buf.height {
            continue;
        }
        let mut line: Vec<char> = text.chars().collect();
        line.extend([' ', '·', ' ']);
        let line_w = line.len() as i32;
        if line_w == 0 {
            continue;
        }
        let offset = (drift + band as i32 * 23).rem_euclid(line_w);
        let mut x = area.x as i32 - offset;
        while x < (area.x + area.width) as i32 {
            for (i, _) in line.iter().enumerate() {
                let px = x + i as i32;
                if px < area.x as i32 || px >= (area.x + area.width) as i32 {
                    continue;
                }
                let start = (y - area_buf.y) as usize * width
                    + (px as u16 - area_buf.x) as usize;
                if let Some(cell) = buf.content.get_mut(start) {
                    let under = match cell.bg {
                        Color::Rgb(r, g, b) => (r, g, b),
                        _ => continue,
                    };
                    let mixed = mix(under, tint, alpha);
                    cell.set_bg(color(mixed));
                    if cell.symbol() == " " {
                        cell.set_fg(color(mixed));
                    }
                }
            }
            x += line_w + 6;
        }
    }
}

fn palette_bg(app: &App) -> Rgb {
    app.palette.current().bg_deep
}

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

/// Icono/etiqueta corta del proveedor de la fuente descargada.
fn source_tag(provider: Option<&str>) -> &'static str {
    match provider.unwrap_or("") {
        "youtube" | "yt" | "yt-dlp" => "▶",
        "soundcloud" | "sc" => "☁",
        "spotify" => "●",
        "deezer" | "dz" => "◆",
        "local" | "mpris" => "⌂",
        "" => "·",
        _ => "↓",
    }
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

/// Rect del panel de contenido. Sin pista no hay portada al lado, así que el
/// panel se centra en el hueco en vez de quedarse pegado a la izquierda.
fn content_rect(app: &App, layout: &Layout) -> Rect {
    if app.now.track.is_some() {
        return layout.content;
    }
    let width = layout.content.width.min(64).max(layout.center.width.min(40));
    Rect::new(
        layout.center.x + layout.center.width.saturating_sub(width) / 2,
        layout.content.y,
        width,
        layout.content.height,
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
    let area = *buf.area();
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
    let area = *buf.area();
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
    let area = *buf.area();
    let clip = rect.intersection(area);
    if clip.width == 0 || clip.height == 0 {
        return;
    }
    let width = area.width as usize;
    let fg = color(bg);
    for y in clip.y..clip.y + clip.height {
        let start = (y - area.y) as usize * width + (clip.x - area.x) as usize;
        for cell in &mut buf.content[start..start + clip.width as usize] {
            cell.set_symbol(" ");
            cell.set_fg(fg);
            cell.set_bg(fg);
        }
    }
}

/// Rellena mezclando con lo que ya hay debajo. `amount` 1.0 = opaco. Es lo que
/// deja ver el visualizador de fondo a través de los paneles.
fn fill_soft(buf: &mut Buffer, rect: Rect, target: Rgb, amount: f32, fallback: Rgb) {
    let amount = amount.clamp(0.0, 1.0);
    let area = *buf.area();
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if x >= area.x + area.width || y >= area.y + area.height {
                continue;
            }
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            let under = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => fallback,
            };
            let mixed = mix(under, target, amount);
            cell.set_symbol(" ");
            cell.set_fg(color(mixed));
            cell.set_bg(color(mixed));
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
    let buf_area = *buf.area();
    let width = buf_area.width as usize;
    let height = area.height.min(buf_area.height) as usize;
    for row in 0..height {
        let y = area.y + row as u16;
        let t = row as f32 / height.max(1) as f32;
        let base = mix_oklab(pal.bg_deep, pal.bg, t * 0.8);
        let base = mix_oklab(pal.bg_deep, base, intro);
        let fg = color(base);
        let start = (y - buf_area.y) as usize * width;
        let range = start..start + width;
        for cell in &mut buf.content[range] {
            cell.set_symbol(" ");
            cell.set_fg(fg);
            cell.set_bg(fg);
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

/// Tinta el panel con el canvas ambiental. Mantiene el glifo (texto/borde) y
/// solo mezcla el fondo, así el efecto se ve dentro del panel sin ensuciar la
/// lectura.
fn ambient_tint(app: &App, buf: &mut Buffer, rect: Rect, strength: f32) {
    let layer = &app.ambient.output;
    if layer.w == 0 || layer.h == 0 || strength <= 0.0 {
        return;
    }
    let opacity = app.cfg.ui.fx.ambient_opacity.min(40) as f32 / 40.0;
    if opacity <= 0.0 {
        return;
    }
    let area = *buf.area();
    let clip = rect.intersection(area);
    if clip.width == 0 || clip.height == 0 {
        return;
    }
    let width = area.width as usize;
    for y in clip.y..clip.y + clip.height {
        let start = (y - area.y) as usize * width + (clip.x - area.x) as usize;
        let sy = ((y - area.y) as u32 * layer.h as u32 / area.height.max(1) as u32)
            .min(layer.h.saturating_sub(1) as u32) as i32;
        for (offset, cell) in buf.content[start..start + clip.width as usize]
            .iter_mut()
            .enumerate()
        {
            let x = clip.x + offset as u16;
            let sx = ((x - area.x) as u32 * layer.w as u32 / area.width.max(1) as u32)
                .min(layer.w.saturating_sub(1) as u32) as i32;
            let Some(src) = layer.get(sx, sy) else { continue };
            if src.alpha <= 0.01 {
                continue;
            }
            let under = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => continue,
            };
            let mixed = mix(under, src.fg, strength * opacity * src.alpha);
            cell.set_bg(color(mixed));
            if cell.symbol() == " " {
                cell.set_fg(color(mixed));
            }
        }
    }
}

fn panel(app: Option<&App>, buf: &mut Buffer, rect: Rect, pal: &Palette, active: bool, alpha: f32, title: Option<&str>, pulse: f32) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let fill_color = mix(pal.bg, pal.surface, alpha.clamp(0.0, 1.0));
    // alpha 0 = panel transparente: se usa sobre el arte para no taparlo.
    let opaque = alpha > 0.001;
    let bg = if opaque { Some(color(fill_color)) } else { None };
    if opaque {
        fill_soft(buf, rect, fill_color, if soft_background(pal) { 0.92 } else { 1.0 }, pal.bg);
        if let Some(app) = app {
            let mode = app.cfg.ui.fx.ambient.as_str();
            if !matches!(mode, "off" | "cover" | "lyrics") {
                let strength = if app.cfg.ui.fx.glitch_panels { 0.08 } else { 0.02 };
                ambient_tint(app, buf, rect, strength);
            }
        }
    }
    // Cada instrumento tiñe el borde: bombo en el acento primario, caja en
    // secundario, charles en el brillo y voz en el color de texto. Así la UI
    // entera vibra con la canción, no solo la franja inferior.
    let (kick, snare, hat, vocal) = app
        .map(|app| {
            (
                app.audio.kick_pulse,
                app.audio.snare_pulse,
                app.audio.hat_pulse,
                app.audio.vocal_pulse,
            )
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    let accent = if vocal >= kick && vocal >= snare && vocal >= hat {
        pal.text
    } else if hat >= kick && hat >= snare {
        pal.ok
    } else if snare >= kick {
        pal.secondary
    } else {
        pal.primary
    };
    let song = app.map(|app| app.song_intensity).unwrap_or(0.0);
    let hit = kick
        .max(snare * 0.85)
        .max(hat * 0.55)
        .max(vocal * 0.7)
        * (0.55 + song * 0.75);
    let base_border = if active {
        mix_oklab(pal.border, pal.primary, 0.55)
    } else if alpha >= 0.6 {
        pal.border
    } else {
        pal.border_dim
    };
    let border = if app.is_some_and(|app| app.cfg.ui.fx.instrument_borders) {
        // Un mínimo de tinte hace que los bordes se vean incluso entre golpes;
        // antes solo cambiaban durante un pico y parecían estáticos.
        let glow = (0.10 + hit * 0.85 + pulse * 0.18 + song * 0.12).min(0.95);
        mix_oklab(base_border, accent, glow)
    } else {
        base_border
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bar_width_respects_setting_and_keeps_margins() {
        assert_eq!(bar_width(100, 70), 70);
        assert_eq!(bar_width(100, 100), 80);
        assert_eq!(bar_width(30, 55), 16);
        assert_eq!(bar_width(10, 100), 10);
    }

}
