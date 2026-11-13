//! Estado de descargas y su progreso.
use super::*;

impl App {
    pub fn title_for_key(&self, key: &str) -> String {
        if let Some(track) = &self.now.track {
            if track.key == key {
                return format!("{} · {}", track.title, track.artist);
            }
        }
        if let Some(row) = self.view.iter().find(|row| row.track.key == key) {
            return format!("{} · {}", row.track.title, row.track.artist);
        }
        if let Some(entry) = self.queue.iter().find(|entry| entry.track.key == key) {
            return format!("{} · {}", entry.track.title, entry.track.artist);
        }
        if let Some(item) = self.search.results.iter().find(|item| item.to_track().key == key) {
            return format!("{} · {}", item.title, item.artist);
        }
        key.to_string()
    }

    /// Alta/actualización de una descarga. Se guarda el paso para que la sesión
    /// muestre qué está pasando aunque cambies de pestaña.

    pub fn track_download(&mut self, key: String, stage: String, percent: f32, failed: bool) {
        let percent = percent.clamp(0.0, 100.0);
        if let Some(info) = self.downloads.iter_mut().find(|info| info.key == key) {
            info.stage = stage;
            info.percent = percent;
            info.failed = failed;
            return;
        }
        let title = self.title_for_key(&key);
        self.downloads.push(DownloadInfo {
            key,
            title,
            stage,
            percent,
            failed,
            finished_at: None,
        });
    }


    pub(super) fn finish_download(&mut self, key: &str, failed: bool) {
        let title = self.title_for_key(key);
        if let Some(info) = self.downloads.iter_mut().find(|info| info.key == key) {
            info.stage = if failed { "error".into() } else { "lista".into() };
            info.percent = if failed { info.percent } else { 100.0 };
            info.failed = failed;
            info.finished_at = Some(Instant::now());
            return;
        }
        self.downloads.push(DownloadInfo {
            key: key.to_string(),
            title,
            stage: if failed { "error".into() } else { "lista".into() },
            percent: if failed { 0.0 } else { 100.0 },
            failed,
            finished_at: Some(Instant::now()),
        });
    }

    /// Descarga activa de una pista (para el badge de la fila).

    pub fn download_for(&self, key: &str) -> Option<&DownloadInfo> {
        self.downloads
            .iter()
            .find(|info| info.key == key && info.active())
    }

    /// Tira las descargas terminadas que ya nadie necesita ver.

    pub(super) fn prune_downloads(&mut self) {
        let now = Instant::now();
        self.downloads.retain(|info| match info.finished_at {
            Some(done) => now.duration_since(done) < Duration::from_secs(6),
            None => true,
        });
        // No acumular si se encolan muchas de golpe.
        if self.downloads.len() > 8 {
            self.downloads.drain(..self.downloads.len() - 8);
        }
    }

}
