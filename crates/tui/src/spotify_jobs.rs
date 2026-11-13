//! Trabajos de Spotify en segundo plano: importar catálogo y recolectar audio.
//!
//! Vive fuera de `app.rs` a propósito: el estado de la UI está en construcción
//! activa y aquí sólo hay lógica de trabajos + un `impl App` con los manejadores.
//! Así los enganches en `app.rs` se reducen a un campo, una inicialización y una
//! llamada por tick.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use spote_core::catalog::{self, Catalog};
use spote_core::config::Config;
use spote_core::discord::{Activity, DiscordWorker};
use spote_core::harvest::{self, HarvestOptions, Progress};

use crate::app::{App, ToastKind};

/// Eventos que llegan desde los hilos de trabajo.
pub enum JobEvent {
    Catalog(Catalog),
    CatalogFailed(String),
    HarvestProgress(Progress),
    HarvestDone {
        path: PathBuf,
        pages: usize,
        coverage: f64,
        seconds: f64,
        speed: f64,
        key: String,
        title: String,
    },
    HarvestFailed(String),
}

pub struct SpotifyJobs {
    tx: Sender<JobEvent>,
    rx: Receiver<JobEvent>,
}

impl SpotifyJobs {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { tx, rx }
    }

    /// Carga una colección pública (playlist, álbum, artista o pista).
    pub fn import(&self, url: String) {
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("spote-catalog".into())
            .spawn(move || match catalog::fetch(&url) {
                Ok(catalog) => {
                    let _ = tx.send(JobEvent::Catalog(catalog));
                }
                Err(err) => {
                    let _ = tx.send(JobEvent::CatalogFailed(err.to_string()));
                }
            })
            .ok();
    }

    /// Extrae de la memoria del cliente la pista que está sonando.
    pub fn harvest(&self, target: Option<String>, out_dir: PathBuf) {
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("spote-harvest".into())
            .spawn(move || {
                let mut options = HarvestOptions::new(out_dir);
                options.target = target;
                let progress = tx.clone();
                let result = harvest::harvest(&options, move |p| {
                    let _ = progress.send(JobEvent::HarvestProgress(p.clone()));
                });
                match result {
                    Ok(report) => {
                        let _ = tx.send(JobEvent::HarvestDone {
                            path: report.path,
                            pages: report.pages,
                            coverage: report.coverage,
                            seconds: report.seconds,
                            speed: report.speed,
                            key: report.track.key.clone(),
                            title: report.track.label(),
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(JobEvent::HarvestFailed(err.to_string()));
                    }
                }
            })
            .ok();
    }

    fn try_recv(&self) -> Option<JobEvent> {
        self.rx.try_recv().ok()
    }
}

impl App {
    /// Engancha los trabajos al bucle de la UI. Se llama desde `tick`.
    pub fn poll_spotify_jobs(&mut self) {
        self.discord.refresh(&self.now);
        while let Some(event) = self.spotify_jobs.try_recv() {
            match event {
                JobEvent::Catalog(catalog) => self.apply_catalog(catalog),
                JobEvent::CatalogFailed(message) => {
                    self.search.message = format!("no pude cargar: {message}");
                    self.toast("catálogo de Spotify: error", ToastKind::Error);
                }
                JobEvent::HarvestProgress(progress) => {
                    self.set_status(&format!(
                        "extrayendo {:.0}s/{:.0}s · {:.0}% · {:.2}×",
                        progress.position_ms as f64 / 1000.0,
                        progress.total_ms as f64 / 1000.0,
                        progress.coverage,
                        progress.speed
                    ));
                }
                JobEvent::HarvestDone {
                    path,
                    pages,
                    coverage,
                    seconds,
                    speed,
                    key,
                    title,
                } => {
                    if let Ok(Some(row)) = self.lib.track_by_key(&key) {
                        let size = std::fs::metadata(&path).ok().map(|meta| meta.len());
                        self.lib
                            .set_source(
                                row.id,
                                "harvest",
                                None,
                                Some(&path),
                                spote_core::library::SourceStatus::Ready,
                                size,
                            )
                            .ok();
                    }
                    self.toast(
                        &format!("extraída «{title}» · {coverage:.1}% · {speed:.1}×"),
                        ToastKind::Ok,
                    );
                    self.set_status(&format!(
                        "guardada en {} ({pages} páginas, {seconds:.0}s)",
                        path.display()
                    ));
                }
                JobEvent::HarvestFailed(message) => {
                    self.toast(&format!("no pude extraer: {message}"), ToastKind::Error);
                }
            }
        }
    }

    /// Deja las pistas del catálogo en la librería y encola su descarga.
    fn apply_catalog(&mut self, catalog: Catalog) {
        let refs = catalog.to_track_refs();
        let total = refs.len();

        // Alta en la librería. Si es un artista, no se crea playlist: sólo temas.
        for track in &refs {
            self.lib.upsert_track(track).ok();
        }
        if catalog.kind != "artist" {
            if let Ok(playlist_id) = match self
                .lib
                .playlists()
                .unwrap_or_default()
                .into_iter()
                .find(|playlist| playlist.name == catalog.name)
            {
                Some(existing) => Ok(existing.id),
                None => self.lib.create_playlist(&catalog.name),
            } {
                for track in &refs {
                    self.lib.add_to_playlist(playlist_id, &track.key).ok();
                }
            }
        }

        // Encola lo que falte, con matching estricto en el worker.
        let media = self.cfg.media_dir();
        let mut queued = 0;
        for track in &refs {
            if self.lib.local_path(&track.key).is_some() {
                continue;
            }
            self.downloader.resolve_and_download(
                &track.key,
                &track.artist,
                &track.title,
                track.duration_ms,
                media.clone(),
            );
            queued += 1;
        }

        self.toast(
            &format!("«{}»: {total} pistas, {queued} en cola", catalog.name),
            ToastKind::Ok,
        );
        self.set_status(&format!("importado desde Spotify: {}", catalog.name));
    }

    /// Arranca la extracción de lo que esté sonando en el cliente oficial.
    /// Sin `target`: se usa la pista que reporta MPRIS en ese momento.
    pub fn start_harvest(&mut self) {
        let out = self.cfg.media_dir();
        self.set_status("extrayendo del cliente…");
        self.toast("extrayendo la pista de Spotify (salta por ella)", ToastKind::Info);
        self.spotify_jobs.harvest(None, out);
    }
}

// ---------------------------------------------------------------------------
// Discord Rich Presence
// ---------------------------------------------------------------------------

/// Puente entre el reproductor y Discord.
///
/// Se actualiza desde `poll_spotify_jobs` (una llamada por tick); el trabajo
/// real y la reconexión viven en un hilo aparte, así que aquí no se bloquea la
/// UI nunca. La marca de tiempo se recalcula cuando cambia la pista o cuando el
/// usuario salta, que es lo único que Discord no puede deducir solo.
pub struct DiscordPresence {
    worker: Option<DiscordWorker>,
    signature: String,
    started_at_ms: Option<i64>,
    buttons: bool,
    small_image: String,
}

impl DiscordPresence {
    pub fn start(cfg: &Config) -> Self {
        let worker = (cfg.discord.enabled && !cfg.discord.client_id.trim().is_empty())
            .then(|| DiscordWorker::start(cfg.discord.client_id.clone()));
        Self {
            worker,
            signature: String::new(),
            started_at_ms: None,
            buttons: cfg.discord.buttons,
            small_image: cfg.discord.small_image.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.worker.is_some()
    }

    /// Recalcula y envía la actividad. Barato si nada cambió.
    pub fn refresh(&mut self, now: &crate::app::NowPlaying) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        let Some(track) = now.track.as_ref().filter(|_| now.playing) else {
            if !self.signature.is_empty() {
                self.signature.clear();
                self.started_at_ms = None;
                worker.update(None);
            }
            return;
        };

        let epoch = epoch_ms();
        // Nueva pista o salto: el inicio es "ahora menos la posición".
        let expected = self.started_at_ms.map(|start| epoch - start);
        let jumped = expected.is_none_or(|elapsed| elapsed.abs_diff(now.position_ms as i64) > 2_000);
        if self.signature != track.key || jumped {
            self.signature = track.key.clone();
            self.started_at_ms = Some(epoch - now.position_ms as i64);
        }

        let start = self.started_at_ms.unwrap_or(epoch);
        let duration = if now.duration_ms > 0 {
            now.duration_ms as i64
        } else {
            track.duration_ms as i64
        };

        let mut state = track.artist.clone();
        if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
            if !state.is_empty() {
                state.push_str(" · ");
            }
            state.push_str(album);
        }

        let mut buttons = Vec::new();
        if self.buttons {
            if let Some(id) = track.key.strip_prefix("spotify:track:") {
                buttons.push((
                    "Abrir en Spotify".to_string(),
                    format!("https://open.spotify.com/track/{id}"),
                ));
            }
        }

        worker.update(Some(Activity {
            details: track.title.clone(),
            state,
            start_ms: Some(start),
            end_ms: (duration > 0).then_some(start + duration),
            large_image: track.art_url.clone(),
            large_text: track.album.clone(),
            small_image: (!self.small_image.is_empty()).then(|| self.small_image.clone()),
            small_text: Some("spote".to_string()),
            buttons,
        }));
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
