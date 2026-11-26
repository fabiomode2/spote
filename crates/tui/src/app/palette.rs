//! Menú de paletas derivadas de la portada.

use super::*;

impl App {
    fn current_album_palette_key(&self) -> Option<String> {
        let track = self.now.track.as_ref()?;
        let album = track.album.clone().unwrap_or_else(|| track.title.clone());
        Some(format!(
            "album_palette:{}",
            spote_core::util::normalize(&album)
        ))
    }

    pub fn palette_preview(&mut self) {
        let variant = PaletteVariant::ALL[self.palette_selected.min(PaletteVariant::ALL.len() - 1)];
        self.palette_variant = variant;
        let chosen = self.dominant.variant(variant);
        self.palette.go(chosen);
        self.needs_redraw = true;
    }

    pub fn palette_cancel(&mut self) {
        self.palette_variant = self.palette_original;
        let chosen = self.dominant.variant(self.palette_original);
        self.palette.go(chosen);
        self.overlay = Overlay::None;
        self.needs_redraw = true;
    }

    /// `save_album` guarda el override por álbum; si no, solo cambia el
    /// ajuste global de la app.
    pub fn palette_apply(&mut self, save_album: bool) {
        let variant = PaletteVariant::ALL[self.palette_selected.min(PaletteVariant::ALL.len() - 1)];
        self.palette_variant = variant;
        self.cfg.ui.palette_variant = variant.config_value().to_string();
        self.cfg.save().ok();
        if save_album {
            if let Some(key) = self.current_album_palette_key() {
                self.lib.set_setting(&key, variant.config_value()).ok();
                self.toast(
                    format!("paleta {} guardada para este álbum", variant.label()),
                    ToastKind::Ok,
                );
            }
        } else {
            self.toast(
                format!("paleta {} aplicada", variant.label()),
                ToastKind::Ok,
            );
        }
        let chosen = self.dominant.variant(variant);
        self.palette.go(chosen);
        self.overlay = Overlay::None;
        self.needs_redraw = true;
    }
}
