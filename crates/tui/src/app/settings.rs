//! Ajustes: filas, valores y persistencia.
use super::*;

fn cycle_name(current: &str, names: &[&str], delta: i32) -> String {
    let index = names
        .iter()
        .position(|name| *name == current)
        .unwrap_or(0) as i32;
    let next = (index + delta).rem_euclid(names.len() as i32) as usize;
    names[next].to_string()
}

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
            SettingRow::NormalizeTracks => onoff(self.cfg.audio.normalize_tracks),
            SettingRow::PanelOpacity => format!("{}%", self.cfg.ui.panel_opacity.clamp(55, 100)),
            SettingRow::ChromeOpacity => format!("{}%", self.cfg.ui.chrome_opacity.clamp(0, 100)),
            SettingRow::BottomOpacity => format!("{}%", self.cfg.ui.bottom_opacity.clamp(0, 100)),
            SettingRow::OverlayScrim => format!("{}%", self.cfg.ui.overlay_scrim.clamp(0, 95)),
            SettingRow::PanelBorder => match self.cfg.ui.panel_border.as_str() {
                "none" => "sin borde",
                "color" | "rim" => "color de portada",
                _ => "línea fina",
            }
            .to_string(),
            SettingRow::BorderOpacity => {
                format!("{}%", self.cfg.ui.border_opacity.clamp(0, 100))
            }
            SettingRow::RimReactive => onoff(self.cfg.ui.rim_reactive),
            SettingRow::RimIntensity => {
                format!("{}%", self.cfg.ui.rim_intensity.clamp(0, 100))
            }
            SettingRow::NavPosition => match self.cfg.ui.nav_position.as_str() {
                "top" => "arriba",
                "side" => "lateral",
                "both" => "arriba + lateral",
                _ => "auto",
            }
            .to_string(),
            SettingRow::DetailMode => match self.cfg.ui.detail_mode.as_str() {
                "expand" => "expandido",
                "auto" => "auto",
                _ => "lateral",
            }
            .to_string(),
            SettingRow::Backdrop => match self.cfg.ui.fx.backdrop.as_str() {
                "wallpaper" => "wallpaper",
                "gradient" => "degradado",
                "off" => "sin fondo",
                _ => "mosaico",
            }
            .to_string(),
            SettingRow::MosaicMode => match self.cfg.ui.fx.mosaic_mode.as_str() {
                "plain" => "plain",
                "dither" => "pixel",
                "braille" => "braille",
                _ => "half",
            }
            .to_string(),
            SettingRow::MosaicOpacity => {
                format!("{}%", self.cfg.ui.fx.mosaic_opacity.clamp(5, 50))
            }
            SettingRow::MosaicAuto => onoff(self.cfg.ui.fx.mosaic_auto),
            SettingRow::FullscreenBackdrop => match self.cfg.ui.fx.fullscreen_backdrop.as_str() {
                "black" => "negro",
                _ => "mosaico",
            }
            .to_string(),
            SettingRow::LyricsScenario => match self.cfg.ui.lyrics_scenario.as_str() {
                "centered" => "centrada",
                "behind" => "capa detrás",
                "mosaic" => "solo fondo",
                "off" => "normal",
                _ => "lateral",
            }
            .to_string(),
            SettingRow::LyricsUnit => match self.cfg.ui.lyrics_scenario_unit.as_str() {
                "word" => "palabra",
                _ => "línea",
            }
            .to_string(),
            SettingRow::MosaicMotion => match self.cfg.ui.fx.mosaic_motion.as_str() {
                "rows" => "filas",
                "columns" => "columnas",
                "diagonal" => "diagonal",
                "rotate" => "giro",
                "auto" => "auto",
                _ => "deriva",
            }
            .to_string(),
            SettingRow::MosaicSpeed => format!("{} / 10", self.cfg.ui.fx.mosaic_speed.clamp(1, 10)),
            SettingRow::MosaicBeat => onoff(self.cfg.ui.fx.mosaic_beat),
            SettingRow::LayoutEditor => "E".to_string(),
            SettingRow::VizBackground => onoff(self.viz_background),
            SettingRow::Cards => onoff(self.list_cards),
            SettingRow::Vcenter => onoff(self.vcenter),
            SettingRow::Zen => onoff(self.zen),
            SettingRow::Thumb => self.thumb_mode.label().to_string(),
            SettingRow::ProgressInViz => onoff(self.progress_in_viz),
            SettingRow::BarWidth => format!("{}%", self.bar_width_pct),
            SettingRow::ButtonPad => format!("{}", self.button_pad),
            SettingRow::Audio => {
                if self.cfg.audio.enabled {
                    "abrir · on".to_string()
                } else {
                    "abrir · off".to_string()
                }
            }
            SettingRow::Spotify => onoff(self.follow_spotify),
            SettingRow::Save => "enter".to_string(),
        }
    }

    #[allow(dead_code)]

    pub fn setting_at(&self, index: usize) -> Option<SettingRow> {
        SettingRow::from_index(index)
    }


    /// Índices de `SettingRow::ALL` que pasan el filtro actual. Se busca en
    /// etiqueta, sección y ayuda para que «bpm», «arte» o «volumen» encuentren
    /// la fila aunque la palabra no esté en el título.
    pub fn settings_visible(&self) -> Vec<usize> {
        let query = util::normalize(self.settings_query.trim());
        SettingRow::ALL
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                if query.is_empty() {
                    return Some(index);
                }
                let haystack = util::normalize(&format!(
                    "{} {} {}",
                    row.label(),
                    row.section(),
                    row.hint()
                ));
                haystack.contains(&query).then_some(index)
            })
            .collect()
    }

    /// Mueve la selección entre las filas visibles, no entre todas.
    pub fn settings_move(&mut self, delta: i32) {
        let visible = self.settings_visible();
        if visible.is_empty() {
            return;
        }
        let position = visible
            .iter()
            .position(|index| *index == self.settings_selected);
        let next = match position {
            Some(position) => {
                (position as i32 + delta).clamp(0, visible.len() as i32 - 1) as usize
            }
            None if delta < 0 => visible.len() - 1,
            None => 0,
        };
        self.settings_selected = visible[next];
    }

    /// Si el filtro deja fuera la fila seleccionada, cae en la primera visible.
    pub fn settings_clamp_selection(&mut self) {
        let visible = self.settings_visible();
        if !visible.is_empty() && !visible.contains(&self.settings_selected) {
            self.settings_selected = visible[0];
        }
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
                    self.cycle_viz(1);
                } else {
                    self.cycle_viz(-1);
                }
                self.cfg.ui.viz = self.viz.preset_name().to_string();
            }
            SettingRow::Ambient => {
                const AMBIENTS: [&str; 8] = [
                    "off", "void", "aurora", "neural", "glitch", "lyrics", "cover", "mosaic",
                ];
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
                    (self.cfg.ui.fx.ambient_opacity as i32 + delta * 2).clamp(5, 50) as u8;
            }
            SettingRow::PanelOpacity => {
                self.cfg.ui.panel_opacity =
                    (self.cfg.ui.panel_opacity as i32 + delta * 2).clamp(55, 100) as u8;
            }
            SettingRow::ChromeOpacity => {
                self.cfg.ui.chrome_opacity =
                    (self.cfg.ui.chrome_opacity as i32 + delta * 2).clamp(0, 100) as u8;
            }
            SettingRow::BottomOpacity => {
                self.cfg.ui.bottom_opacity =
                    (self.cfg.ui.bottom_opacity as i32 + delta * 2).clamp(0, 100) as u8;
            }
            SettingRow::OverlayScrim => {
                self.cfg.ui.overlay_scrim =
                    (self.cfg.ui.overlay_scrim as i32 + delta * 5).clamp(0, 95) as u8;
            }
            SettingRow::BorderOpacity => {
                self.cfg.ui.border_opacity =
                    (self.cfg.ui.border_opacity as i32 + delta * 5).clamp(0, 100) as u8;
            }
            SettingRow::RimIntensity => {
                self.cfg.ui.rim_intensity =
                    (self.cfg.ui.rim_intensity as i32 + delta * 5).clamp(0, 100) as u8;
            }
            SettingRow::MosaicOpacity => {
                self.cfg.ui.fx.mosaic_opacity =
                    (self.cfg.ui.fx.mosaic_opacity as i32 + delta * 2).clamp(5, 50) as u8;
            }
            SettingRow::PanelBorder => {
                const BORDERS: [&str; 3] = ["line", "color", "none"];
                self.cfg.ui.panel_border = cycle_name(&self.cfg.ui.panel_border, &BORDERS, delta);
            }
            SettingRow::NavPosition => {
                const POSITIONS: [&str; 4] = ["auto", "side", "top", "both"];
                self.cfg.ui.nav_position = cycle_name(&self.cfg.ui.nav_position, &POSITIONS, delta);
            }
            SettingRow::DetailMode => {
                const MODES: [&str; 3] = ["docked", "expand", "auto"];
                self.cfg.ui.detail_mode = cycle_name(&self.cfg.ui.detail_mode, &MODES, delta);
            }
            SettingRow::Backdrop => {
                const BACKDROPS: [&str; 3] = ["mosaic", "gradient", "off"];
                self.cfg.ui.fx.backdrop = cycle_name(&self.cfg.ui.fx.backdrop, &BACKDROPS, delta);
            }
            SettingRow::MosaicMode => {
                const MODES: [&str; 4] = ["half", "plain", "dither", "braille"];
                self.cfg.ui.fx.mosaic_mode = cycle_name(&self.cfg.ui.fx.mosaic_mode, &MODES, delta);
            }
            SettingRow::FullscreenBackdrop => {
                const MODES: [&str; 2] = ["mosaic", "black"];
                self.cfg.ui.fx.fullscreen_backdrop =
                    cycle_name(&self.cfg.ui.fx.fullscreen_backdrop, &MODES, delta);
            }
            SettingRow::LyricsScenario => {
                const MODES: [&str; 5] = ["off", "lateral", "centered", "behind", "mosaic"];
                self.cfg.ui.lyrics_scenario =
                    cycle_name(&self.cfg.ui.lyrics_scenario, &MODES, delta);
            }
            SettingRow::LyricsUnit => {
                const UNITS: [&str; 2] = ["line", "word"];
                self.cfg.ui.lyrics_scenario_unit =
                    cycle_name(&self.cfg.ui.lyrics_scenario_unit, &UNITS, delta);
            }
            SettingRow::LyricsSize => {
                self.adjust_lyrics_size(delta);
            }
            SettingRow::MosaicMotion => {
                const MOTIONS: [&str; 6] = ["auto", "drift", "rows", "columns", "diagonal", "rotate"];
                let current = MOTIONS
                    .iter()
                    .position(|name| *name == self.cfg.ui.fx.mosaic_motion)
                    .unwrap_or(0) as i32;
                let next = (current + delta).rem_euclid(MOTIONS.len() as i32) as usize;
                self.cfg.ui.fx.mosaic_motion = MOTIONS[next].to_string();
            }
            SettingRow::MosaicSpeed => {
                self.cfg.ui.fx.mosaic_speed =
                    (self.cfg.ui.fx.mosaic_speed as i32 + delta).clamp(1, 10) as u8;
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
            SettingRow::MosaicBeat => self.cfg.ui.fx.mosaic_beat = !self.cfg.ui.fx.mosaic_beat,
            SettingRow::MosaicAuto => self.cfg.ui.fx.mosaic_auto = !self.cfg.ui.fx.mosaic_auto,
            SettingRow::RimReactive => self.cfg.ui.rim_reactive = !self.cfg.ui.rim_reactive,
            SettingRow::NormalizeTracks => {
                self.cfg.audio.normalize_tracks = !self.cfg.audio.normalize_tracks;
                self.apply_audio_fx();
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
            SettingRow::Audio => self.open_audio(),
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
