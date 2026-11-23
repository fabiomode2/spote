//! Catálogo público de Spotify sin autenticación.
//!
//! Lee las páginas `/embed/{tipo}/{id}` que sirve el reproductor web: son
//! públicas, no piden cuenta ni token, y llevan un `__NEXT_DATA__` con la
//! lista de pistas (título, artista, duración, portada). Es la vía que usa la
//! app para importar playlists sin depender de la API oficial (que exige
//! Premium para crear una app) ni de la cookie `sp_dc`.
//!
//! Límites conocidos: el embed de playlist devuelve como mucho 50 pistas y
//! ignora `?offset=`. Los álbumes entran completos. El de artista da nombre,
//! imagen y **10 temas destacados**, pero no la discografía: para listar
//! álbumes haría falta token (la API oficial exige Premium para crear la app y
//! la página normal del artista se sirve vacía, también con UA de bot).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use spote_protocol::TrackRef;

use crate::downloader::SearchItem;
use crate::util;

const EMBED_BASE: &str = "https://open.spotify.com/embed";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/146.0 Safari/537.36";

/// Una pista tal y como la describe el catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogTrack {
    /// ID de Spotify (22 chars), sin prefijo.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub playable: bool,
}

/// Una colección (playlist, álbum o pista suelta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// `playlist`, `album` o `track`.
    pub kind: String,
    pub id: String,
    pub name: String,
    pub owner: Option<String>,
    pub cover: Option<String>,
    /// Solo en álbumes: nombre del álbum, para etiquetar las pistas.
    pub album: Option<String>,
    pub tracks: Vec<CatalogTrack>,
}

impl Catalog {
    /// La portada de la colección solo se hereda en álbumes: ahí todas las
    /// pistas comparten carátula de verdad. En listas y artistas cada tema
    /// busca la suya, o acabarían todas con la misma imagen.
    pub fn to_track_refs(&self) -> Vec<TrackRef> {
        let fallback_art = (self.kind == "album").then(|| self.cover.clone()).flatten();
        self.tracks
            .iter()
            .map(|track| TrackRef {
                key: format!("spotify:track:{}", track.id),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: self.album.clone(),
                duration_ms: track.duration_ms,
                art_url: fallback_art.clone(),
                source_hint: Some(spote_protocol::SourceHint {
                    provider: "spotify".to_string(),
                    id: track.id.clone(),
                    duration_ms: track.duration_ms,
                }),
            })
            .collect()
    }
}

/// Acepta `https://open.spotify.com/playlist/<id>`, con prefijo `/intl-es/`,
/// con query string, o el URI `spotify:playlist:<id>`.
pub fn parse_url(input: &str) -> Result<(&'static str, String)> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(anyhow!("URL vacía"));
    }

    let mut parts: Vec<&str> = if let Some(rest) = raw.strip_prefix("spotify:") {
        rest.split(':').collect()
    } else {
        // Corta esquema y host, deja el path.
        let without_scheme = raw.split_once("://").map(|(_, rest)| rest).unwrap_or(raw);
        let path = without_scheme
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or("");
        let path = path.split(['?', '#']).next().unwrap_or(path);
        path.split('/').filter(|s| !s.is_empty()).collect()
    };
    // `open.spotify.com/intl-es/playlist/<id>`
    if parts.first().is_some_and(|first| first.starts_with("intl-")) {
        parts.remove(0);
    }

    let kind = parts
        .iter()
        .position(|part| matches!(*part, "playlist" | "album" | "track" | "artist"))
        .ok_or_else(|| anyhow!("no veo playlist/album/track/artist en «{input}»"))?;
    let kind_str = match parts[kind] {
        "playlist" => "playlist",
        "album" => "album",
        "track" => "track",
        "artist" => "artist",
        _ => unreachable!(),
    };
    let id = parts
        .get(kind + 1)
        .map(|value| value.trim().to_string())
        .filter(|value| is_spotify_id(value))
        .ok_or_else(|| anyhow!("ID de Spotify inválido en «{input}»"))?;
    Ok((kind_str, id))
}

fn is_spotify_id(value: &str) -> bool {
    (16..=32).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Descarga y parsea una colección pública. Bloquea: llámalo desde un hilo.
pub fn fetch(input: &str) -> Result<Catalog> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .context("no se pudo crear el cliente HTTP")?;
    fetch_with(&client, input)
}

pub fn fetch_with(client: &reqwest::blocking::Client, input: &str) -> Result<Catalog> {
    let (kind, id) = parse_url(input)?;
    let url = format!("{EMBED_BASE}/{kind}/{id}");
    let body = client
        .get(&url)
        .send()
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("Spotify respondió con error para {url}"))?
        .text()
        .context("respuesta ilegible")?;
    parse_page(&body).with_context(|| format!("no pude interpretar {url}"))
}

/// Extrae el `__NEXT_DATA__` y lo convierte en `Catalog`. Puro y testeable.
pub fn parse_page(html: &str) -> Result<Catalog> {
    let marker = "id=\"__NEXT_DATA__\"";
    let start = html
        .find(marker)
        .ok_or_else(|| anyhow!("la página no trae __NEXT_DATA__"))?;
    let after = &html[start..];
    let open = after
        .find('>')
        .ok_or_else(|| anyhow!("__NEXT_DATA__ malformado"))?;
    let rest = &after[open + 1..];
    let close = rest
        .find("</script>")
        .ok_or_else(|| anyhow!("__NEXT_DATA__ sin cierre"))?;
    let json: Value = serde_json::from_str(&rest[..close]).context("__NEXT_DATA__ no es JSON")?;

    let entity = json
        .pointer("/props/pageProps/state/data/entity")
        .ok_or_else(|| anyhow!("la página no trae entidad"))?;

    let kind = entity
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("playlist")
        .to_string();
    let id = entity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = entity
        .get("name")
        .or_else(|| entity.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Sin título")
        .to_string();
    let owner = entity
        .get("subtitle")
        .and_then(Value::as_str)
        .map(str::to_string);
    // Playlist/álbum: `coverArt`. Artista: la imagen va en `visualIdentity.image`.
    let cover = entity
        .pointer("/coverArt/sources/0/url")
        .or_else(|| entity.pointer("/visualIdentity/image/0/url"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut tracks = Vec::new();
    if kind == "track" {
        if let Some(track) = single_track(entity) {
            tracks.push(track);
        }
    } else if let Some(list) = entity.get("trackList").and_then(Value::as_array) {
        for item in list {
            let Some(uri) = item.get("uri").and_then(Value::as_str) else {
                continue;
            };
            let Some(track_id) = uri.rsplit(':').next().filter(|v| is_spotify_id(v)) else {
                continue;
            };
            tracks.push(CatalogTrack {
                id: track_id.to_string(),
                title: item
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Sin título")
                    .to_string(),
                artist: item
                    .get("subtitle")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                duration_ms: item.get("duration").and_then(Value::as_u64).unwrap_or(0),
                playable: item
                    .get("isPlayable")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            });
        }
    }

    if tracks.is_empty() {
        return Err(anyhow!("la colección no trae pistas"));
    }

    Ok(Catalog {
        album: (kind == "album").then(|| name.clone()),
        kind,
        id,
        name,
        owner,
        cover,
        tracks,
    })
}

fn single_track(entity: &Value) -> Option<CatalogTrack> {
    let id = entity
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| is_spotify_id(value))?
        .to_string();
    let artist = entity
        .pointer("/artists/0/name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| entity.get("subtitle").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    Some(CatalogTrack {
        id,
        title: entity
            .get("name")
            .or_else(|| entity.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("Sin título")
            .to_string(),
        artist,
        duration_ms: entity.get("duration").and_then(Value::as_u64).unwrap_or(0),
        playable: entity
            .get("isPlayable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

// ---------------------------------------------------------------------------
// Matching contra resultados de yt-dlp
// ---------------------------------------------------------------------------

/// Elige la mejor fuente para una pista del catálogo, o `None` si ninguna
/// llega al mínimo. Preferimos no bajar nada antes que bajar un cover o un
/// vídeo con intro: un mal match es peor que un hueco.
pub fn pick_best<'a>(
    items: &'a [SearchItem],
    artist: &str,
    title: &str,
    duration_ms: u64,
) -> Option<&'a SearchItem> {
    let want_title = normalize(title);
    let want_artist = normalize(artist);

    let mut best: Option<(&SearchItem, i32)> = None;
    for item in items {
        let raw_title = normalize(&item.title);
        let got_artist = normalize(&format!("{} {}", item.artist, item.channel));
        // yt-dlp a veces deja "Artista - Tema" en el título aunque el artista
        // ya venga aparte; se compara contra la parte que es el tema. Hay que
        // separar antes de normalizar porque `normalize` convierte el guion en
        // espacio.
        let got_title = if let Some((left, right)) = item.title.split_once(" - ") {
            let left = normalize(left);
            let right = normalize(right);
            if !want_artist.is_empty() && left.contains(&want_artist) && !right.is_empty() {
                right
            } else {
                raw_title.clone()
            }
        } else {
            raw_title.clone()
        };

        let exact_title = !want_title.is_empty() && got_title == want_title;
        let loose_title = !want_title.is_empty()
            && (got_title.contains(&want_title) || want_title.contains(&got_title));
        // Un título parecido no basta: antes se colaba otra canción del mismo
        // artista porque la duración cuadraba. El título es obligatorio.
        if !exact_title && !loose_title {
            continue;
        }

        let mut score = if exact_title { 60 } else { 25 };

        if !want_artist.is_empty() {
            let artist_match =
                got_artist.contains(&want_artist) || want_artist.contains(&got_artist);
            if artist_match {
                score += 30;
            } else if exact_title && !got_artist.trim().is_empty() {
                // Mismo título pero otro artista: casi seguro un cover, una
                // versión de otro canal o una canción homónima. No se baja.
                continue;
            } else {
                score -= 10;
            }
        }

        if item.channel.ends_with(" - Topic") {
            score += 15;
        }

        // Variantes que no son el tema del catálogo: live, remix, cover…
        // Solo penaliza si la pista buscada no es ya de ese tipo.
        let raw_title = crate::util::normalize(&item.title);
        let wanted_lower = crate::util::normalize(title);
        for noise in [
            "live",
            "remix",
            "cover",
            "karaoke",
            "instrumental",
            "8d",
            "sped up",
            "nightcore",
            "acoustic",
            "reaction",
            "tutorial",
            "extended",
            "1 hour",
        ] {
            if contains_word(&raw_title, noise) && !contains_word(&wanted_lower, noise) {
                score -= 25;
            }
        }

        if duration_ms > 0 && item.duration_ms > 0 {
            let diff = duration_ms.abs_diff(item.duration_ms);
            // Un vídeo oficial puede ir 10-20 s por delante del catálogo por
            // intro/outro. Con título exacto se tolera, pero se penaliza; sin
            // título exacto no se acepta una diferencia grande.
            score += match diff {
                0..=3_000 => 40,
                3_001..=8_000 => 20,
                8_001..=15_000 => if exact_title { 0 } else { -20 },
                15_001..=30_000 => if exact_title { -25 } else { -60 },
                _ => -100,
            };
            let limit = if exact_title { 30_000 } else { 10_000 };
            if diff > limit {
                continue;
            }
        }

        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((item, score));
        }
    }

    best.filter(|(_, score)| *score >= 60).map(|(item, _)| item)
}

/// Minúsculas, sin acentos, sin paréntesis ni "feat.", espacios colapsados.
fn normalize(text: &str) -> String {
    // Misma normalización que el resto de spote: conserva el núcleo de
    // títulos simbólicos como "(_8(|)" y corta colaboraciones.
    crate::util::normalize(text)
}

/// Nombres de fichero seguros para la colección.
pub fn playlist_slug(name: &str) -> String {
    let slug = util::slug(name, 60);
    if slug.is_empty() {
        "playlist".to_string()
    } else {
        slug
    }
}

/// Palabra/frase completa: "live" no debe activarse dentro de "deliver" ni
/// "cover" dentro de "discover".
fn contains_word(text: &str, phrase: &str) -> bool {
    if text.is_empty() || phrase.is_empty() {
        return false;
    }
    format!(" {text} ").contains(&format!(" {phrase} "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><body><script id="__NEXT_DATA__" type="application/json">
    {"props":{"pageProps":{"state":{"data":{"entity":{
      "type":"playlist","name":"Prueba","subtitle":"alguien",
      "coverArt":{"sources":[{"url":"https://i.scdn.co/image/abc"}]},
      "trackList":[
        {"uri":"spotify:track:3zOYmiNOuyDL0ClxJcEJ0T","title":"Stop The Wedding!","subtitle":"Ashe","duration":198840,"isPlayable":true},
        {"uri":"spotify:track:2yg9UN4eo5eMVJ7OB4RWj3","title":"Survival","subtitle":"Drake","duration":136186,"isPlayable":true}
      ]}}}}}}</script></body></html>"#;

    fn item(title: &str, artist: &str, channel: &str, duration_ms: u64) -> SearchItem {
        SearchItem {
            provider_id: "x".into(),
            provider: "spotify".into(),
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms,
            channel: channel.into(),
            url: "https://youtu.be/x".into(),
            art_url: None,
            isrc: None,
        }
    }

    #[test]
    fn parsea_urls_de_todos_los_formatos() {
        for (input, kind, id) in [
            ("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc", "playlist", "37i9dQZF1DXcBWIGoYBM5M"),
            ("https://open.spotify.com/intl-es/album/1ATL5GLyefJaxhQzSPVrLX", "album", "1ATL5GLyefJaxhQzSPVrLX"),
            ("spotify:track:3zOYmiNOuyDL0ClxJcEJ0T", "track", "3zOYmiNOuyDL0ClxJcEJ0T"),
            ("https://open.spotify.com/artist/3kjuyTCjPG1WMFCiyc5IuB", "artist", "3kjuyTCjPG1WMFCiyc5IuB"),
            ("spotify:artist:3kjuyTCjPG1WMFCiyc5IuB", "artist", "3kjuyTCjPG1WMFCiyc5IuB"),
            ("open.spotify.com/track/3zOYmiNOuyDL0ClxJcEJ0T", "track", "3zOYmiNOuyDL0ClxJcEJ0T"),
        ] {
            let (got_kind, got_id) = parse_url(input).expect(input);
            assert_eq!((got_kind, got_id.as_str()), (kind, id), "{input}");
        }
        assert!(parse_url("https://example.com/hola").is_err());
        assert!(parse_url("https://open.spotify.com/playlist/corto").is_err());
    }

    #[test]
    fn parsea_la_pagina_embed() {
        let catalog = parse_page(PAGE).expect("parsea");
        assert_eq!(catalog.kind, "playlist");
        assert_eq!(catalog.name, "Prueba");
        assert_eq!(catalog.owner.as_deref(), Some("alguien"));
        assert_eq!(catalog.cover.as_deref(), Some("https://i.scdn.co/image/abc"));
        assert_eq!(catalog.tracks.len(), 2);
        assert_eq!(catalog.tracks[0].id, "3zOYmiNOuyDL0ClxJcEJ0T");
        assert_eq!(catalog.tracks[0].artist, "Ashe");
        assert_eq!(catalog.tracks[0].duration_ms, 198_840);

        let refs = catalog.to_track_refs();
        assert_eq!(refs[0].key, "spotify:track:3zOYmiNOuyDL0ClxJcEJ0T");
        // En una lista, la portada de la colección no es la de cada pista.
        assert!(refs[0].art_url.is_none());
        assert_eq!(
            refs[0].source_hint.as_ref().map(|hint| hint.id.as_str()),
            Some("3zOYmiNOuyDL0ClxJcEJ0T")
        );
        assert_eq!(refs[0].title, "Stop The Wedding!");
    }

    const ARTIST_PAGE: &str = r#"<html><script id="__NEXT_DATA__" type="application/json">
    {"props":{"pageProps":{"state":{"data":{"entity":{
      "type":"artist","name":"Arcade Fire","id":"3kjuyTCjPG1WMFCiyc5IuB","subtitle":"Top tracks",
      "visualIdentity":{"image":[{"url":"https://i.scdn.co/image/artista"}]},
      "trackList":[
        {"uri":"spotify:track:5iItYl3Q6wCnKVfpK1uNVf","title":"The Suburbs","subtitle":"Arcade Fire","duration":315026,"isPlayable":true}
      ]}}}}}}</script></html>"#;

    #[test]
    fn parsea_perfil_de_artista() {
        let catalog = parse_page(ARTIST_PAGE).expect("parsea");
        assert_eq!(catalog.kind, "artist");
        assert_eq!(catalog.name, "Arcade Fire");
        assert_eq!(
            catalog.cover.as_deref(),
            Some("https://i.scdn.co/image/artista")
        );
        assert_eq!(catalog.tracks.len(), 1);
        assert_eq!(catalog.tracks[0].title, "The Suburbs");
        assert_eq!(catalog.tracks[0].artist, "Arcade Fire");
        let refs = catalog.to_track_refs();
        assert_eq!(refs[0].key, "spotify:track:5iItYl3Q6wCnKVfpK1uNVf");
        assert!(refs[0].album.is_none());
        // La foto del artista no es la portada de sus canciones.
        assert!(refs[0].art_url.is_none());
    }

    #[test]
    fn rechaza_paginas_sin_entidad() {
        assert!(parse_page("<html></html>").is_err());
    }

    #[test]
    fn prefiere_topic_y_duracion() {
        let items = vec![
            item("Stop The Wedding! (Official Video)", "Ashe", "Ashe", 250_000),
            item("Stop The Wedding!", "Ashe", "Ashe - Topic", 198_840),
            item("Stop The Wedding! (8D AUDIO)", "Ashe", "8D Tunes", 200_000),
        ];
        let best = pick_best(&items, "Ashe", "Stop The Wedding!", 198_840).expect("hay match");
        assert_eq!(best.channel, "Ashe - Topic");
    }

    #[test]
    fn no_cae_en_otra_cancion_del_mismo_artista() {
        // Guxo tiene varias canciones de ~3 min. Antes se elegía "El Precio del
        // Amor" porque su duración encajaba mejor que el vídeo oficial largo.
        let items = vec![
            item("Guxo - El Precio del Amor [Official Music Video]", "Guxo", "Guxo", 178_000),
            item("Guxo - Kariñosa (Official Video)", "Guxo", "Guxo", 188_000),
        ];
        let best = pick_best(&items, "Guxo", "Kariñosa", 176_062).expect("hay match");
        assert!(best.title.contains("Kariñosa"), "{}", best.title);
    }

    #[test]
    fn descarta_duraciones_muy_distintas() {
        let items = vec![item("Cancion", "Artista", "Artista - Topic", 400_000)];
        assert!(pick_best(&items, "Artista", "Cancion", 180_000).is_none());
    }

    #[test]
    fn noise_detection_uses_whole_words() {
        assert!(!contains_word("deliver", "live"));
        assert!(!contains_word("alive", "live"));
        assert!(!contains_word("discover", "cover"));
        assert!(contains_word("song live version", "live"));
        assert!(contains_word("undercover", "cover") == false);
    }

    #[test]
    fn normaliza_titulos() {
        assert_eq!(normalize("Canción (Official Video) feat. Otro"), "cancion");
        assert_eq!(normalize("ARTISTA & OTRO"), "artista and otro");
    }
}
