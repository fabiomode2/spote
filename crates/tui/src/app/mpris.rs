//! Publicación del estado de spote en MPRIS y ejecución de sus comandos.
use super::*;

use spote_core::mpris_server::{MprisCommand, MprisSnapshot};

impl App {
    /// Se llama tras crear la app. Si el nombre MPRIS está ocupado o no hay
    /// bus de sesión, la TUI sigue funcionando y el usuario ve el motivo.
    pub fn start_mpris(&mut self) {
        if self.mpris_server.is_some() {
            return;
        }
        match spote_core::mpris_server::MprisServer::start(&self.cfg.app.name) {
            Ok(server) => self.mpris_server = Some(server),
            Err(err) => self.toast(format!("MPRIS no disponible: {err}"), ToastKind::Warn),
        }
    }

    pub(super) fn poll_mpris(&mut self) {
        let commands: Vec<MprisCommand> = self
            .mpris_server
            .as_ref()
            .map(|server| std::iter::from_fn(|| server.try_recv()).collect())
            .unwrap_or_default();
        for command in commands {
            match command {
                MprisCommand::Play => {
                    if self.follow_spotify {
                        if let Some(worker) = &self.spotify_worker {
                            worker.send(SystemCommand::Play);
                        }
                    } else if !self.now.playing && self.now.track.is_some() {
                        self.toggle_play();
                    }
                }
                MprisCommand::Pause => {
                    if self.follow_spotify {
                        if let Some(worker) = &self.spotify_worker {
                            worker.send(SystemCommand::Pause);
                        }
                    } else if self.now.playing {
                        self.toggle_play();
                    }
                }
                MprisCommand::PlayPause => {
                    if self.follow_spotify {
                        if let Some(worker) = &self.spotify_worker {
                            worker.send(SystemCommand::PlayPause);
                        }
                    } else if self.now.track.is_some() {
                        self.toggle_play();
                    }
                }
                MprisCommand::Next => self.next_track(),
                MprisCommand::Previous => self.prev_track(),
                MprisCommand::Stop => {
                    if self.follow_spotify {
                        if let Some(worker) = &self.spotify_worker {
                            worker.send(SystemCommand::Stop);
                        }
                    } else {
                        if self.now.playing {
                            self.toggle_play();
                        }
                        self.seek_to(0);
                    }
                }
                MprisCommand::SeekRelative(us) => {
                    if self.follow_spotify {
                        // El reproductor seguido mueve su propia posición; no
                        // se recalcula desde el reloj local, que va con retraso.
                        if let Some(worker) = &self.spotify_worker {
                            worker.send(SystemCommand::SeekRelative(us));
                        }
                    } else {
                        let delta = us / 1_000;
                        let target = (self.display_position_ms() as i64 + delta).max(0) as u64;
                        self.seek_to(target);
                    }
                }
                MprisCommand::SetPosition(ms) => self.seek_to(ms),
                MprisCommand::SetVolume(volume) => self.set_volume(volume.clamp(0.0, 1.0)),
                MprisCommand::SetShuffle(on) => {
                    if self.shuffle_on != on {
                        self.toggle_shuffle();
                    }
                }
                MprisCommand::SetRepeat(mode) => {
                    let mode = match mode.as_str() {
                        "all" | "playlist" => RepeatMode::All,
                        "one" | "track" => RepeatMode::One,
                        _ => RepeatMode::Off,
                    };
                    self.set_repeat_mode(mode);
                }
                MprisCommand::OpenUri(uri) => {
                    self.set_status(format!("MPRIS: abrir {uri} todavía no está soportado"));
                }
                MprisCommand::Raise => {}
                MprisCommand::Quit => {
                    self.persist_volume();
                    self.should_quit = true;
                }
            }
        }

        if self.mpris_last_update.elapsed() >= Duration::from_millis(250) {
            let snapshot = self.mpris_snapshot();
            if let Some(server) = &self.mpris_server {
                server.update(snapshot);
            }
            self.mpris_last_update = Instant::now();
        }
    }

    fn mpris_snapshot(&self) -> MprisSnapshot {
        let track = self.now.track.as_ref();
        let art_url = track.and_then(|track| {
            track.art_url.clone().or_else(|| {
                self.lib
                    .art_path(&track.key)
                    .ok()
                    .flatten()
                    .map(|path| format!("file://{}", path.display()))
            })
        });
        MprisSnapshot {
            playing: self.now.playing,
            title: track.map(|t| t.title.clone()).unwrap_or_default(),
            artist: track.map(|t| t.artist.clone()).unwrap_or_default(),
            album: track.and_then(|t| t.album.clone()),
            duration_ms: self.now.duration_ms,
            position_ms: self.display_position_ms(),
            art_url,
            track_key: track.map(|t| t.key.clone()).unwrap_or_default(),
            volume: self.now.volume,
            shuffle: self.shuffle_on,
            repeat: match self.repeat_mode {
                RepeatMode::Off => "off",
                RepeatMode::All => "all",
                RepeatMode::One => "one",
            }
            .to_string(),
        }
    }
}
