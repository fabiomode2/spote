//! Imágenes reales en terminales Kitty: la portada a resolución completa.
//!
//! La TUI sigue dibujando su versión half-block como fallback. Cuando el
//! terminal habla el protocolo de Kitty, se transmite el PNG una sola vez y se
//! coloca encima con el tamaño exacto del hueco. No se reenvía cada frame.

use std::io::Write;
use std::sync::OnceLock;

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
    /// Secuencia de ids de la portada; no se reutiliza al borrar la imagen.
    image_seq: u32,
    sent_key: String,
    sent_rect: Option<Rect>,
    /// Hash del PNG ya transmitido: si cambia el contenido con la misma clave
    /// y el mismo hueco hay que reenviarlo igualmente.
    sent_hash: u64,
    /// Wallpaper de fondo: imagen independiente, colocada con z=-1.
    wallpaper_id: u32,
    wallpaper_key: String,
    wallpaper_rect: Option<Rect>,
    wallpaper_hash: u64,
    wallpaper_seq: u32,
    /// Una imagen por miniatura visible. Se indexa por la clave de
    /// colocación (pista + posición) para que la misma portada pueda salir
    /// dos veces en pantalla sin compartir id.
    thumbs: std::collections::HashMap<String, ThumbImage>,
    thumb_seq: u32,
    /// Hash de contenido de cada PNG de miniatura, calculado una vez. La
    /// entrada guarda también el tamaño: si cambia, se recalcula. Antes la
    /// clave era `(String, usize)` y se asignaba una cadena nueva en cada
    /// frame.
    thumb_prints: std::collections::HashMap<String, (usize, u64)>,
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
        self.thumb_prints.clear();
        self.sent_key.clear();
        self.sent_rect = None;
        self.sent_hash = 0;
        if self.wallpaper_id != 0 {
            let _ = delete(out, self.wallpaper_id);
            self.wallpaper_id = 0;
        }
        self.wallpaper_key.clear();
        self.wallpaper_rect = None;
        self.wallpaper_hash = 0;
        let _ = out.flush();
    }
}

/// El soporte de imágenes del terminal no cambia en toda la sesión: se
/// consultaba el entorno en cada frame.
fn kitty_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("SPOTE_NO_KITTY").is_some() {
            return false;
        }
        match std::env::var("SPOTE_IMAGES").as_deref() {
            Ok("off") | Ok("none") => return false,
            Ok("kitty") | Ok("on") => return true,
            _ => {}
        }
        // Sin passthrough de tmux, mandar Kitty graphics dentro de una sesión
        // deja basura. Se usa el fallback half-block salvo override explícito.
        if std::env::var_os("TMUX").is_some() {
            return false;
        }
        // Kitty graphics lo hablan Kitty, WezTerm, Ghostty y Konsole reciente.
        // iTerm2 usa otro protocolo y Alacritty/foot/xterm no lo soportan.
        if std::env::var_os("KITTY_WINDOW_ID").is_some()
            || std::env::var_os("WEZTERM_PANE").is_some()
            || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
            || std::env::var_os("KONSOLE_VERSION").is_some()
        {
            return true;
        }
        match std::env::var("TERM_PROGRAM").as_deref() {
            Ok("WezTerm") | Ok("ghostty") | Ok("konsole") => return true,
            _ => {}
        }
        std::env::var("TERM")
            .map(|term| term.contains("kitty") || term.contains("wezterm"))
            .unwrap_or(false)
    })
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
    z: i32,
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
                "\x1b_Ga=T,f=100,i={id},p=1,s={},v={},c={cols},r={rows},C=1,z={z},q=2,m={marker};",
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

/// Coloca una imagen ya transmitida en otro hueco sin reenviar el PNG.
/// El id de colocación `p=1` hace que el terminal mueva la colocación
/// existente en vez de crear otra: sin él, cada recolocación dejaba un
/// fantasma en la posición anterior.
fn place(out: &mut impl Write, id: u32, rect: Rect, z: i32) -> std::io::Result<()> {
    let cols = rect.width.max(1);
    let rows = rect.height.max(1);
    write!(out, "\x1b[{};{}H", rect.y + 1, rect.x + 1)?;
    write!(
        out,
        "\x1b_Ga=p,i={id},p=1,c={cols},r={rows},C=1,z={z},q=2;\x1b\\"
    )
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

    if let Some((rect, png, size)) = wanted {
        let hash = content_hash(png);
        let same_image =
            state.image_id != 0 && state.sent_key == app.art_png_key && state.sent_hash == hash;
        if same_image {
            // La misma imagen sigue en la terminal (el terminal no olvida los
            // ids al redibujar texto). Si solo cambió el hueco, recolocarla es
            // mucho más barato que reenviarla.
            if state.sent_rect != Some(rect) && place(out, state.image_id, rect, 1).is_ok() {
                state.sent_rect = Some(rect);
            }
        } else {
            // Clave nueva, contenido nuevo o imagen perdida: se borra la vieja
            // y se transmite la nueva.
            if state.image_id != 0 {
                let _ = delete(out, state.image_id);
                state.image_id = 0;
            }
            state.sent_rect = None;
            state.sent_key.clear();
            state.sent_hash = 0;
            state.image_seq = state.image_seq.wrapping_add(1).max(1);
            // IDs únicos por proceso: el desplazamiento evita reutilizar uno de
            // otra sesión que la terminal aún recuerde.
            let id = std::process::id().wrapping_shl(8) ^ state.image_seq;
            if transmit(out, id, png, size, rect, 1).is_ok() {
                state.image_id = id;
                state.sent_rect = Some(rect);
                state.sent_key = app.art_png_key.clone();
                state.sent_hash = hash;
            }
        }
    } else if state.image_id != 0 {
        let _ = delete(out, state.image_id);
        state.image_id = 0;
        state.sent_rect = None;
        state.sent_key.clear();
        state.sent_hash = 0;
    }
    render_wallpaper(out, app, state);
    render_thumbs(out, app, state);
    let _ = out.flush();
}

/// Wallpaper de portada a pantalla completa. Va con z=-1 para que el texto y
/// los paneles queden por encima. Solo se envía cuando cambia la portada o el
/// tamaño; no se reencodea por frame.
fn render_wallpaper(out: &mut impl Write, app: &App, state: &mut HiResState) {
    let wanted = app
        .wallpaper_rect
        .filter(|rect| rect.width >= 8 && rect.height >= 4)
        .and_then(|rect| app.art_png.as_ref().map(|png| (rect, png)));
    let Some((rect, png)) = wanted else {
        if state.wallpaper_id != 0 {
            let _ = delete(out, state.wallpaper_id);
            state.wallpaper_id = 0;
            state.wallpaper_key.clear();
            state.wallpaper_rect = None;
            state.wallpaper_hash = 0;
        }
        return;
    };
    let hash = content_hash(png);
    let same = state.wallpaper_id != 0
        && state.wallpaper_key == app.art_png_key
        && state.wallpaper_hash == hash;
    if same {
        if state.wallpaper_rect != Some(rect) {
            let _ = place(out, state.wallpaper_id, rect, -1);
            state.wallpaper_rect = Some(rect);
        }
        return;
    }
    if state.wallpaper_id != 0 {
        let _ = delete(out, state.wallpaper_id);
        state.wallpaper_id = 0;
    }
    state.wallpaper_seq = state.wallpaper_seq.wrapping_add(1).max(1);
    let id = std::process::id().wrapping_shl(12) ^ 0x8000_0000 ^ state.wallpaper_seq;
    let size = app.art_png_size.unwrap_or((1, 1));
    if transmit(out, id, png, size, rect, -1).is_ok() {
        state.wallpaper_id = id;
        state.wallpaper_key = app.art_png_key.clone();
        state.wallpaper_rect = Some(rect);
        state.wallpaper_hash = hash;
    }
}

/// FNV-1a barato y estable sobre una muestra del PNG. Recorrer todos los bytes
/// en cada frame costaba; alcanza para detectar que el contenido cambió.
fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes.iter().step_by(7).take(512).chain(bytes.last()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

impl HiResState {
    /// El mismo PNG se reutiliza en todas las miniaturas de una pista; su
    /// hash también. Solo se asigna una cadena nueva cuando falta la entrada.
    fn print_of(&mut self, image_key: &str, png: &[u8]) -> u64 {
        if let Some((len, value)) = self.thumb_prints.get(image_key) {
            if *len == png.len() {
                return *value;
            }
        }
        let value = content_hash(png);
        self.thumb_prints
            .insert(image_key.to_string(), (png.len(), value));
        value
    }
}

/// Coloca las miniaturas como imágenes reales (Kitty). El half-block que
/// dibuja ratatui queda debajo como respaldo para terminales sin soporte.
fn render_thumbs(out: &mut impl Write, app: &App, state: &mut HiResState) {
    let show = app.overlay == Overlay::None && app.fullscreen != Fullscreen::Viz && !app.zen;
    // Vec en vez de HashMap: son un puñado de miniaturas y así no se asigna
    // una tabla nueva en cada frame.
    let mut visible: Vec<(&str, &str, Rect, u64)> = Vec::with_capacity(app.hi_thumbs.len());
    if show {
        for (place_key, image_key, rect) in &app.hi_thumbs {
            if rect.width < 4 || rect.height < 2 {
                continue;
            }
            let Some((png, _)) = app.thumb_pngs.get(image_key) else {
                continue;
            };
            let print = state.print_of(image_key, png);
            visible.push((place_key.as_str(), image_key.as_str(), *rect, print));
        }
    }

    // Entradas que ya no se ven o cuyo contenido cambió. Se extraen con
    // propiedad para poder reutilizar su id si la misma imagen reaparece en
    // otra celda (scroll): recolocar es gratis, retransmitir no.
    let mut stale: Vec<(String, ThumbImage)> = if state.thumbs.is_empty() {
        Vec::new()
    } else {
        state
            .thumbs
            .extract_if(|key, image| {
                !visible.iter().any(|(place, image_key, rect, print)| {
                    place == &key.as_str()
                        && *image_key == image.image_key
                        && *rect == image.rect
                        && *print == image.fingerprint
                })
            })
            .collect()
    };

    for (place_key, image_key, rect, print) in visible {
        if state.thumbs.contains_key(place_key) {
            continue;
        }
        // La misma portada en otro hueco: se mueve el id ya transmitido.
        if let Some(index) = stale
            .iter()
            .position(|(_, image)| image.image_key == image_key && image.fingerprint == print)
        {
            let (_, image) = stale.swap_remove(index);
            if place(out, image.id, rect, 1).is_ok() {
                state
                    .thumbs
                    .insert(place_key.to_string(), ThumbImage { rect, ..image });
                continue;
            }
            let _ = delete(out, image.id);
        }
        let Some((png, size)) = app.thumb_pngs.get(image_key) else {
            continue;
        };
        state.thumb_seq = state.thumb_seq.wrapping_add(1);
        let id = std::process::id().wrapping_shl(12) ^ 0x4000_0000 ^ state.thumb_seq;
        if transmit(out, id, png, *size, rect, 1).is_ok() {
            state.thumbs.insert(
                place_key.to_string(),
                ThumbImage {
                    id,
                    image_key: image_key.to_string(),
                    rect,
                    fingerprint: print,
                },
            );
        }
    }
    for (_, image) in stale {
        let _ = delete(out, image.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_reuses_the_id_without_a_payload() {
        let mut out = Vec::new();
        place(&mut out, 42, Rect::new(3, 2, 10, 5), 1).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("\x1b[3;4H"), "{text:?}");
        assert!(
            text.contains("\x1b_Ga=p,i=42,p=1,c=10,r=5,C=1,z=1,q=2;\x1b\\"),
            "{text:?}"
        );
    }

    #[test]
    fn content_hash_notices_when_the_png_changes() {
        let a = vec![0u8; 4096];
        let mut b = a.clone();
        b[7] = 1;
        assert_eq!(content_hash(&a), content_hash(&a));
        assert_ne!(content_hash(&a), content_hash(&b));
    }
}
