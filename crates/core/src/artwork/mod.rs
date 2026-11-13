//! Portada → paleta → arte ASCII. La portada dicta el tema de toda la UI.

mod palette;
mod render;

pub use palette::{extract_palette, Palette};
pub use render::render_cover;

use image::DynamicImage;

pub type Rgb = (u8, u8, u8);

/// Modos de dibujar la portada en la terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArtMode {
    /// Pixels ordenados con Bayer 8x8 y cuantizados a la rampa de la paleta.
    /// Es el look "TUI": se lee como arte, no como foto comprimida.
    Dither,
    /// Media celda: `▀` con color arriba y color abajo. Máxima fidelidad y el
    /// que mejor aguanta una portada grande como imagen principal.
    #[default]
    Half,
    /// Media celda con los colores originales de la imagen: la portada tal
    /// cual, sin cuantizar a la paleta ni pulsos. Es el modo "foto".
    Plain,
    /// Braille (2x4 puntos por celda), tinta plana del acento.
    Braille,
    /// Sin arte: solo el degradado de fondo.
    Off,
    /// Degradado de la paleta + inicial del artista. Es el estado "sin
    /// portada" y también un modo artístico más de la rotación.
    Gradient,
}

impl ArtMode {
    /// Los tres modos de portada grandes: foto fiel, pixel art y braille.
    /// `Half` se mantiene como alias interno de `Plain`.
    pub fn next(self) -> Self {
        match self {
            Self::Plain | Self::Half => Self::Dither,
            Self::Dither => Self::Braille,
            Self::Braille => Self::Off,
            Self::Gradient | Self::Off => Self::Plain,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Plain | Self::Half | Self::Gradient => Self::Off,
            Self::Dither => Self::Plain,
            Self::Braille => Self::Dither,
            Self::Off => Self::Braille,
        }
    }

    /// Modos que participan en la rotación automática. `Gradient` no está:
    /// es solo el recurso de cuando no hay portada.
    pub const ROTATION: [ArtMode; 3] = [ArtMode::Plain, ArtMode::Dither, ArtMode::Braille];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dither => "pixel",
            Self::Half | Self::Plain => "default",
            Self::Braille => "braille",
            Self::Off => "off",
            Self::Gradient => "gradient",
        }
    }

    pub fn from_str(text: &str) -> Option<Self> {
        match text {
            "dither" | "pixel" | "pixelado" => Some(Self::Dither),
            "half" | "halfblocks" => Some(Self::Half),
            "plain" | "default" | "foto" | "photo" => Some(Self::Plain),
            "braille" => Some(Self::Braille),
            "off" => Some(Self::Off),
            "gradient" => Some(Self::Gradient),
            _ => None,
        }
    }
}

/// Una celda de arte ya resuelta a carácter + colores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArtCell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
}

impl Default for ArtCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: (0, 0, 0),
            bg: None,
        }
    }
}

/// Imagen ya rasterizada al tamaño de la terminal.
#[derive(Debug, Clone, Default)]
pub struct CoverArt {
    pub w: u16,
    pub h: u16,
    pub cells: Vec<ArtCell>,
}

impl CoverArt {
    pub fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![ArtCell::default(); w as usize * h as usize],
        }
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<&ArtCell> {
        if x >= self.w || y >= self.h {
            return None;
        }
        self.cells.get(y as usize * self.w as usize + x as usize)
    }

    pub fn set(&mut self, x: u16, y: u16, cell: ArtCell) {
        if x < self.w && y < self.h {
            let idx = y as usize * self.w as usize + x as usize;
            self.cells[idx] = cell;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0 || self.cells.is_empty()
    }
}

/// Decodifica bytes de imagen (jpg/png/webp/gif).
pub fn decode_image(bytes: &[u8]) -> image::ImageResult<DynamicImage> {
    image::load_from_memory(bytes)
}

/// Miniatura cuadrada recortada por el centro (object-fit: cover).
pub fn square_thumb(img: &DynamicImage, size: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return img.clone();
    }
    let side = w.min(h);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    let cropped = img.crop_imm(x, y, side, side);
    cropped.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
}
