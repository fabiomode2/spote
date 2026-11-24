//! Resumen de escucha tipo Wrapped.
use super::*;

impl App {
    pub fn open_wrapped(&mut self) {
        self.wrapped_all_time = false;
        self.reload_wrapped();
        self.overlay = Overlay::Wrapped;
        self.needs_redraw = true;
    }

    pub(super) fn reload_wrapped(&mut self) {
        match self.lib.wrapped_stats(self.wrapped_all_time) {
            Ok(stats) => self.wrapped = Some(stats),
            Err(err) => {
                self.wrapped = None;
                self.toast(format!("no pude calcular el resumen: {err}"), ToastKind::Warn);
            }
        }
    }

    pub(super) fn toggle_wrapped_period(&mut self) {
        self.wrapped_all_time = !self.wrapped_all_time;
        self.reload_wrapped();
    }
}
