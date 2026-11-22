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
        self.overlay = Overlay::Audio;
        self.audio_selected = 0;
        self.needs_redraw = true;
    }

    pub(super) fn handle_audio_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.audio_selected = (self.audio_selected + 15) % 16;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.audio_selected = (self.audio_selected + 1) % 16;
            }
            KeyCode::Left | KeyCode::Char('h') => self.adjust_audio(-1),
            KeyCode::Right | KeyCode::Char('l') => self.adjust_audio(1),
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

    pub fn adjust_audio(&mut self, delta: i32) {
        match self.audio_selected {
            i if i < BANDS => {
                let value = self.cfg.audio.bands[i] + delta as f32;
                self.cfg.audio.bands[i] = value.clamp(-12.0, 12.0);
            }
            PREAMP => {
                self.cfg.audio.preamp_db =
                    (self.cfg.audio.preamp_db + delta as f32).clamp(-12.0, 12.0);
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
                self.cfg.audio.normalizer_strength = value.clamp(0.0, 1.0);
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
            _ => self.toggle_audio_control(),
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
        let chain = self.cfg.audio.filter_chain();
        if let Some(player) = &self.player {
            player.set_audio_filters(chain.as_deref());
        }
    }

    fn audio_changed(&mut self) {
        self.apply_audio_fx();
        self.cfg.save().ok();
        self.needs_redraw = true;
    }

    pub(super) fn handle_audio_action(&mut self, action: Action) {
        match action {
            Action::AudioBand(index) => {
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
            _ => {}
        }
    }
}
