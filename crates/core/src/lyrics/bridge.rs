//! Puente con el motor de letras de lyricbar (`/home/fabio/Code/lyricbar`).
//!
//! lyricbar resuelve LRCLIB, Genius, captions de YouTube y el reparto de letras
//! planas, y afina los tiempos con su caché. En vez de reimplementar eso en
//! Rust, se llama a su `LyricsService` desde un script Python y se convierte la
//! respuesta a `LyricsDoc`. Si falla cualquier cosa (no está el checkout, no
//! hay red, cambia su API) se devuelve `None` y spote usa su proveedor interno.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use spote_protocol::TrackRef;

use super::{LyricLine, LyricsDoc, LyricsError, LyricsSource};

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub python: String,
    pub script: PathBuf,
    pub root: PathBuf,
    pub timeout: Duration,
}

impl BridgeConfig {
    pub fn resolve(cfg: &crate::config::LyricsCfg) -> Option<Self> {
        if !cfg.bridge {
            return None;
        }
        let script = resolve_script(&cfg.script)?;
        let root = PathBuf::from(if cfg.root.is_empty() {
            std::env::var("LYRICBAR_ROOT").unwrap_or_else(|_| "/home/fabio/Code/lyricbar".into())
        } else {
            cfg.root.clone()
        });
        if !root.join("lyricbar").is_dir() {
            return None;
        }
        let mut python = if cfg.python.trim().is_empty() {
            "python3".to_string()
        } else {
            cfg.python.clone()
        };
        // Si el intérprete es el del sistema, preferimos el venv de lyricbar:
        // allí están requests, bs4 y compañía.
        if python == "python3" {
            let venv = root.join(".venv/bin/python");
            if venv.is_file() {
                python = venv.to_string_lossy().into_owned();
            }
        }
        Some(Self {
            python,
            script,
            root,
            // El ASR local puede tardar; solo bloquea el hilo de letras.
            timeout: Duration::from_secs(90),
        })
    }
}

/// Busca el script del puente en los sitios razonables.
fn resolve_script(configured: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(configured);
    if candidate.is_file() {
        return Some(candidate);
    }
    if let Some(root) = std::env::var_os("SPOTE_ROOT") {
        let path = PathBuf::from(root).join(configured);
        if path.is_file() {
            return Some(path);
        }
    }
    let local = Path::new(configured);
    if local.is_file() {
        return Some(local.to_path_buf());
    }
    let repo = Path::new("/home/fabio/Code/spote").join(configured);
    if repo.is_file() {
        return Some(repo);
    }
    None
}

#[derive(Debug, Deserialize)]
struct BridgeResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    synced: bool,
    #[serde(default)]
    estimated: bool,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    aligned: bool,
    #[serde(default)]
    lines: Vec<BridgeLine>,
}

#[derive(Debug, Deserialize)]
struct BridgeLine {
    start_ms: u64,
    #[serde(default)]
    end_ms: Option<u64>,
    text: String,
}

/// Pide la letra a lyricbar. Bloquea; se llama desde el hilo de letras.
pub fn fetch(
    bridge: &BridgeConfig,
    track: &TrackRef,
    audio: Option<&std::path::Path>,
) -> Result<Option<LyricsDoc>, LyricsError> {
    let payload = serde_json::json!({
        "title": track.title,
        "artists": [track.artist],
        "album": track.album,
        "duration_ms": track.duration_ms,
        "root": bridge.root.to_string_lossy(),
        "audio_path": audio.map(|path| path.to_string_lossy().into_owned()),
    });

    let mut child = Command::new(&bridge.python)
        .arg(&bridge.script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| LyricsError::Http(format!("puente: {err}")))?;

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(serde_json::to_string(&payload).unwrap_or_default().as_bytes());
    }
    drop(child.stdin.take());

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > bridge.timeout => {
                let _ = child.kill();
                return Ok(None);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => return Ok(None),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| LyricsError::Http(format!("puente: {err}")))?;
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let parsed: BridgeResponse = serde_json::from_slice(&output.stdout)
        .map_err(|err| LyricsError::Parse(format!("puente: {err}")))?;
    if !parsed.ok {
        let _ = parsed.error;
        return Ok(None);
    }
    if parsed.lines.is_empty() {
        return Ok(None);
    }
    // Unas líneas estimadas de Genius no son karaoke: si no se pudieron
    // alinear contra el audio, se prefiere que el proveedor interno pruebe
    // LRCLIB antes que aceptar tiempos inventados.
    if !parsed.aligned && (!parsed.synced || parsed.estimated) {
        return Ok(None);
    }

    let mut lines: Vec<LyricLine> = parsed
        .lines
        .into_iter()
        .map(|line| LyricLine {
            start_ms: line.start_ms,
            end_ms: line.end_ms.unwrap_or(0),
            text: line.text,
            words: Vec::new(),
            words_exact: false,
        })
        .collect();
    lines.sort_by_key(|line| line.start_ms);
    for i in 0..lines.len() {
        let next = lines
            .get(i + 1)
            .map(|line| line.start_ms)
            .unwrap_or(lines[i].start_ms + 6_000);
        if lines[i].end_ms <= lines[i].start_ms || lines[i].end_ms > next + 50 {
            lines[i].end_ms = next;
        }
    }

    let source = match parsed.provider.as_str() {
        "lrclib" => LyricsSource::Lrclib,
        _ => LyricsSource::Local,
    };
    tracing::info!(
        target: "spote::lyrics",
        "letra desde lyricbar: {} lineas ({}), provider={}",
        lines.len(),
        if parsed.synced { "sincronizada" } else { "estimada" },
        parsed.provider
    );
    Ok(Some(LyricsDoc {
        lines,
        synced: (parsed.synced && !parsed.estimated) || parsed.aligned,
        instrumental: false,
        source,
        offset_ms: 0,
        aligned: parsed.aligned,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_requires_a_script() {
        let cfg = crate::config::LyricsCfg {
            bridge: true,
            python: "python3".into(),
            script: "/no/existe/lyricbar_bridge.py".into(),
            root: "/home/fabio/Code/lyricbar".into(),
        };
        assert!(BridgeConfig::resolve(&cfg).is_none());
    }

    #[test]
    fn resolve_picks_the_venv_when_available() {
        let cfg = crate::config::LyricsCfg::default();
        if let Some(bridge) = BridgeConfig::resolve(&cfg) {
            assert!(bridge.script.is_file());
            assert!(bridge.python.contains("lyricbar"));
        }
    }
}
