//! Imágenes reales en terminales Kitty: la portada a resolución completa.
//!
//! La TUI sigue dibujando su versión half-block como fallback. Cuando el
//! terminal habla el protocolo de Kitty, se transmite el PNG una sola vez y se
//! coloca encima con el tamaño exacto del hueco. No se reenvía cada frame.

use std::io::Write;

use ratatui::layout::Rect;

use crate::app::{App, Fullscreen, Overlay};
use spote_core::artwork::ArtMode;

#[derive(Debug)]
struct ThumbImage {
    id: u32,
    image_key: String,
    rect: Rect,
    fingerprint: u64,
}

#[derive(Debug, Default)]
pub struct HiResState {
    image_id: u32,
    sent_key: String,
    sent_rect: Option<Rect>,
    /// Una imagen por miniatura visible. Se indexa por la clave de
    /// colocación (pista + posición) para que la misma portada pueda salir
    /// dos veces en pantalla sin compartir id.
    thumbs: std::collections::HashMap<String, ThumbImage>,
    thumb_seq: u32,
}

impl HiResState {
    /// Borra la imagen y olvida el estado: tras un clear de pantalla o un
    /// cambio de layout hay que volver a colocarla.
    pub fn reset(&mut self, out: &mut impl Write) {
        if self.image_id != 0 {
            let _ = delete(out, self.image_id);
            self.image_id = 0;
        }
        for image in self.thumbs.values() {
            let _ = delete(out, image.id);
        }
        self.thumbs.clear();
        self.sent_key.clear();
        self.sent_rect = None;
        let _ = out.flush();
    }
}

fn kitty_enabled() -> bool {
    if std::env::var_os("SPOTE_NO_KITTY").is_some() {
        return false;
    }
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM")
            .map(|term| term.contains("kitty"))
            .unwrap_or(false)
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn delete(out: &mut impl Write, id: u32) -> std::io::Result<()> {
    if id != 0 {
        write!(out, "\x1b_Ga=d,d=I,i={id},q=2;\x1b\\")?;
    }
    Ok(())
}

fn transmit(
    out: &mut impl Write,
    id: u32,
    png: &[u8],
    size: (u32, u32),
    rect: Rect,
) -> std::io::Result<()> {
    let encoded = base64(png);
    let cols = rect.width.max(1);
    let rows = rect.height.max(1);
    // El cursor se coloca en la esquina y `C=1` evita que el protocolo lo mueva.
    write!(out, "\x1b[{};{}H", rect.y + 1, rect.x + 1)?;
    let mut first = true;
    for (index, chunk) in encoded.as_bytes().chunks(4096).enumerate() {
        let last = (index + 1) * 4096 >= encoded.len();
        let marker = if last { 0 } else { 1 };
        if first {
            write!(
                out,
                "\x1b_Ga=T,f=100,i={id},s={},v={},c={cols},r={rows},C=1,z=1,q=2,m={marker};",
                size.0, size.1
            )?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={marker};")?;
        }
        out.write_all(chunk)?;
        write!(out, "\x1b\\")?;
    }
    Ok(())
}

/// Se llama después de `terminal.draw`: si cambió la portada o el hueco, borra
/// la imagen anterior y coloca la nueva. En terminales sin Kitty no hace nada.
pub fn render(out: &mut impl Write, app: &App, state: &mut HiResState) {
    if !kitty_enabled() {
        return;
    }
    let wanted = app
        .art_visible_rect
        .filter(|_| app.overlay == Overlay::None)
        .filter(|_| app.fullscreen != Fullscreen::Viz)
        .filter(|_| !app.zen || app.fullscreen == Fullscreen::Art)
        .filter(|_| app.art_mode == ArtMode::Plain)
        .filter(|rect| rect.width >= 12 && rect.height >= 6)
        .and_then(|rect| {
            let png = app.art_png.as_ref()?;
            let size = app.art_png_size?;
            // Un píxel de margen para no tapar el marco de la portada.
            let rect = Rect::new(
                rect.x + 1,
                rect.y + 1,
                rect.width.saturating_sub(2),
                rect.height.saturating_sub(2),
            );
            Some((rect, png, size))
        });

    let same = match (&wanted, state.sent_rect) {
        (Some((rect, _, _)), Some(last)) => {
            *rect == last && state.sent_key == app.art_png_key && state.image_id != 0
        }
        (None, None) => true,
        _ => false,
    };
    if !same {
        if state.image_id != 0 {
            let _ = delete(out, state.image_id);
            state.image_id = 0;
            state.sent_rect = None;
            state.sent_key.clear();
        }
        if let Some((rect, png, size)) = wanted {
            state.image_id = state.image_id.wrapping_add(1).max(1);
            // IDs únicos por proceso: el timestamp evita reutilizar uno de otra sesión.
            let id = std::process::id().wrapping_shl(8) ^ state.image_id;
            state.image_id = id;
            if transmit(out, id, png, size, rect).is_ok() {
                state.sent_rect = Some(rect);
                state.sent_key = app.art_png_key.clone();
            }
        }
    }
    render_thumbs(out, app, state);
    let _ = out.flush();
}

/// Fingerprint barato para saber si la portada de una miniatura cambió.
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes.iter().step_by(7).take(512).chain(bytes.last()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Coloca las miniaturas como imágenes reales (Kitty). El half-block que
/// dibuja ratatui queda debajo como respaldo para terminales sin soporte.
fn render_thumbs(out: &mut impl Write, app: &App, state: &mut HiResState) {
    use std::collections::HashMap;

    let show = app.overlay == Overlay::None && app.fullscreen != Fullscreen::Viz && !app.zen;
    let mut visible: HashMap<String, (&str, Rect, u64, &Vec<u8>, (u32, u32))> = HashMap::new();
    if show {
        for (place_key, image_key, rect) in &app.hi_thumbs {
            if rect.width < 4 || rect.height < 2 {
                continue;
            }
            let Some((png, size)) = app.thumb_pngs.get(image_key) else {
                continue;
            };
            visible.insert(
                place_key.clone(),
                (image_key.as_str(), *rect, fingerprint(png), png, *size),
            );
        }
    }

    // Fuera de pantalla o con arte nuevo: se borra antes de recolocar.
    state.thumbs.retain(|key, image| {
        let keep = visible
            .get(key)
            .is_some_and(|(image_key, rect, print, _, _)| {
                *image_key == image.image_key && *rect == image.rect && *print == image.fingerprint
            });
        if !keep {
            let _ = delete(out, image.id);
        }
        keep
    });

    for (place_key, (image_key, rect, print, png, size)) in visible {
        if state.thumbs.contains_key(&place_key) {
            continue;
        }
        state.thumb_seq = state.thumb_seq.wrapping_add(1);
        let id = std::process::id().wrapping_shl(12) ^ 0x4000_0000 ^ state.thumb_seq;
        if transmit(out, id, png, size, rect).is_ok() {
            state.thumbs.insert(
                place_key,
                ThumbImage {
                    id,
                    image_key: image_key.to_string(),
                    rect,
                    fingerprint: print,
                },
            );
        }
    }
}
