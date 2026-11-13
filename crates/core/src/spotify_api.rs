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
const SPOTIFY_PLAYLIST_URL: &str = "https://api.spotify.com/v1/playlists";
const DEEZER_PLAYLIST_URL: &str = "https://api.deezer.com/playlist";
/// Tope de pistas al importar: una playlist de 10 000 temas no entra en el
/// importador (descargar cada pista es lo caro) y el TUI no las va a mostrar.
const MAX_PLAYLIST_TRACKS: usize = 300;
const ANON_BLOCKED: &str =
    "Spotify bloquea el token anónimo; define SPOTIFY_CLIENT_ID/SECRET o usa una URL de Deezer";
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

/// Playlist ya resuelta y lista para crear la lista local.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistInfo {
    /// `spotify` o `deezer`.
    pub provider: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub tracks: Vec<SpotifyTrack>,
}

/// De dónde sale la playlist. Sin URL no hay proveedor: se prueban los dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistProvider {
    Spotify,
    Deezer,
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

/// Artistas de un objeto de Spotify, unidos por coma en el orden original.
fn join_artists(node: &Value) -> String {
    node.get("artists")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|artist| artist.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Portada de mayor resolución de un objeto con `images[]` (álbum, lista...).
fn widest_image(node: &Value) -> Option<String> {
    node.get("images")
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
        })
}

/// Objeto de pista de Spotify → `SpotifyTrack`. Sin id ni nombre no vale.
fn spotify_track(node: &Value) -> Option<SpotifyTrack> {
    let id = node.get("id").and_then(Value::as_str)?.to_string();
    let title = node.get("name").and_then(Value::as_str)?.to_string();
    let album = node
        .pointer("/album/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let art_url = node.pointer("/album").and_then(widest_image);
    let duration_ms = node.get("duration_ms").and_then(Value::as_u64).unwrap_or(0);
    let isrc = node
        .pointer("/external_ids/isrc")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(SpotifyTrack {
        provider: "spotify".into(),
        id,
        title,
        artists: join_artists(node),
        album,
        duration_ms,
        art_url,
        isrc,
    })
}

fn parse_search(body: &Value) -> Vec<SpotifyTrack> {
    let Some(items) = body
        .pointer("/tracks/items")
        .and_then(Value::as_array)
        .or_else(|| body.get("tracks").and_then(Value::as_array))
    else {
        return Vec::new();
    };
    items.iter().filter_map(spotify_track).collect()
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

// ---------------------------------------------------------------------------
// Importación de playlists
// ---------------------------------------------------------------------------

/// Importa una playlist para convertirla en lista local.
///
/// Acepta una URL de Spotify (`…/playlist/<id>?si=…`), una de Deezer
/// (`www.deezer.com/playlist/<id>`), el URI `spotify:playlist:<id>` o el id
/// pelado. Con el id pelado se prueba Spotify y, si falla, Deezer. Bloquea:
/// llámalo desde un hilo.
pub fn playlist(input: &str) -> Result<PlaylistInfo, String> {
    let (provider, id) = parse_playlist_ref(input)?;
    match provider {
        Some(PlaylistProvider::Spotify) => spotify_playlist(&id),
        Some(PlaylistProvider::Deezer) => deezer_playlist(&id),
        None => match spotify_playlist(&id) {
            Ok(playlist) => Ok(playlist),
            // Id sin proveedor: Deezer es el plan B (y el error de Spotify
            // suele ser el informativo, p. ej. el 403 del token anónimo).
            Err(spotify_error) => deezer_playlist(&id)
                .map_err(|deezer_error| format!("{spotify_error}; {deezer_error}")),
        },
    }
}

/// Proveedor e id de la entrada. `None` = sin URL, hay que probar los dos.
fn parse_playlist_ref(input: &str) -> Result<(Option<PlaylistProvider>, String), String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("URL o id de playlist vacío".to_string());
    }

    // URI nativo: `spotify:playlist:<id>`.
    if let Some(rest) = raw.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        let kind = parts.next().unwrap_or_default();
        let id = parts.next().unwrap_or_default().trim();
        if kind != "playlist" || !is_spotify_id(id) {
            return Err(format!("no es un URI de playlist de Spotify: «{input}»"));
        }
        return Ok((Some(PlaylistProvider::Spotify), id.to_string()));
    }

    // Id pelado: sin host ni ruta.
    if !raw.contains('/') {
        if !is_playlist_token(raw) {
            return Err(format!("«{input}» no parece un id de playlist"));
        }
        return Ok((None, raw.to_string()));
    }

    let lower = raw.to_ascii_lowercase();
    let provider = if lower.contains("spotify.com") {
        PlaylistProvider::Spotify
    } else if lower.contains("deezer.com") {
        PlaylistProvider::Deezer
    } else {
        return Err("solo entiendo URLs de open.spotify.com o de deezer.com".to_string());
    };

    // Corta esquema y query string, y busca el segmento `playlist`: vale igual
    // `open.spotify.com/playlist/<id>` que `/intl-es/playlist/<id>`.
    let path = raw.split_once("://").map(|(_, rest)| rest).unwrap_or(raw);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let position = segments
        .iter()
        .position(|part| *part == "playlist")
        .ok_or_else(|| format!("no veo «playlist» en «{input}»"))?;
    let id = segments
        .get(position + 1)
        .map(|part| part.trim())
        .unwrap_or_default();
    let valid = match provider {
        PlaylistProvider::Spotify => is_spotify_id(id),
        PlaylistProvider::Deezer => !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
    };
    if !valid {
        return Err(format!("id de playlist inválido en «{input}»"));
    }
    Ok((Some(provider), id.to_string()))
}

fn is_spotify_id(value: &str) -> bool {
    (16..=32).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Id pelado: base62 de Spotify o numérico de Deezer.
fn is_playlist_token(value: &str) -> bool {
    is_spotify_id(value) || (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
}

/// `access_token` traduciendo el 403 del token anónimo al mensaje que el
/// usuario puede accionar. Ojo: en algunas redes el bloqueo llega aquí (Fastly
/// corta `open.spotify.com/get_access_token`), no a la respuesta de la API.
fn anon_token(force: bool) -> Result<String, String> {
    access_token(force).map_err(|err| {
        let message = err.to_string();
        if message.contains("403") {
            ANON_BLOCKED.to_string()
        } else {
            message
        }
    })
}

fn spotify_playlist(id: &str) -> Result<PlaylistInfo, String> {
    let client = client().map_err(|err| err.to_string())?;
    let has_creds = std::env::var("SPOTIFY_CLIENT_ID").is_ok()
        && std::env::var("SPOTIFY_CLIENT_SECRET").is_ok();
    let url = format!("{SPOTIFY_PLAYLIST_URL}/{id}");

    let mut token = String::new();
    let mut last_error = String::from("spotify playlist");
    let mut body: Option<Value> = None;
    for attempt in 0..2 {
        token = if has_creds {
            client_credentials_token(&client).map_err(|err| err.to_string())?
        } else {
            anon_token(attempt > 0)?
        };
        let response = client
            .get(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .send();
        match response {
            Ok(response) if response.status().as_u16() == 401 && attempt == 0 => {
                last_error = "spotify: token caducado".into();
            }
            Ok(response) if response.status().is_success() => {
                let page = response
                    .json()
                    .map_err(|err| format!("json de la playlist: {err}"))?;
                body = Some(page);
                break;
            }
            Ok(response) if response.status().as_u16() == 403 && !has_creds => {
                return Err(ANON_BLOCKED.to_string());
            }
            Ok(response) if response.status().as_u16() == 404 => {
                return Err(format!(
                    "Spotify no encuentra la playlist {id} (¿es privada?)"
                ));
            }
            Ok(response) => last_error = format!("spotify playlist: http {}", response.status()),
            Err(err) => last_error = err.to_string(),
        }
    }
    let body = body.ok_or(last_error)?;

    let mut playlist = parse_spotify_playlist(&body, id)?;
    let mut next = spotify_next_url(&body);
    while playlist.tracks.len() < MAX_PLAYLIST_TRACKS {
        let Some(url) = next.take() else { break };
        // Si falla una página extra se devuelve lo que ya hay: importar 200 de
        // 300 pistas es mejor que no importar nada.
        let Ok(response) = client.get(&url).bearer_auth(&token).send() else {
            break;
        };
        if !response.status().is_success() {
            break;
        }
        let Ok(page) = response.json::<Value>() else {
            break;
        };
        playlist.tracks.extend(parse_spotify_tracks(&page));
        next = spotify_next_url(&page);
    }
    playlist.tracks.truncate(MAX_PLAYLIST_TRACKS);
    Ok(playlist)
}

fn parse_spotify_playlist(body: &Value, id: &str) -> Result<PlaylistInfo, String> {
    let title = body
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or_else(|| "la playlist de Spotify no trae nombre".to_string())?;
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let cover_url = body
        .pointer("/images/0/url")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(PlaylistInfo {
        provider: "spotify".into(),
        id: id.to_string(),
        title: title.to_string(),
        description,
        cover_url,
        tracks: parse_spotify_tracks(body),
    })
}

/// Pistas de un listado de Spotify. Vale tanto la playlist completa
/// (`tracks.items`) como una página de `/playlists/{id}/tracks` (`items`).
fn parse_spotify_tracks(body: &Value) -> Vec<SpotifyTrack> {
    let Some(items) = body
        .pointer("/tracks/items")
        .and_then(Value::as_array)
        .or_else(|| body.get("items").and_then(Value::as_array))
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let track = item.get("track").unwrap_or(item);
            // `track: null` = pista borrada o no disponible en este mercado.
            if track.is_null() {
                return None;
            }
            spotify_track(track).filter(|track| !track.title.trim().is_empty())
        })
        .collect()
}

fn spotify_next_url(body: &Value) -> Option<String> {
    body.pointer("/tracks/next")
        .and_then(Value::as_str)
        .or_else(|| body.get("next").and_then(Value::as_str))
        .map(str::to_string)
}

fn deezer_playlist(id: &str) -> Result<PlaylistInfo, String> {
    let client = client().map_err(|err| err.to_string())?;
    let url = format!("{DEEZER_PLAYLIST_URL}/{id}");
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .map_err(|err| format!("deezer playlist: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("deezer playlist: http {}", response.status()));
    }
    let body: Value = response
        .json()
        .map_err(|err| format!("json de deezer: {err}"))?;
    // Deezer contesta 200 con `{"error":{...}}` cuando el id no existe.
    if let Some(message) = body.pointer("/error/message").and_then(Value::as_str) {
        return Err(format!("deezer: {message}"));
    }
    parse_deezer_playlist(&body, id)
}

fn parse_deezer_playlist(body: &Value, id: &str) -> Result<PlaylistInfo, String> {
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or_else(|| "la playlist de Deezer no trae título".to_string())?;
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let cover_url = body
        .get("picture_xl")
        .or_else(|| body.get("cover_xl"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string);
    let tracks = body
        .pointer("/tracks/data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| deezer_playlist_track(item, cover_url.as_deref()))
                .collect()
        })
        .unwrap_or_default();
    Ok(PlaylistInfo {
        provider: "deezer".into(),
        id: id.to_string(),
        title: title.to_string(),
        description,
        cover_url,
        tracks,
    })
}

/// Pista de `playlist.tracks.data[]`. Sigue el formato `dz:<id>` del catálogo.
fn deezer_playlist_track(item: &Value, playlist_cover: Option<&str>) -> Option<SpotifyTrack> {
    let id = item
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| format!("dz:{id}"))?;
    let title = item
        .get("title")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
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
    let duration_ms = item.get("duration").and_then(Value::as_u64).unwrap_or(0) * 1000;
    // Sin portada de álbum se usa la de la playlist: el importador siempre
    // pinta algo mientras descarga.
    let art_url = item
        .pointer("/album/cover_xl")
        .or_else(|| item.pointer("/album/cover_big"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .or_else(|| playlist_cover.map(str::to_string));
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
    fn detects_provider_and_id() {
        let spotify_id = "37i9dQZF1DXcBWIGoYBM5M";
        assert_eq!(
            parse_playlist_ref(&format!(
                "https://open.spotify.com/playlist/{spotify_id}?si=abc123"
            ))
            .unwrap(),
            (Some(PlaylistProvider::Spotify), spotify_id.to_string())
        );
        // Con prefijo de idioma y sin query string.
        assert_eq!(
            parse_playlist_ref(&format!("open.spotify.com/intl-es/playlist/{spotify_id}")).unwrap(),
            (Some(PlaylistProvider::Spotify), spotify_id.to_string())
        );
        // URI nativo.
        assert_eq!(
            parse_playlist_ref(&format!("spotify:playlist:{spotify_id}")).unwrap(),
            (Some(PlaylistProvider::Spotify), spotify_id.to_string())
        );
        // Deezer.
        assert_eq!(
            parse_playlist_ref("https://www.deezer.com/es/playlist/2881941882").unwrap(),
            (Some(PlaylistProvider::Deezer), "2881941882".to_string())
        );
        // Id pelado: sin proveedor, toca probar Spotify y luego Deezer.
        assert_eq!(
            parse_playlist_ref(&format!("  {spotify_id}  ")).unwrap(),
            (None, spotify_id.to_string())
        );
        assert_eq!(
            parse_playlist_ref("2881941882").unwrap(),
            (None, "2881941882".to_string())
        );
        // Basura: error claro y sin red.
        assert!(parse_playlist_ref("").is_err());
        assert!(parse_playlist_ref("https://youtube.com/playlist?list=PL1").is_err());
        assert!(parse_playlist_ref("https://open.spotify.com/playlist/corto").is_err());
        assert!(
            parse_playlist_ref("https://open.spotify.com/album/37i9dQZF1DXcBWIGoYBM5M").is_err()
        );
        assert!(parse_playlist_ref("nada-que-ver").is_err());
        assert!(playlist("https://youtube.com/playlist?list=PL1").is_err());
        assert!(playlist("").is_err());
    }

    #[test]
    fn parses_a_spotify_playlist() {
        let body: Value = serde_json::from_str(
            r#"{
              "name": "  Road trip  ",
              "description": "Para el coche",
              "images": [{"url": "https://i.scdn.co/300.jpg", "width": 300},
                         {"url": "https://i.scdn.co/640.jpg", "width": 640}],
              "tracks": {
                "next": "https://api.spotify.com/v1/playlists/37i9dQZF1DXcBWIGoYBM5M/tracks?offset=100&limit=100",
                "items": [
                  {"track": {"id": "4cOdK2wGLETKBW3PvgPWqT", "name": "Borderline",
                    "duration_ms": 237000,
                    "artists": [{"name": "Tame Impala"}, {"name": "Guest"}],
                    "album": {"name": "The Slow Rush", "images": [
                      {"url": "https://i.scdn.co/64.jpg", "width": 64},
                      {"url": "https://i.scdn.co/640b.jpg", "width": 640}]},
                    "external_ids": {"isrc": "AUUM71900931"}}},
                  {"track": null},
                  {"track": {"id": "7oK9VyNzrYvRFo7nQEYkWN", "name": "Alright",
                    "duration_ms": 177000,
                    "artists": [{"name": "Kendrick Lamar"}],
                    "album": {"name": "To Pimp a Butterfly"}}}
                ]
              }
            }"#,
        )
        .unwrap();
        let list = parse_spotify_playlist(&body, "37i9dQZF1DXcBWIGoYBM5M").unwrap();
        assert_eq!(list.provider, "spotify");
        assert_eq!(list.id, "37i9dQZF1DXcBWIGoYBM5M");
        assert_eq!(list.title, "Road trip");
        assert_eq!(list.description, "Para el coche");
        // Spec: primera imagen de la playlist.
        assert_eq!(list.cover_url.as_deref(), Some("https://i.scdn.co/300.jpg"));
        // La pista nula se cae.
        assert_eq!(list.tracks.len(), 2);
        let first = &list.tracks[0];
        assert_eq!(first.provider, "spotify");
        assert_eq!(first.id, "4cOdK2wGLETKBW3PvgPWqT");
        assert_eq!(first.title, "Borderline");
        assert_eq!(first.artists, "Tame Impala, Guest");
        assert_eq!(first.album, "The Slow Rush");
        assert_eq!(first.duration_ms, 237_000);
        assert_eq!(first.isrc.as_deref(), Some("AUUM71900931"));
        assert_eq!(first.art_url.as_deref(), Some("https://i.scdn.co/640b.jpg"));
        let second = &list.tracks[1];
        assert_eq!(second.artists, "Kendrick Lamar");
        assert_eq!(second.duration_ms, 177_000);
        assert_eq!(second.isrc, None);
        assert_eq!(second.art_url, None);
        assert_eq!(
            spotify_next_url(&body).as_deref(),
            Some("https://api.spotify.com/v1/playlists/37i9dQZF1DXcBWIGoYBM5M/tracks?offset=100&limit=100")
        );
    }

    #[test]
    fn parses_a_playlist_tracks_page() {
        // Segunda página: `/playlists/{id}/tracks` cuelga `items`/`next` arriba.
        let body: Value = serde_json::from_str(
            r#"{"items": [
              {"track": {"id": "4cOdK2wGLETKBW3PvgPWqT", "name": "Borderline",
                "duration_ms": 237000, "artists": [{"name": "Tame Impala"}],
                "album": {"name": "The Slow Rush"}}}
            ], "next": null}"#,
        )
        .unwrap();
        let tracks = parse_spotify_tracks(&body);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].provider, "spotify");
        assert_eq!(tracks[0].title, "Borderline");
        assert_eq!(spotify_next_url(&body), None);
    }

    #[test]
    fn parses_a_deezer_playlist() {
        let body: Value = serde_json::from_str(
            r#"{
              "id": 2881941882,
              "title": "Road trip",
              "description": "Para el coche",
              "picture_xl": "https://cdn-images.dzcdn.net/cover/playlist/1000x1000.jpg",
              "tracks": {"data": [
                {"id": 872345282, "title": "Borderline", "duration": 238,
                 "isrc": "AUUM71900956",
                 "artist": {"name": "Tame Impala"},
                 "album": {"title": "The Slow Rush",
                           "cover_xl": "https://cdn-images.dzcdn.net/cover/album/1000x1000.jpg"}},
                {"id": 872345283, "title": "Alright", "duration": 177,
                 "artist": {"name": "Kendrick Lamar"},
                 "album": {"title": "To Pimp a Butterfly"}}
              ]}
            }"#,
        )
        .unwrap();
        let list = parse_deezer_playlist(&body, "2881941882").unwrap();
        assert_eq!(list.provider, "deezer");
        assert_eq!(list.id, "2881941882");
        assert_eq!(list.title, "Road trip");
        assert_eq!(list.description, "Para el coche");
        assert_eq!(
            list.cover_url.as_deref(),
            Some("https://cdn-images.dzcdn.net/cover/playlist/1000x1000.jpg")
        );
        assert_eq!(list.tracks.len(), 2);
        let first = &list.tracks[0];
        assert_eq!(first.provider, "deezer");
        assert_eq!(first.id, "dz:872345282");
        assert_eq!(first.title, "Borderline");
        assert_eq!(first.artists, "Tame Impala");
        assert_eq!(first.album, "The Slow Rush");
        assert_eq!(first.duration_ms, 238_000);
        assert_eq!(first.isrc.as_deref(), Some("AUUM71900956"));
        assert_eq!(
            first.art_url.as_deref(),
            Some("https://cdn-images.dzcdn.net/cover/album/1000x1000.jpg")
        );
        // Sin cover de álbum cae a la portada de la playlist.
        let second = &list.tracks[1];
        assert_eq!(second.art_url, list.cover_url);
        assert_eq!(second.isrc, None);
        assert_eq!(second.download_query(), "Kendrick Lamar - Alright");
    }

    #[test]
    fn playlist_parsers_need_a_title() {
        let spotify: Value = serde_json::from_str(r#"{"tracks":{"items":[]}}"#).unwrap();
        assert!(parse_spotify_playlist(&spotify, "x").is_err());
        let deezer: Value = serde_json::from_str(r#"{"tracks":{"data":[]}}"#).unwrap();
        assert!(parse_deezer_playlist(&deezer, "1").is_err());
        // Playlist vacía pero con título: válida, cero pistas.
        let empty: Value =
            serde_json::from_str(r#"{"name":"Vacía","tracks":{"items":[]}}"#).unwrap();
        let list = parse_spotify_playlist(&empty, "x").unwrap();
        assert!(list.tracks.is_empty());
        assert_eq!(list.cover_url, None);
        assert_eq!(list.description, "");
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
