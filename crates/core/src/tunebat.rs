//! Datos de Tunebat para la pista que suena: BPM, tonalidad, Camelot y los
//! índices de energy/danceability/happiness.
//!
//! La web usa una API interna (`api.tunebat.com/api/tracks/search`) que
//! responde JSON sin autenticación. Solo se envía el título y el artista; la
//! respuesta se cachea en memoria desde el TUI. Es una API no documentada:
//! cualquier fallo se trata como «sin datos», nunca como un error de spote.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::util::normalize;

const SEARCH_URL: &str = "https://api.tunebat.com/api/tracks/search";
const TRACK_URL: &str = "https://api.tunebat.com/api/tracks";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:153.0) Gecko/20100101 Firefox/153.0";
const TIMEOUT: Duration = Duration::from_secs(12);

/// Datos musicales que Tunebat conoce de una pista.
#[derive(Debug, Clone, PartialEq)]
pub struct TunebatInfo {
    /// Identidad en Spotify; sirve para depurar y cachear.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub bpm: f32,
    /// `D major`, `A♭ minor`…
    pub key: String,
    /// Notación Camelot (`10B`, `9A`…).
    pub camelot: String,
    /// 0.0..=1.0.
    pub energy: f32,
    pub danceability: f32,
    pub happiness: f32,
    pub acousticness: f32,
    pub instrumentalness: f32,
    pub liveness: f32,
    pub speechiness: f32,
    /// Loudness integrada, en dB (negativa).
    pub loudness: f32,
    /// 0..=100.
    pub popularity: u32,
    pub duration_ms: u64,
    /// Fecha de lanzamiento ISO (`2001-03-12`) si Tunebat la conoce.
    pub release_date: String,
    /// Sello discográfico, si la API lo expone.
    pub label: String,
    /// Contenido explícito según Spotify.
    pub explicit: bool,
}

impl TunebatInfo {
    /// `123 BPM · 10B · D major` con guiones donde falte el dato.
    pub fn bpm_line(&self) -> String {
        let bpm = if self.bpm > 0.0 {
            format!("{:.0} BPM", self.bpm)
        } else {
            "— BPM".to_string()
        };
        let camelot = if self.camelot.is_empty() {
            "—"
        } else {
            self.camelot.as_str()
        };
        let key = if self.key.is_empty() {
            "—"
        } else {
            self.key.as_str()
        };
        format!("{bpm} · {camelot} · {key}")
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Option<SearchData>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: String,
    /// Nombre de la pista.
    #[serde(default, rename = "n")]
    title: String,
    /// Artistas.
    #[serde(default, rename = "as")]
    artists: Vec<String>,
    /// Álbum.
    #[serde(default, rename = "an")]
    album: Option<String>,
    /// Duración en ms.
    #[serde(default, rename = "d")]
    duration_ms: Option<u64>,
    /// BPM.
    #[serde(default, rename = "b")]
    bpm: Option<f64>,
    /// Tonalidad (`D major`).
    #[serde(default, rename = "k")]
    key: Option<String>,
    /// Camelot.
    #[serde(default, rename = "c")]
    camelot: Option<String>,
    #[serde(default, rename = "e")]
    energy: Option<f64>,
    #[serde(default, rename = "da")]
    danceability: Option<f64>,
    #[serde(default, rename = "h")]
    happiness: Option<f64>,
    #[serde(default, rename = "ac")]
    acousticness: Option<f64>,
    #[serde(default, rename = "i")]
    instrumentalness: Option<f64>,
    #[serde(default, rename = "li")]
    liveness: Option<f64>,
    #[serde(default, rename = "s")]
    speechiness: Option<f64>,
    #[serde(default, rename = "lo")]
    loudness: Option<f64>,
    #[serde(default, rename = "rd")]
    release_date: Option<String>,
    #[serde(default, rename = "p")]
    popularity: Option<u32>,
    #[serde(default, rename = "l")]
    label: Option<String>,
    #[serde(default, rename = "ie")]
    explicit: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TrackResponse {
    data: Option<Item>,
}

/// `spotify:track:ID` → `ID`. Otras claves (`sha1:`, `isrc:`) no permiten
/// consultar el endpoint exacto y caen a la búsqueda por texto.
pub fn spotify_id_from_key(key: &str) -> Option<&str> {
    key.strip_prefix("spotify:track:")
        .map(str::trim)
        .filter(|id| id.len() == 22 && id.chars().all(|ch| ch.is_ascii_alphanumeric()))
}

/// Busca una pista por título/artista y devuelve sus datos de Tunebat.
///
/// Si la pista tiene un ID de Spotify (`spotify:track:…`), se pide primero
/// la ficha exacta; así no se confunde con otra versión. El resto cae a la
/// búsqueda por título limpio + artista.
pub fn lookup(
    title: &str,
    artist: &str,
    album: Option<&str>,
    duration_ms: u64,
    spotify_id: Option<&str>,
) -> Result<Option<TunebatInfo>> {
    let client = client()?;
    if let Some(id) = spotify_id.filter(|id| {
        id.len() == 22 && id.chars().all(|ch| ch.is_ascii_alphanumeric())
    }) {
        if let Some(info) = lookup_by_spotify_id(&client, id)? {
            return Ok(Some(info));
        }
    }
    let title = title.trim();
    if title.is_empty() {
        return Ok(None);
    }
    let original_title = title;
    let artist = artist.trim();
    let title = search_title(title);
    let term = if artist.is_empty() {
        title.clone()
    } else {
        format!("{title} {artist}")
    };
    let items = search(&client, &term)?;
    if let Some(info) = best_match(&items, original_title, artist, album, duration_ms) {
        return Ok(Some(info));
    }
    // Colaboraciones raras: el término con todos los artistas puede no
    // devolver nada; se reintenta solo con el título y el matching filtra.
    if !artist.is_empty() {
        let items = search(&client, &title)?;
        if let Some(info) = best_match(&items, original_title, artist, album, duration_ms) {
            return Ok(Some(info));
        }
    }
    Ok(None)
}

fn client() -> Result<reqwest::blocking::Client> {
    crate::crypto::install();
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(UA)
        .build()
        .context("cliente de tunebat")
}

fn search(client: &reqwest::blocking::Client, term: &str) -> Result<Vec<Item>> {
    let mut url = reqwest::Url::parse(SEARCH_URL).context("url de tunebat")?;
    url.query_pairs_mut()
        .append_pair("term", term)
        .append_pair("page", "1");
    let response = get(client, url)?;
    let Some(response) = response else {
        return Ok(Vec::new());
    };
    let body: SearchResponse = response.json().context("json de tunebat")?;
    Ok(body.data.map(|data| data.items).unwrap_or_default())
}

fn lookup_by_spotify_id(
    client: &reqwest::blocking::Client,
    id: &str,
) -> Result<Option<TunebatInfo>> {
    let mut url = reqwest::Url::parse(TRACK_URL).context("url de tunebat")?;
    url.query_pairs_mut().append_pair("trackId", id);
    let response = get(client, url)?;
    let Some(response) = response else {
        return Ok(None);
    };
    let body: TrackResponse = response.json().context("json de tunebat")?;
    Ok(body.data.map(|item| info_from(&item)))
}

fn get(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
) -> Result<Option<reqwest::blocking::Response>> {
    let response = client
        .get(url)
        // La API interna exige el origen de la web; sin estas cabeceras
        // Cloudflare devuelve el challenge en vez del JSON.
        .header("Origin", "https://tunebat.com")
        .header("Referer", "https://tunebat.com/")
        .send()
        .context("tunebat")?;
    if response.status().is_success() {
        Ok(Some(response))
    } else {
        Ok(None)
    }
}

/// Título para la búsqueda: quita la versión entre paréntesis y el sufijo
/// « - Radio Edit» que añaden Spotify/Deezer. El matching posterior sigue
/// usando el título original; esto solo mejora la consulta a Tunebat.
fn search_title(title: &str) -> String {
    let title = title.trim();
    let base = title.split(['(', '[']).next().unwrap_or(title).trim();
    let base = base.split(" - ").next().unwrap_or(base).trim();
    if base.chars().filter(|ch| ch.is_alphanumeric()).count() >= 2 {
        base.to_string()
    } else {
        // Títulos simbólicos como «(_8(|)»: el paréntesis es parte del nombre.
        title.to_string()
    }
}

fn best_match(
    items: &[Item],
    title: &str,
    artist: &str,
    album: Option<&str>,
    duration_ms: u64,
) -> Option<TunebatInfo> {
    let wanted_title = normalize(title);
    if wanted_title.is_empty() {
        return None;
    }
    let wanted_artist = normalize(artist);
    let wanted_album = album.map(normalize).filter(|album| !album.is_empty());
    let mut best: Option<(i32, &Item)> = None;
    for item in items {
        let got_title = normalize(&item.title);
        if got_title.is_empty() {
            continue;
        }
        let title_score = if got_title == wanted_title {
            60
        } else if got_title.contains(&wanted_title) || wanted_title.contains(&got_title) {
            25
        } else {
            continue;
        };
        let got_artist = normalize(&item.artists.join(" "));
        if !artist_matches(&wanted_artist, &got_artist) {
            continue;
        }
        let mut score = title_score + 35;
        if let Some(wanted_album) = &wanted_album {
            if item
                .album
                .as_deref()
                .map(normalize)
                .is_some_and(|album| album == *wanted_album)
            {
                score += 10;
            }
        }
        if let Some(candidate_ms) = item.duration_ms {
            if duration_ms > 0 {
                let diff = candidate_ms.abs_diff(duration_ms);
                if diff <= 3_000 {
                    score += 15;
                } else if diff <= 10_000 {
                    score += 5;
                }
            }
        }
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, item));
        }
    }
    best.map(|(_, item)| info_from(item))
}

fn info_from(item: &Item) -> TunebatInfo {
    let clamp = |value: Option<f64>| -> f32 {
        value
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0) as f32)
            .unwrap_or(0.0)
    };
    TunebatInfo {
        id: item.id.clone(),
        title: item.title.clone(),
        artist: item.artists.join(", "),
        album: item.album.clone().unwrap_or_default(),
        bpm: item
            .bpm
            .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
            .map(|bpm| bpm as f32)
            .unwrap_or(0.0),
        key: item.key.clone().unwrap_or_default(),
        camelot: item.camelot.clone().unwrap_or_default(),
        energy: clamp(item.energy),
        danceability: clamp(item.danceability),
        happiness: clamp(item.happiness),
        acousticness: clamp(item.acousticness),
        instrumentalness: clamp(item.instrumentalness),
        liveness: clamp(item.liveness),
        speechiness: clamp(item.speechiness),
        loudness: item
            .loudness
            .filter(|value| value.is_finite())
            .map(|value| value as f32)
            .unwrap_or(0.0),
        popularity: item.popularity.unwrap_or(0).min(100),
        duration_ms: item.duration_ms.unwrap_or(0),
        release_date: item.release_date.clone().unwrap_or_default(),
        label: item.label.clone().unwrap_or_default(),
        explicit: item.explicit.unwrap_or(false),
    }
}

/// Compara artistas por nombre completo, contención o partes de una
/// colaboración separada por comas.
fn artist_matches(wanted: &str, got: &str) -> bool {
    if wanted.is_empty() {
        return true;
    }
    if got.is_empty() {
        return false;
    }
    if wanted == got {
        return true;
    }
    if got.contains(wanted) || wanted.contains(got) {
        return true;
    }
    let got_tokens: Vec<&str> = got.split_whitespace().collect();
    let wanted_tokens: Vec<&str> = wanted.split_whitespace().collect();
    if !wanted_tokens.is_empty() && wanted_tokens.iter().all(|token| got_tokens.contains(token)) {
        return true;
    }
    // "Biberon, Flako NKO, yoyo.stay" debe casar con un item de solo Biberon.
    wanted
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| {
            if part == got {
                return true;
            }
            let part_tokens: Vec<&str> = part.split_whitespace().collect();
            !part_tokens.is_empty()
                && part_tokens.len() == got_tokens.len()
                && part_tokens.iter().all(|token| got_tokens.contains(token))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        title: &str,
        artist: &str,
        album: &str,
        duration_ms: u64,
        bpm: f64,
        key: &str,
        camelot: &str,
    ) -> Item {
        Item {
            id: format!("id:{title}"),
            title: title.into(),
            artists: vec![artist.into()],
            album: Some(album.into()),
            duration_ms: Some(duration_ms),
            bpm: Some(bpm),
            key: Some(key.into()),
            camelot: Some(camelot.into()),
            energy: Some(0.7),
            danceability: Some(0.6),
            happiness: Some(0.5),
            acousticness: Some(0.02),
            instrumentalness: Some(0.0),
            liveness: Some(0.33),
            speechiness: Some(0.13),
            loudness: Some(-8.6),
            release_date: Some("2001-03-12".into()),
            popularity: Some(80),
            label: None,
            explicit: Some(false),
        }
    }

    #[test]
    fn picks_the_version_that_matches_album_and_duration() {
        let items = vec![
            item(
                "One More Time",
                "Daft Punk",
                "One More Time",
                320_293,
                123.0,
                "D major",
                "10B",
            ),
            item(
                "One More Time",
                "Daft Punk",
                "Discovery",
                320_357,
                123.0,
                "D major",
                "10B",
            ),
            item(
                "One More Time - Radio Edit",
                "Daft Punk",
                "Musique, Vol. 1",
                235_466,
                123.0,
                "D major",
                "10B",
            ),
        ];
        let info = best_match(
            &items,
            "One More Time",
            "Daft Punk",
            Some("Discovery"),
            320_000,
        )
        .expect("debe encontrar la versión del álbum");
        assert_eq!(info.album, "Discovery");
        assert_eq!(info.bpm, 123.0);
        assert_eq!(info.camelot, "10B");
    }

    #[test]
    fn spotify_ids_only_come_from_exact_keys() {
        assert_eq!(
            spotify_id_from_key("spotify:track:0DiWol3AO6WpXZgp0goxAV"),
            Some("0DiWol3AO6WpXZgp0goxAV")
        );
        assert!(spotify_id_from_key("sha1:0DiWol3AO6WpXZgp0goxAV").is_none());
        assert!(spotify_id_from_key("spotify:track:corto").is_none());
    }

    #[test]
    fn search_title_drops_version_suffixes_but_keeps_symbolic_titles() {
        assert_eq!(search_title("One More Time (Romanthony's Unplugged)"), "One More Time");
        assert_eq!(search_title("Let It Happen - Radio Edit"), "Let It Happen");
        assert_eq!(search_title("Song (feat. Other)"), "Song");
        assert_eq!(search_title("(_8(|)"), "(_8(|)");
    }

    #[test]
    fn refuses_another_artist_with_the_same_title() {
        let items = vec![item(
            "One More Time",
            "Another Band",
            "Album",
            200_000,
            100.0,
            "C major",
            "8B",
        )];
        assert!(best_match(&items, "One More Time", "Daft Punk", None, 0).is_none());
    }

    #[test]
    fn collaboration_artist_matches_its_first_name() {
        assert!(artist_matches("biberon flako nko yoyo stay", "biberon"));
        assert!(artist_matches("biberon, flako nko, yoyo stay", "biberon"));
        assert!(artist_matches("daft punk", "daft punk"));
        assert!(!artist_matches("daft punk", "the weeknd"));
    }

    #[test]
    #[ignore = "diagnóstico manual con red"]
    fn live_tunebat_by_spotify_id_is_exact() {
        let id = spotify_id_from_key("spotify:track:0DiWol3AO6WpXZgp0goxAV")
            .expect("ID de Spotify válido");
        let info = lookup("", "", None, 0, Some(id))
            .expect("sin error de red")
            .expect("ficha exacta de One More Time");
        eprintln!("spotify id: {} · {}", info.bpm_line(), info.popularity);
        assert_eq!(info.bpm, 123.0);
        assert_eq!(info.camelot, "10B");
        assert_eq!(info.key, "D major");
    }

    #[test]
    #[ignore = "diagnóstico manual con red"]
    fn live_tunebat_one_more_time() {
        let info = lookup("One More Time", "Daft Punk", Some("Discovery"), 320_000, None)
            .expect("sin error de red")
            .expect("tunebat debe conocer la pista");
        eprintln!(
            "tunebat: {} · {} · {}",
            info.bpm_line(),
            info.album,
            info.popularity
        );
        assert_eq!(info.bpm, 123.0);
        assert_eq!(info.key, "D major");
        assert_eq!(info.camelot, "10B");
    }
}
