//! Ajustes: filas, valores y persistencia.
use super::*;

impl App {
    pub fn settings_rows(&self) -> Vec<(&'static str, String)> {
        SettingRow::ALL
            .iter()
            .map(|row| (row.label(), self.setting_value(*row)))
            .collect()
    }


    pub fn setting_value(&self, row: SettingRow) -> String {
        match row {
            SettingRow::Rotate => onoff(self.art_rotate),
            SettingRow::RotateSecs => format!("{} s", self.cfg.ui.art_rotate_secs.max(5)),
            SettingRow::RotateOff => onoff(self.art_rotate_off),
            SettingRow::ArtMode => self.art_mode.label().to_string(),
            SettingRow::VizPreset => format!("{} · {}", self.viz.preset_name(), self.viz.preset_description()),
            SettingRow::Ambient => self.cfg.ui.fx.ambient.clone(),
            SettingRow::AmbientOpacity => format!("{}%", self.cfg.ui.fx.ambient_opacity),
            SettingRow::InstrumentBorders => onoff(self.cfg.ui.fx.instrument_borders),
            SettingRow::EdgeFlame => onoff(self.cfg.ui.fx.edge_flame),
            SettingRow::BloomPlay => onoff(self.cfg.ui.fx.bloom_play),
            SettingRow::CleanSpill => onoff(self.cfg.ui.fx.clean_spill),
            SettingRow::GlitchPanels => onoff(self.cfg.ui.fx.glitch_panels),
            SettingRow::LyricsSize => match self.cfg.ui.lyrics_size {
                0 => "compacta".to_string(),
                2 => "grande".to_string(),
                _ => "normal".to_string(),
            },
            SettingRow::LayoutEditor => "E".to_string(),
            SettingRow::VizBackground => onoff(self.viz_background),
            SettingRow::Cards => onoff(self.list_cards),
            SettingRow::Vcenter => onoff(self.vcenter),
            SettingRow::Zen => onoff(self.zen),
            SettingRow::Thumb => self.thumb_mode.label().to_string(),
            SettingRow::ProgressInViz => onoff(self.progress_in_viz),
            SettingRow::BarWidth => format!("{}%", self.bar_width_pct),
            SettingRow::ButtonPad => format!("{}", self.button_pad),
            SettingRow::Spotify => onoff(self.follow_spotify),
            SettingRow::Save => "enter".to_string(),
        }
    }

    #[allow(dead_code)]

    pub fn setting_at(&self, index: usize) -> Option<SettingRow> {
        SettingRow::from_index(index)
    }


    pub fn adjust_setting(&mut self, row: usize, delta: i32) {
        let Some(setting) = SettingRow::from_index(row) else {
            return;
        };
        match setting {
            SettingRow::RotateSecs => {
                let secs = self.cfg.ui.art_rotate_secs as i32 + delta * 5;
                self.cfg.ui.art_rotate_secs = secs.clamp(5, 300) as u64;
                self.art_next_at = Instant::now()
                    + Duration::from_secs(self.cfg.ui.art_rotate_secs.max(5));
            }
            SettingRow::ArtMode => {
                let next = if delta > 0 {
                    self.art_mode.next()
                } else {
                    self.art_mode.prev()
                };
                self.set_art_mode(next);
            }
            SettingRow::VizPreset => {
                if delta > 0 {
                    self.viz.next_preset();
                } else {
                    self.viz.prev_preset();
                }
                self.cfg.ui.viz = self.viz.preset_name().to_string();
            }
            SettingRow::Ambient => {
                const AMBIENTS: [&str; 7] =
                    ["off", "void", "aurora", "neural", "glitch", "lyrics", "cover"];
                let current = AMBIENTS
                    .iter()
                    .position(|name| *name == self.cfg.ui.fx.ambient)
                    .unwrap_or(1) as i32;
                let next = (current + delta).rem_euclid(AMBIENTS.len() as i32) as usize;
                self.cfg.ui.fx.ambient = AMBIENTS[next].to_string();
                self.ambient = Viz::from_preset(&self.cfg.ui.fx.ambient);
            }
            SettingRow::AmbientOpacity => {
                self.cfg.ui.fx.ambient_opacity =
                    (self.cfg.ui.fx.ambient_opacity as i32 + delta * 2).clamp(0, 40) as u8;
            }
            SettingRow::LyricsSize => {
                self.cfg.ui.lyrics_size =
                    (self.cfg.ui.lyrics_size as i32 + delta).clamp(0, 2) as u8;
            }
            SettingRow::Thumb => {
                let next = if delta > 0 {
                    self.thumb_mode.next()
                } else {
                    let mut mode = self.thumb_mode;
                    for _ in 0..2 {
                        mode = mode.next();
                    }
                    mode
                };
                self.thumb_mode = next;
                self.cfg.ui.thumb_mode = next.config_value().to_string();
            }
            SettingRow::BarWidth => {
                let pct = (i32::from(self.bar_width_pct) + delta * 5).clamp(55, 100);
                self.bar_width_pct = pct as u8;
                self.cfg.ui.bar_width_pct = self.bar_width_pct;
            }
            SettingRow::ButtonPad => {
                let pad = (i32::from(self.button_pad) + delta).clamp(1, 4);
                self.button_pad = pad as u8;
                self.cfg.ui.button_pad = self.button_pad;
            }
            _ => self.toggle_setting(row),
        }
        self.cfg.save().ok();
        self.needs_redraw = true;
    }


    pub fn toggle_setting(&mut self, row: usize) {
        let Some(setting) = SettingRow::from_index(row) else {
            return;
        };
        match setting {
            SettingRow::Rotate => {
                self.art_rotate = !self.art_rotate;
                self.cfg.ui.art_rotate = self.art_rotate;
            }
            SettingRow::RotateOff => {
                self.art_rotate_off = !self.art_rotate_off;
                self.cfg.ui.art_rotate_off = self.art_rotate_off;
            }
            SettingRow::VizBackground => {
                self.viz_background = !self.viz_background;
                self.cfg.ui.viz_background = self.viz_background;
            }
            SettingRow::Cards => {
                self.list_cards = !self.list_cards;
                self.cfg.ui.list_cards = self.list_cards;
            }
            SettingRow::Vcenter => {
                self.vcenter = !self.vcenter;
                self.cfg.ui.layout.vcenter = self.vcenter;
            }
            SettingRow::InstrumentBorders => {
                self.cfg.ui.fx.instrument_borders = !self.cfg.ui.fx.instrument_borders
            }
            SettingRow::EdgeFlame => self.cfg.ui.fx.edge_flame = !self.cfg.ui.fx.edge_flame,
            SettingRow::BloomPlay => self.cfg.ui.fx.bloom_play = !self.cfg.ui.fx.bloom_play,
            SettingRow::CleanSpill => self.cfg.ui.fx.clean_spill = !self.cfg.ui.fx.clean_spill,
            SettingRow::GlitchPanels => {
                self.cfg.ui.fx.glitch_panels = !self.cfg.ui.fx.glitch_panels
            }
            SettingRow::LayoutEditor => {
                self.overlay = Overlay::Layout;
                self.layout_selected = 0;
            }
            SettingRow::Zen => self.set_zen(!self.zen),
            SettingRow::ProgressInViz => {
                self.progress_in_viz = !self.progress_in_viz;
                self.cfg.ui.progress_in_viz = self.progress_in_viz;
            }
            SettingRow::Spotify => self.toggle_spotify(),
            SettingRow::Save => match self.cfg.save() {
                Ok(()) => self.toast("ajustes guardados", ToastKind::Ok),
                Err(err) => self.toast(format!("no se pudo guardar: {err}"), ToastKind::Error),
            },
            _ => {}
        }
        if setting != SettingRow::Save {
            self.cfg.save().ok();
            self.needs_redraw = true;
        }
    }


    pub fn set_zen(&mut self, zen: bool) {
        self.zen = zen;
        self.cfg.ui.zen = zen;
        self.needs_redraw = true;
    }

    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    // ------------------------------------------------------------- Spotify
}

fn onoff(value: bool) -> String {
    if value { "sí".into() } else { "no".into() }}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_indices_round_trip() {
        for (index, row) in SettingRow::ALL.iter().enumerate() {
            assert_eq!(SettingRow::from_index(index), Some(*row));
        }
        assert_eq!(SettingRow::from_index(SettingRow::ALL.len()), None);
        assert!(SettingRow::BarWidth.adjustable());
        assert!(!SettingRow::Save.adjustable());
    }
}
