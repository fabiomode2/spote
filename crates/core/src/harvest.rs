//! Extracción del audio ya descifrado desde la memoria del cliente.
//!
//! El cliente de Spotify descifra **justo a tiempo**: mantiene ~2 s por delante
//! del punto de reproducción y libera los buffers enseguida. Eso significa que
//! leer su memoria no acelera nada… salvo que se le obligue a descifrar otras
//! posiciones: con `SetPosition` cada salto genera una ventana nueva de ~2 s.
//!
//! El truco, medido: saltar en pasos de ~1 s, escanear sólo las regiones de
//! memoria que ya sabemos que contienen páginas Ogg (decenas de MB, no 800) y
//! quedarse con las páginas cuya posición (granule) cae en la ventana pedida.
//! Sale ~2-5× tiempo real y las páginas son las originales: **no se reencoda
//! nada**, el `.ogg` resultante es el Vorbis de 160 kbps del CDN.
//!
//! Sin `ptrace`: sólo `pread` sobre `/proc/<pid>/mem`, mismo usuario. Linux.

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use spote_protocol::TrackRef;

use crate::mpris;
use crate::spotify::SpotifyWatcher;

const SAMPLE_RATE: f64 = 44_100.0;
const CHUNK: usize = 8 << 20;
/// Solape entre trozos para no cortar una página por la mitad.
const OVERLAP: usize = 64 << 10;
/// Silencio entre salto y escaneo: dar tiempo a que el cliente descifre.
const SEEK_SETTLE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone)]
pub struct HarvestOptions {
    /// URL/URI a abrir en el cliente; si es `None`, se usa lo que esté sonando.
    pub target: Option<String>,
    /// Paso del barrido. ~1 s da ~96 % de cobertura; menos, más cobertura y más tiempo.
    pub step_ms: u64,
    pub out_dir: PathBuf,
    /// Tope de tiempo del proceso completo.
    pub budget: Duration,
}

impl HarvestOptions {
    pub fn new(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            target: None,
            step_ms: 1_000,
            out_dir: out_dir.into(),
            budget: Duration::from_secs(900),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub position_ms: u64,
    pub total_ms: u64,
    pub pages: usize,
    pub coverage: f64,
    /// Velocidad respecto a tiempo real (2.4 = 2,4× más rápido que escuchar).
    pub speed: f64,
}

#[derive(Debug, Clone)]
pub struct HarvestReport {
    pub path: PathBuf,
    pub track: TrackRef,
    pub pages: usize,
    pub coverage: f64,
    pub seconds: f64,
    pub speed: f64,
}

#[derive(Debug, Clone)]
struct Page {
    data: Vec<u8>,
    /// Posición en segundos dentro de la pista (granule / 44100).
    gran: f64,
    header_type: u8,
}

/// Tabla CRC de Ogg (polinomio 0x04C11DB7, MSB-first, sin reflejar).
fn crc_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = (index as u32) << 24;
            for _ in 0..8 {
                value = if value & 0x8000_0000 != 0 {
                    (value << 1) ^ 0x04C1_1DB7
                } else {
                    value << 1
                };
            }
            *slot = value;
        }
        table
    })
}

fn ogg_crc(bytes: &[u8]) -> u32 {
    let table = crc_table();
    let mut crc: u32 = 0;
    for byte in bytes {
        crc = (crc << 8) ^ table[(((crc >> 24) & 0xFF) ^ *byte as u32) as usize];
    }
    crc
}

/// Busca páginas Ogg válidas en `buf` y se queda con las que caen en
/// `[lo, hi]` segundos. Devuelve `true` si encontró alguna.
fn collect_pages(
    buf: &[u8],
    lo: f64,
    hi: f64,
    pages: &mut BTreeMap<u32, Page>,
    found_at: &mut Vec<u64>,
    base: u64,
) -> bool {
    let mut any = false;
    let mut i = 0usize;
    while i + 27 < buf.len() {
        // Busca el siguiente 'O' de "OggS" a mano: es lo más rápido.
        let Some(offset) = buf[i..].iter().position(|byte| *byte == b'O') else {
            break;
        };
        i += offset;
        if i + 27 >= buf.len() {
            break;
        }
        if &buf[i..i + 4] != b"OggS" || buf[i + 4] != 0 {
            i += 1;
            continue;
        }
        let header_type = buf[i + 5];
        if !matches!(header_type, 0 | 2 | 4 | 6) {
            i += 1;
            continue;
        }
        let segments = buf[i + 26] as usize;
        if segments == 0 || i + 27 + segments > buf.len() {
            i += 1;
            continue;
        }
        let size = 27 + segments + buf[i + 27..i + 27 + segments].iter().map(|v| *v as usize).sum::<usize>();
        if size > 70_000 || i + size > buf.len() {
            i += 1;
            continue;
        }
        // Si quedan datos, la página siguiente debe empezar justo aquí: es el
        // filtro que descarta falsos positivos de "OggS" en memoria.
        if i + size + 4 <= buf.len() && &buf[i + size..i + size + 4] != b"OggS" {
            i += 1;
            continue;
        }
        let mut page = buf[i..i + size].to_vec();
        let stored = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
        page[22] = 0;
        page[23] = 0;
        page[24] = 0;
        page[25] = 0;
        if ogg_crc(&page) != stored {
            i += 1;
            continue;
        }
        // Devolver el CRC a su sitio: se guarda la página tal cual, no la copia
        // con ceros que se usa para el cálculo.
        page[22..26].copy_from_slice(&stored.to_le_bytes());
        let seq = u32::from_le_bytes([page[18], page[19], page[20], page[21]]);
        let gran = u64::from_le_bytes([
            page[6], page[7], page[8], page[9], page[10], page[11], page[12], page[13],
        ]) as f64
            / SAMPLE_RATE;
        if gran + 0.5 >= lo && gran - 0.5 <= hi && !pages.contains_key(&seq) {
            pages.insert(
                seq,
                Page {
                    data: page,
                    gran,
                    header_type,
                },
            );
            found_at.push(base + i as u64);
            any = true;
        }
        i += size;
    }
    any
}

fn read_regions(pid: u32) -> Vec<(u64, u64)> {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/maps")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(range) = parts.next() else { continue };
        let Some(perms) = parts.next() else { continue };
        if !perms.starts_with('r') {
            continue;
        }
        let name = parts.nth(3).unwrap_or("");
        // Sólo memoria anónima: es donde aparecen los buffers descifrados.
        let anonymous = name.is_empty()
            || name.starts_with("[heap]")
            || name.starts_with("[stack]")
            || name.starts_with("[anon");
        if !anonymous {
            continue;
        }
        let Some((start, end)) = range.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16))
        else {
            continue;
        };
        if end > start && end - start < (1 << 30) {
            out.push((start, end));
        }
    }
    out
}

fn scan(
    mem: &File,
    regions: &[(u64, u64)],
    lo: f64,
    hi: f64,
    pages: &mut BTreeMap<u32, Page>,
) -> (usize, Vec<(u64, u64)>) {
    let mut found_at: Vec<u64> = Vec::new();
    let mut buffer = vec![0u8; CHUNK + OVERLAP];
    for (start, end) in regions {
        let mut offset = 0u64;
        while offset < end - start {
            let remaining = end - start - offset;
            let take = remaining.min(CHUNK as u64) as usize;
            // Leer con solape para no perder una página partida por el corte.
            let want = (take + OVERLAP).min(remaining as usize).min(buffer.len());
            let Ok(read) = mem.read_at(&mut buffer[..want], *start + offset) else {
                break;
            };
            if read == 0 {
                break;
            }
            collect_pages(&buffer[..read], lo, hi, pages, &mut found_at, *start + offset);
            if read < take {
                break;
            }
            offset += take as u64;
        }
    }
    let hot = found_at
        .iter()
        .filter_map(|addr| {
            regions
                .iter()
                .copied()
                .find(|(start, end)| addr >= start && addr < end)
        })
        .collect::<Vec<_>>();
    let mut unique = hot.clone();
    unique.sort_unstable();
    unique.dedup();
    (found_at.len(), unique)
}

fn coverage(pages: &BTreeMap<u32, Page>) -> (usize, u32, u32, f64) {
    if pages.is_empty() {
        return (0, 0, 0, 0.0);
    }
    let first = *pages.keys().next().unwrap();
    let last = *pages.keys().next_back().unwrap();
    let total = (last - first + 1) as usize;
    let holes = total - pages.len();
    (pages.len(), first, last, 100.0 * (1.0 - holes as f64 / total as f64))
}

/// Extrae la pista del cliente saltando por ella. `on_progress` se llama tras
/// cada salto (no imprime nada: el core no sabe de terminales).
pub fn harvest<F>(options: &HarvestOptions, mut on_progress: F) -> Result<HarvestReport>
where
    F: FnMut(&Progress),
{
    if let Some(target) = &options.target {
        mpris::open_uri(target).context("no pude abrir la pista en el cliente")?;
        std::thread::sleep(Duration::from_millis(1_500));
    }

    let mut watcher = SpotifyWatcher::new();
    let state = watcher
        .poll()
        .ok_or_else(|| anyhow!("no veo el cliente de Spotify por MPRIS"))?;
    let track = state
        .track
        .ok_or_else(|| anyhow!("Spotify no está reproduciendo nada"))?;
    let spotify_id = track
        .key
        .strip_prefix("spotify:track:")
        .map(str::to_string)
        .ok_or_else(|| anyhow!("la pista no tiene id de Spotify; no puedo posicionarme"))?;
    if track.duration_ms == 0 {
        return Err(anyhow!("la pista no reporta duración"));
    }

    let pid = mpris::client_pid()?;
    let mem = File::open(format!("/proc/{pid}/mem"))
        .with_context(|| format!("no pude abrir /proc/{pid}/mem"))?;

    let started = Instant::now();
    let mut pages: BTreeMap<u32, Page> = BTreeMap::new();
    let mut all_regions = read_regions(pid);
    if all_regions.is_empty() {
        return Err(anyhow!("no encuentro memoria anónima en el proceso {pid}"));
    }
    let mut hot: Vec<(u64, u64)> = Vec::new();

    // Pasada principal: de principio a fin en pasos de `step_ms`.
    let mut position = 0u64;
    let mut cycles = 0u32;
    while position + 800 < track.duration_ms && started.elapsed() < options.budget {
        let _ = mpris::set_position(&spotify_id, position);
        std::thread::sleep(SEEK_SETTLE);
        let lo = position as f64 / 1000.0 - 0.5;
        let hi = (position + options.step_ms) as f64 / 1000.0 + 0.5;
        let regions = if hot.is_empty() { &all_regions } else { &hot };
        let (found, regions_hit) = scan(&mem, regions, lo, hi, &mut pages);
        if found > 0 && !regions_hit.is_empty() {
            hot = regions_hit;
        }
        position += options.step_ms;
        cycles += 1;
        // Cada 40 saltos, refrescar el mapa: las asignaciones se mueven.
        if cycles % 40 == 0 {
            all_regions = read_regions(pid);
            hot.clear();
        }
        let (count, _, _, cover) = coverage(&pages);
        let elapsed = started.elapsed().as_secs_f64();
        on_progress(&Progress {
            position_ms: position,
            total_ms: track.duration_ms,
            pages: count,
            coverage: cover,
            speed: if elapsed > 0.0 {
                (position as f64 / 1000.0) / elapsed
            } else {
                0.0
            },
        });
    }

    // El barrido puede haber dejado el cliente en la pista siguiente (o parado):
    // para rellenar huecos hay que estar otra vez en la pista buena.
    let same_track = watcher
        .poll()
        .and_then(|state| state.track)
        .map(|current| current.key == track.key)
        .unwrap_or(false);
    if !same_track {
        let _ = mpris::open_uri(&format!("spotify:track:{spotify_id}"));
        std::thread::sleep(Duration::from_millis(1_500));
    }

    // Relleno de huecos: interpolar la posición de cada seq que falta y saltar ahí.
    for _pass in 0..4 {
        let (_, first, last, cover) = coverage(&pages);
        if cover >= 99.5 {
            break;
        }
        let missing: Vec<u32> = (first..=last).filter(|seq| !pages.contains_key(seq)).collect();
        if missing.is_empty() {
            break;
        }
        for seq in missing.iter().take(120) {
            if started.elapsed() > options.budget {
                break;
            }
            // Vecinos conocidos para estimar la posición de esta página.
            let before = pages.range(..*seq).next_back().map(|(_, p)| p.gran);
            let after = pages.range(*seq..).next().map(|(_, p)| p.gran);
            let target = match (before, after) {
                (Some(low), Some(high)) => (low + high) / 2.0,
                (Some(low), None) => low + 1.0,
                (None, Some(high)) => (high - 1.0).max(0.0),
                (None, None) => continue,
            };
            // Tres intentos alrededor de la posición estimada: la ventana que
            // descifra el cliente es de ~2 s y no siempre cae centrada.
            for delta in [0.0f64, -0.4, 0.4] {
                let at = (target + delta).max(0.0);
                let _ = mpris::set_position(&spotify_id, (at * 1000.0) as u64);
                std::thread::sleep(SEEK_SETTLE);
                let regions = if hot.is_empty() { &all_regions } else { &hot };
                let (_, hit) = scan(&mem, regions, at - 1.5, at + 1.5, &mut pages);
                if !hit.is_empty() {
                    hot = hit;
                }
            }
        }
        let (count, _, _, cover) = coverage(&pages);
        on_progress(&Progress {
            position_ms: track.duration_ms,
            total_ms: track.duration_ms,
            pages: count,
            coverage: cover,
            speed: (track.duration_ms as f64 / 1000.0) / started.elapsed().as_secs_f64(),
        });
    }

    // Montar el fichero: páginas en orden de secuencia.
    let (_, first, last, cover) = coverage(&pages);
    if pages.is_empty() {
        return Err(anyhow!("no recuperé ninguna página de audio"));
    }
    let first_page = pages
        .get(&first)
        .ok_or_else(|| anyhow!("falta la primera página"))?;
    if first_page.header_type & 0x02 == 0 {
        return Err(anyhow!("la primera página no es de inicio de flujo (BOS)"));
    }
    if cover < 80.0 {
        return Err(anyhow!(
            "cobertura insuficiente ({cover:.1} %): no monto un fichero roto"
        ));
    }

    let mut audio = Vec::with_capacity(pages.values().map(|page| page.data.len()).sum());
    for seq in first..=last {
        if let Some(page) = pages.get(&seq) {
            audio.extend_from_slice(&page.data);
        }
    }

    std::fs::create_dir_all(&options.out_dir).ok();
    let slug = crate::util::slug(&format!("{} {}", track.artist, track.title), 70);
    let stem = if slug.is_empty() {
        "harvest".to_string()
    } else {
        slug
    };
    let path = options.out_dir.join(format!("{stem}.ogg"));
    std::fs::write(&path, &audio).context("no pude escribir el Ogg")?;

    let seconds = started.elapsed().as_secs_f64();
    let speed = (track.duration_ms as f64 / 1000.0) / seconds.max(0.001);
    Ok(HarvestReport {
        path,
        track,
        pages: pages.len(),
        coverage: cover,
        seconds,
        speed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_de_ogg_es_estable() {
        // La tabla bien construida da este valor para "123456789".
        assert_eq!(ogg_crc(b"123456789"), 0x89A1_897F);
    }

    #[test]
    fn cobertura_cuenta_huecos() {
        let mut pages = BTreeMap::new();
        for seq in [0u32, 1, 2, 4] {
            pages.insert(
                seq,
                Page {
                    data: vec![0u8; 4],
                    gran: seq as f64,
                    header_type: 0,
                },
            );
        }
        let (count, first, last, cover) = coverage(&pages);
        assert_eq!((count, first, last), (4, 0, 4));
        assert!((cover - 80.0).abs() < 0.01);
    }
}
