//! Overlay de resumen de escucha (Wrapped).
use super::*;

pub(super) fn draw_wrapped(app: &mut App, buf: &mut Buffer, layout: &Layout, pal: &Palette) {
    let rect = overlay_frame(layout, pal, buf, 86, 26, "wrapped · resumen de escucha");
    app.hit.add(rect, Action::CloseOverlay);
    let inner = inset(rect, 2, 2);
    let Some(stats) = app.wrapped.clone() else {
        text_at(
            buf,
            Rect::new(inner.x, inner.y + 4, inner.width, 1),
            "sin datos de escucha todavía · reproduce algo y vuelve a abrir W",
            Style::new().fg(color(pal.dim)),
            Alignment::Center,
        );
        return;
    };

    let period = if stats.all_time {
        "todo el tiempo".to_string()
    } else {
        format!("{}", stats.year)
    };
    text_at(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        format!(
            "{}  ·  {} h {} min  ·  {} reproducciones  ·  {} artistas",
            period,
            stats.ms_played / 3_600_000,
            (stats.ms_played % 3_600_000) / 60_000,
            stats.plays,
            stats.distinct_artists
        ),
        Style::new().fg(color(pal.primary)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );

    let mut y = inner.y + 2;
    text_at(
        buf,
        Rect::new(inner.x, y, 44, 1),
        "CANCIONES MÁS ESCUCHADAS",
        Style::new().fg(color(pal.secondary)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    text_at(
        buf,
        Rect::new(inner.x + 46, y, inner.width.saturating_sub(46), 1),
        "ARTISTAS Y DISCOS",
        Style::new().fg(color(pal.secondary)).add_modifier(Modifier::BOLD),
        Alignment::Left,
    );
    y += 1;

    if stats.top_tracks.is_empty() {
        text_at(
            buf,
            Rect::new(inner.x, y, 44, 1),
            "sin reproducciones en este periodo",
            Style::new().fg(color(pal.dim)),
            Alignment::Left,
        );
    }
    for (index, track) in stats.top_tracks.iter().take(8).enumerate() {
        let row = Rect::new(inner.x, y + index as u16, 44, 1);
        let title = truncate(&format!("{}. {} · {}", index + 1, track.title, track.artist), 42);
        put(
            buf,
            row.x,
            row.y,
            &title,
            color(if index == 0 { pal.text } else { pal.muted }),
            None,
        );
        let time = format!("{}m", (track.ms_played / 60_000).max(1));
        put(
            buf,
            inner.x + 45 - time.len() as u16,
            row.y,
            &time,
            color(pal.dim),
            None,
        );
    }

    for (index, artist) in stats.top_artists.iter().take(4).enumerate() {
        let row = Rect::new(inner.x + 46, y + index as u16, inner.width.saturating_sub(46), 1);
        put(
            buf,
            row.x,
            row.y,
            &truncate(
                &format!(
                    "♪ {}  ·  {}m  ·  {}x",
                    artist.artist,
                    (artist.ms_played / 60_000).max(1),
                    artist.plays
                ),
                row.width.saturating_sub(1) as usize,
            ),
            color(if index == 0 { pal.text } else { pal.muted }),
            None,
        );
    }
    let albums_y = y + 5;
    for (index, album) in stats.top_albums.iter().take(4).enumerate() {
        let row = Rect::new(
            inner.x + 46,
            albums_y + index as u16,
            inner.width.saturating_sub(46),
            1,
        );
        put(
            buf,
            row.x,
            row.y,
            &truncate(&format!("▤ {} · {}", album.album, album.artist), row.width.saturating_sub(1) as usize),
            color(pal.muted),
            None,
        );
    }

    let chart_y = inner.y + inner.height - 3;
    text_at(
        buf,
        Rect::new(inner.x, chart_y, 44, 1),
        "por meses",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
    let max = stats.by_month.iter().copied().max().unwrap_or(0).max(1);
    let bars = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    for (month, ms) in stats.by_month.iter().enumerate() {
        let level = ((*ms as f32 / max as f32) * 8.0).round() as usize;
        put(
            buf,
            inner.x + month as u16 * 3,
            chart_y + 1,
            bars[level.min(8)],
            color(if *ms > 0 { pal.primary } else { pal.border_dim }),
            None,
        );
    }
    text_at(
        buf,
        Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        "a  este año / todo  ·  esc  cerrar",
        Style::new().fg(color(pal.dim)),
        Alignment::Left,
    );
}

fn truncate(text: &str, max: usize) -> String {
    let mut out = text.to_string();
    truncate_chars(&mut out, max);
    out
}
