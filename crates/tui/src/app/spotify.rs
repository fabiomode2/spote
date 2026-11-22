//! Modo acompañante de cualquier reproductor MPRIS del sistema.
use super::*;

impl App {
    pub fn enable_spotify(&mut self) {
        if self.spotify_worker.is_none() {
            self.spotify_worker = Some(SystemMediaWorker::start());
        }
        self.follow_spotify = true;
        self.cfg.ui.follow_spotify = true;
        self.cfg.save().ok();
        self.spotify_warned = false;
        self.last_spotify_poll = Instant::now();
        // El modo acompañante manda: nada de audio local sonando por debajo,
        // ni streams/descargas pendientes que puedan arrancarlo después.
        self.clear_followed_track();
        if let Some(player) = &self.player {
            player.stop();
        }
        self.spectrum.decode_system();
        self.toast(
            "siguiendo el reproductor MPRIS del sistema",
            ToastKind::Info,
        );
    }

    pub fn disable_spotify(&mut self) {
        self.follow_spotify = false;
        self.cfg.ui.follow_spotify = false;
        self.cfg.save().ok();
        self.spotify_worker = None;
        self.spectrum.stop();
        // Vuelve el modo local limpio: fuera la pista remota y fuera el
        // volumen/shuffle/repeat que impuso el reproductor seguido.
        self.spotify = None;
        self.clear_followed_track();
        self.now.volume = self.cfg.player.volume.clamp(0.0, 1.0);
        self.muted = false;
        self.shuffle_on = self.cfg.playback.shuffle;
        self.repeat_mode = match self.cfg.playback.repeat.as_str() {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        };
        self.loop_file = self.repeat_mode == RepeatMode::One;
        if let Some(player) = &self.player {
            player.set_volume(self.now.volume);
            player.set_loop(self.loop_file);
        }
        self.toast(
            "modo sistema desactivado · vuelve el modo local",
            ToastKind::Info,
        );
    }

    /// Deja la UI vacía cuando el reproductor seguido cierra o pierde la
    /// pista. Mantiene `follow_spotify` activo para que el worker reconecte.
    fn clear_followed_track(&mut self) {
        self.pending_stream = None;
        self.pending_play = None;
        self.pending_seek_ms = None;
        self.stream_autoplay = true;
        self.streaming = false;
        self.stream_cache_pending = None;
        self.stream_resolver.cancel();
        self.now.track = None;
        self.now.path = None;
        self.now.position_ms = 0;
        self.now.duration_ms = 0;
        self.now.position_unknown = false;
        self.now.playing = false;
        self.now.loading = false;
        self.now.ended = false;
        self.now.message.clear();
        self.lyrics = None;
        self.lyrics_key.clear();
        self.lyrics_status.clear();
        self.lyrics_loading = false;
        self.lyrics_auto_offset = None;
        self.reset_lyrics_view();
        self.art = None;
        self.art_key.clear();
        self.art_variants.clear();
        self.art_png = None;
        self.art_png_key.clear();
        self.ambient_art = None;
        self.ambient_art_key.clear();
        self.song_map = None;
        self.song_map_key.clear();
        self.spectrum.stop();
    }

    pub(super) fn apply_spotify(&mut self, state: SystemMediaState) {
        if state.track.is_none() {
            if self.now.track.is_some() {
                self.clear_followed_track();
                self.spectrum.stop();
            }
            self.spotify = Some(state);
            return;
        }
        if let Some(mut track) = state.track.clone() {
            // Si la pista está descargada, usa SU clave (y su portada local):
            // MPRIS da otro id y si no, la letra/arte no encuentran nada.
            let mut matched_local = false;
            if let Ok(Some(row)) =
                self.lib
                    .find_by_meta(&track.artist, &track.title, track.duration_ms)
            {
                matched_local = true;
                track.key = row.track.key.clone();
                if track.album.is_none() {
                    track.album = row.track.album.clone();
                }
                if track.art_url.is_none() {
                    track.art_url = row.track.art_url.clone();
                }
            }
            let changed = self.now.track.as_ref().map(|t| &t.key) != Some(&track.key);
            self.now.track = Some(track.clone());
            self.now.duration_ms = track.duration_ms;
            // MPRIS informa con ~400 ms de retraso. Si se aplica tal cual en
            // cada sondeo, la posición retrocede y la letra se queda clavada.
            // Solo se resincroniza cuando la deriva es de verdad.
            let drift = state.position_ms as i64 - self.now.position_ms as i64;
            let was_unknown = self.now.position_unknown;
            if changed || (state.position_known && was_unknown) {
                self.now.position_ms = state.position_ms;
                self.now.position_seen_ms = state.position_ms;
                self.now.position_at = Instant::now();
            } else if state.position_known && (drift.abs() > 1500 || drift < -700) {
                self.now.position_ms = state.position_ms;
                self.now.position_seen_ms = state.position_ms;
                self.now.position_at = Instant::now();
            }
            self.now.playing = state.playing;
            self.now.position_unknown = !state.position_known;
            self.now.path = None;
            if changed {
                self.lyrics = None;
                self.lyrics_status.clear();
                self.lyrics_key.clear();
                // Una petición de la pista anterior no debe bloquear la nueva:
                // el worker descarta por key y el flag se queda colgado.
                self.lyrics_loading = false;
                self.lyrics_auto_offset = None;
                self.reset_lyrics_view();
                self.art = None;
                self.art_key.clear();
                self.lyrics_offset_ms = self.lib.lyrics_offset(&track.key).unwrap_or(0);
                if !matched_local {
                    self.lib.upsert_track(&track).ok();
                }
                let has_local = self
                    .lib
                    .art_path(&track.key)
                    .ok()
                    .flatten()
                    .is_some_and(|path| path.exists())
                    || self
                        .lib
                        .local_path(&track.key)
                        .map(|path| path.with_extension("jpg").exists())
                        .unwrap_or(false);
                if !has_local {
                    if let Some(url) = track.art_url.clone() {
                        self.lib.set_art(&track.key, None, Some(&url)).ok();
                    }
                }
                self.load_lyrics(&track);
            }
        }
        if let Some(volume) = state.volume {
            self.now.volume = volume.clamp(0.0, 1.0);
        }
        // El estado de shuffle/repeat lo manda el reproductor externo. Se
        // refleja en la UI/mini/MPRIS, pero no se guarda como preferencia
        // local: al desactivar follow se restaura la de spote.
        if let Some(shuffle) = state.shuffle {
            self.shuffle_on = shuffle;
        }
        if let Some(repeat) = &state.repeat {
            self.repeat_mode = match repeat.as_str() {
                "Track" | "track" | "one" => RepeatMode::One,
                "Playlist" | "playlist" | "all" => RepeatMode::All,
                _ => RepeatMode::Off,
            };
            self.loop_file = self.repeat_mode == RepeatMode::One;
        }
        self.spotify = Some(state);
    }

    // --------------------------------------------------------------- letras

    pub(super) fn poll_spotify(&mut self, _dt: f32) {
        if !self.follow_spotify {
            return;
        }
        if let Some(worker) = &self.spotify_worker {
            if let Some(state) = worker.try_recv() {
                self.apply_spotify(state);
                self.last_spotify_poll = Instant::now();
            }
        }
        if self
            .spotify
            .as_ref()
            .map(|state| state.player.is_empty())
            .unwrap_or(true)
            && !self.spotify_warned
            && self.last_spotify_poll.elapsed() > Duration::from_secs(4)
        {
            self.spotify_warned = true;
            self.toast(
                "ningún reproductor MPRIS activo · abre Spotify, SoundCloud o VLC",
                ToastKind::Warn,
            );
        }
    }

    pub fn toggle_spotify(&mut self) {
        if self.follow_spotify {
            self.disable_spotify();
        } else {
            self.enable_spotify();
        }
    }
}
