//! Grabación de la salida del sistema.
//!
//! Es el único camino para contenido que no existe en ninguna otra fuente:
//! exclusivas de Spotify (*Spotify Singles*, sesiones), podcasts y audiolibros.
//! Se captura el stream ya decodificado —sin DRM, sin claves— desde el monitor
//! del sink por defecto, así que la calidad es la del cliente y no hay
//! recompresión adicional si se guarda en FLAC.
//!
//! Coste: 1× tiempo real. No hay atajo: el cliente descifra justo a tiempo, así
//! que ni leyendo su memoria se puede ir más rápido.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use spote_protocol::TrackRef;

use crate::spotify::SpotifyWatcher;
use crate::util;

/// Formato del fichero final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFormat {
    /// Opus 160 kbps: lo mismo que el resto de la librería, ~6 MB por pista.
    Opus,
    /// FLAC: captura sin pérdida del stream decodificado. ~30-50 MB por pista.
    Flac,
}

#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub format: RecordFormat,
    /// Rebobina al principio antes de grabar (lo normal si quieres la pista entera).
    pub from_start: bool,
    /// Corta a los N segundos aunque la pista siga.
    pub seconds: Option<u64>,
    pub out_dir: PathBuf,
}

impl RecordOptions {
    pub fn new(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            format: RecordFormat::Opus,
            from_start: true,
            seconds: None,
            out_dir: out_dir.into(),
        }
    }
}

/// Resultado de una grabación.
#[derive(Debug, Clone)]
pub struct Recording {
    pub path: PathBuf,
    pub track: TrackRef,
    pub seconds: f64,
    pub peak_db: Option<f64>,
    /// Ganancia aplicada al fichero final (negativa = atenuado para no recortar).
    pub gain_db: f64,
}

/// Grabación en curso. El consumidor decide cuándo parar y llama a `finish`.
pub struct Recorder {
    parec: Child,
    encoder: Child,
    started: Instant,
    master: PathBuf,
    work_dir: PathBuf,
    track: TrackRef,
    /// Duración objetivo de la grabación (la pista, o el límite pedido).
    target_ms: u64,
}

impl Recorder {
    /// Arranca la captura. Si `from_start`, rebobina la pista en el cliente.
    pub fn start(options: &RecordOptions) -> Result<Self> {
        if util::which("parec").is_none() {
            return Err(anyhow!(
                "hace falta `parec` (pipewire-pulse / pulseaudio-utils) para grabar la salida"
            ));
        }
        if util::which("ffmpeg").is_none() {
            return Err(anyhow!("hace falta `ffmpeg` para codificar la grabación"));
        }

        let mut watcher = SpotifyWatcher::new();
        let state = watcher
            .poll()
            .ok_or_else(|| anyhow!("no veo el cliente de Spotify por MPRIS"))?;
        let track = state
            .track
            .ok_or_else(|| anyhow!("Spotify no está reproduciendo nada"))?;

        if options.from_start {
            // Pausa, rebobina y luego reanudamos ya con la captura en marcha.
            let _ = crate::mpris::call("Pause", &());
            if let Some(id) = track.key.strip_prefix("spotify:track:") {
                let _ = crate::mpris::set_position(id, 0);
            }
        }

        let work_dir = crate::config::cache_dir().join("record");
        std::fs::create_dir_all(&work_dir).context("no se pudo crear el directorio temporal")?;
        let stem = {
            let slug = util::slug(&format!("{} {}", track.artist, track.title), 70);
            if slug.is_empty() {
                "recording".to_string()
            } else {
                slug
            }
        };
        let master = work_dir.join(format!("{stem}.wav"));

        // float32: el Vorbis decodifica por encima de 1.0 en masters fuertes y
        // convertir a s16 recortaría. En float no se pierde nada.
        let mut parec = Command::new("parec")
            .args([
                "--format=float32le",
                "--rate=44100",
                "--channels=2",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("no se pudo arrancar parec")?;

        let stdout = parec
            .stdout
            .take()
            .ok_or_else(|| anyhow!("parec no expone stdout"))?;

        // WAV float32 como master: sin pérdida y sin límites de encoder.
        let encoder = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "f32le",
                "-ar",
                "44100",
                "-ac",
                "2",
                "-i",
                "-",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&master)
            .stdin(Stdio::from(stdout))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("no se pudo arrancar ffmpeg")?;

        if state.playing || options.from_start {
            // Esperar a que la captura esté viva (parec tarda ~1 s en conectar y
            // mientras tanto no se graba nada): si damos play antes, se pierde
            // el principio de la pista.
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                let size = std::fs::metadata(&master).map(|meta| meta.len()).unwrap_or(0);
                if size > 64 * 1024 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            let _ = crate::mpris::call("Play", &());
        }

        Ok(Self {
            parec,
            encoder,
            started: Instant::now(),
            master,
            work_dir,
            target_ms: options
                .seconds
                .map(|seconds| seconds * 1000)
                .unwrap_or(track.duration_ms),
            track,
        })
    }

    pub fn track(&self) -> &TrackRef {
        &self.track
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Para la captura, codifica al formato final y devuelve el resultado.
    pub fn finish(mut self, format: RecordFormat, out_dir: &Path) -> Result<Recording> {
        let seconds = self.started.elapsed().as_secs_f64();

        // Matar parec cierra el pipe: ffmpeg ve EOF y finaliza el FLAC.
        let _ = self.parec.kill();
        let _ = self.parec.wait();
        let status = self.encoder.wait().context("ffmpeg no terminó")?;
        if !status.success() {
            return Err(anyhow!("ffmpeg falló al cerrar la grabación"));
        }
        if !self.master.exists() || std::fs::metadata(&self.master)?.len() < 4096 {
            return Err(anyhow!("la grabación salió vacía (¿hay audio sonando?)"));
        }

        let peak_db = measure_peak(&self.master).ok();
        // Llevamos el pico a -1 dBFS: si el máster decodifica por encima de 0
        // atenuamos (evita clipping), y si la captura salió floja amplificamos.
        // Dejamos 3 dB de margen: Opus sobremodula en transientes y con -1 dB
        // el fichero final volvía a pasarse de 0 dBFS.
        let gain_db = peak_db.map(|peak| -3.0 - peak).unwrap_or(0.0);
        // El arranque de parec deja 1-2 s de silencio: se recorta antes de
        // aplicar la duración objetivo, que de paso corta la pista siguiente si
        // el cliente ya cambió.
        let filters = format!(
            "silenceremove=start_periods=1:start_silence=0:start_threshold=-50dB,volume={gain_db:.2}dB"
        );
        let limit = (self.target_ms > 500).then(|| (self.target_ms as f64 / 1000.0) - 0.05);

        std::fs::create_dir_all(out_dir).ok();
        let slug = util::slug(&format!("{} {}", self.track.artist, self.track.title), 70);
        let stem = if slug.is_empty() {
            "recording".to_string()
        } else {
            slug
        };
        let path = match format {
            RecordFormat::Flac => {
                let target = out_dir.join(format!("{stem}.flac"));
                let mut command = Command::new("ffmpeg");
                command.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
                command.arg(&self.master).args(["-af", &filters]);
                if let Some(limit) = limit {
                    command.args(["-t", &format!("{limit:.3}")]);
                }
                command.args(["-c:a", "flac"]).arg(&target);
                let status = command.status().context("no se pudo convertir a FLAC")?;
                if !status.success() {
                    return Err(anyhow!("ffmpeg falló al convertir a FLAC"));
                }
                target
            }
            RecordFormat::Opus => {
                let target = out_dir.join(format!("{stem}.opus"));
                let mut command = Command::new("ffmpeg");
                command.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
                command.arg(&self.master).args(["-af", &filters]);
                if let Some(limit) = limit {
                    command.args(["-t", &format!("{limit:.3}")]);
                }
                command
                    .args(["-c:a", "libopus", "-b:a", "160k"])
                    .arg(&target)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let status = command.status().context("no se pudo reencodificar a Opus")?;
                if !status.success() {
                    return Err(anyhow!("ffmpeg falló al convertir a Opus"));
                }
                target
            }
        };

        let _ = std::fs::remove_file(&self.master);
        let _ = std::fs::remove_dir(&self.work_dir);
        let seconds = probe_duration(&path).unwrap_or(seconds);

        Ok(Recording {
            path,
            track: self.track,
            seconds,
            peak_db,
            gain_db,
        })
    }
}

/// Pico real en dBFS del fichero, **sin recortar a 0**: el material decodificado
/// puede pasar de full scale y `volumedetect` lo tapa (siempre dice 0.0).
fn measure_peak(path: &Path) -> Result<f64> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .args(["-af", "astats=metadata=0", "-f", "null", "-"])
        .output()
        .context("no se pudo medir el pico")?;
    let text = String::from_utf8_lossy(&output.stderr);
    let mut peak: Option<f64> = None;
    for line in text.lines() {
        if let Some(rest) = line.split("Peak level dB:").nth(1) {
            if let Ok(value) = rest.trim().parse::<f64>() {
                // astats imprime una línea por canal: nos quedamos con la mayor.
                peak = Some(peak.map_or(value, |current: f64| current.max(value)));
            }
        }
    }
    peak.ok_or_else(|| anyhow!("astats no reportó Peak level"))
}

/// Duración real del fichero, en segundos.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opciones_por_defecto() {
        let options = RecordOptions::new("/tmp/x");
        assert_eq!(options.format, RecordFormat::Opus);
        assert!(options.from_start);
        assert!(options.seconds.is_none());
    }

    #[test]
    fn detecta_pico_inexistente() {
        // Un fichero que no existe no debe romper: el error se propaga como Result.
        assert!(measure_peak(Path::new("/tmp/no-existe-jamas.flac")).is_err());
    }
}
