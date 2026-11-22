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
        self.pending_stream = None;
        self.pending_play = None;
        self.streaming = false;
        self.now.loading = false;
        if let Some(player) = &self.player {
            player.stop();
        }
        self.now.playing = false;
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
        // Vuelve el modo local limpio: fuera la pista remota.
        self.spotify = None;
        self.now.track = None;
        self.now.path = None;
        self.now.position_ms = 0;
        self.now.playing = false;
        self.lyrics = None;
        self.lyrics_key.clear();
        self.lyrics_status.clear();
        self.art = None;
        self.art_key.clear();
        self.art_variants.clear();
        self.toast(
            "modo sistema desactivado · vuelve el modo local",
            ToastKind::Info,
        );
    }

    pub(super) fn apply_spotify(&mut self, state: SystemMediaState) {
        if let Some(mut track) = state.track.clone() {
            // Si la pista está descargada, usa SU clave (y su portada local):
            // MPRIS da otro id y si no, la letra/arte no encuentran nada.
            if let Ok(Some(row)) =
                self.lib
                    .find_by_meta(&track.artist, &track.title, track.duration_ms)
            {
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
            if changed || drift.abs() > 1500 || drift < -700 {
                self.now.position_ms = state.position_ms;
                self.now.position_seen_ms = state.position_ms;
                self.now.position_at = Instant::now();
            }
            self.now.playing = state.playing;
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
                self.lib.upsert_track(&track).ok();
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
        self.spotify = Some(state);
    }

    // --------------------------------------------------------------- letras

    pub(super) fn poll_spotify(&mut self, dt: f32) {
        if !self.follow_spotify {
            return;
        }
        if let Some(worker) = &self.spotify_worker {
            if let Some(state) = worker.try_recv() {
                self.apply_spotify(state);
                self.last_spotify_poll = Instant::now();
            }
        }
        if self.spotify.is_none()
            && !self.spotify_warned
            && self.last_spotify_poll.elapsed() > Duration::from_secs(4)
        {
            self.spotify_warned = true;
            self.toast(
                "ningún reproductor MPRIS activo · abre Spotify, SoundCloud o VLC",
                ToastKind::Warn,
            );
        }
        // Interpola la posición entre sondeos de MPRIS.
        // MPRIS solo informa de la posición cada ~400 ms; entre sondeos el
        // reloj lo lleva spote. Antes se cortaba a los 3 s y la letra se
        // quedaba congelada.
        if self.now.playing {
            self.now.position_ms = self
                .now
                .position_ms
                .saturating_add((dt * 1000.0) as u64)
                .min(self.now.duration_ms.max(self.now.position_ms));
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
