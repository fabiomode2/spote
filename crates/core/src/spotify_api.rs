//! Búsqueda de metadatos en Spotify sin cuenta de usuario.
//!
//! Se usa el token anónimo que sirve el propio reproductor web. La lista de
//! resultados sale de Spotify (título, artistas, álbum, duración y portada) y
//! la descarga se resuelve después con yt-dlp usando esos metadatos, que es
//! mucho más fiable que buscar el texto libre directamente en YouTube.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const TOKEN_URL: &str =
    "https://open.spotify.com/get_access_token?reason=transport&productType=web_player";
const SEARCH_URL: &str = "https://api.spotify.com/v1/search";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                  Chrome/126.0 Safari/537.36";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotifyTrack {
    /// `spotify` o `deezer` (catálogo de respaldo cuando Spotify no da token).
    pub provider: String,
    pub id: String,
    pub title: String,
    /// Artistas separados por coma, en orden.
    pub artists: String,
    pub album: String,
    pub duration_ms: u64,
    pub art_url: Option<String>,
    pub isrc: Option<String>,
}

impl SpotifyTrack {
    /// Query para yt-dlp: artista + título, sin adornos.
    pub fn download_query(&self) -> String {
        if self.artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artists, self.title)
        }
    }

    pub fn source(&self) -> &str {
        &self.provider
    }

    pub fn label(&self) -> String {
        format!("{}  ·  {}", self.title, self.artists)
    }
}

static TOKEN: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();

fn token_slot() -> &'static Mutex<Option<(String, Instant)>> {
    TOKEN.get_or_init(|| Mutex::new(None))
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(8))
        .build()
        .context("cliente http")
}

/// Token anónimo, cacheado hasta poco antes de caducar.
fn access_token(force: bool) -> Result<String> {
    {
        let slot = token_slot().lock().unwrap_or_else(|err| err.into_inner());
        if !force {
            if let Some((token, at)) = slot.as_ref() {
                if at.elapsed() < Duration::from_secs(30 * 60) {
                    return Ok(token.clone());
                }
            }
        }
    }
    let response = client()?
        .get(TOKEN_URL)
        .header("Accept", "application/json")
        .send()
        .context("token de spotify")?;
    if !response.status().is_success() {
        return Err(anyhow!("spotify token: http {}", response.status()));
    }
    let body: Value = response.json().context("json del token")?;
    let token = body
        .get("accessToken")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("spotify token vacío"))?
        .to_string();
    let mut slot = token_slot().lock().unwrap_or_else(|err| err.into_inner());
    *slot = Some((token.clone(), Instant::now()));
    Ok(token)
}

/// Busca pistas. Sin red o con Spotify caído devuelve error (el TUI cae a
/// yt-dlp como plan B).
pub fn search(query: &str, limit: usize) -> Result<Vec<SpotifyTrack>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    match search_spotify(query, limit) {
        Ok(tracks) if !tracks.is_empty() => Ok(tracks),
        other => {
            // Spotify anónimo está bloqueado en algunas redes (403 de Fastly).
            // Deezer sirve el mismo tipo de metadatos (título limpio, artistas,
            // álbum, duración, ISRC y portada) sin cuenta.
            match search_deezer(query, limit) {
                Ok(tracks) if !tracks.is_empty() => Ok(tracks),
                _ => other,
            }
        }
    }
}

/// Token de aplicación (client credentials). Solo si hay credenciales en el
/// entorno: es la vía fiable cuando el token anónimo está bloqueado.
fn client_credentials_token(client: &reqwest::blocking::Client) -> Result<String> {
    let id = std::env::var("SPOTIFY_CLIENT_ID").map_err(|_| anyhow!("sin client id"))?;
    let secret = std::env::var("SPOTIFY_CLIENT_SECRET").map_err(|_| anyhow!("sin client secret"))?;
    let response = client
        .post("https://accounts.spotify.com/api/token")
        .basic_auth(id, Some(secret))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .context("token client credentials")?;
    if !response.status().is_success() {
        return Err(anyhow!("spotify auth: http {}", response.status()));
    }
    let body: Value = response.json().context("json del token")?;
    body.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("token vacío"))
}

fn search_spotify(query: &str, limit: usize) -> Result<Vec<SpotifyTrack>> {
    let client = client()?;
    let has_creds = std::env::var("SPOTIFY_CLIENT_ID").is_ok()
        && std::env::var("SPOTIFY_CLIENT_SECRET").is_ok();
    let mut last_error = None;
    for attempt in 0..2 {
        let token = if has_creds {
            client_credentials_token(&client)?
        } else {
            access_token(attempt > 0)?
        };
        let mut url = reqwest::Url::parse(SEARCH_URL).context("url de búsqueda")?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("type", "track")
            .append_pair("limit", &limit.clamp(1, 20).to_string())
            .append_pair("market", "ES");
        let response = client
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .send();
        match response {
            Ok(response) if response.status().as_u16() == 401 && attempt == 0 => {
                last_error = Some(anyhow!("token caducado"));
                continue;
            }
            Ok(response) if response.status().is_success() => {
                let body: Value = response.json().context("json de búsqueda")?;
                return Ok(parse_search(&body));
            }
            Ok(response) => {
                last_error = Some(anyhow!("spotify search: http {}", response.status()));
            }
            Err(err) => last_error = Some(err.into()),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("spotify search")))
}

fn parse_search(body: &Value) -> Vec<SpotifyTrack> {
    let Some(items) = body
        .pointer("/tracks/items")
        .and_then(Value::as_array)
        .or_else(|| body.get("tracks").and_then(Value::as_array))
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let title = item.get("name").and_then(Value::as_str)?.to_string();
            let artists = item
                .get("artists")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|artist| artist.get("name").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let album = item
                .pointer("/album/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let art_url = item
                .pointer("/album/images")
                .and_then(Value::as_array)
                .and_then(|images| {
                    images
                        .iter()
                        .filter_map(|image| {
                            let url = image.get("url")?.as_str()?;
                            let width = image.get("width").and_then(Value::as_u64).unwrap_or(0);
                            Some((width, url.to_string()))
                        })
                        .max_by_key(|(width, _)| *width)
                        .map(|(_, url)| url)
                });
            let duration_ms = item
                .get("duration_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let isrc = item
                .pointer("/external_ids/isrc")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(SpotifyTrack {
                provider: "spotify".into(),
                id,
                title,
                artists,
                album,
                duration_ms,
                art_url,
                isrc,
            })
        })
        .collect()
}

fn search_deezer(query: &str, limit: usize) -> Result<Vec<SpotifyTrack>> {
    let client = client()?;
    let mut url = reqwest::Url::parse("https://api.deezer.com/search").context("url deezer")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("limit", &limit.clamp(1, 25).to_string());
    let body: Value = client
        .get(url)
        .send()
        .context("deezer search")?
        .json()
        .context("json de deezer")?;
    Ok(parse_deezer(&body))
}

fn parse_deezer(body: &Value) -> Vec<SpotifyTrack> {
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_u64)
                .map(|id| format!("dz:{id}"))?;
            let title = item.get("title").and_then(Value::as_str)?.to_string();
            let artists = item
                .pointer("/artist/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let album = item
                .pointer("/album/title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let duration_ms = item
                .get("duration")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                * 1000;
            let art_url = item
                .pointer("/album/cover_xl")
                .or_else(|| item.pointer("/album/cover_big"))
                .and_then(Value::as_str)
                .map(|url| url.replace("500x500", "1000x1000"));
            let isrc = item.get("isrc").and_then(Value::as_str).map(str::to_string);
            Some(SpotifyTrack {
                provider: "deezer".into(),
                id,
                title,
                artists,
                album,
                duration_ms,
                art_url,
                isrc,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_search_payload() {
        let body: Value = serde_json::from_str(
            r#"{"tracks":{"items":[
              {"id":"abc","name":"Borderline","duration_ms":237000,
               "artists":[{"name":"Tame Impala"}],
               "album":{"name":"The Slow Rush","images":[
                 {"url":"small.jpg","width":64},{"url":"big.jpg","width":640}]},
               "external_ids":{"isrc":"AUUM71900931"}}
            ]}}"#,
        )
        .unwrap();
        let tracks = parse_search(&body);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Borderline");
        assert_eq!(tracks[0].provider, "spotify");
        assert_eq!(tracks[0].artists, "Tame Impala");
        assert_eq!(tracks[0].album, "The Slow Rush");
        assert_eq!(tracks[0].duration_ms, 237_000);
        assert_eq!(tracks[0].art_url.as_deref(), Some("big.jpg"));
        assert_eq!(tracks[0].isrc.as_deref(), Some("AUUM71900931"));
        assert_eq!(tracks[0].download_query(), "Tame Impala - Borderline");
    }

    #[test]
    fn parses_deezer_results() {
        let body: Value = serde_json::from_str(
            r#"{"data":[{"id":872345282,"title":"Borderline","duration":238,
              "isrc":"AUUM71900956",
              "artist":{"name":"Tame Impala"},
              "album":{"title":"The Slow Rush",
                "cover_xl":"https://cdn/500x500.jpg"}}]}"#,
        )
        .unwrap();
        let tracks = parse_deezer(&body);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].provider, "deezer");
        assert_eq!(tracks[0].artists, "Tame Impala");
        assert_eq!(tracks[0].duration_ms, 238_000);
        assert_eq!(
            tracks[0].art_url.as_deref(),
            Some("https://cdn/1000x1000.jpg")
        );
    }

    #[test]
    fn download_query_without_artist_is_the_title() {
        let track = SpotifyTrack {
            provider: "spotify".into(),
            id: "x".into(),
            title: "Untitled".into(),
            artists: String::new(),
            album: String::new(),
            duration_ms: 0,
            art_url: None,
            isrc: None,
        };
        assert_eq!(track.download_query(), "Untitled");
    }
}
