//! Teclado y ajustes del overlay de audio.
use super::*;

const BANDS: usize = 10;
const PREAMP: usize = 10;
const NORMALIZER: usize = 11;
const STRENGTH: usize = 12;
const LIMITER: usize = 13;
const PRESET: usize = 14;
const MASTER: usize = 15;

impl App {
    pub fn open_audio(&mut self) {
        self.overlay = Overlay::None;
        self.tab = Tab::Audio;
        self.audio_selected = 0;
        self.focus = Pane::Center;
        self.needs_redraw = true;
    }

    /// Teclas de la pestaña de audio. Devuelve `true` si la tecla era suya:
    /// así espacio (play) y el resto de atajos globales siguen funcionando.
    pub fn handle_audio_tab_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.tab = Tab::Now;
                self.needs_redraw = true;
            }
            KeyCode::Up | KeyCode::Char('k') => self.adjust_audio(1),
            KeyCode::Down | KeyCode::Char('j') => self.adjust_audio(-1),
            KeyCode::Left | KeyCode::Char('h') => self.move_audio_selection(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_audio_selection(1),
            KeyCode::Enter => self.toggle_audio_control(),
            KeyCode::Char('0') => self.reset_audio_control(),
            KeyCode::Char('p') => self.cycle_audio_preset(1),
            KeyCode::Char('P') => self.cycle_audio_preset(-1),
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.cfg.audio.enabled = !self.cfg.audio.enabled;
                self.audio_changed();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.cfg.audio.eq_enabled = !self.cfg.audio.eq_enabled;
                self.audio_changed();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.toggle_normalize_tracks(),
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.cfg.audio.bands = [0.0; 10];
                self.cfg.audio.preamp_db = 0.0;
                self.audio_changed();
            }
            _ => return false,
        }
        self.needs_redraw = true;
        true
    }

    pub fn toggle_normalize_tracks(&mut self) {
        self.cfg.audio.normalize_tracks = !self.cfg.audio.normalize_tracks;
        self.audio_changed();
        self.set_status(if self.cfg.audio.normalize_tracks {
            "nivelación entre pistas: on"
        } else {
            "nivelación entre pistas: off"
        });
    }

    pub(super) fn handle_audio_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Up | KeyCode::Char('k') => self.adjust_audio(1),
            KeyCode::Down | KeyCode::Char('j') => self.adjust_audio(-1),
            KeyCode::Left | KeyCode::Char('h') => self.move_audio_selection(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_audio_selection(1),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_audio_control(),
            KeyCode::Char('0') => self.reset_audio_control(),
            KeyCode::Char('p') => self.cycle_audio_preset(1),
            KeyCode::Char('P') => self.cycle_audio_preset(-1),
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.cfg.audio.enabled = !self.cfg.audio.enabled;
                self.audio_changed();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.cfg.audio.eq_enabled = !self.cfg.audio.eq_enabled;
                self.audio_changed();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.cfg.audio.bands = [0.0; 10];
                self.cfg.audio.preamp_db = 0.0;
                self.audio_changed();
            }
            _ => {}
        }
        self.needs_redraw = true;
    }

    pub fn move_audio_selection(&mut self, delta: i32) {
        self.audio_selected = (self.audio_selected as i32 + delta).rem_euclid(16) as usize;
        self.needs_redraw = true;
    }

    /// Texto corto del control seleccionado para que la pestaña diga siempre
    /// dónde estás y cuánto vale.
    pub fn audio_selection_label(&self) -> String {
        match self.audio_selected {
            i if i < BANDS => format!(
                "{} Hz · {:+.0} dB",
                spote_core::audio_fx::BAND_LABELS[i],
                self.cfg.audio.bands[i]
            ),
            PREAMP => format!("preamp · {:+.1} dB", self.cfg.audio.preamp_db),
            NORMALIZER => format!("directo · {}", self.cfg.audio.normalizer.label()),
            STRENGTH => format!(
                "intensidad · {:.0}%",
                self.cfg.audio.normalizer_strength * 100.0
            ),
            LIMITER => format!(
                "limitador · {}",
                if self.cfg.audio.limiter { "on" } else { "off" }
            ),
            PRESET => {
                let name = spote_core::audio_fx::EQ_PRESETS
                    .iter()
                    .find(|(_, bands)| *bands == self.cfg.audio.bands)
                    .map(|(name, _)| *name)
                    .unwrap_or("custom");
                format!("preset · {name}")
            }
            _ => format!(
                "proceso · {}",
                if self.cfg.audio.enabled { "on" } else { "off" }
            ),
        }
    }

    pub fn adjust_audio(&mut self, delta: i32) {
        match self.audio_selected {
            i if i < BANDS => {
                let value = self.cfg.audio.bands[i] + delta as f32;
                if value.is_finite() {
                    self.cfg.audio.bands[i] = value.clamp(-12.0, 12.0);
                } else {
                    self.cfg.audio.bands[i] = 0.0;
                }
            }
            PREAMP => {
                let value = self.cfg.audio.preamp_db + delta as f32;
                self.cfg.audio.preamp_db = if value.is_finite() {
                    value.clamp(-12.0, 12.0)
                } else {
                    0.0
                };
            }
            NORMALIZER => {
                self.cfg.audio.normalizer = if delta > 0 {
                    self.cfg.audio.normalizer.next()
                } else {
                    let mut mode = self.cfg.audio.normalizer;
                    for _ in 0..2 {
                        mode = mode.next();
                    }
                    mode
                };
            }
            STRENGTH => {
                let value = self.cfg.audio.normalizer_strength + delta as f32 * 0.05;
                self.cfg.audio.normalizer_strength = if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    0.5
                };
            }
            LIMITER => self.cfg.audio.limiter = !self.cfg.audio.limiter,
            PRESET => self.cycle_audio_preset(delta),
            MASTER => self.cfg.audio.enabled = !self.cfg.audio.enabled,
            _ => {}
        }
        self.audio_changed();
    }

    pub fn toggle_audio_control(&mut self) {
        match self.audio_selected {
            NORMALIZER => self.cfg.audio.normalizer = self.cfg.audio.normalizer.next(),
            LIMITER => self.cfg.audio.limiter = !self.cfg.audio.limiter,
            MASTER => self.cfg.audio.enabled = !self.cfg.audio.enabled,
            PRESET => self.cycle_audio_preset(1),
            i if i < BANDS => self.cfg.audio.bands[i] = 0.0,
            PREAMP => self.cfg.audio.preamp_db = 0.0,
            _ => {}
        }
        self.audio_changed();
    }

    pub fn reset_audio_control(&mut self) {
        match self.audio_selected {
            i if i < BANDS => self.cfg.audio.bands[i] = 0.0,
            PREAMP => self.cfg.audio.preamp_db = 0.0,
            STRENGTH => self.cfg.audio.normalizer_strength = 0.5,
            NORMALIZER => self.cfg.audio.normalizer = spote_core::audio_fx::NormalizerMode::Off,
            LIMITER => self.cfg.audio.limiter = false,
            PRESET => self.cfg.audio.bands = [0.0; 10],
            MASTER => self.cfg.audio.enabled = true,
            _ => {}
        }
        self.audio_changed();
    }

    pub fn cycle_audio_preset(&mut self, delta: i32) {
        let presets = spote_core::audio_fx::EQ_PRESETS;
        let current = presets
            .iter()
            .position(|(_, bands)| *bands == self.cfg.audio.bands)
            .unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(presets.len() as i32) as usize;
        self.cfg.audio.bands = presets[next].1;
        self.audio_changed();
    }

    pub fn apply_audio_fx(&mut self) {
        let gain = if self.cfg.audio.normalize_tracks {
            self.track_gain_db.unwrap_or(0.0)
        } else {
            0.0
        };
        let fallback = self.track_gain_db.is_none();
        let chain = self.cfg.audio.filter_chain_with_gain(gain, fallback);
        if let Some(player) = &self.player {
            player.set_audio_filters(chain.as_deref());
        }
    }

    pub(super) fn audio_changed(&mut self) {
        self.apply_audio_fx();
        self.cfg.save().ok();
        self.needs_redraw = true;
    }

    pub(super) fn handle_audio_action(&mut self, action: Action) {
        match action {
            Action::AudioBand(index) | Action::AudioBandSet(index) => {
                self.audio_selected = index.min(BANDS - 1);
            }
            Action::AudioPreamp => self.audio_selected = PREAMP,
            Action::AudioNormalizer => {
                self.cfg.audio.normalizer = self.cfg.audio.normalizer.next();
                self.audio_changed();
            }
            Action::AudioStrength => self.audio_selected = STRENGTH,
            Action::AudioLimiter => {
                self.cfg.audio.limiter = !self.cfg.audio.limiter;
                self.audio_changed();
            }
            Action::AudioPreset(index) => {
                if let Some((_, bands)) = spote_core::audio_fx::EQ_PRESETS.get(index) {
                    self.cfg.audio.bands = *bands;
                    self.audio_changed();
                }
            }
            Action::AudioToggle => {
                self.cfg.audio.enabled = !self.cfg.audio.enabled;
                self.audio_changed();
            }
            Action::AudioEqToggle => {
                self.cfg.audio.eq_enabled = !self.cfg.audio.eq_enabled;
                self.audio_changed();
            }
            Action::AudioNormalize => self.toggle_normalize_tracks(),
            _ => {}
        }
    }
}
