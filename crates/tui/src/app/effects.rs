//! Rotación configurable de efectos.

use super::*;

impl App {
    pub fn effect_enabled(&self, name: &str) -> bool {
        self.cfg
            .ui
            .effects_enabled
            .iter()
            .any(|item| item.as_str() == name)
    }

    pub fn toggle_effect_enabled(&mut self, name: &str) {
        if let Some(index) = self
            .cfg
            .ui
            .effects_enabled
            .iter()
            .position(|item| item == name)
        {
            // No dejar la rotación vacía: si es el último, se ignora.
            if self.cfg.ui.effects_enabled.len() > 1 {
                self.cfg.ui.effects_enabled.remove(index);
            }
        } else {
            self.cfg.ui.effects_enabled.push(name.to_string());
        }
        self.cfg.save().ok();
        self.needs_redraw = true;
    }

    pub fn effects_selected_name(&self) -> Option<&'static str> {
        crate::viz::PRESETS
            .get(self.effects_selected)
            .map(|preset| preset.name)
    }

    pub fn toggle_selected_effect(&mut self) {
        if let Some(name) = self.effects_selected_name() {
            self.toggle_effect_enabled(name);
        }
    }

    pub fn select_effect(&mut self, delta: i32) {
        let len = crate::viz::PRESETS.len();
        if len == 0 {
            return;
        }
        self.effects_selected =
            (self.effects_selected as i32 + delta).rem_euclid(len as i32) as usize;
        if let Some(preset) = crate::viz::PRESETS.get(self.effects_selected) {
            if let Some(index) = crate::viz::preset_by_name(preset.name) {
                self.viz.set_preset(index);
                self.cfg.ui.viz = preset.name.to_string();
                self.cfg.save().ok();
                self.needs_redraw = true;
            }
        }
    }
}
