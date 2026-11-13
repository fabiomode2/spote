//! Dibujo de la interfaz. Todo se pinta sobre `Buffer` para controlar color,
//! posición y glifo celda a celda.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use spote_core::artwork::{ArtMode, Palette, Rgb};
use spote_core::util::{format_ms, luma, mix, mix_oklab, scale};

use crate::app::{Action, App, Overlay, Tab, ThumbMode, ToastKind};
use crate::theme::ease_out_cubic;
use crate::viz::Blend;

mod art;
mod now;
mod lists;
mod settings;
mod side;
mod chrome;
mod overlays;
use art::*;
use now::*;
use lists::*;
use settings::*;
use side::*;
use chrome::*;
use overlays::*;

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
        vcenter: bool,
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
        if vcenter {
            // El bloque (el "cuadrado" de paneles) se ajusta a la portada y se
            // centra en el alto disponible. Antes esto solo centraba el texto
            // dentro de paneles estirados de borde a borde.
            let center_w = out.main.width.saturating_sub(nav_width + side_width);
            let block_h = ((center_w as u32 * 3 / 10) as u16 + 2).min(out.main.height);
            let spare = out.main.height.saturating_sub(block_h);
            out.main = Rect::new(
                out.main.x,
                out.main.y + spare / 2,
                out.main.width,
                block_h,
            );
        }
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
    let vcenter = app.vcenter && app.fullscreen == crate::app::Fullscreen::None;
    let layout = Layout::compute(area, app.show_viz, app.fullscreen, art_ratio, vcenter);
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

/// Rect del panel de contenido. Sin pista no hay portada al lado, así que el
/// panel se centra en el hueco en vez de quedarse pegado a la izquierda.
fn content_rect(app: &App, layout: &Layout) -> Rect {
    if app.now.track.is_some() {
        return layout.content;
    }
    let width = layout.content.width.min(64).max(layout.center.width.min(40));
    Rect::new(
        layout.full.x + layout.full.width.saturating_sub(width) / 2,
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
            let layout = Layout::compute(area, true, Fullscreen::None, 6, true);
            assert!(layout.center.width > 0);
            assert!(layout.center.height > 0);
            assert!(layout.center.x + layout.center.width <= area.x + area.width);
            assert!(layout.main.y + layout.main.height <= area.y + area.height);
        }
    }

    #[test]
    fn vcenter_shrinks_and_centers_the_block() {
        let area = Rect::new(0, 0, 200, 70);
        let normal = Layout::compute(area, true, Fullscreen::None, 6, false);
        let centered = Layout::compute(area, true, Fullscreen::None, 6, true);
        assert!(centered.main.height < normal.main.height);
        let spare = normal.main.height - centered.main.height;
        assert!((centered.main.y as i32 - normal.main.y as i32 - spare as i32 / 2).abs() <= 1);
    }

    #[test]
    fn bar_width_respects_setting_and_keeps_margins() {
        assert_eq!(bar_width(100, 70), 70);
        assert_eq!(bar_width(100, 100), 80);
        assert_eq!(bar_width(30, 55), 16);
        assert_eq!(bar_width(10, 100), 10);
    }

    #[test]
    fn layout_collapses_nav_and_side_on_narrow() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = Layout::compute(area, true, Fullscreen::None, 6, true);
        assert_eq!(layout.nav.width, 0);
        assert_eq!(layout.side.width, 0);
        assert_eq!(layout.center.width, 80);
    }

    #[test]
    fn fullscreen_removes_viz_strip() {
        let area = Rect::new(0, 0, 160, 50);
        let layout = Layout::compute(area, true, Fullscreen::Viz, 6, false);
        assert_eq!(layout.viz.height, 0);
        assert!(layout.main.height >= area.height - 4);
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        for (w, h) in [(1, 1), (10, 4), (19, 7)] {
            let layout = Layout::compute(Rect::new(0, 0, w, h), true, Fullscreen::None, 6, true);
            assert!(layout.center.width <= w);
        }
    }
}
