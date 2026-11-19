//! Puente opcional con el motor de letras de lyricbar.
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
        // Sin ruta configurada ni variable de entorno, se usa el lyricbar
        // que haya enlazado el instalador en el directorio de datos.
        let root = if cfg.root.trim().is_empty() {
            let env_root = std::env::var_os("LYRICBAR_ROOT").map(PathBuf::from);
            let managed = crate::config::data_dir().join("lyricbar");
            match env_root {
                Some(path) if path.join("lyricbar").is_dir() => path,
                _ if managed.join("lyricbar").is_dir() => managed,
                _ => return None,
            }
        } else {
            PathBuf::from(&cfg.root)
        };
        if !root.join("lyricbar").is_dir() {
            return None;
        }
        let mut python = if cfg.python.trim().is_empty() {
            "python3".to_string()
        } else {
            cfg.python.clone()
        };
        // Si el intérprete es el del sistema, preferimos el venv gestionado
        // (o el de lyricbar), donde están requests, bs4, yt-dlp y compañía.
        if python == "python3" {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(path) = std::env::var_os("SPOTE_LYRICBAR_PYTHON") {
                candidates.push(PathBuf::from(path));
            }
            candidates.push(crate::config::data_dir().join("venv/bin/python"));
            candidates.push(root.join(".venv/bin/python"));
            if let Some(venv) = candidates.into_iter().find(|path| path.is_file()) {
                python = venv.to_string_lossy().into_owned();
            }
        }
        Some(Self {
            python,
            script,
            root,
            // El ASR local puede tardar (lectura completa + relecturas de las
            // zonas cojas); solo bloquea el hilo de letras.
            timeout: Duration::from_secs(150),
        })
    }
}

/// Busca un script auxiliar en los sitios razonables.
pub(crate) fn resolve_script(configured: &str) -> Option<PathBuf> {
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
    // Instalación sin el checkout: los scripts se copian junto a los datos.
    if let Some(name) = Path::new(configured).file_name() {
        let installed = crate::config::data_dir().join("scripts").join(name);
        if installed.is_file() {
            return Some(installed);
        }
    }
    // Raíz del workspace deducida en compilación, sin rutas de nadie.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(configured);
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
pub fn fetch(bridge: &BridgeConfig, track: &TrackRef) -> Result<Option<LyricsDoc>, LyricsError> {
    let payload = serde_json::json!({
        "title": track.title,
        "artists": [track.artist],
        "album": track.album,
        "duration_ms": track.duration_ms,
        "root": bridge.root.to_string_lossy(),
    });

    let mut child = Command::new(&bridge.python)
        .arg(&bridge.script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| LyricsError::Http(format!("puente: {err}")))?;

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(
            serde_json::to_string(&payload)
                .unwrap_or_default()
                .as_bytes(),
        );
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
    // Genius devuelve la letra sin marcas; lyricbar la reparte por la duración
    // y la marca `estimated`. No es karaoke exacto, pero es perfectamente
    // utilizable como letra "aprox" (y se afina con ASR si hay audio local).
    // Antes se descartaba: spote decía "sin letra" mientras lyricbar sí la
    // había encontrado. Solo se rechaza lo que no tiene ninguna línea.

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
    let plain = !parsed.synced && !parsed.aligned;
    // Si lyricbar ya repartió los tiempos (cuando conoce la duración), los
    // respetamos: son mejores que un reparto propio. Solo repartimos cuando
    // todas las marcas están a 0, que es lo que llega sin duración.
    if plain && lines.iter().all(|line| line.start_ms == 0) {
        distribute_plain_lines(&mut lines, track.duration_ms);
    }
    if parsed.aligned {
        sanitize_asr_lines(&mut lines);
    }
    for i in 0..lines.len() {
        let next = lines
            .get(i + 1)
            .map(|line| line.start_ms)
            .unwrap_or(lines[i].start_ms + 6_000);
        if lines[i].end_ms <= lines[i].start_ms || lines[i].end_ms > next + 50 {
            lines[i].end_ms = next;
        }
    }

    let synced = (parsed.synced && !parsed.estimated) || parsed.aligned;
    let source = if plain && parsed.estimated {
        LyricsSource::Estimated
    } else if plain {
        LyricsSource::Plain
    } else {
        match parsed.provider.as_str() {
            "lrclib" => LyricsSource::Lrclib,
            _ => LyricsSource::Local,
        }
    };
    tracing::info!(
        target: "spote::lyrics",
        "letra desde lyricbar: {} lineas ({}), provider={}",
        lines.len(),
        if synced { "sincronizada" } else { "plana" },
        parsed.provider
    );
    Ok(Some(LyricsDoc {
        lines,
        synced,
        instrumental: false,
        source,
        offset_ms: 0,
        aligned: parsed.aligned,
    }))
}

/// Reparte líneas sin marcas temporales a lo largo de la canción. Es el mismo
/// criterio que `parse_lrc` usa para letra plana, con un techo para que un LRC
/// de 60 líneas no se coma toda la pista.
fn distribute_plain_lines(lines: &mut [LyricLine], duration_ms: u64) {
    if lines.is_empty() {
        return;
    }
    let per_line = if duration_ms > 0 {
        (duration_ms / lines.len() as u64).clamp(2_000, 6_000)
    } else {
        4_000
    };
    for (index, line) in lines.iter_mut().enumerate() {
        line.start_ms = index as u64 * per_line;
        line.end_ms = (index as u64 + 1) * per_line;
    }
}

/// El alineado forzado puede emparejar solo la mitad de los tokens de un verso
/// (letra repetida, estribillos, adlibs) y anclar la línea siguiente en mitad
/// de la anterior. En ese caso la frase "cambia a mitad de frase" desde el
/// propio origen; lo empujamos al final del verso anterior.
fn sanitize_asr_lines(lines: &mut [LyricLine]) {
    lines.sort_by_key(|line| line.start_ms);
    for i in 1..lines.len() {
        let previous = &lines[i - 1];
        let span = previous.end_ms.saturating_sub(previous.start_ms);
        if span == 0 {
            continue;
        }
        let midpoint = previous.start_ms + span / 2;
        if lines[i].start_ms >= midpoint && lines[i].start_ms < previous.end_ms {
            tracing::warn!(
                target: "spote::lyrics",
                "start ASR {} cae dentro del verso anterior [{}, {}]; se desplaza al final",
                lines[i].start_ms,
                previous.start_ms,
                previous.end_ms
            );
            lines[i].start_ms = previous.end_ms;
        }
    }
    lines.sort_by_key(|line| line.start_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comprobación manual del caso reportado. Se ejecuta con:
    /// `LYRICBAR_ROOT=… cargo test -p spote-core bridge_finds_estimated -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita checkout de lyricbar y red"]
    fn bridge_finds_estimated_lyrics_from_genius() {
        let root = std::env::var("LYRICBAR_ROOT").unwrap_or_default();
        if root.is_empty() {
            return;
        }
        let config = BridgeConfig {
            python: "python3".into(),
            script: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/lyricbar_bridge.py"),
            root: PathBuf::from(root),
            timeout: Duration::from_secs(90),
        };
        let mut track = TrackRef::new("test:si-me-matas", "Si me matas", "9louro");
        track.duration_ms = 203_000;
        let doc = fetch(&config, &track)
            .expect("llamada al puente")
            .expect("el puente encontró la letra en Genius");
        assert!(!doc.lines.is_empty());
        assert!(!doc.synced, "es una letra estimada, no un LRC con marcas");
        println!("{} líneas, fuente {:?}", doc.lines.len(), doc.source);
    }

    /// Igual que el test anterior pero por la API real (`LyricsService`),
    /// incluido el fallback a LRCLIB interno cuando la letra es estimada.
    #[tokio::test]
    #[ignore = "necesita checkout de lyricbar y red"]
    async fn service_returns_estimated_lyrics_from_genius() {
        let root = std::env::var("LYRICBAR_ROOT").unwrap_or_default();
        if root.is_empty() {
            return;
        }
        let config = BridgeConfig {
            python: "python3".into(),
            script: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/lyricbar_bridge.py"),
            root: PathBuf::from(root),
            timeout: Duration::from_secs(90),
        };
        let mut track = TrackRef::new("test:si-me-matas", "Si me matas", "9louro");
        track.duration_ms = 203_000;
        use crate::lyrics::LyricsService;
        let service = LyricsService::with_bridge(config);
        let doc = service
            .fetch(&track)
            .await
            .expect("sin error de red")
            .expect("la letra de Genius debe llegar a la app");
        assert!(!doc.lines.is_empty());
        assert!(!doc.synced);
        println!("{} líneas, fuente {:?}", doc.lines.len(), doc.source);
    }

    #[test]
    fn plain_bridge_lines_get_even_timing() {
        let mut lines: Vec<LyricLine> = (0..4)
            .map(|i| LyricLine {
                text: format!("verso {i}"),
                ..LyricLine::default()
            })
            .collect();
        distribute_plain_lines(&mut lines, 16_000);
        assert_eq!(lines[0].start_ms, 0);
        assert_eq!(lines[1].start_ms, 4_000);
        assert_eq!(lines[3].end_ms, 16_000);
        // Sin duración, 4 s por línea como en `parse_lrc`.
        distribute_plain_lines(&mut lines, 0);
        assert_eq!(lines[2].start_ms, 8_000);
    }

    #[test]
    fn asr_start_inside_previous_verse_is_pushed_to_its_end() {
        let mut lines = vec![
            LyricLine {
                start_ms: 10_000,
                end_ms: 14_000,
                text: "primera".into(),
                ..LyricLine::default()
            },
            LyricLine {
                start_ms: 12_000, // mitad de la frase anterior
                end_ms: 16_000,
                text: "segunda".into(),
                ..LyricLine::default()
            },
        ];
        sanitize_asr_lines(&mut lines);
        assert_eq!(lines[1].start_ms, 14_000);
    }

    #[test]
    fn asr_start_early_in_the_previous_verse_is_left_alone() {
        let mut lines = vec![
            LyricLine {
                start_ms: 10_000,
                end_ms: 14_000,
                text: "primera".into(),
                ..LyricLine::default()
            },
            LyricLine {
                start_ms: 10_500, // solapamiento legítimo al principio
                end_ms: 14_500,
                text: "segunda".into(),
                ..LyricLine::default()
            },
        ];
        sanitize_asr_lines(&mut lines);
        assert_eq!(lines[1].start_ms, 10_500);
    }

    #[test]
    fn resolve_requires_a_script() {
        let cfg = crate::config::LyricsCfg {
            bridge: true,
            python: "python3".into(),
            script: "/no/existe/lyricbar_bridge.py".into(),
            root: "/no/existe/lyricbar".into(),
            ..crate::config::LyricsCfg::default()
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
