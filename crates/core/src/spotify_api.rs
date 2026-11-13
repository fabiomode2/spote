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
const SPOTIFY_ARTIST_URL: &str = "https://api.spotify.com/v1/artists";
const SPOTIFY_ALBUM_URL: &str = "https://api.spotify.com/v1/albums";
const DEEZER_SEARCH_URL: &str = "https://api.deezer.com/search";
const DEEZER_SEARCH_ARTIST_URL: &str = "https://api.deezer.com/search/artist";
const DEEZER_SEARCH_ALBUM_URL: &str = "https://api.deezer.com/search/album";
const DEEZER_ARTIST_URL: &str = "https://api.deezer.com/artist";
const DEEZER_ALBUM_URL: &str = "https://api.deezer.com/album";
/// Top de pistas del perfil: más no cabe en la pantalla de artista.
const MAX_ARTIST_TRACKS: usize = 10;
/// Álbumes por artista en la ficha: suficiente para la pestaña y una llamada.
const MAX_ALBUMS: usize = 20;
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

/// Ficha de artista: metadatos, imagen y sus pistas más populares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistProfile {
    /// `spotify` o `deezer`.
    pub provider: String,
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub genres: Vec<String>,
    pub followers: u64,
    /// 0..100. Deezer no lo expone: ahí vale 0.
    pub popularity: u32,
    pub top_tracks: Vec<SpotifyTrack>,
    /// Discografía reciente. Puede ir vacía si la llamada extra falla o el
    /// artista no tiene álbumes; el resto del perfil sigue valiendo.
    #[serde(default)]
    pub albums: Vec<AlbumSummary>,
    /// Biografía; `None` cuando el proveedor no la da.
    pub bio: Option<String>,
}

/// Álbum en un listado (búsqueda mixta o discografía de un artista).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumSummary {
    /// `spotify` o `deezer`.
    pub provider: String,
    /// Id del proveedor. En Deezer va pelado (numérico); `album()` también
    /// acepta el prefijo `dz:` que llevan las pistas.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    /// Solo el año (`2020`), aunque el proveedor mande la fecha completa.
    pub release_year: Option<u16>,
    pub track_count: u32,
}

/// Artista en un listado de búsqueda mixta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    /// `spotify` o `deezer`.
    pub provider: String,
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub followers: u64,
}

/// Resultado de una búsqueda mixta: pistas, álbumes y artistas de un tirón.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchAll {
    /// `spotify` o `deezer` (vacío si no hubo consulta).
    pub provider: String,
    pub tracks: Vec<SpotifyTrack>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
}

impl SearchAll {
    /// Sin resultados de ningún tipo: el llamante puede probar otro proveedor.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.albums.is_empty() && self.artists.is_empty()
    }

    /// Recorta cada bloque a `limit` (Deezer mezcla tipos en una sola lista).
    fn trim(&mut self, limit: usize) {
        self.tracks.truncate(limit);
        self.albums.truncate(limit);
        self.artists.truncate(limit);
    }
}

/// Álbum abierto: metadatos y tracklist completo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumInfo {
    /// `spotify` o `deezer`.
    pub provider: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    /// Tal cual la manda el proveedor (`2020-02-14` en Spotify, `2020` a veces
    /// en Deezer).
    pub release_date: Option<String>,
    pub tracks: Vec<SpotifyTrack>,
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

/// Hay credenciales de aplicación en el entorno (vía fiable cuando el token
/// anónimo está bloqueado).
fn has_credentials() -> bool {
    std::env::var("SPOTIFY_CLIENT_ID").is_ok() && std::env::var("SPOTIFY_CLIENT_SECRET").is_ok()
}

fn search_spotify(query: &str, limit: usize) -> Result<Vec<SpotifyTrack>> {
    let client = client()?;
    let has_creds = has_credentials();
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
    let mut url = reqwest::Url::parse(DEEZER_SEARCH_URL).context("url deezer")?;
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
    items.iter().filter_map(deezer_search_track).collect()
}

/// Pista del buscador de Deezer. Las respuestas mixtas de `/search` mezclan
/// pistas, álbumes y artistas: aquí solo entran las que traen `duration`.
fn deezer_search_track(item: &Value) -> Option<SpotifyTrack> {
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
    let duration_ms = item.get("duration").and_then(Value::as_u64).unwrap_or(0) * 1000;
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
}

// ---------------------------------------------------------------------------
// Búsqueda mixta (pistas + álbumes + artistas)
// ---------------------------------------------------------------------------

/// Busca pistas, álbumes y artistas de una vez.
///
/// Spotify resuelve los tres bloques con una sola petición
/// (`type=track,album,artist`). Si falla o vuelve todo vacío se usa la
/// búsqueda mixta de Deezer, que devuelve los tres tipos en la misma lista.
/// Nunca se mezclan resultados de los dos proveedores.
pub fn search_all(query: &str, limit: usize) -> Result<SearchAll> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchAll::default());
    }
    match spotify_search_all(query, limit) {
        Ok(all) if !all.is_empty() => Ok(all),
        // Un Spotify vacío no vale como respuesta si Deezer tiene algo; si
        // también falla Deezer se devuelve lo de Spotify (vacío o el error).
        spotify => match deezer_search_all(query, limit) {
            Ok(all) if !all.is_empty() => Ok(all),
            _ => spotify,
        },
    }
}

fn spotify_search_all(query: &str, limit: usize) -> Result<SearchAll> {
    let mut url = reqwest::Url::parse(SEARCH_URL).context("url de búsqueda")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("type", "track,album,artist")
        .append_pair("limit", &limit.clamp(1, 20).to_string())
        .append_pair("market", "ES");
    let body = spotify_get(url.as_str())?;
    Ok(parse_spotify_search_all(&body))
}

/// `tracks` + `albums` + `artists` de un `/v1/search` mixto.
fn parse_spotify_search_all(body: &Value) -> SearchAll {
    SearchAll {
        provider: "spotify".into(),
        tracks: parse_search(body),
        albums: page_items(body, "/albums/items")
            .iter()
            .filter_map(spotify_album_summary)
            .collect(),
        artists: page_items(body, "/artists/items")
            .iter()
            .filter_map(spotify_artist_summary)
            .collect(),
    }
}

fn deezer_search_all(query: &str, limit: usize) -> Result<SearchAll> {
    let client = client()?;
    let limit = limit.clamp(1, 25);
    let mut url = reqwest::Url::parse(DEEZER_SEARCH_URL).context("url deezer")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("limit", &limit.to_string());
    let body = deezer_get(&client, url.as_str())?;
    let mut all = parse_deezer_search_all(&body);
    // `/search` de Deezer devuelve casi siempre solo pistas; artistas y
    // álbumes viven en endpoints separados. Se piden en paralelo lógico y se
    // ignoran los fallos parciales: una búsqueda de canciones sigue sirviendo.
    let mut artist_url = reqwest::Url::parse(DEEZER_SEARCH_ARTIST_URL).context("url deezer")?;
    artist_url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("limit", &limit.min(8).to_string());
    if let Ok(body) = deezer_get(&client, artist_url.as_str()) {
        if let Some(items) = body.get("data").and_then(Value::as_array) {
            all.artists.extend(items.iter().filter_map(deezer_artist_summary));
        }
    }
    let mut album_url = reqwest::Url::parse(DEEZER_SEARCH_ALBUM_URL).context("url deezer")?;
    album_url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("limit", &limit.min(8).to_string());
    if let Ok(body) = deezer_get(&client, album_url.as_str()) {
        if let Some(items) = body.get("data").and_then(Value::as_array) {
            all.albums.extend(items.iter().filter_map(deezer_album_summary));
        }
    }
    all.artists.truncate(limit.min(8));
    all.albums.truncate(limit.min(8));
    all.trim(limit);
    Ok(all)
}

/// Lista mixta de `/search?q=` de Deezer: cada ítem es pista, álbum o artista
/// según su campo `type` (o su forma, si no viene).
fn parse_deezer_search_all(body: &Value) -> SearchAll {
    let mut all = SearchAll {
        provider: "deezer".into(),
        ..SearchAll::default()
    };
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return all;
    };
    for item in items {
        match deezer_kind(item) {
            Some("track") => all.tracks.extend(deezer_search_track(item)),
            Some("album") => all.albums.extend(deezer_album_summary(item)),
            Some("artist") => all.artists.extend(deezer_artist_summary(item)),
            _ => {}
        }
    }
    all
}

/// Tipo de un ítem de `/search` de Deezer. Las respuestas mixtas traen `type`;
/// si falta se deduce por los campos que solo tiene cada tipo.
fn deezer_kind(item: &Value) -> Option<&'static str> {
    match item.get("type").and_then(Value::as_str) {
        Some("track") => return Some("track"),
        Some("album") => return Some("album"),
        Some("artist") => return Some("artist"),
        // playlists, podcasts...: no pintan nada en una búsqueda mixta.
        Some(_) => return None,
        None => {}
    }
    if item.get("duration").is_some() && item.get("album").is_some() {
        Some("track")
    } else if item.get("nb_tracks").is_some() {
        Some("album")
    } else if item.get("nb_fan").is_some() || item.get("picture_xl").is_some() {
        Some("artist")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Artistas
// ---------------------------------------------------------------------------

/// Fallo de una llamada autenticada a Spotify: 401 (token caducado) o el resto.
enum FetchError {
    Unauthorized,
    Failed(anyhow::Error),
}

impl FetchError {
    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Unauthorized => anyhow!("spotify: token caducado"),
            Self::Failed(err) => err,
        }
    }
}

impl From<anyhow::Error> for FetchError {
    fn from(err: anyhow::Error) -> Self {
        Self::Failed(err)
    }
}

/// Perfil de artista por nombre: imagen, géneros, seguidores, popularidad y
/// sus 10 pistas más escuchadas.
///
/// Spotify primero; si falla por lo que sea (token anónimo bloqueado, sin red,
/// artista inexistente) se cae a Deezer. Bloquea: llámalo desde un hilo.
pub fn artist(name: &str) -> Result<ArtistProfile> {
    let query = name.trim();
    if query.is_empty() {
        return Err(anyhow!("nombre de artista vacío"));
    }
    match spotify_artist(query) {
        Ok(profile) => Ok(profile),
        // Cualquier fallo de Spotify cae a Deezer; si también falla, el error
        // que se devuelve es el de Deezer (el último).
        Err(_) => deezer_artist(query),
    }
}

/// GET autenticado → JSON. `Err(Unauthorized)` si el token ya no vale.
fn fetch_json(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
) -> std::result::Result<Value, FetchError> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .context("spotify")?;
    if response.status().as_u16() == 401 {
        // El llamante pide un token nuevo y reintenta una vez.
        return Err(FetchError::Unauthorized);
    }
    if !response.status().is_success() {
        return Err(anyhow!("spotify: http {}", response.status()).into());
    }
    Ok(response.json().context("json de spotify")?)
}

/// Ejecuta una consulta autenticada a Spotify reintentando una vez si el token
/// estaba caducado. Un 403 (token anónimo bloqueado) sale como error para que
/// el llamante caiga a Deezer.
fn with_spotify<T>(
    fetch: impl Fn(&reqwest::blocking::Client, &str) -> std::result::Result<T, FetchError>,
) -> Result<T> {
    let client = client()?;
    let has_creds = has_credentials();
    let mut last_error = None;
    for attempt in 0..2 {
        let token = if has_creds {
            client_credentials_token(&client)?
        } else {
            access_token(attempt > 0)?
        };
        match fetch(&client, &token) {
            Ok(value) => return Ok(value),
            Err(FetchError::Unauthorized) if attempt == 0 => {
                last_error = Some(anyhow!("spotify: token caducado"));
            }
            Err(err) => return Err(err.into_error()),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("spotify")))
}

/// GET autenticado a Spotify → JSON, resolviendo el token sobre la marcha.
fn spotify_get(url: &str) -> Result<Value> {
    with_spotify(|client, token| fetch_json(client, url, token))
}

fn spotify_artist(name: &str) -> Result<ArtistProfile> {
    with_spotify(|client, token| spotify_artist_flow(client, token, name))
}

/// Búsqueda + ficha + top-tracks de Spotify con un token ya resuelto.
fn spotify_artist_flow(
    client: &reqwest::blocking::Client,
    token: &str,
    name: &str,
) -> std::result::Result<ArtistProfile, FetchError> {
    let mut search_url = reqwest::Url::parse(SEARCH_URL).context("url de búsqueda")?;
    search_url
        .query_pairs_mut()
        .append_pair("q", name)
        .append_pair("type", "artist")
        .append_pair("limit", "1")
        .append_pair("market", "ES");
    let search = fetch_json(client, search_url.as_str(), token)?;
    let id = parse_artist_search(&search)
        .ok_or_else(|| anyhow!("Spotify no encuentra al artista «{name}»"))?;
    spotify_profile(client, token, &id)
}

/// Ficha + top-tracks + álbumes de Spotify con un token ya resuelto. La
/// discografía es best-effort: si la llamada extra falla, el perfil sale sin
/// ella en vez de fallar entero.
fn spotify_profile(
    client: &reqwest::blocking::Client,
    token: &str,
    id: &str,
) -> std::result::Result<ArtistProfile, FetchError> {
    let detail = fetch_json(client, &format!("{SPOTIFY_ARTIST_URL}/{id}"), token)?;
    let mut top_url = reqwest::Url::parse(&format!("{SPOTIFY_ARTIST_URL}/{id}/top-tracks"))
        .context("url de top-tracks")?;
    top_url.query_pairs_mut().append_pair("market", "ES");
    let top = fetch_json(client, top_url.as_str(), token)?;
    let mut profile = parse_spotify_artist(&detail, &top)
        .ok_or_else(|| anyhow!("la ficha de artista de Spotify llega incompleta"))?;
    if let Ok(albums) = spotify_artist_albums_at(client, token, id) {
        profile.albums = albums;
    }
    Ok(profile)
}

/// Id del primer artista de `/v1/search?type=artist`.
fn parse_artist_search(body: &Value) -> Option<String> {
    body.pointer("/artists/items/0/id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Lista de cadenas de un campo tipo `genres`.
fn string_list(node: Option<&Value>) -> Vec<String> {
    node.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Objeto `/v1/artists/{id}` + `/top-tracks` → perfil.
fn parse_spotify_artist(body: &Value, top_tracks: &Value) -> Option<ArtistProfile> {
    let id = body.get("id").and_then(Value::as_str)?.to_string();
    let name = body.get("name").and_then(Value::as_str)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let popularity = body
        .get("popularity")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(100) as u32;
    Some(ArtistProfile {
        provider: "spotify".into(),
        id,
        name,
        image_url: widest_image(body),
        genres: string_list(body.get("genres")),
        followers: body
            .pointer("/followers/total")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        popularity,
        top_tracks: parse_top_tracks(top_tracks),
        albums: Vec::new(),
        // La API pública no expone biografía.
        bio: None,
    })
}

/// `tracks[]` de `/v1/artists/{id}/top-tracks`, recortado a `MAX_ARTIST_TRACKS`.
fn parse_top_tracks(body: &Value) -> Vec<SpotifyTrack> {
    let mut tracks: Vec<SpotifyTrack> = body
        .get("tracks")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(spotify_track).collect())
        .unwrap_or_default();
    tracks.truncate(MAX_ARTIST_TRACKS);
    tracks
}

/// Deezer contesta 200 con `{"error":{...}}` cuando no le gusta la petición.
fn deezer_error(body: &Value) -> Option<String> {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn deezer_artist(name: &str) -> Result<ArtistProfile> {
    let client = client()?;
    let mut search_url = reqwest::Url::parse(DEEZER_SEARCH_ARTIST_URL).context("url deezer")?;
    search_url
        .query_pairs_mut()
        .append_pair("q", name)
        .append_pair("limit", "5");
    let search = deezer_get(&client, search_url.as_str())?;
    let id = parse_deezer_artist_search(&search, name)
        .ok_or_else(|| anyhow!("Deezer no encuentra al artista «{name}»"))?;
    deezer_profile(&client, &id)
}

/// Ficha + top-tracks + álbumes de Deezer. La discografía es best-effort.
fn deezer_profile(client: &reqwest::blocking::Client, id: &str) -> Result<ArtistProfile> {
    let detail = deezer_get(client, &format!("{DEEZER_ARTIST_URL}/{id}"))?;
    let top = deezer_get(
        client,
        &format!("{DEEZER_ARTIST_URL}/{id}/top?limit={MAX_ARTIST_TRACKS}"),
    )?;
    let mut profile = parse_deezer_artist(&detail, &top)
        .ok_or_else(|| anyhow!("la ficha de artista de Deezer llega incompleta"))?;
    if let Ok(mut albums) = deezer_artist_albums_with(client, id) {
        let name = profile.name.clone();
        fill_artist(&mut albums, &name);
        profile.albums = albums;
    }
    Ok(profile)
}

/// GET a Deezer → JSON, con el mensaje de error que manda Deezer en el cuerpo.
fn deezer_get(client: &reqwest::blocking::Client, url: &str) -> Result<Value> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .context("deezer")?;
    if !response.status().is_success() {
        return Err(anyhow!("deezer: http {}", response.status()));
    }
    let body: Value = response.json().context("json de deezer")?;
    match deezer_error(&body) {
        Some(message) => Err(anyhow!("deezer: {message}")),
        None => Ok(body),
    }
}

/// Mejor artista de `/search/artist`: se queda con el nombre exacto si lo hay,
/// no con el primer homónimo cutre que devuelva Deezer.
fn parse_deezer_artist_search(body: &Value, wanted: &str) -> Option<String> {
    let wanted = crate::util::normalize(wanted);
    let items = body.get("data").and_then(Value::as_array)?;
    let mut best: Option<(u8, String)> = None;
    for item in items {
        let Some(id) = item.get("id").and_then(deezer_id) else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .map(crate::util::normalize)
            .unwrap_or_default();
        let score = if name == wanted {
            3
        } else if name.contains(&wanted) || wanted.contains(&name) {
            2
        } else if !name.is_empty() {
            1
        } else {
            0
        };
        if best.as_ref().is_none_or(|(current, _)| score > *current) {
            best = Some((score, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Id de Deezer: llega como número JSON y se guarda como texto.
fn deezer_id(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }
}

/// Objeto `/artist/{id}` + `/artist/{id}/top` → perfil.
fn parse_deezer_artist(body: &Value, top_tracks: &Value) -> Option<ArtistProfile> {
    let id = body.get("id").and_then(deezer_id)?;
    let name = body.get("name").and_then(Value::as_str)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(ArtistProfile {
        provider: "deezer".into(),
        id,
        name,
        image_url: body
            .get("picture_xl")
            .or_else(|| body.get("picture_big"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string),
        // Deezer no da géneros ni popularidad en el objeto de artista.
        genres: Vec::new(),
        followers: body.get("nb_fan").and_then(Value::as_u64).unwrap_or(0),
        popularity: 0,
        top_tracks: parse_deezer_top_tracks(top_tracks),
        albums: Vec::new(),
        bio: None,
    })
}

/// `data[]` de `/artist/{id}/top`, recortado a `MAX_ARTIST_TRACKS`.
fn parse_deezer_top_tracks(body: &Value) -> Vec<SpotifyTrack> {
    let mut tracks: Vec<SpotifyTrack> = body
        .get("data")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(deezer_artist_track).collect())
        .unwrap_or_default();
    tracks.truncate(MAX_ARTIST_TRACKS);
    tracks
}

/// Pista de `/artist/{id}/top`. Mismo id `dz:<n>` que usa el resto del catálogo.
fn deezer_artist_track(item: &Value) -> Option<SpotifyTrack> {
    let id = format!("dz:{}", item.get("id").and_then(deezer_id)?);
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
    let art_url = item
        .pointer("/album/cover_xl")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string);
    Some(SpotifyTrack {
        provider: "deezer".into(),
        id,
        title,
        artists,
        album,
        duration_ms,
        art_url,
        // El top de Deezer no es una fuente fiable de ISRC.
        isrc: None,
    })
}

// ---------------------------------------------------------------------------
// Álbumes y fichas por id
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Spotify,
    Deezer,
}

/// Proveedor a partir del campo `provider` de lo que devolvimos antes.
fn provider_of(value: &str) -> Option<Provider> {
    match value.trim().to_ascii_lowercase().as_str() {
        "spotify" => Some(Provider::Spotify),
        "deezer" | "dz" => Some(Provider::Deezer),
        _ => None,
    }
}

/// Un id solo de dígitos es de Deezer: Spotify usa base62 de 22 caracteres.
fn is_numeric_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

/// Id pelado: sin espacios y sin el prefijo `dz:` de las pistas de Deezer.
fn clean_id(id: &str) -> &str {
    let id = id.trim();
    match id.strip_prefix("dz:") {
        Some(rest) => rest.trim(),
        None => id,
    }
}

/// Orden de intento para un id. Con proveedor explícito se respeta, pero tras
/// Spotify se prueba Deezer: el token anónimo da 403 en muchas redes y un
/// fallo de más es mejor que una ficha vacía. Sin proveedor manda la forma
/// del id (los de Spotify nunca son numéricos).
fn provider_chain(provider: &str, id: &str) -> [Option<Provider>; 2] {
    match provider_of(provider) {
        Some(Provider::Deezer) => [Some(Provider::Deezer), None],
        Some(Provider::Spotify) => [Some(Provider::Spotify), Some(Provider::Deezer)],
        None if is_numeric_id(id) => [Some(Provider::Deezer), Some(Provider::Spotify)],
        None => [Some(Provider::Spotify), Some(Provider::Deezer)],
    }
}

/// Prueba los proveedores en orden y se queda con el primer acierto; si todos
/// fallan devuelve el último error (el del proveedor de respaldo).
fn try_providers<T>(
    provider: &str,
    id: &str,
    mut fetch: impl FnMut(Provider) -> Result<T>,
) -> Result<T> {
    let mut last_error = None;
    for candidate in provider_chain(provider, id).into_iter().flatten() {
        match fetch(candidate) {
            Ok(value) => return Ok(value),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("proveedor desconocido")))
}

/// Álbum abierto: portada, fecha y tracklist completo. Bloquea: llámalo desde
/// un hilo.
pub fn album(provider: &str, id: &str) -> Result<AlbumInfo> {
    let id = clean_id(id);
    if id.is_empty() {
        return Err(anyhow!("id de álbum vacío"));
    }
    try_providers(provider, id, |provider| match provider {
        Provider::Spotify => spotify_album(id),
        Provider::Deezer => deezer_album(id),
    })
}

fn spotify_album(id: &str) -> Result<AlbumInfo> {
    let mut url =
        reqwest::Url::parse(&format!("{SPOTIFY_ALBUM_URL}/{id}")).context("url de álbum")?;
    url.query_pairs_mut().append_pair("market", "ES");
    let body = spotify_get(url.as_str())?;
    parse_spotify_album(&body, id)
        .ok_or_else(|| anyhow!("la ficha del álbum de Spotify llega incompleta"))
}

/// `/v1/albums/{id}` → álbum. Sus pistas cuelgan de `tracks.items` y no traen
/// el álbum ni la portada: se heredan de la ficha.
fn parse_spotify_album(body: &Value, fallback_id: &str) -> Option<AlbumInfo> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(fallback_id);
    let title = body.get("name").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() || title.is_empty() {
        return None;
    }
    let cover_url = widest_image(body);
    Some(AlbumInfo {
        provider: "spotify".into(),
        id: id.to_string(),
        title: title.clone(),
        artist: join_artists(body),
        cover_url: cover_url.clone(),
        release_date: body
            .get("release_date")
            .and_then(Value::as_str)
            .map(str::to_string),
        tracks: page_items(body, "/tracks/items")
            .iter()
            .filter_map(|node| spotify_album_track(node, &title, cover_url.as_deref()))
            .collect(),
    })
}

/// Pista de `/albums/{id}`: hereda nombre de álbum y portada de la ficha.
fn spotify_album_track(node: &Value, album: &str, cover: Option<&str>) -> Option<SpotifyTrack> {
    let mut track = spotify_track(node)?;
    if track.album.is_empty() {
        track.album = album.to_string();
    }
    if track.art_url.is_none() {
        track.art_url = cover.map(str::to_string);
    }
    Some(track)
}

fn deezer_album(id: &str) -> Result<AlbumInfo> {
    let client = client()?;
    let body = deezer_get(&client, &format!("{DEEZER_ALBUM_URL}/{id}"))?;
    parse_deezer_album(&body, id)
        .ok_or_else(|| anyhow!("la ficha del álbum de Deezer llega incompleta"))
}

/// `/album/{id}` → álbum. El tracklist viene en `tracks.data`.
fn parse_deezer_album(body: &Value, fallback_id: &str) -> Option<AlbumInfo> {
    let id = body
        .get("id")
        .and_then(deezer_id)
        .unwrap_or_else(|| fallback_id.to_string());
    let title = body
        .get("title")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if id.is_empty() || title.is_empty() {
        return None;
    }
    let cover_url = deezer_cover(body);
    let artist = body
        .pointer("/artist/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(AlbumInfo {
        provider: "deezer".into(),
        id,
        title: title.clone(),
        artist,
        cover_url: cover_url.clone(),
        release_date: body
            .get("release_date")
            .and_then(Value::as_str)
            .map(str::to_string),
        tracks: page_items(body, "/tracks/data")
            .iter()
            .filter_map(|node| deezer_album_track(node, &title, cover_url.as_deref()))
            .collect(),
    })
}

/// Pista de `/album/{id}`: hereda nombre de álbum y portada de la ficha.
fn deezer_album_track(node: &Value, album: &str, cover: Option<&str>) -> Option<SpotifyTrack> {
    let mut track = deezer_artist_track(node)?;
    if track.album.is_empty() {
        track.album = album.to_string();
    }
    if track.art_url.is_none() {
        track.art_url = cover.map(str::to_string);
    }
    Some(track)
}

/// Discografía reciente del artista (álbumes y sencillos). Bloquea.
pub fn albums_of(provider: &str, artist_id: &str) -> Result<Vec<AlbumSummary>> {
    let id = clean_id(artist_id);
    if id.is_empty() {
        return Err(anyhow!("id de artista vacío"));
    }
    let deezer = || deezer_artist_albums(id);
    // Un Spotify vacío tampoco vale como respuesta si Deezer tiene algo.
    let spotify = || match spotify_artist_albums(id) {
        Ok(albums) if !albums.is_empty() => Ok(albums),
        Ok(_) => Err(anyhow!("spotify: sin álbumes para {id}")),
        Err(err) => Err(err),
    };
    match provider_of(provider) {
        Some(Provider::Deezer) => deezer(),
        Some(Provider::Spotify) => spotify().or_else(|_| deezer()),
        None if is_numeric_id(id) => deezer().or_else(|_| spotify()),
        None => spotify().or_else(|_| deezer()),
    }
}

fn spotify_artist_albums(id: &str) -> Result<Vec<AlbumSummary>> {
    with_spotify(|client, token| spotify_artist_albums_at(client, token, id))
}

fn spotify_artist_albums_at(
    client: &reqwest::blocking::Client,
    token: &str,
    id: &str,
) -> std::result::Result<Vec<AlbumSummary>, FetchError> {
    let mut url = reqwest::Url::parse(&format!("{SPOTIFY_ARTIST_URL}/{id}/albums"))
        .context("url de álbumes")?;
    url.query_pairs_mut()
        .append_pair("include_groups", "album,single")
        .append_pair("market", "ES")
        .append_pair("limit", &MAX_ALBUMS.to_string());
    let body = fetch_json(client, url.as_str(), token)?;
    Ok(parse_spotify_albums(&body))
}

fn parse_spotify_albums(body: &Value) -> Vec<AlbumSummary> {
    page_items(body, "/items")
        .iter()
        .filter_map(spotify_album_summary)
        .collect()
}

fn deezer_artist_albums(id: &str) -> Result<Vec<AlbumSummary>> {
    let client = client()?;
    deezer_artist_albums_with(&client, id)
}

fn deezer_artist_albums_with(
    client: &reqwest::blocking::Client,
    id: &str,
) -> Result<Vec<AlbumSummary>> {
    let body = deezer_get(
        client,
        &format!("{DEEZER_ARTIST_URL}/{id}/albums?limit={MAX_ALBUMS}"),
    )?;
    Ok(parse_deezer_albums(&body))
}

fn parse_deezer_albums(body: &Value) -> Vec<AlbumSummary> {
    body.get("data")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(deezer_album_summary).collect())
        .unwrap_or_default()
}

/// Ficha de artista por id, en el mismo formato que `artist(name)`: incluye
/// top-tracks y discografía. Bloquea.
pub fn artist_by_id(provider: &str, id: &str) -> Result<ArtistProfile> {
    let id = clean_id(id);
    if id.is_empty() {
        return Err(anyhow!("id de artista vacío"));
    }
    try_providers(provider, id, |provider| match provider {
        Provider::Spotify => spotify_artist_by_id(id),
        Provider::Deezer => deezer_artist_by_id(id),
    })
}

fn spotify_artist_by_id(id: &str) -> Result<ArtistProfile> {
    with_spotify(|client, token| spotify_profile(client, token, id))
}

fn deezer_artist_by_id(id: &str) -> Result<ArtistProfile> {
    let client = client()?;
    deezer_profile(&client, id)
}

/// Resumen de un álbum de Spotify (`/search` o `/artists/{id}/albums`).
fn spotify_album_summary(node: &Value) -> Option<AlbumSummary> {
    let id = node.get("id").and_then(Value::as_str)?.trim().to_string();
    let title = node.get("name").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() || title.is_empty() {
        return None;
    }
    Some(AlbumSummary {
        provider: "spotify".into(),
        id,
        title,
        artist: join_artists(node),
        cover_url: widest_image(node),
        release_year: release_year(node.get("release_date").and_then(Value::as_str)),
        track_count: field_u32(node, "total_tracks"),
    })
}

/// Resumen de un álbum de Deezer (`/search` o `/artist/{id}/albums`).
fn deezer_album_summary(node: &Value) -> Option<AlbumSummary> {
    let id = node.get("id").and_then(deezer_id)?;
    let title = node
        .get("title")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
    Some(AlbumSummary {
        provider: "deezer".into(),
        id,
        title,
        artist: node
            .pointer("/artist/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        cover_url: deezer_cover(node),
        release_year: release_year(node.get("release_date").and_then(Value::as_str)),
        track_count: field_u32(node, "nb_tracks"),
    })
}

/// Resumen de un artista de `/v1/search?type=artist`.
fn spotify_artist_summary(node: &Value) -> Option<ArtistSummary> {
    let id = node.get("id").and_then(Value::as_str)?.trim().to_string();
    let name = node.get("name").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() || name.is_empty() {
        return None;
    }
    Some(ArtistSummary {
        provider: "spotify".into(),
        id,
        name,
        image_url: widest_image(node),
        followers: node
            .pointer("/followers/total")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

/// Resumen de un artista de `/search` de Deezer.
fn deezer_artist_summary(node: &Value) -> Option<ArtistSummary> {
    let id = node.get("id").and_then(deezer_id)?;
    let name = node.get("name").and_then(Value::as_str)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(ArtistSummary {
        provider: "deezer".into(),
        id,
        name,
        image_url: deezer_picture(node),
        followers: node.get("nb_fan").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// `items[]` de un bloque de resultados de Spotify (`/tracks/items`, `/items`…).
fn page_items<'a>(body: &'a Value, pointer: &str) -> &'a [Value] {
    body.pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Portada de Deezer, de mayor a menor resolución.
fn deezer_cover(node: &Value) -> Option<String> {
    first_url(node, &["cover_xl", "cover_big", "cover_medium", "cover"])
}

/// Foto de artista de Deezer, de mayor a menor resolución.
fn deezer_picture(node: &Value) -> Option<String> {
    first_url(
        node,
        &["picture_xl", "picture_big", "picture_medium", "picture"],
    )
}

/// Primera URL no vacía de una lista de claves.
fn first_url(node: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_str))
        .find(|url| !url.is_empty())
        .map(str::to_string)
}

/// Año de un `release_date` tipo `2020-02-14` o `2020`.
fn release_year(date: Option<&str>) -> Option<u16> {
    let year = date?.trim().get(..4)?;
    if !year.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    year.parse().ok()
}

/// Contador de un campo numérico (`total_tracks`, `nb_tracks`) sin desbordar.
fn field_u32(node: &Value, key: &str) -> u32 {
    node.get(key)
        .and_then(Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

/// Rellena el nombre del artista en los álbumes que no lo traen (el listado
/// de Deezer a veces lo omite).
fn fill_artist(albums: &mut [AlbumSummary], artist: &str) {
    for album in albums.iter_mut().filter(|album| album.artist.is_empty()) {
        album.artist = artist.to_string();
    }
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
        Some(PlaylistProvider::Deezer) => {
            Err("solo se importan listas de Spotify".to_string())
        }
        None => spotify_playlist(&id),
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
    let has_creds = has_credentials();
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
    fn public_playlist_rejects_deezer_urls() {
        let error = playlist("https://www.deezer.com/playlist/908622995").unwrap_err();
        assert!(error.contains("solo se importan listas de Spotify"), "{error}");
    }

    #[test]
    fn playlist_parsers_need_a_title() {
        let spotify: Value = serde_json::from_str(r#"{"tracks":{"items":[]}}"#).unwrap();
        assert!(parse_spotify_playlist(&spotify, "x").is_err());
        // Playlist vacía pero con título: válida, cero pistas.
        let empty: Value =
            serde_json::from_str(r#"{"name":"Vacía","tracks":{"items":[]}}"#).unwrap();
        let list = parse_spotify_playlist(&empty, "x").unwrap();
        assert!(list.tracks.is_empty());
        assert_eq!(list.cover_url, None);
        assert_eq!(list.description, "");
    }

    #[test]
    fn parses_a_spotify_artist_profile() {
        let search: Value = serde_json::from_str(
            r#"{"artists":{"items":[{"id":"5INjqkS1o8h1imAzPqGZBb","name":"Tame Impala"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_artist_search(&search).as_deref(),
            Some("5INjqkS1o8h1imAzPqGZBb")
        );

        let detail: Value = serde_json::from_str(
            r#"{
              "id": "5INjqkS1o8h1imAzPqGZBb",
              "name": "Tame Impala",
              "genres": ["neo-psychedelic", "psychedelic pop"],
              "followers": {"total": 5654321, "href": null},
              "popularity": 78,
              "images": [{"url": "https://i.scdn.co/160", "width": 160},
                         {"url": "https://i.scdn.co/640", "width": 640},
                         {"url": "https://i.scdn.co/1000", "width": 1000}]
            }"#,
        )
        .unwrap();
        let top: Value = serde_json::from_str(
            r#"{"tracks":[
              {"id":"4cOdK2wGLETKBW3PvgPWqT","name":"Borderline","duration_ms":237000,
               "artists":[{"name":"Tame Impala"}],
               "album":{"name":"The Slow Rush","images":[
                 {"url":"small.jpg","width":64},{"url":"big.jpg","width":640}]},
               "external_ids":{"isrc":"AUUM71900931"}},
              {"id":"7oK9VyNzrYvRFo7nQEYkWN","name":"The Less I Know the Better",
               "duration_ms":216000,"artists":[{"name":"Tame Impala"}],
               "album":{"name":"Currents"}}
            ]}"#,
        )
        .unwrap();

        let profile = parse_spotify_artist(&detail, &top).unwrap();
        assert_eq!(profile.provider, "spotify");
        assert_eq!(profile.id, "5INjqkS1o8h1imAzPqGZBb");
        assert_eq!(profile.name, "Tame Impala");
        assert_eq!(profile.image_url.as_deref(), Some("https://i.scdn.co/1000"));
        assert_eq!(profile.genres, vec!["neo-psychedelic", "psychedelic pop"]);
        assert_eq!(profile.followers, 5_654_321);
        assert_eq!(profile.popularity, 78);
        assert_eq!(profile.bio, None);
        assert_eq!(profile.top_tracks.len(), 2);
        let first = &profile.top_tracks[0];
        assert_eq!(first.provider, "spotify");
        assert_eq!(first.id, "4cOdK2wGLETKBW3PvgPWqT");
        assert_eq!(first.title, "Borderline");
        assert_eq!(first.artists, "Tame Impala");
        assert_eq!(first.album, "The Slow Rush");
        assert_eq!(first.duration_ms, 237_000);
        assert_eq!(first.art_url.as_deref(), Some("big.jpg"));
        assert_eq!(first.isrc.as_deref(), Some("AUUM71900931"));
        assert_eq!(profile.top_tracks[1].art_url, None);
    }

    #[test]
    fn parses_a_deezer_artist_profile() {
        let search: Value = serde_json::from_str(
            r#"{"data":[{"id":13383903,"name":"Tame Impala",
              "picture_xl":"https://cdn/artist/1000x1000.jpg"}]}"#,
        )
        .unwrap();
        assert_eq!(
            parse_deezer_artist_search(&search, "Rosalía").as_deref(),
            Some("13383903")
        );

        let detail: Value = serde_json::from_str(
            r#"{"id":13383903,"name":"Tame Impala","nb_fan":1846233,
              "picture_xl":"https://cdn/artist/1000x1000.jpg"}"#,
        )
        .unwrap();
        let top: Value = serde_json::from_str(
            r#"{"data":[
              {"id":872345282,"title":"Borderline","duration":238,
               "isrc":"AUUM71900956",
               "artist":{"id":13383903,"name":"Tame Impala"},
               "album":{"id":1,"title":"The Slow Rush",
                 "cover_xl":"https://cdn/cover/1000x1000.jpg"}},
              {"id":872345283,"title":"The Less I Know the Better","duration":216,
               "artist":{"id":13383903,"name":"Tame Impala"},
               "album":{"id":2,"title":"Currents"}}
            ]}"#,
        )
        .unwrap();

        let profile = parse_deezer_artist(&detail, &top).unwrap();
        assert_eq!(profile.provider, "deezer");
        assert_eq!(profile.id, "13383903");
        assert_eq!(profile.name, "Tame Impala");
        assert_eq!(
            profile.image_url.as_deref(),
            Some("https://cdn/artist/1000x1000.jpg")
        );
        assert_eq!(profile.followers, 1_846_233);
        assert!(profile.genres.is_empty());
        assert_eq!(profile.popularity, 0);
        assert_eq!(profile.bio, None);
        assert_eq!(profile.top_tracks.len(), 2);
        let first = &profile.top_tracks[0];
        assert_eq!(first.provider, "deezer");
        assert_eq!(first.id, "dz:872345282");
        assert_eq!(first.title, "Borderline");
        assert_eq!(first.artists, "Tame Impala");
        assert_eq!(first.album, "The Slow Rush");
        assert_eq!(first.duration_ms, 238_000);
        assert_eq!(
            first.art_url.as_deref(),
            Some("https://cdn/cover/1000x1000.jpg")
        );
        assert_eq!(first.isrc, None);
        assert_eq!(profile.top_tracks[1].art_url, None);
        assert_eq!(
            profile.top_tracks[1].download_query(),
            "Tame Impala - The Less I Know the Better"
        );
    }

    #[test]
    fn artist_parsers_need_id_and_name() {
        let no_id: Value = serde_json::from_str(r#"{"name":"Alguien"}"#).unwrap();
        let empty: Value = serde_json::from_str(r#"{"tracks":[],"data":[]}"#).unwrap();
        assert!(parse_spotify_artist(&no_id, &empty).is_none());
        assert!(parse_deezer_artist(&no_id, &empty).is_none());
        let blank: Value = serde_json::from_str(r#"{"id":"x1","name":"   ","genres":[]}"#).unwrap();
        assert!(parse_spotify_artist(&blank, &empty).is_none());
        // Sin top-tracks el perfil sigue siendo válido, con cero pistas.
        let artist_body: Value =
            serde_json::from_str(r#"{"id":"x1","name":"Alguien","popularity":140}"#).unwrap();
        let profile = parse_spotify_artist(&artist_body, &empty).unwrap();
        assert!(profile.top_tracks.is_empty());
        assert!(profile.genres.is_empty());
        assert_eq!(profile.followers, 0);
        assert_eq!(profile.image_url, None);
        // Popularidad fuera de rango: se recorta a 100.
        assert_eq!(profile.popularity, 100);
        // Nada de artistas en el JSON de búsqueda.
        assert_eq!(parse_artist_search(&empty), None);
        assert_eq!(parse_deezer_artist_search(&empty, "x"), None);
    }

    #[test]
    fn artist_top_tracks_are_capped_at_ten() {
        // Mismo formato para los dos proveedores: Spotify llama `name` al
        // título y Deezer `title`.
        let items = |key: &str| {
            (0..12)
                .map(move |i| {
                    format!(
                        r#"{{"id":"t{i}","{key}":"T{i}","duration":1,"duration_ms":1000,
                         "artists":[{{"name":"A"}}],"album":{{"name":"B"}}}}"#
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let spotify: Value =
            serde_json::from_str(&format!(r#"{{"tracks":[{}]}}"#, items("name"))).unwrap();
        assert_eq!(parse_top_tracks(&spotify).len(), MAX_ARTIST_TRACKS);
        let deezer: Value =
            serde_json::from_str(&format!(r#"{{"data":[{}]}}"#, items("title"))).unwrap();
        assert_eq!(parse_deezer_top_tracks(&deezer).len(), MAX_ARTIST_TRACKS);
    }

    #[test]
    fn artist_rejects_an_empty_name() {
        // Sin red: el nombre vacío corta antes de cualquier petición.
        assert!(artist("   ").is_err());
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

    #[test]
    fn parses_a_mixed_spotify_search() {
        let body: Value = serde_json::from_str(
            r#"{
              "tracks": {"items": [
                {"id": "4cOdK2wGLETKBW3PvgPWqT", "name": "Borderline",
                 "duration_ms": 237000, "artists": [{"name": "Tame Impala"}],
                 "album": {"name": "The Slow Rush", "images": [
                   {"url": "t-64.jpg", "width": 64},
                   {"url": "t-640.jpg", "width": 640}]},
                 "external_ids": {"isrc": "AUUM71900931"}}
              ]},
              "albums": {"items": [
                {"id": "5m8RcYFZq0h1h6jVtGmIuX", "name": "The Slow Rush",
                 "release_date": "2020-02-14", "total_tracks": 12,
                 "artists": [{"name": "Tame Impala"}],
                 "images": [{"url": "a-300.jpg", "width": 300},
                            {"url": "a-640.jpg", "width": 640}]}
              ]},
              "artists": {"items": [
                {"id": "5INjqkS1o8h1imAzPqGZBb", "name": "Tame Impala",
                 "followers": {"total": 5654321},
                 "images": [{"url": "ar-160.jpg", "width": 160},
                            {"url": "ar-640.jpg", "width": 640}]},
                {"id": "", "name": "Sin id"}
              ]}
            }"#,
        )
        .unwrap();
        let all = parse_spotify_search_all(&body);
        assert_eq!(all.provider, "spotify");
        assert!(!all.is_empty());
        assert_eq!(all.tracks.len(), 1);
        assert_eq!(all.tracks[0].title, "Borderline");
        assert_eq!(all.albums.len(), 1);
        let album = &all.albums[0];
        assert_eq!(album.provider, "spotify");
        assert_eq!(album.id, "5m8RcYFZq0h1h6jVtGmIuX");
        assert_eq!(album.title, "The Slow Rush");
        assert_eq!(album.artist, "Tame Impala");
        assert_eq!(album.cover_url.as_deref(), Some("a-640.jpg"));
        assert_eq!(album.release_year, Some(2020));
        assert_eq!(album.track_count, 12);
        // El artista sin id se cae.
        assert_eq!(all.artists.len(), 1);
        let artist = &all.artists[0];
        assert_eq!(artist.provider, "spotify");
        assert_eq!(artist.id, "5INjqkS1o8h1imAzPqGZBb");
        assert_eq!(artist.name, "Tame Impala");
        assert_eq!(artist.image_url.as_deref(), Some("ar-640.jpg"));
        assert_eq!(artist.followers, 5_654_321);

        // Payload vacío: ni error ni resultados inventados.
        let empty: Value = serde_json::from_str(
            r#"{"tracks":{"items":[]},"albums":{"items":[]},"artists":{"items":[]}}"#,
        )
        .unwrap();
        let none = parse_spotify_search_all(&empty);
        assert_eq!(none.provider, "spotify");
        assert!(none.is_empty());
        // Sin consulta no hay red ni proveedor.
        let blank = search_all("   ", 10).unwrap();
        assert!(blank.is_empty());
        assert_eq!(blank.provider, "");
    }

    #[test]
    fn parses_a_mixed_deezer_search() {
        let body: Value = serde_json::from_str(
            r#"{"data": [
              {"id": 872345282, "title": "Borderline", "duration": 238,
               "isrc": "AUUM71900956", "type": "track",
               "artist": {"name": "Tame Impala"},
               "album": {"title": "The Slow Rush",
                         "cover_xl": "https://cdn/500x500.jpg"}},
              {"id": 1188985, "title": "The Slow Rush", "nb_tracks": 12,
               "release_date": "2020-02-14",
               "cover_xl": "https://cdn/album/1000x1000.jpg",
               "artist": {"name": "Tame Impala"}},
              {"id": 13383903, "name": "Tame Impala", "nb_fan": 1846233,
               "picture_xl": "https://cdn/artist/1000x1000.jpg", "type": "artist"},
              {"id": 99, "title": "Una playlist", "type": "playlist"}
            ]}"#,
        )
        .unwrap();
        let all = parse_deezer_search_all(&body);
        assert_eq!(all.provider, "deezer");
        assert_eq!(all.tracks.len(), 1);
        assert_eq!(all.tracks[0].id, "dz:872345282");
        assert_eq!(all.tracks[0].provider, "deezer");
        assert_eq!(
            all.tracks[0].art_url.as_deref(),
            Some("https://cdn/1000x1000.jpg")
        );
        // El álbum venía sin `type`: se deduce por `nb_tracks`.
        assert_eq!(all.albums.len(), 1);
        let album = &all.albums[0];
        assert_eq!(album.provider, "deezer");
        assert_eq!(album.id, "1188985");
        assert_eq!(album.title, "The Slow Rush");
        assert_eq!(album.artist, "Tame Impala");
        assert_eq!(
            album.cover_url.as_deref(),
            Some("https://cdn/album/1000x1000.jpg")
        );
        assert_eq!(album.release_year, Some(2020));
        assert_eq!(album.track_count, 12);
        assert_eq!(all.artists.len(), 1);
        let artist = &all.artists[0];
        assert_eq!(artist.provider, "deezer");
        assert_eq!(artist.id, "13383903");
        assert_eq!(artist.name, "Tame Impala");
        assert_eq!(
            artist.image_url.as_deref(),
            Some("https://cdn/artist/1000x1000.jpg")
        );
        assert_eq!(artist.followers, 1_846_233);

        let empty: Value = serde_json::from_str(r#"{"data": []}"#).unwrap();
        assert!(parse_deezer_search_all(&empty).is_empty());
    }

    #[test]
    fn parses_a_spotify_album() {
        let body: Value = serde_json::from_str(
            r#"{
              "album_type": "album", "id": "5m8RcYFZq0h1h6jVtGmIuX",
              "name": "The Slow Rush", "release_date": "2020-02-14",
              "total_tracks": 2, "artists": [{"name": "Tame Impala"}],
              "images": [{"url": "cover-640.jpg", "width": 640}],
              "tracks": {"items": [
                {"id": "4cOdK2wGLETKBW3PvgPWqT", "name": "Borderline",
                 "duration_ms": 237000, "artists": [{"name": "Tame Impala"}],
                 "external_ids": {"isrc": "AUUM71900931"}},
                {"id": "7oK9VyNzrYvRFo7nQEYkWN", "name": "One More Year",
                 "duration_ms": 322000, "artists": [{"name": "Tame Impala"}]},
                {"id": null, "name": "Fantasma"}
              ]}
            }"#,
        )
        .unwrap();
        let album = parse_spotify_album(&body, "fallback").unwrap();
        assert_eq!(album.provider, "spotify");
        assert_eq!(album.id, "5m8RcYFZq0h1h6jVtGmIuX");
        assert_eq!(album.title, "The Slow Rush");
        assert_eq!(album.artist, "Tame Impala");
        assert_eq!(album.cover_url.as_deref(), Some("cover-640.jpg"));
        assert_eq!(album.release_date.as_deref(), Some("2020-02-14"));
        // La pista sin id se cae.
        assert_eq!(album.tracks.len(), 2);
        assert_eq!(album.tracks[0].provider, "spotify");
        assert_eq!(album.tracks[0].title, "Borderline");
        assert_eq!(album.tracks[0].album, "The Slow Rush");
        assert_eq!(album.tracks[0].duration_ms, 237_000);
        assert_eq!(album.tracks[0].art_url.as_deref(), Some("cover-640.jpg"));
        assert_eq!(album.tracks[0].isrc.as_deref(), Some("AUUM71900931"));
        assert_eq!(album.tracks[1].album, "The Slow Rush");
        assert_eq!(album.tracks[1].isrc, None);

        // Sin nombre no hay ficha.
        let nameless: Value = serde_json::from_str(r#"{"id": "x"}"#).unwrap();
        assert!(parse_spotify_album(&nameless, "x").is_none());
    }

    #[test]
    fn parses_a_deezer_album() {
        let body: Value = serde_json::from_str(
            r#"{
              "id": 1188985, "title": "The Slow Rush",
              "release_date": "2020-02-14", "nb_tracks": 2,
              "cover": "https://cdn/album/250.jpg",
              "cover_big": "https://cdn/album/500.jpg",
              "cover_xl": "https://cdn/album/1000x1000.jpg",
              "artist": {"id": 13383903, "name": "Tame Impala"},
              "tracks": {"data": [
                {"id": 872345282, "title": "Borderline", "duration": 238,
                 "artist": {"name": "Tame Impala"}},
                {"id": 872345283, "title": "One More Year", "duration": 322,
                 "artist": {"name": "Tame Impala"}}
              ]}
            }"#,
        )
        .unwrap();
        let album = parse_deezer_album(&body, "fallback").unwrap();
        assert_eq!(album.provider, "deezer");
        assert_eq!(album.id, "1188985");
        assert_eq!(album.title, "The Slow Rush");
        assert_eq!(album.artist, "Tame Impala");
        assert_eq!(
            album.cover_url.as_deref(),
            Some("https://cdn/album/1000x1000.jpg")
        );
        assert_eq!(album.release_date.as_deref(), Some("2020-02-14"));
        assert_eq!(album.tracks.len(), 2);
        assert_eq!(album.tracks[0].provider, "deezer");
        assert_eq!(album.tracks[0].id, "dz:872345282");
        assert_eq!(album.tracks[0].duration_ms, 238_000);
        assert_eq!(album.tracks[0].album, "The Slow Rush");
        assert_eq!(
            album.tracks[0].art_url.as_deref(),
            Some("https://cdn/album/1000x1000.jpg")
        );

        // Sin id en el cuerpo se usa el que pidió el llamante.
        let no_id: Value =
            serde_json::from_str(r#"{"title": "Sin id", "artist": {"name": "A"}}"#).unwrap();
        assert_eq!(parse_deezer_album(&no_id, "42").unwrap().id, "42");
        // Ni título ni portada: ficha vacía pero válida.
        let bare: Value = serde_json::from_str(r#"{"id": 7, "title": "Bare"}"#).unwrap();
        let bare = parse_deezer_album(&bare, "7").unwrap();
        assert_eq!(bare.cover_url, None);
        assert_eq!(bare.artist, "");
        assert!(bare.tracks.is_empty());
    }

    #[test]
    fn parses_spotify_artist_albums() {
        let body: Value = serde_json::from_str(
            r#"{"items": [
              {"id": "5m8RcYFZq0h1h6jVtGmIuX", "name": "The Slow Rush",
               "release_date": "2020-02-14", "total_tracks": 12,
               "artists": [{"name": "Tame Impala"}],
               "images": [{"url": "a-300.jpg", "width": 300},
                          {"url": "a-640.jpg", "width": 640}]},
              {"id": "3m2qLdxrtGHKZ3pVDgkjq1", "name": "Currents",
               "release_date": "2015-07-17", "total_tracks": 13,
               "artists": [{"name": "Tame Impala"}], "images": []},
              {"id": null, "name": "Roto"}
            ]}"#,
        )
        .unwrap();
        let albums = parse_spotify_albums(&body);
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].provider, "spotify");
        assert_eq!(albums[0].cover_url.as_deref(), Some("a-640.jpg"));
        assert_eq!(albums[0].release_year, Some(2020));
        assert_eq!(albums[0].track_count, 12);
        assert_eq!(albums[1].title, "Currents");
        assert_eq!(albums[1].cover_url, None);
        assert_eq!(albums[1].release_year, Some(2015));
        assert_eq!(albums[1].track_count, 13);
    }

    #[test]
    fn parses_deezer_artist_albums() {
        let body: Value = serde_json::from_str(
            r#"{"data": [
              {"id": 1188985, "title": "The Slow Rush", "nb_tracks": 12,
               "release_date": "2020-02-14",
               "cover_xl": "https://cdn/album/1000x1000.jpg",
               "artist": {"name": "Tame Impala"}},
              {"id": 96735432, "title": "Currents", "nb_tracks": 13,
               "cover_big": "https://cdn/album/c.jpg"}
            ], "total": 2}"#,
        )
        .unwrap();
        let mut albums = parse_deezer_albums(&body);
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].provider, "deezer");
        assert_eq!(albums[0].id, "1188985");
        assert_eq!(albums[0].artist, "Tame Impala");
        assert_eq!(albums[0].release_year, Some(2020));
        assert_eq!(albums[0].track_count, 12);
        // El listado a veces omite el artista: se rellena desde el perfil.
        assert_eq!(albums[1].artist, "");
        assert_eq!(
            albums[1].cover_url.as_deref(),
            Some("https://cdn/album/c.jpg")
        );
        assert_eq!(albums[1].release_year, None);
        fill_artist(&mut albums, "Tame Impala");
        assert_eq!(albums[1].artist, "Tame Impala");
        // Un ítem que no es álbum no cuenta.
        let junk: Value = serde_json::from_str(r#"{"data": [{"title": "Sin id"}]}"#).unwrap();
        assert!(parse_deezer_albums(&junk).is_empty());
    }

    #[test]
    fn artist_profile_serializes_albums() {
        let detail: Value = serde_json::from_str(
            r#"{"id": "5INjqkS1o8h1imAzPqGZBb", "name": "Tame Impala",
                "followers": {"total": 5654321}, "popularity": 78}"#,
        )
        .unwrap();
        let top: Value = serde_json::from_str(r#"{"tracks": []}"#).unwrap();
        let albums: Value = serde_json::from_str(
            r#"{"items": [{"id": "5m8RcYFZq0h1h6jVtGmIuX", "name": "The Slow Rush",
                             "release_date": "2020-02-14", "total_tracks": 12,
                             "artists": [{"name": "Tame Impala"}]}]}"#,
        )
        .unwrap();
        let mut profile = parse_spotify_artist(&detail, &top).unwrap();
        assert!(profile.albums.is_empty());
        profile.albums = parse_spotify_albums(&albums);
        assert_eq!(profile.albums.len(), 1);

        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"albums\""), "{json}");
        let back: ArtistProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
        assert_eq!(back.albums[0].title, "The Slow Rush");

        // Un perfil viejo (sin `albums`) sigue deserializando.
        let legacy = r#"{"provider":"spotify","id":"x","name":"X",
            "image_url":null,"genres":[],"followers":0,"popularity":0,
            "top_tracks":[],"bio":null}"#;
        let old: ArtistProfile = serde_json::from_str(legacy).unwrap();
        assert!(old.albums.is_empty());
    }

    #[test]
    fn provider_chain_prefers_the_right_catalogue() {
        let spotify_id = "5INjqkS1o8h1imAzPqGZBb";
        assert_eq!(
            provider_chain("deezer", "13383903"),
            [Some(Provider::Deezer), None]
        );
        assert_eq!(
            provider_chain("spotify", spotify_id),
            [Some(Provider::Spotify), Some(Provider::Deezer)]
        );
        // Sin proveedor manda la forma del id.
        assert_eq!(
            provider_chain("", "13383903"),
            [Some(Provider::Deezer), Some(Provider::Spotify)]
        );
        assert_eq!(
            provider_chain("", spotify_id),
            [Some(Provider::Spotify), Some(Provider::Deezer)]
        );
        assert_eq!(clean_id(" dz:42 "), "42");
        assert_eq!(clean_id("  42 "), "42");
        assert_eq!(clean_id(spotify_id), spotify_id);
        assert!(is_numeric_id("13383903"));
        assert!(!is_numeric_id(spotify_id));
        assert!(!is_numeric_id(""));
        assert_eq!(provider_of(" Spotify "), Some(Provider::Spotify));
        assert_eq!(provider_of("dz"), Some(Provider::Deezer));
        assert_eq!(provider_of(""), None);
        // `release_year` no inventa años con fechas raras.
        assert_eq!(release_year(Some("2020-02-14")), Some(2020));
        assert_eq!(release_year(Some("2020")), Some(2020));
        assert_eq!(release_year(Some("20")), None);
        assert_eq!(release_year(Some("aaaa")), None);
        assert_eq!(release_year(None), None);
    }

    #[test]
    fn id_entry_points_reject_empty_ids() {
        // Sin id no hay red: todo corta antes.
        assert!(album("spotify", "   ").is_err());
        assert!(album("deezer", "").is_err());
        assert!(albums_of("deezer", " dz:  ").is_err());
        assert!(artist_by_id("spotify", "").is_err());
    }
}
