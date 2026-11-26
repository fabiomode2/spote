//! Dibujo de la interfaz. Todo se pinta sobre `Buffer` para controlar color,
//! posición y glifo celda a celda.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use spote_core::artwork::{ArtMode, CoverArt, Palette, PaletteVariant, Rgb};
use spote_core::util::{format_ms, luma, mix, mix_oklab, scale};

use crate::app::{
    Action, App, LibFilter, Overlay, Pane, SocialPeopleMode, Tab, ThumbMode, ToastKind,
};
use crate::theme::ease_out_cubic;
use crate::viz::Blend;

mod albums;
mod art;
mod audio;
mod chrome;
mod detail;
use detail::*;
mod layout;
mod lists;
mod mosaic;
mod now;
mod overlays;
mod settings;
mod side;
mod wrapped;
use albums::*;
use art::*;
use audio::*;
use chrome::*;
pub use layout::Layout;
use lists::*;
use now::*;
use overlays::*;
use settings::*;
use side::*;
use wrapped::*;

const PLAY: &str = "⏵";
const PAUSE: &str = "⏸";

pub fn draw(app: &mut App, buf: &mut Buffer, area: Rect) {
    app.art_visible_rect = None;
    app.wallpaper_rect = if app.cfg.ui.fx.backdrop == "wallpaper" && !app.zen {
        Some(area)
    } else {
        None
    };
    app.hi_thumbs.clear();
    if app.overlay != app.overlay_seen {
        app.overlay_seen = app.overlay.clone();
        app.needs_redraw = true;
    }
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
            // Buscar enseña la portada como protagonista: puede ocupar todo
            // el alto disponible. La lista conserva su columna a la izquierda.
            Tab::Search => 10,
            Tab::Library | Tab::Room => 3,
            Tab::Audio | Tab::Albums | Tab::Settings => 0,
        }
    };
    let detail_expand = app.detail.is_some()
        && match app.cfg.ui.detail_mode.as_str() {
            "expand" => true,
            "auto" => area.width < 150,
            _ => false,
        };
    let mut layout_cfg = app.cfg.ui.layout.clone();
    if app.cfg.ui.nav_position == "top" {
        layout_cfg.nav = false;
    }
    let layout = Layout::compute(
        area,
        &layout_cfg,
        app.show_viz,
        app.fullscreen,
        art_ratio,
        app.detail.is_some() && !detail_expand,
    );
    app.last_main = layout.main;
    app.hit.clear();
    let pal = *app.palette.current();
    let zen = app.zen && app.show_viz;
    // Con el visualizador a pantalla completa, el fondo ambiental queda detrás
    // de una capa opaca: no se regenera (era el doble de trabajo por frame).
    let viz_fullscreen =
        (app.viz_background || zen || app.fullscreen == crate::app::Fullscreen::Viz)
            && app.show_viz
            && layout.main.height > 2;

    // Con mosaico, el fondo base no debe latir: si cambia cada frame, el
    // mosaico mezcla contra colores nuevos y la terminal acaba repintando
    // toda la pantalla a 60 Hz aunque el mosaico avance a 30.
    let reactive_background = app.cfg.ui.fx.backdrop != "mosaic";
    background(
        buf,
        area,
        &pal,
        app.intro,
        if reactive_background {
            app.audio.bass
        } else {
            0.0
        },
        if reactive_background {
            app.audio.kick_pulse
        } else {
            0.0
        },
        if reactive_background {
            app.song_intensity
        } else {
            0.0
        },
    );
    // Mosaico de portada: va detrás de absolutamente todo. Se pinta antes que
    // los paneles para que estos solo lo dejen entrever con su transparencia.
    let fullscreen_viz = app.fullscreen == crate::app::Fullscreen::Viz && viz_fullscreen;
    let black_fullscreen = fullscreen_viz && app.cfg.ui.fx.fullscreen_backdrop == "black";
    if !black_fullscreen
        && (app.cfg.ui.fx.backdrop == "mosaic" || app.cfg.ui.fx.ambient == "mosaic")
    {
        mosaic::draw(app, buf, area, &pal);
    }
    // Fondo ambiental: un efecto ruidoso (void/neural/glitch/aurora) a baja
    // opacidad detrás de los paneles. Los paneles son translúcidos, así que se
    // lee a través de ellos sin quitar legibilidad.
    let fx_opacity = app.cfg.ui.fx.ambient_opacity.min(50) as f32 / 100.0;
    if fx_opacity > 0.001 && !viz_fullscreen {
        match app.cfg.ui.fx.ambient.as_str() {
            "off" | "cover" | "lyrics" | "mosaic" => {}
            _ => {
                let w = (area.width / 2).max(12);
                let h = (area.height / 2).max(6);
                let hz = app.ambient_effects_hz();
                app.pump_ambient(w, h, hz, &pal);
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
    if viz_fullscreen {
        let hz = app.full_effects_hz();
        // En zen el efecto se ancla abajo: el fondo/mosaico se ve arriba y la
        // animación empieza más abajo, como una banda de escenario.
        let zen_viz = if zen && app.fullscreen != crate::app::Fullscreen::Viz {
            let pct = app.cfg.ui.zen_viz_pct.clamp(30, 100) as u32;
            let h = (area.height as u32 * pct / 100) as u16;
            Some(Rect::new(
                area.x,
                area.y + area.height.saturating_sub(h),
                area.width,
                h.max(6),
            ))
        } else {
            None
        };
        let viz_rect = zen_viz.unwrap_or(layout.full);
        app.pump_viz(viz_rect.width.max(20), viz_rect.height.max(6), hz, &pal);
        if fullscreen_viz {
            // A pantalla completa manda el fondo propio del efecto: las celdas
            // vacías del canvas llevan el gradiente de la paleta. Si el ajuste
            // pide mosaico se deja asomar un poco; en negro, manda el efecto.
            let bg_amount = if app.cfg.ui.fx.fullscreen_backdrop == "black" {
                1.0
            } else {
                0.82
            };
            blit_viz_cover(app, buf, viz_rect, 0, bg_amount, VIZ_SCALE);
        } else if zen {
            blit_viz(app, buf, viz_rect, 0, 0.95, 0.0, VIZ_SCALE);
        } else {
            blit_viz(app, buf, viz_rect, 0, 0.32, 0.10, VIZ_SCALE);
        }
    }
    if app.zen {
        // Zen de verdad: visual a pantalla completa y una sola línea de
        // progreso. Sin transporte gigante, hint ni barras que tapen el arte.
        chrome::draw_zen_bar(app, buf, &layout, &pal);
        draw_overlay(app, buf, &layout, &pal);
        seal_frame(buf, &pal);
        return;
    }
    if layout.main.width < 20 {
        draw_too_small(buf, area, &pal);
        return;
    }

    // Portada a pantalla completa real: sin top, nav, cola ni nowbar.
    if app.fullscreen == crate::app::Fullscreen::Art {
        draw_now(app, buf, &layout, &pal);
        if layout.transport.height > 0 {
            draw_transport(app, buf, &layout, &pal);
        }
        if layout.hint.height > 0 {
            draw_hint(app, buf, &layout, &pal);
        }
        draw_overlay(app, buf, &layout, &pal);
        return;
    }

    // Visualizador a pantalla completa: el efecto manda. Nada de transporte
    // gigante ni hint: solo una línea de progreso, como en zen.
    if app.fullscreen == crate::app::Fullscreen::Viz {
        chrome::draw_zen_bar(app, buf, &layout, &pal);
        draw_overlay(app, buf, &layout, &pal);
        seal_frame(buf, &pal);
        return;
    }

    if layout.top.height > 0 {
        draw_top(app, buf, &layout, &pal);
    }
    if !zen && layout.nav.width > 0 {
        draw_nav(app, buf, &layout, &pal);
    }
    if detail_expand {
        // El detalle manda y ocupa el ancho del contenido, respetando el
        // lateral de secciones si está visible.
        let mut rect = layout.main;
        if layout.nav.width > 0 {
            if app.cfg.ui.layout.nav_left() {
                rect.x += layout.nav.width;
                rect.width = rect.width.saturating_sub(layout.nav.width);
            } else {
                rect.width = rect.width.saturating_sub(layout.nav.width);
            }
        }
        draw_detail(app, buf, rect, &pal);
    } else {
        match app.tab {
            Tab::Now => draw_now(app, buf, &layout, &pal),
            Tab::Search => draw_search(app, buf, &layout, &pal),
            Tab::Library => draw_library(app, buf, &layout, &pal),
            Tab::Audio => draw_audio_tab(app, buf, &layout, &pal),
            Tab::Albums => draw_albums(app, buf, &layout, &pal),
            Tab::Room => draw_room(app, buf, &layout, &pal),
            Tab::Settings => draw_settings(app, buf, &layout, &pal),
        }
        if !zen && layout.side.width > 0 {
            draw_side(app, buf, &layout, &pal);
        }
    }
    if layout.viz.height > 0 {
        draw_viz(app, buf, &layout, &pal);
    }
    if layout.nowbar.height > 0 {
        draw_nowbar(app, buf, &layout, &pal);
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
            // Con el mosaico activo, `cover` sería una segunda capa de la
            // misma portada: se ignora. `lyrics` sigue siendo útil encima.
            "cover" if app.cfg.ui.fx.backdrop != "mosaic" => {
                draw_ambient_cover_tint(app, buf, area, fx_opacity)
            }
            "lyrics" => draw_ambient_lyrics_tint(app, buf, area, fx_opacity),
            _ => {}
        }
    }
    // Las asas van al final: el borde del panel registra su foco antes y, si
    // no, el clic de redimensionar nunca llegaría.
    if let Some(split) = layout.nav_split {
        app.hit.add(split, Action::Resize(Pane::Nav));
    }
    if let Some(split) = layout.side_split.filter(|_| app.detail.is_none()) {
        app.hit.add(split, Action::Resize(crate::app::Pane::Side));
    }
    draw_toasts(app, buf, &layout, &pal);
    draw_overlay(app, buf, &layout, &pal);
    seal_frame(buf, &pal);
}

/// Cierra el frame: rellena cualquier celda que se haya quedado con el estilo
/// por defecto y fuerza el diff de los bordes. Algunos terminales dejan un
/// píxel/celda sin pintar en la primera fila tras limpiar o mover imágenes
/// Kitty; forzar esa fila evita el hueco persistente.
fn seal_frame(buf: &mut Buffer, pal: &Palette) {
    let area = *buf.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let bg = color(pal.bg_deep);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            if matches!(cell.bg, Color::Reset) {
                cell.set_bg(bg);
            }
            if matches!(cell.fg, Color::Reset) {
                cell.set_fg(bg);
            }
        }
    }
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
        .and_then(|doc| {
            let target = doc.aligned_ms(app.display_position_ms());
            doc.index_at_target(target)
                .and_then(|index| doc.lines.get(index))
        })
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
    let drift = (app.display_position_ms() / 90) as i32;
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
                let start = (y - area_buf.y) as usize * width + (px as u16 - area_buf.x) as usize;
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
        "youtube" | "yt" | "ytmusic" | "yt-dlp" => "▶",
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
    let width = layout
        .content
        .width
        .min(64)
        .max(layout.center.width.min(40));
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

/// Escribe `text` repartido por celdas desde `(x, y)`.
///
/// Antes la cadena entera acababa como símbolo de una sola celda: el terminal
/// la pintaba desbordando a la derecha mientras ratatui solo apuntaba una
/// celda, así que al acortarse un texto el diff no limpiaba los caracteres
/// sobrantes y se quedaban en pantalla. `set_stringn` coloca cada grafema en su
/// celda, respeta el ancho de los caracteres anchos y recorta al borde.
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, fg: Color, bg: Option<Color>) {
    let area = *buf.area();
    if x < area.x || y < area.y || x >= area.x + area.width || y >= area.y + area.height {
        return;
    }
    let mut style = Style::new().fg(fg);
    if let Some(bg) = bg {
        style = style.bg(bg);
    }
    let max_width = (area.x + area.width - x) as usize;
    buf.set_stringn(x, y, text, max_width, style);
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

fn chrome_alpha(app: &App) -> f32 {
    app.cfg.ui.chrome_opacity.clamp(0, 100) as f32 / 100.0
}

fn bottom_alpha(app: &App) -> f32 {
    app.cfg.ui.bottom_opacity.clamp(0, 100) as f32 / 100.0
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

/// Texto clicable con una pista visual permanente (subrayado tenue) y un
/// resaltado animado al pasar el ratón: la luz recorre las letras de izquierda
/// a derecha mientras el fondo pulsa con la paleta. Devuelve el ancho pintado
/// para que el llamante registre el hitbox exacto.
#[allow(clippy::too_many_arguments)]
fn link_at(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    text: &str,
    pal: &Palette,
    base: Rgb,
    hovered: bool,
    time: f32,
) -> u16 {
    if max_w == 0 || text.is_empty() {
        return 0;
    }
    let width = text.chars().count().min(max_w as usize) as u16;
    if !hovered {
        let label: String = text.chars().take(width as usize).collect();
        text_at(
            buf,
            Rect::new(x, y, width, 1),
            label,
            Style::new()
                .fg(color(base))
                .add_modifier(Modifier::UNDERLINED),
            Alignment::Left,
        );
        return width;
    }

    // Hover: onda de color que recorre el texto + fondo con brillo viajero.
    let backdrop = mix(pal.surface, pal.bg, 0.35);
    for (index, ch) in text.chars().take(width as usize).enumerate() {
        let phase = (time * 3.2 - index as f32 * 0.55).rem_euclid(std::f32::consts::TAU);
        let wave = 0.5 + 0.5 * phase.sin();
        let fg = mix_oklab(mix_oklab(pal.primary, pal.secondary, wave), pal.text, 0.22);
        let bg = mix_oklab(backdrop, pal.primary, 0.10 + wave * 0.18);
        let mut glyph = [0u8; 4];
        let symbol = ch.encode_utf8(&mut glyph);
        if let Some(cell) = buf.cell_mut((x + index as u16, y)) {
            cell.set_symbol(symbol);
            cell.set_fg(color(fg));
            cell.set_bg(color(bg));
            cell.modifier.insert(Modifier::BOLD | Modifier::UNDERLINED);
        }
    }
    width
}

fn background(
    buf: &mut Buffer,
    area: Rect,
    pal: &Palette,
    intro: f32,
    bass: f32,
    kick: f32,
    song: f32,
) {
    let intro = ease_out_cubic(intro);
    let buf_area = *buf.area();
    let width = buf_area.width as usize;
    let height = area.height.min(buf_area.height) as usize;
    // Luz de ambiente en los bordes: no es un pulso de un frame, sube con el
    // bajo y se queda unos instantes para que la pantalla "respire" con la
    // canción. Los paneles tapan el centro y dejan ver el marco.
    let glow = (bass * 0.10 + kick * 0.08 + song * 0.06).clamp(0.0, 0.16);
    for row in 0..height {
        let y = area.y + row as u16;
        let t = row as f32 / height.max(1) as f32;
        let base = mix_oklab(pal.bg_deep, pal.bg, t * 0.8);
        let base = mix_oklab(pal.bg_deep, base, intro);
        let start = (y - buf_area.y) as usize * width;
        let range = start..start + width;
        let edge_y = ((t * 2.0 - 1.0).abs()).powf(1.7);
        for (column, cell) in buf.content[range].iter_mut().enumerate() {
            let nx = column as f32 / width.max(1) as f32;
            let edge = edge_y.max(((nx * 2.0 - 1.0).abs()).powf(1.7));
            let lit = mix(base, pal.primary, glow * (0.20 + edge * 0.80));
            let fg = color(lit);
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
/// Tinta el panel con el canvas ambiental. Mantiene el glifo (texto/borde) y
/// solo mezcla el fondo, así el efecto se ve dentro del panel sin ensuciar la
/// lectura.
fn ambient_tint(app: &App, buf: &mut Buffer, rect: Rect, strength: f32) {
    // Con el visualizador a pantalla completa no se regenera el canvas
    // ambiental (quedaba debajo y no se veía): el tinte de los paneles sale
    // del propio visualizador, que es el mismo movimiento.
    let layer = if app.viz_background && app.viz.output.w > 0 {
        &app.viz.output
    } else {
        &app.ambient.output
    };
    if layer.w == 0 || layer.h == 0 || strength <= 0.0 {
        return;
    }
    let opacity = app.cfg.ui.fx.ambient_opacity.min(50) as f32 / 50.0;
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
            let Some(src) = layer.get(sx, sy) else {
                continue;
            };
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

fn panel(
    app: Option<&App>,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    active: bool,
    alpha: f32,
    title: Option<&str>,
    pulse: f32,
) {
    if rect.width < 3 || rect.height < 2 {
        return;
    }
    let fill_color = mix(pal.bg, pal.surface, alpha.clamp(0.0, 1.0));
    // alpha 0 = panel transparente: se usa sobre el arte para no taparlo.
    let opaque = alpha > 0.001;
    let bg = if opaque {
        Some(color(fill_color))
    } else {
        None
    };
    if opaque {
        // La transparencia real la manda `panel_opacity`; el `alpha` de
        // entrada solo elige el color del panel.
        let opacity = app
            .map(|app| app.cfg.ui.panel_opacity.clamp(55, 100) as f32 / 100.0)
            .unwrap_or(1.0);
        fill_soft(buf, rect, fill_color, opacity, pal.bg);
        if let Some(app) = app {
            let mode = app.cfg.ui.fx.ambient.as_str();
            if !matches!(mode, "off" | "cover" | "lyrics" | "mosaic") {
                let strength = if app.cfg.ui.fx.glitch_panels {
                    0.08
                } else {
                    0.02
                };
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
    let hit = kick.max(snare * 0.85).max(hat * 0.55).max(vocal * 0.7) * (0.55 + song * 0.75);
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
        let glow = (0.18 + hit * 0.82 + pulse * 0.22 + song * 0.15).min(1.0);
        mix_oklab(base_border, accent, glow)
    } else {
        base_border
    };
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width - 1, rect.y + rect.height - 1);
    let border_mode = app
        .map(|app| app.cfg.ui.panel_border.as_str())
        .unwrap_or("line");
    let border_opacity = app
        .map(|app| app.cfg.ui.border_opacity.clamp(0, 100) as f32 / 100.0)
        .unwrap_or(1.0);
    match border_mode {
        "none" => {}
        "rim" | "color" => {
            // Banda de píxeles de 1 celda. Los colores salen de la paleta de
            // la portada y recorren el perímetro; la canción modula brillo y
            // fase. No se mueve la geometría, solo el color/patrón.
            let rim_strength = app
                .map(|app| app.cfg.ui.rim_intensity.clamp(0, 100) as f32 / 100.0)
                .unwrap_or(0.6);
            let reactive = app.is_some_and(|app| app.cfg.ui.rim_reactive);
            let time = app.map(|app| app.mosaic_time).unwrap_or(0.0);
            let amount = if reactive {
                (0.35 + hit * 0.65).clamp(0.0, 1.0)
            } else {
                0.55
            };
            let perimeter = 2 * (rect.width as usize + rect.height as usize).max(1);
            let mut index = 0usize;
            let mut paint = |x: u16, y: u16| {
                let t = if perimeter > 1 {
                    index as f32 / perimeter as f32
                } else {
                    0.0
                };
                let wave = (t * std::f32::consts::TAU * 2.0 + time * 1.7).sin() * 0.5 + 0.5;
                let base = app
                    .and_then(|app| art::cover_edge_color(app, t))
                    .unwrap_or_else(|| mix(pal.primary, pal.secondary, wave));
                let lit = mix(
                    pal.bg_deep,
                    base,
                    (0.65 + amount * 0.35) * rim_strength.clamp(0.15, 1.0),
                );
                let lit = mix(fill_color, lit, border_opacity);
                put(buf, x, y, "█", color(lit), bg);
                index += 1;
            };
            for x in x0..=x1 {
                paint(x, y0);
            }
            for y in (y0 + 1)..=y1 {
                paint(x1, y);
            }
            if y1 > y0 {
                for x in (x0..x1).rev() {
                    paint(x, y1);
                }
            }
            if x1 > x0 {
                for y in ((y0 + 1)..y1).rev() {
                    paint(x0, y);
                }
            }
        }
        _ => {
            let fg = color(mix(fill_color, border, border_opacity));
            for x in x0 + 1..x1 {
                put(buf, x, y0, "─", fg, bg);
                put(buf, x, y1, "─", fg, bg);
            }
            for y in y0 + 1..y1 {
                put(buf, x0, y, "│", fg, bg);
                put(buf, x1, y, "│", fg, bg);
            }
            for (x, y, glyph) in [(x0, y0, "╭"), (x1, y0, "╮"), (x0, y1, "╰"), (x1, y1, "╯")]
            {
                put(buf, x, y, glyph, fg, bg);
            }
        }
    }
    if let Some(title) = title {
        if rect.width > 8 {
            let max = rect.width.saturating_sub(6) as usize;
            let mut label = title.to_uppercase();
            truncate_chars(&mut label, max);
            let fg = if active { pal.text } else { pal.muted };
            if opaque {
                put(buf, x0 + 2, y0, " ", color(fill_color), bg);
                put(buf, x0 + 3, y0, &label, color(fg), Some(color(fill_color)));
            } else {
                put(buf, x0 + 3, y0, &label, color(fg), bg);
            }
        }
    }
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
        let rect = Rect::new(x + ((1.0 - slide) * 6.0) as u16, y, width, 1);
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

    #[test]
    fn put_spreads_text_across_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        put(&mut buf, 2, 0, "hola", Color::Red, None);
        let symbols: Vec<&str> = (2..7).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(symbols, ["h", "o", "l", "a", " "]);
    }

    #[test]
    fn put_clips_text_at_buffer_edge() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        put(&mut buf, 2, 0, "hola", Color::Red, None);
        assert_eq!(buf[(2, 0)].symbol(), "h");
        assert_eq!(buf[(3, 0)].symbol(), "o");
    }

    #[test]
    fn link_marks_clickable_text_and_highlights_on_hover() {
        let pal = Palette::dark();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let width = link_at(
            &mut buf,
            1,
            0,
            8,
            "Biberon",
            &pal,
            pal.secondary,
            false,
            0.0,
        );
        assert_eq!(width, 7);
        assert_eq!(buf[(1, 0)].symbol(), "B");
        assert!(buf[(1, 0)].modifier.contains(Modifier::UNDERLINED));
        assert!(!buf[(1, 0)].modifier.contains(Modifier::BOLD));

        let width = link_at(&mut buf, 1, 0, 8, "Biberon", &pal, pal.secondary, true, 0.4);
        assert_eq!(width, 7);
        assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(matches!(buf[(1, 0)].bg, Color::Rgb(..)));
    }

    #[test]
    fn seal_frame_leaves_no_default_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
        seal_frame(&mut buf, &Palette::dark());
        for y in 0..3 {
            for x in 0..6 {
                assert!(
                    !matches!(buf[(x, y)].bg, Color::Reset),
                    "celda {x},{y} sin fondo"
                );
            }
        }
    }
}
