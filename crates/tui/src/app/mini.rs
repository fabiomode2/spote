//! Puente entre la app principal y el mini reproductor de escritorio.
use super::*;

use spote_protocol::mini::{
    MiniCommand, MiniSnapshot, MiniVizCell, MINI_REPEAT_ALL, MINI_REPEAT_OFF, MINI_REPEAT_ONE,
};

impl App {
    pub fn open_mini(&mut self) {
        if self.mini_hub.is_none() {
            let reason = self
                .mini_error
                .clone()
                .unwrap_or_else(|| "el socket del mini no está disponible".to_string());
            self.toast(format!("mini no disponible: {reason}"), ToastKind::Error);
            return;
        }
        if let Some(child) = &mut self.mini_child {
            if matches!(child.try_wait(), Ok(None)) {
                self.toast("el mini reproductor ya está abierto", ToastKind::Info);
                return;
            }
            self.mini_child = None;
        }
        let exe = std::env::current_exe().ok();
        let sibling = exe
            .as_ref()
            .and_then(|path| path.parent())
            .map(|dir| dir.join("spote-mini"));
        let spawned = if let Some(path) = sibling.filter(|path| path.is_file()) {
            std::process::Command::new(path).spawn()
        } else {
            std::process::Command::new("spote-mini").spawn()
        };
        match spawned {
            Ok(child) => {
                self.mini_child = Some(child);
                self.toast("mini reproductor abierto", ToastKind::Ok);
            }
            Err(err) => self.toast(format!("no pude abrir spote-mini: {err}"), ToastKind::Error),
        }
    }

    pub(super) fn poll_mini(&mut self) {
        let commands: Vec<MiniCommand> = self
            .mini_hub
            .as_ref()
            .map(|hub| std::iter::from_fn(|| hub.try_recv()).collect())
            .unwrap_or_default();
        for command in commands {
            match command {
                MiniCommand::PlayPause => self.toggle_play(),
                MiniCommand::Next => self.next_track(),
                MiniCommand::Previous => self.prev_track(),
                MiniCommand::Seek(ms) => self.seek_to(ms),
                MiniCommand::SetVolume(volume) => self.set_volume(volume),
                MiniCommand::Shuffle => self.toggle_shuffle(),
                MiniCommand::CycleRepeat => self.cycle_repeat(),
                MiniCommand::CycleViz => {
                    self.viz.next_preset();
                    self.cfg.ui.viz = self.viz.preset_name().to_string();
                    self.cfg.save().ok();
                    self.needs_redraw = true;
                }
                MiniCommand::Quit => {}
            }
        }
        if self.mini_last_publish.elapsed() >= Duration::from_millis(50) {
            if let Some(hub) = &self.mini_hub {
                // Sin ventana abierta no se consulta SQLite ni se serializa
                // un snapshot cada 50 ms.
                if hub.has_clients() {
                    let snapshot = self.mini_snapshot();
                    hub.publish(&snapshot);
                }
            }
            self.mini_last_publish = Instant::now();
        }
        if let Some(child) = &mut self.mini_child {
            if !matches!(child.try_wait(), Ok(None)) {
                self.mini_child = None;
            }
        }
    }

    fn mini_snapshot(&self) -> MiniSnapshot {
        let track = self.now.track.clone();
        let key = track
            .as_ref()
            .map(|track| track.key.clone())
            .unwrap_or_default();
        let art_path = track.as_ref().and_then(|track| {
            self.lib
                .art_path(&track.key)
                .ok()
                .flatten()
                .filter(|path| path.exists())
                .or_else(|| {
                    self.lib
                        .local_path(&track.key)
                        .map(|path| path.with_extension("jpg"))
                        .filter(|path| path.exists())
                })
                .or_else(|| {
                    self.now
                        .path
                        .as_ref()
                        .map(|path| path.with_extension("jpg"))
                        .filter(|path| path.exists())
                })
                .map(|path| path.to_string_lossy().into_owned())
        });
        let accent = self.palette.current().primary;
        let repeat = match self.repeat_mode {
            RepeatMode::Off => MINI_REPEAT_OFF,
            RepeatMode::All => MINI_REPEAT_ALL,
            RepeatMode::One => MINI_REPEAT_ONE,
        };
        let status = if track.is_none() {
            "idle"
        } else if self.now.loading {
            "cargando"
        } else if self.now.playing {
            "playing"
        } else {
            "paused"
        };
        let mut snapshot = MiniSnapshot::new(
            track.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
            track.as_ref().map(|t| t.artist.clone()).unwrap_or_default(),
        )
        .with_album(
            track
                .as_ref()
                .and_then(|t| t.album.clone())
                .unwrap_or_default(),
        )
        .with_playing(self.now.playing)
        .with_duration_ms(self.now.duration_ms)
        .with_position_ms(self.display_position_ms())
        .with_volume(self.now.volume)
        .with_shuffle(self.shuffle_on)
        .with_repeat(repeat)
        .with_accent([accent.0, accent.1, accent.2])
        .with_status(status)
        .with_track_key(key);
        if let Some(path) = art_path {
            snapshot = snapshot.with_art_path(path);
        }
        // 64 bandas -> 32, promediando pares para que el mini no pierda graves.
        let bands: Vec<f32> = self
            .audio
            .bands
            .chunks(2)
            .map(|pair| pair.iter().sum::<f32>() / pair.len() as f32)
            .collect();
        let (cols, rows, cells) = mini_viz_frame(&self.viz);
        snapshot.viz_preset = self.viz.preset_name().to_string();
        snapshot.viz_presets = crate::viz::Viz::STRIP
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        snapshot.viz_cols = cols;
        snapshot.viz_rows = rows;
        snapshot.viz = cells;
        snapshot.with_bands(bands)
    }
}

/// Reduce el canvas del visual de la TUI a una rejilla pequeña que el mini
/// puede dibujar con texto monoespaciado sin enviar miles de celdas por tick.
fn mini_viz_frame(viz: &crate::viz::Viz) -> (u16, u16, Vec<MiniVizCell>) {
    let out = &viz.output;
    if out.w == 0 || out.h == 0 {
        return (0, 0, Vec::new());
    }
    let cols = out.w.min(56);
    let rows = out.h.min(9);
    let mut cells = Vec::with_capacity(cols as usize * rows as usize);
    for y in 0..rows {
        let sy = (u32::from(y) * u32::from(out.h) / u32::from(rows)) as u16;
        for x in 0..cols {
            let sx = (u32::from(x) * u32::from(out.w) / u32::from(cols)) as u16;
            let cell = out.get(i32::from(sx), i32::from(sy)).unwrap_or_default();
            if cell.alpha <= 0.04 || cell.ch == ' ' {
                continue;
            }
            cells.push(MiniVizCell {
                ch: cell.ch,
                fg: [cell.fg.0, cell.fg.1, cell.fg.2],
                alpha: (cell.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            });
        }
    }
    (cols, rows, cells)
}
