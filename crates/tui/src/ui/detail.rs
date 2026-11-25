//! Panel lateral persistente de detalle: perfil de artista o ficha de álbum.
//!
//! No es un overlay: vive en `layout.side` y se queda abierto hasta que se
//! pulsa volver. El dibujo solo fija `detail_album_visible` y
//! `detail_albums_area`, que son las medidas que necesita la rueda del ratón
//! para la tira horizontal de discografía.

use super::*;
use crate::app::{AlbumPage, Detail};

const BACK: &str = "‹ volver";
const CARD_GAP: u16 = 2;

pub(super) fn draw_detail(app: &mut App, buf: &mut Buffer, rect: Rect, pal: &Palette) {
    // La rueda y el desplazamiento de la discografía se recalculan cada
    // frame: si no se pinta la sección, la zona queda vacía para que un clic
    // fuera de las tarjetas no scrollee nada.
    app.detail_albums_area = Rect::default();
    app.detail_album_visible = 0;

    if rect.width < 6 || rect.height < 3 {
        return;
    }
    let Some(detail) = app.detail else {
        return;
    };

    panel(
        Some(app),
        buf,
        rect,
        pal,
        true,
        0.86,
        None,
        app.audio.kick_pulse,
    );

    let inner_x = rect.x + 2;
    let inner_w = rect.width.saturating_sub(4);
    if inner_w < 4 {
        return;
    }

    let (section, provider, status, loading) = match detail {
        Detail::Artist => (
            "ARTISTA",
            app.artist
                .as_ref()
                .map(|profile| profile.provider.clone())
                .filter(|provider| !provider.is_empty() && provider.as_str() != "search"),
            app.artist_status.clone(),
            app.artist_loading,
        ),
        Detail::Album => (
            "ÁLBUM",
            app.album_page
                .as_ref()
                .map(|page| page.provider.clone())
                .filter(|provider| !provider.is_empty() && provider.as_str() != "search"),
            app.album_page_status.clone(),
            app.album_page_loading,
        ),
    };

    let mut y = rect.y + 1;
    draw_header(
        app,
        buf,
        y,
        pal,
        section,
        provider.as_deref(),
        inner_x,
        inner_w,
    );
    y += 1;

    // En el perfil de artista el estado se queda como resumen («spotify ·
    // 1.2M fans · 12 pistas») al terminar; no hace falta repetirlo si la ficha
    // ya está pintada. En el álbum, un estado sin carga sí es un error real.
    let show_status = match detail {
        Detail::Artist => loading || app.artist.is_none(),
        Detail::Album => loading || !status.trim().is_empty(),
    };
    if show_status
        && y < rect.y + rect.height.saturating_sub(1)
        && (!status.trim().is_empty() || loading)
    {
        draw_status(app, buf, inner_x, y, inner_w, pal, status.trim(), loading);
        y += 1;
    }

    // Hairline degradada: separa la cabecera sin dibujar un segundo marco.
    if y < rect.y + rect.height.saturating_sub(1) {
        draw_hairline(buf, inner_x, y, inner_w, pal);
        y += 1;
    }

    let limit = rect.y + rect.height.saturating_sub(2);
    if y >= limit {
        return;
    }
    let content = Rect::new(inner_x, y, inner_w, limit - y);
    match detail {
        Detail::Artist => draw_artist(app, buf, content, pal),
        Detail::Album => draw_album(app, buf, content, pal),
    }
}

// ---------------------------------------------------------------------------
// Cabecera y estados
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_header(
    app: &mut App,
    buf: &mut Buffer,
    y: u16,
    pal: &Palette,
    section: &str,
    provider: Option<&str>,
    x: u16,
    width: u16,
) {
    let back_w = chars_w(BACK).min(width);
    let back = Rect::new(x, y, back_w, 1);
    let hover = app.hover == Some(Action::CloseDetail);
    if hover {
        fill(buf, back, mix(pal.surface, pal.primary, 0.16));
    }
    let style = Style::new()
        .fg(color(if hover { pal.text } else { pal.secondary }))
        .add_modifier(if hover {
            Modifier::UNDERLINED | Modifier::BOLD
        } else {
            Modifier::empty()
        });
    text_at(buf, back, BACK, style, Alignment::Left);
    app.hit.add(back, Action::CloseDetail);

    // A la derecha, la sección y el proveedor. Se omite el proveedor si el
    // ancho no da para los dos sin comerse el botón de volver.
    let max_right = width.saturating_sub(back_w + 2);
    if max_right < 4 {
        return;
    }
    let mut title = section.to_uppercase();
    if let Some(provider) = provider.filter(|provider| !provider.is_empty()) {
        let extra = 3 + chars_w(provider);
        if max_right as usize >= title.chars().count() + extra as usize {
            title.push_str(" · ");
            title.push_str(&provider.to_uppercase());
        }
    }
    let title = fit(&title, max_right);
    let title_w = chars_w(&title);
    let title_x = x + width - title_w;
    text_at(
        buf,
        Rect::new(title_x, y, title_w, 1),
        title,
        Style::new()
            .fg(color(mix_oklab(pal.muted, pal.primary, 0.55)))
            .add_modifier(Modifier::BOLD),
        Alignment::Right,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_status(
    app: &App,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    pal: &Palette,
    status: &str,
    loading: bool,
) {
    if width < 2 {
        return;
    }
    // Sin carga, el estado solo se pinta cuando hay un problema real (el
    // resumen del perfil ya vive en la ficha), así que se marca en rojo.
    let (glyph, fg) = if loading {
        (spinner(app), pal.warn)
    } else {
        ("!", pal.error)
    };
    put(buf, x, y, glyph, color(fg), None);
    let message = if status.is_empty() {
        if loading {
            "cargando…"
        } else {
            return;
        }
    } else {
        status
    };
    let message = fit(message, width.saturating_sub(2));
    if message.is_empty() {
        return;
    }
    text_at(
        buf,
        Rect::new(x + 2, y, width.saturating_sub(2), 1),
        message,
        Style::new().fg(color(if loading { pal.muted } else { pal.error })),
        Alignment::Left,
    );
}

fn draw_hairline(buf: &mut Buffer, x: u16, y: u16, width: u16, pal: &Palette) {
    for offset in 0..width {
        let t = offset as f32 / width.max(1) as f32;
        let c = mix_oklab(pal.border, pal.surface, 0.30 + t * 0.55);
        put(buf, x + offset, y, "─", color(c), None);
    }
}

#[allow(clippy::too_many_arguments)]
fn empty_state(
    app: &App,
    buf: &mut Buffer,
    area: Rect,
    pal: &Palette,
    glyph: &str,
    title: &str,
    hint: &str,
    loading: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cy = area.y + area.height / 2;
    let show_glyph = area.height >= 4;
    let show_hint = area.height >= 3;
    let glyph_y = if show_glyph { cy.saturating_sub(1) } else { cy };
    if show_glyph {
        let symbol = if loading { spinner(app) } else { glyph };
        text_at(
            buf,
            Rect::new(area.x, glyph_y, area.width, 1),
            symbol.to_string(),
            Style::new()
                .fg(color(if loading { pal.warn } else { pal.primary }))
                .add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
    }
    text_at(
        buf,
        Rect::new(area.x, cy, area.width, 1),
        title.to_string(),
        Style::new()
            .fg(color(pal.text))
            .add_modifier(Modifier::BOLD),
        Alignment::Center,
    );
    if show_hint && cy + 1 < area.y + area.height {
        text_at(
            buf,
            Rect::new(area.x, cy + 1, area.width, 1),
            hint.to_string(),
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
    }
}

// ---------------------------------------------------------------------------
// Artista
// ---------------------------------------------------------------------------

fn draw_artist(app: &mut App, buf: &mut Buffer, area: Rect, pal: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(profile) = app.artist.clone() else {
        let loading = app.artist_loading;
        empty_state(
            app,
            buf,
            area,
            pal,
            "♪",
            if loading {
                "cargando perfil"
            } else {
                "sin perfil de artista"
            },
            if loading {
                "buscando en el catálogo…"
            } else {
                "abre un artista desde una pista o una búsqueda"
            },
            loading,
        );
        return;
    };

    let bottom = area.y + area.height;
    let show_image = area.height >= 11 && area.width >= 38;
    let show_popularity = profile.popularity > 0 && area.height >= 5;
    let show_bio = area.height >= 30
        && area.width >= 44
        && profile
            .bio
            .as_deref()
            .is_some_and(|bio| !bio.trim().is_empty());
    let text_rows = 2 + u16::from(show_popularity) + u16::from(show_bio);
    let image_h = if show_image { 6 } else { 0 };
    let hero_h = image_h.max(text_rows).min(area.height);

    if hero_h >= 2 {
        fill_soft(
            buf,
            Rect::new(area.x, area.y, area.width, hero_h),
            mix(pal.surface, pal.primary, 0.10),
            0.55,
            pal.bg,
        );
    }

    let image_w = if show_image { 12 } else { 0 };
    if show_image {
        let image = Rect::new(area.x, area.y, image_w, image_h);
        if let Some(art) = app.artist_art.as_ref().filter(|art| !art.is_empty()) {
            draw_cached_art(buf, image, art, pal, true);
        } else {
            art_placeholder(buf, image, pal, &profile.name);
        }
        frame_art(buf, image, pal);
    }

    let text_x = area.x + if show_image { image_w + 2 } else { 0 };
    let text_w = area.width.saturating_sub(text_x - area.x);
    // Con imagen, el bloque de texto se centra verticalmente en la foto.
    let text_y = if show_image {
        area.y + hero_h.saturating_sub(text_rows) / 2
    } else {
        area.y
    };

    let name = if profile.name.trim().is_empty() {
        "artista desconocido".to_string()
    } else {
        profile.name.clone()
    };
    let name = fit(&name, text_w);
    text_at(
        buf,
        Rect::new(text_x, text_y, text_w, 1),
        name,
        Style::new()
            .fg(color(pal.text))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );

    // Proveedor, fans y géneros en una sola línea de segmentos: si el panel
    // es estrecho se recortan por la derecha en vez de pisar el borde.
    let mut parts: Vec<(String, Rgb)> = Vec::new();
    if !profile.provider.is_empty() {
        parts.push((profile.provider.clone(), pal.dim));
    }
    if profile.followers > 0 {
        parts.push((
            format!("{} fans", format_count(profile.followers)),
            pal.muted,
        ));
    }
    if !profile.genres.is_empty() {
        let genres = profile
            .genres
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        parts.push((genres, pal.secondary));
    }
    if text_y + 1 < area.y + hero_h {
        draw_segments(buf, text_x, text_y + 1, text_w, pal, &parts);
    }

    let mut row = text_y + 2;
    if show_popularity && row < area.y + hero_h {
        popularity_bar(buf, text_x, row, text_w, pal, profile.popularity.min(100));
        row += 1;
    }
    if show_bio && row < area.y + hero_h {
        if let Some(bio) = profile.bio.as_deref() {
            let bio = one_line(bio);
            let bio = fit(&bio, text_w);
            text_at(
                buf,
                Rect::new(text_x, row, text_w, 1),
                bio,
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
    }

    // Dos tamaños de tarjeta sin saltos raros de cantidad: a partir de 52
    // columnas la portada crece, pero el número de tarjetas nunca baja.
    let card_w = if area.width >= 52 { 14 } else { 12 };
    let cover_h = card_w / 2;
    let card_h = cover_h + 2;
    let visible = ((area.width + CARD_GAP) / (card_w + CARD_GAP)).max(1) as usize;
    app.detail_album_visible = if profile.albums.is_empty() {
        0
    } else {
        visible
    };

    // El aire entre bloques se quita en paneles muy bajos: una fila extra de
    // pistas vale más que un hueco decorativo.
    let mut y = area.y + hero_h;
    if bottom.saturating_sub(y) >= 9 {
        y += 1;
    }
    if y >= bottom {
        return;
    }

    let remaining = bottom.saturating_sub(y);
    // La discografía solo entra si después queda al menos la cabecera del top
    // y una pista. En paneles bajos se prefiere el top completo.
    if !profile.albums.is_empty() && remaining >= 1 + card_h + 1 + 2 {
        draw_artist_albums(
            app,
            buf,
            Rect::new(area.x, y, area.width, 1 + card_h),
            pal,
            &profile.albums,
            &profile.name,
            card_w,
            cover_h,
            visible,
        );
        y += 1 + card_h + 1;
    }

    let remaining = bottom.saturating_sub(y);
    if profile.top_tracks.is_empty() {
        if remaining >= 1 {
            put(buf, area.x, y, "sin pistas populares", color(pal.dim), None);
        }
        return;
    }
    if remaining < 2 {
        return;
    }

    draw_section_header(
        buf,
        Rect::new(area.x, y, area.width, 1),
        pal,
        "POPULARES",
        0,
    );
    y += 1;
    draw_artist_tracks(
        app,
        buf,
        Rect::new(area.x, y, area.width, bottom - y),
        pal,
        &profile.top_tracks,
        app.artist_selected,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_artist_albums(
    app: &mut App,
    buf: &mut Buffer,
    area: Rect,
    pal: &Palette,
    albums: &[spote_core::spotify_api::AlbumSummary],
    fallback_artist: &str,
    card_w: u16,
    cover_h: u16,
    visible: usize,
) {
    if albums.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let arrow_w = 3u16;
    let scrollable = albums.len() > visible;
    let reserved = if scrollable { arrow_w * 2 + 1 } else { 0 };
    draw_section_header(
        buf,
        Rect::new(area.x, area.y, area.width, 1),
        pal,
        "DISCOGRAFÍA",
        reserved,
    );

    let start = app
        .artist_album_scroll
        .min(albums.len().saturating_sub(visible));
    if scrollable {
        let max_start = albums.len().saturating_sub(visible);
        let prev = Rect::new(
            area.x + area.width.saturating_sub(reserved) + 1,
            area.y,
            arrow_w,
            1,
        );
        let next = Rect::new(
            area.x + area.width.saturating_sub(arrow_w),
            area.y,
            arrow_w,
            1,
        );
        draw_arrow(
            app,
            buf,
            prev,
            pal,
            "‹",
            start > 0,
            Action::ArtistAlbumsScroll(-1),
        );
        draw_arrow(
            app,
            buf,
            next,
            pal,
            "›",
            start < max_start,
            Action::ArtistAlbumsScroll(1),
        );
    }

    let cards_y = area.y + 1;
    let gap = CARD_GAP;
    for offset in 0..visible {
        let index = start + offset;
        let Some(album) = albums.get(index) else {
            break;
        };
        let x = area.x + offset as u16 * (card_w + gap);
        let card = Rect::new(x, cards_y, card_w, cover_h + 2);
        let hover = app.hover == Some(Action::ArtistAlbum(index));
        if hover {
            fill_soft(buf, card, mix(pal.surface, pal.primary, 0.14), 0.62, pal.bg);
        }

        let cover = Rect::new(x, cards_y, card_w, cover_h);
        // La clave lleva el prefijo `albumcard:` para que cada hueco de la
        // tira tenga su propia miniatura aunque el mismo álbum salga en otra
        // sección.
        let thumb_track = spote_protocol::TrackRef {
            key: format!("albumcard:{}:{}", album.provider, album.id),
            title: album.title.clone(),
            artist: if album.artist.is_empty() {
                fallback_artist.to_string()
            } else {
                album.artist.clone()
            },
            album: Some(album.title.clone()),
            duration_ms: 0,
            art_url: album.cover_url.clone(),
            source_hint: None,
        };
        draw_thumb(app, buf, cover, &thumb_track, pal);
        frame_art(buf, cover, pal);
        if hover {
            for offset_x in 0..cover.width {
                put(
                    buf,
                    cover.x + offset_x,
                    cover.y,
                    "─",
                    color(pal.primary),
                    None,
                );
                put(
                    buf,
                    cover.x + offset_x,
                    cover.y + cover.height - 1,
                    "─",
                    color(pal.primary),
                    None,
                );
            }
            for offset_y in 0..cover.height {
                put(
                    buf,
                    cover.x,
                    cover.y + offset_y,
                    "│",
                    color(pal.primary),
                    None,
                );
                put(
                    buf,
                    cover.x + cover.width - 1,
                    cover.y + offset_y,
                    "│",
                    color(pal.primary),
                    None,
                );
            }
            put(buf, cover.x, cover.y, "╭", color(pal.primary), None);
            put(
                buf,
                cover.x + cover.width - 1,
                cover.y,
                "╮",
                color(pal.primary),
                None,
            );
            put(
                buf,
                cover.x,
                cover.y + cover.height - 1,
                "╰",
                color(pal.primary),
                None,
            );
            put(
                buf,
                cover.x + cover.width - 1,
                cover.y + cover.height - 1,
                "╯",
                color(pal.primary),
                None,
            );
        }

        let title = fit(&album.title, card_w);
        link_at(
            buf,
            x,
            cards_y + cover_h,
            chars_w(&title),
            &title,
            pal,
            if hover { pal.primary } else { pal.text },
            hover,
            app.ui_time(),
        );
        let year = album
            .release_year
            .map(|year| year.to_string())
            .unwrap_or_else(|| "—".into());
        let meta = if album.track_count > 0 {
            format!("{year} · {} pistas", album.track_count)
        } else {
            format!("{year} · sin pistas")
        };
        let meta = fit(&meta, card_w);
        put(
            buf,
            x,
            cards_y + cover_h + 1,
            &meta,
            color(if hover { pal.muted } else { pal.dim }),
            None,
        );
        app.hit.add(card, Action::ArtistAlbum(index));
    }

    app.detail_albums_area = Rect::new(area.x, cards_y, area.width, cover_h + 2);
}

fn draw_artist_tracks(
    app: &mut App,
    buf: &mut Buffer,
    area: Rect,
    pal: &Palette,
    tracks: &[spote_core::spotify_api::SpotifyTrack],
    selected_raw: usize,
) {
    if tracks.is_empty() || area.width < 4 || area.height == 0 {
        return;
    }
    let len = tracks.len();
    let selected = selected_raw.min(len - 1);
    // En paneles estrechos, título y artista en dos líneas. Si hay muy poco
    // alto se vuelve a una línea para no quedarse sin pistas.
    let two_line = area.width < 52 && area.height >= 4;
    let row_h = if two_line { 2 } else { 1 };
    let rows = (area.height / row_h) as usize;
    if rows == 0 {
        return;
    }
    let start = super::lists::keep_visible(0, selected, rows, len);
    let bottom = area.y + area.height;

    for offset in 0..rows {
        let index = start + offset;
        let Some(track) = tracks.get(index) else {
            break;
        };
        let y = area.y + offset as u16 * row_h;
        if y + row_h > bottom {
            break;
        }
        let row = Rect::new(area.x, y, area.width, row_h);
        let is_selected = index == selected;
        let hover = app.hover == Some(Action::ArtistTrack(index));
        if is_selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.20));
            put(buf, row.x, y, "▍", color(pal.primary), None);
        } else if hover {
            fill(buf, row, pal.surface_alt);
        }

        let duration = format_ms(track.duration_ms);
        let duration_w = chars_w(&duration);
        let title_x = row.x + 4;
        let show_artist = !two_line && row.width >= 52;
        let artist_w = if show_artist {
            ((row.width as f32 * 0.32) as u16).clamp(8, 18)
        } else {
            0
        };
        let artist_x = if show_artist {
            row.x + row.width - duration_w - 1 - artist_w
        } else {
            row.x + row.width
        };
        let title_end = if show_artist {
            artist_x.saturating_sub(1)
        } else if row.width > duration_w + 6 {
            row.x + row.width - duration_w - 1
        } else {
            row.x + row.width
        };

        let number = if hover {
            format!("{:>2}", "▶")
        } else {
            format!("{:>2}", index + 1)
        };
        put(
            buf,
            row.x + 1,
            y,
            &number,
            color(if is_selected { pal.primary } else { pal.dim }),
            None,
        );

        let title_w = title_end.saturating_sub(title_x);
        let title = fit(&track.title, title_w);
        text_at(
            buf,
            Rect::new(title_x, y, title_w, 1),
            title,
            Style::new().fg(color(if is_selected || hover {
                pal.text
            } else {
                pal.muted
            })),
            Alignment::Left,
        );
        if show_artist {
            let artist = fit(&track.artists, artist_w);
            text_at(
                buf,
                Rect::new(artist_x, y, artist_w, 1),
                artist,
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
        if row.width > duration_w + 2 {
            put(
                buf,
                row.x + row.width - duration_w,
                y,
                &duration,
                color(pal.dim),
                None,
            );
        }
        if two_line {
            let artist_w = row.width.saturating_sub(4);
            let artist = fit(&track.artists, artist_w);
            text_at(
                buf,
                Rect::new(title_x, y + 1, artist_w, 1),
                artist,
                Style::new().fg(color(pal.dim)),
                Alignment::Left,
            );
        }
        app.hit.add(row, Action::ArtistTrack(index));
    }
}

// ---------------------------------------------------------------------------
// Álbum
// ---------------------------------------------------------------------------

fn draw_album(app: &mut App, buf: &mut Buffer, area: Rect, pal: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(page) = app.album_page.clone() else {
        let loading = app.album_page_loading;
        empty_state(
            app,
            buf,
            area,
            pal,
            "▤",
            if loading {
                "cargando álbum"
            } else {
                "sin álbum abierto"
            },
            if loading {
                "preparando la ficha…"
            } else {
                "abre un álbum desde la biblioteca o una búsqueda"
            },
            loading,
        );
        return;
    };

    let bottom = area.y + area.height;
    let show_image = area.width >= 42 && area.height >= 10;
    let show_meta = area.height >= 3;
    let show_chips = area.height >= 5;
    let text_rows = 2 + u16::from(show_meta) + u16::from(show_chips);
    let cover_w = if show_image { 12 } else { 0 };
    let cover_h = cover_w / 2;
    let hero_h = cover_h.max(text_rows).min(area.height);

    if hero_h >= 2 {
        fill_soft(
            buf,
            Rect::new(area.x, area.y, area.width, hero_h),
            mix(pal.surface, pal.primary, 0.10),
            0.55,
            pal.bg,
        );
    }

    if show_image {
        let cover = Rect::new(area.x, area.y, cover_w, cover_h);
        app.request_album_page_art(&page, cover.width, cover.height);
        let thumb_track = spote_protocol::TrackRef {
            key: format!("albumpage:{}:{}", page.provider, page.id),
            title: page.title.clone(),
            artist: page.artist.clone(),
            album: Some(page.title.clone()),
            duration_ms: 0,
            art_url: page.cover_url.clone(),
            source_hint: None,
        };
        draw_thumb(app, buf, cover, &thumb_track, pal);
        frame_art(buf, cover, pal);
    }

    let text_x = area.x + if show_image { cover_w + 2 } else { 0 };
    let text_w = area.width.saturating_sub(text_x - area.x);
    let text_y = if show_image {
        area.y + hero_h.saturating_sub(text_rows) / 2
    } else {
        area.y
    };

    let title = if page.title.trim().is_empty() {
        "álbum sin título".to_string()
    } else {
        page.title.clone()
    };
    let title = fit(&title, text_w);
    text_at(
        buf,
        Rect::new(text_x, text_y, text_w, 1),
        title,
        Style::new()
            .fg(color(pal.text))
            .add_modifier(Modifier::BOLD),
        Alignment::Left,
    );

    if !page.artist.is_empty() && text_y + 1 < area.y + hero_h {
        draw_clickable_artist(app, buf, text_x, text_y + 1, text_w, pal, &page.artist);
    }

    if show_meta && text_y + 2 < area.y + hero_h {
        let ready = page
            .tracks
            .iter()
            .filter(|track| {
                app.lib
                    .local_path(&track.key)
                    .is_some_and(|path| path.exists())
            })
            .count();
        let mut parts: Vec<(String, Rgb)> = Vec::new();
        if let Some(year) = page.release_date.as_deref().and_then(year_of) {
            parts.push((year, pal.dim));
        }
        if !page.tracks.is_empty() {
            parts.push((format!("{} pistas", page.tracks.len()), pal.muted));
            if ready > 0 {
                parts.push((
                    if ready == page.tracks.len() {
                        format!("{ready}/{} · completo", page.tracks.len())
                    } else {
                        format!("{ready}/{} en local", page.tracks.len())
                    },
                    if ready == page.tracks.len() {
                        pal.ok
                    } else {
                        pal.secondary
                    },
                ));
            }
        }
        draw_segments(buf, text_x, text_y + 2, text_w, pal, &parts);
    }

    if show_chips && text_y + 3 < area.y + hero_h {
        let enabled = !app.album_page_loading && !page.tracks.is_empty();
        let compact = text_w < 26;
        let download = if compact {
            "↓ bajar"
        } else {
            "↓ descargar"
        };
        let save = if compact { "+ lista" } else { "+ guardar" };
        let mut x = text_x;
        for (label, action, accent) in [
            (download, Action::AlbumPageDownload, pal.warn),
            (save, Action::AlbumPageSave, pal.secondary),
        ] {
            let width = chars_w(label) + 2;
            if x + width > text_x + text_w {
                break;
            }
            let chip = Rect::new(x, text_y + 3, width, 1);
            draw_chip(app, buf, chip, pal, label, action, accent, enabled);
            x += width + 1;
        }
    }

    let mut y = area.y + hero_h;
    if bottom.saturating_sub(y) >= 9 {
        y += 1;
    }
    if y >= bottom {
        return;
    }

    let remaining = bottom.saturating_sub(y);
    if page.tracks.is_empty() {
        if remaining == 0 {
            return;
        }
        let show_header = remaining >= 3;
        if show_header {
            draw_section_header(buf, Rect::new(area.x, y, area.width, 1), pal, "PISTAS", 0);
            y += 1;
        }
        let message = if app.album_page_loading {
            "cargando pistas…"
        } else {
            "este álbum no tiene pistas"
        };
        let (glyph, fg) = if app.album_page_loading {
            (spinner(app), pal.warn)
        } else {
            ("·", pal.dim)
        };
        put(buf, area.x, y, glyph, color(fg), None);
        let message = fit(message, area.width.saturating_sub(2));
        put(buf, area.x + 2, y, &message, color(pal.muted), None);
        if remaining >= 2 {
            let hint = if app.album_page_loading {
                "consultando el catálogo…"
            } else {
                "prueba desde la búsqueda o vuelve atrás"
            };
            let hint = fit(hint, area.width);
            put(buf, area.x, y + 1, &hint, color(pal.dim), None);
        }
        return;
    }

    if remaining >= 3 {
        draw_section_header(buf, Rect::new(area.x, y, area.width, 1), pal, "PISTAS", 0);
        y += 1;
    }
    draw_album_tracks(
        app,
        buf,
        Rect::new(area.x, y, area.width, bottom - y),
        pal,
        &page,
    );
}

fn draw_album_tracks(app: &mut App, buf: &mut Buffer, area: Rect, pal: &Palette, page: &AlbumPage) {
    if page.tracks.is_empty() || area.width < 10 || area.height == 0 {
        return;
    }
    let len = page.tracks.len();
    let selected = page.selected.min(len - 1);
    let rows = area.height as usize;
    let start = super::lists::keep_visible(0, selected, rows, len);
    let bottom = area.y + area.height;

    for offset in 0..rows {
        let index = start + offset;
        let Some(track) = page.tracks.get(index) else {
            break;
        };
        let y = area.y + offset as u16;
        if y >= bottom {
            break;
        }
        let row = Rect::new(area.x, y, area.width, 1);
        let is_selected = index == selected;
        let hover = app.hover == Some(Action::AlbumTrackPlay(index));
        if is_selected {
            fill(buf, row, mix(pal.surface, pal.primary, 0.20));
        } else if hover {
            fill(buf, row, pal.surface_alt);
        }

        let ready = app
            .lib
            .local_path(&track.key)
            .is_some_and(|path| path.exists());
        let playing = app
            .now
            .track
            .as_ref()
            .is_some_and(|now| now.key == track.key);
        put(
            buf,
            row.x,
            y,
            if ready { "✓" } else { "↓" },
            color(if ready { pal.ok } else { pal.warn }),
            None,
        );

        let number = if playing || hover {
            format!("{:>2}", "▶")
        } else {
            format!("{:>2}", index + 1)
        };
        put(
            buf,
            row.x + 2,
            y,
            &number,
            color(if playing || is_selected || hover {
                pal.primary
            } else {
                pal.dim
            }),
            None,
        );

        let duration = format_ms(track.duration_ms);
        let duration_w = chars_w(&duration);
        let title_x = row.x + 5;
        let title_end = if row.width > duration_w + 8 {
            row.x + row.width - duration_w - 1
        } else {
            row.x + row.width
        };
        let title_w = title_end.saturating_sub(title_x);
        let title = fit(&track.title, title_w);
        text_at(
            buf,
            Rect::new(title_x, y, title_w, 1),
            title,
            Style::new().fg(color(if is_selected || playing || hover {
                pal.text
            } else {
                pal.muted
            })),
            Alignment::Left,
        );
        if row.width > duration_w + 2 {
            put(
                buf,
                row.x + row.width - duration_w,
                y,
                &duration,
                color(pal.dim),
                None,
            );
        }
        app.hit.add(row, Action::AlbumTrackPlay(index));
    }
}

// ---------------------------------------------------------------------------
// Piezas compartidas
// ---------------------------------------------------------------------------

fn draw_clickable_artist(
    app: &mut App,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    pal: &Palette,
    artist: &str,
) {
    if artist.is_empty() || width < 2 {
        return;
    }
    let hover = app.hover == Some(Action::OpenArtist(artist.to_string()));
    let show_arrow = width >= 5;
    let label = fit(artist, if show_arrow { width - 2 } else { width });
    let label_w = link_at(
        buf,
        x,
        y,
        chars_w(&label),
        &label,
        pal,
        pal.secondary,
        hover,
        app.ui_time(),
    );
    if label_w == 0 {
        return;
    }
    let mut hit_w = label_w;
    if show_arrow {
        let arrow_fg = if hover {
            mix_oklab(pal.primary, pal.text, 0.35)
        } else {
            pal.primary
        };
        put(buf, x + label_w, y, " ↗", color(arrow_fg), None);
        hit_w += 2;
    }
    app.hit.add(
        Rect::new(x, y, hit_w, 1),
        Action::OpenArtist(artist.to_string()),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_chip(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    label: &str,
    action: Action,
    accent: Rgb,
    enabled: bool,
) {
    if rect.width < 2 || rect.height == 0 {
        return;
    }
    let hover = enabled && app.hover == Some(action.clone());
    let bg = if hover {
        mix(pal.surface_alt, accent, 0.30)
    } else if enabled {
        mix(pal.surface, pal.bg, 0.30)
    } else {
        mix(pal.surface, pal.bg, 0.55)
    };
    fill(buf, rect, bg);
    text_at(
        buf,
        rect,
        label.to_string(),
        Style::new()
            .fg(color(if !enabled {
                pal.dim
            } else if hover {
                pal.text
            } else {
                accent
            }))
            .add_modifier(if hover {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        Alignment::Center,
    );
    if enabled {
        app.hit.add(rect, action);
    }
}

fn draw_section_header(buf: &mut Buffer, rect: Rect, pal: &Palette, label: &str, reserved: u16) {
    if rect.width < 3 || rect.height == 0 {
        return;
    }
    put(buf, rect.x, rect.y, "▎", color(pal.primary), None);
    let max_label = rect.width.saturating_sub(3 + reserved).max(1);
    let label = fit(&label.to_uppercase(), max_label);
    let label_w = chars_w(&label);
    if label_w > 0 {
        text_at(
            buf,
            Rect::new(rect.x + 2, rect.y, label_w, 1),
            label,
            Style::new()
                .fg(color(pal.text))
                .add_modifier(Modifier::BOLD),
            Alignment::Left,
        );
    }
    let rule_x = rect.x + 2 + label_w + 1;
    let rule_end = rect.x + rect.width.saturating_sub(reserved);
    for x in rule_x..rule_end {
        let t = (x - rule_x) as f32 / (rule_end.saturating_sub(rule_x)).max(1) as f32;
        let c = mix_oklab(pal.border_dim, pal.surface, 0.20 + t * 0.45);
        put(buf, x, rect.y, "─", color(c), None);
    }
}

fn draw_arrow(
    app: &mut App,
    buf: &mut Buffer,
    rect: Rect,
    pal: &Palette,
    glyph: &str,
    enabled: bool,
    action: Action,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    if !enabled {
        text_at(
            buf,
            rect,
            glyph.to_string(),
            Style::new().fg(color(pal.border_dim)),
            Alignment::Center,
        );
        return;
    }
    let hover = app.hover == Some(action.clone());
    if hover {
        fill(buf, rect, mix(pal.surface, pal.primary, 0.20));
    }
    text_at(
        buf,
        rect,
        glyph.to_string(),
        Style::new()
            .fg(color(if hover { pal.text } else { pal.secondary }))
            .add_modifier(if hover {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        Alignment::Center,
    );
    app.hit.add(rect, action);
}

fn draw_segments(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    pal: &Palette,
    parts: &[(String, Rgb)],
) {
    if width == 0 || parts.is_empty() {
        return;
    }
    let right = x + width;
    let mut cx = x;
    let mut first = true;
    for (text, fg) in parts {
        if text.is_empty() {
            continue;
        }
        if !first {
            if cx + 3 > right {
                break;
            }
            put(buf, cx, y, " · ", color(pal.dim), None);
            cx += 3;
        }
        let available = right.saturating_sub(cx);
        if available == 0 {
            break;
        }
        let full_w = chars_w(text);
        let text = fit(text, available);
        let text_w = chars_w(&text);
        if text_w == 0 {
            break;
        }
        // `text_at` recorta por celdas: un género en japonés no puede
        // derramarse sobre el borde aunque el conteo sea por caracteres.
        text_at(
            buf,
            Rect::new(cx, y, available, 1),
            text,
            Style::new().fg(color(*fg)),
            Alignment::Left,
        );
        cx += text_w;
        if full_w > available {
            break;
        }
        first = false;
    }
}

fn popularity_bar(buf: &mut Buffer, x: u16, y: u16, width: u16, pal: &Palette, pct: u32) {
    if width < 5 {
        return;
    }
    let pct = pct.min(100);
    let label = pct.to_string();
    let label_w = chars_w(&label);
    let bar_w = width.saturating_sub(label_w + 1).min(28);
    if bar_w < 3 {
        return;
    }
    let filled = ((bar_w as f32 * pct as f32 / 100.0).round() as u16).min(bar_w);
    for offset in 0..bar_w {
        let t = offset as f32 / bar_w.max(1) as f32;
        let c = if offset < filled {
            mix_oklab(pal.primary, pal.secondary, t)
        } else {
            mix(pal.border_dim, pal.surface, 0.18)
        };
        put(
            buf,
            x + offset,
            y,
            if offset < filled { "━" } else { "─" },
            color(c),
            None,
        );
    }
    put(buf, x + bar_w + 1, y, &label, color(pal.muted), None);
}

fn art_placeholder(buf: &mut Buffer, rect: Rect, pal: &Palette, label: &str) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let top = mix(pal.surface, pal.primary, 0.30);
    let bottom = mix(pal.surface, pal.secondary, 0.18);
    for row in 0..rect.height {
        let t = if rect.height <= 1 {
            0.0
        } else {
            row as f32 / (rect.height - 1) as f32
        };
        fill(
            buf,
            Rect::new(rect.x, rect.y + row, rect.width, 1),
            mix_oklab(top, bottom, t),
        );
    }
    let initial = label
        .chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or('♪')
        .to_uppercase()
        .next()
        .unwrap_or('♪');
    text_at(
        buf,
        Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1),
        initial.to_string(),
        Style::new()
            .fg(color(pal.text))
            .add_modifier(Modifier::BOLD),
        Alignment::Center,
    );
}

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

fn spinner(app: &App) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[(app.viz.time * 9.0) as usize % FRAMES.len()]
}

fn chars_w(text: &str) -> u16 {
    text.chars().count().min(u16::MAX as usize) as u16
}

/// Recorta por caracteres y añade elipsis. Nunca parte un carácter multibyte.
fn fit(text: &str, max: u16) -> String {
    let max = max as usize;
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn format_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 10_000 {
        format!("{:.0}k", count as f64 / 1_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn year_of(date: &str) -> Option<String> {
    let head: String = date.chars().take(4).collect();
    if head.len() == 4 && head.chars().all(|c| c.is_ascii_digit()) {
        Some(head)
    } else {
        None
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
