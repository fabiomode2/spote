//! Limpieza de metadatos y similitud de textos para el matching de letras.
//!
//! Todo es puro y testeable sin red: el matching remoto solo decide con estas
//! funciones qué candidato de LRCLIB se parece más a la pista pedida. Los
//! títulos que llegan de yt-dlp suelen venir sucios ("… (Official Video)",
//! "Artista - Tema | Canal"), así que limpiar bien es lo que más aciertos da.

use std::collections::HashSet;

use crate::util::normalize;

// ---------------------------------------------------------------------------
// Variante de la canción
// ---------------------------------------------------------------------------

/// Variante de una canción deducida del título y del álbum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VersionKind {
    #[default]
    Studio,
    Live,
    Remix,
    Cover,
    Karaoke,
    SpedUp,
    Slowed,
    Instrumental,
    Acoustic,
    Demo,
}

impl VersionKind {
    /// Variantes cuya letra ya no es la de la versión de estudio: karaoke e
    /// instrumental no la cantan y una versión de otro artista cambia el texto.
    pub fn is_incompatible(self) -> bool {
        matches!(self, Self::Karaoke | Self::Instrumental | Self::Cover)
    }
}

/// Deduce la variante mirando título y álbum. Las palabras se buscan completas
/// para que "alive" no cuente como "live".
pub fn version_kind(title: &str, album: Option<&str>) -> VersionKind {
    let haystack = words_of(&format!("{title} {}", album.unwrap_or_default()).to_lowercase());
    const HINTS: &[(VersionKind, &[&str])] = &[
        (
            VersionKind::Karaoke,
            &["karaoke", "backing track", "sin voz"],
        ),
        (
            VersionKind::Instrumental,
            &["instrumental", "instrumental version", "playback"],
        ),
        (
            VersionKind::SpedUp,
            &["sped up", "speed up", "spedup", "nightcore"],
        ),
        (
            VersionKind::Slowed,
            &["slowed", "slowed reverb", "reverb version"],
        ),
        (
            VersionKind::Remix,
            &["remix", "remixed", "rmx", "bootleg", "mashup", "flip"],
        ),
        (
            VersionKind::Cover,
            &["cover", "tribute", "in the style of", "made famous by"],
        ),
        (
            VersionKind::Live,
            &["live", "en vivo", "ao vivo", "unplugged", "concert"],
        ),
        (
            VersionKind::Acoustic,
            &["acoustic", "stripped", "piano version"],
        ),
        (
            VersionKind::Demo,
            &["demo", "early version", "rough mix", "outtake"],
        ),
    ];
    for (kind, needles) in HINTS {
        if needles.iter().any(|needle| has_phrase(&haystack, needle)) {
            return *kind;
        }
    }
    VersionKind::Studio
}

// ---------------------------------------------------------------------------
// Limpieza de títulos y artistas
// ---------------------------------------------------------------------------

/// Marcadores que delatan que un grupo entre paréntesis es ruido de vídeo o de
/// versión, no parte del título.
const NOISE_WORDS: &[&str] = &[
    "official",
    "oficial",
    "video",
    "audio",
    "lyric",
    "lyrics",
    "letra",
    "visualizer",
    "visualiser",
    "mv",
    "hd",
    "4k",
    "8k",
    "topic",
    "remaster",
    "remastered",
    "deluxe",
    "edition",
    "edicion",
    "explicit",
    "clean",
    "sped up",
    "speed up",
    "slowed",
    "nightcore",
    "karaoke",
    "instrumental",
    "acoustic",
    "live",
    "en vivo",
    "ao vivo",
    "demo",
    "cover",
    "remix",
    "feat",
    "ft",
    "featuring",
    "prod",
    "version",
    "soundtrack",
    "trailer",
    "teaser",
    "subtitulado",
    "traducido",
];

/// Ruido que basta con verlo al principio del sufijo para recortar.
const STRONG_NOISE: &[&str] = &[
    "official",
    "oficial",
    "lyric",
    "lyrics",
    "audio",
    "video",
    "visualizer",
    "visualiser",
    "mv",
    "topic",
    "letra",
    "subtitulado",
    "traducido",
    "remaster",
    "remastered",
    "hd",
    "4k",
    "8k",
    "karaoke",
    "instrumental",
    "sped up",
    "speed up",
    "slowed",
    "nightcore",
];

/// Ruido que solo recorta si todo el sufijo es ruido ("Song - Live Forever" no).
const WEAK_NOISE: &[&str] = &[
    "live", "en vivo", "ao vivo", "remix", "cover", "acoustic", "demo", "version", "edition",
    "deluxe",
];

/// Ruido que, pegado al final y sin separador, también sobra: "Song HD".
const TRAILING_NOISE: &[&str] = &[
    "official video",
    "official music video",
    "official audio",
    "official",
    "oficial",
    "lyric video",
    "lyrics video",
    "lyrics",
    "lyric",
    "letra",
    "audio",
    "video",
    "visualizer",
    "visualiser",
    "hd",
    "4k",
    "8k",
    "mv",
    "remastered",
    "remaster",
    "sped up",
    "slowed",
    "nightcore",
    "karaoke",
    "instrumental",
    "topic",
];

const SEPARATORS: &[&str] = &[" | ", " - ", " – ", " — ", " // ", " • "];

/// Título canónico para buscar y comparar: sin sufijos de vídeo, sin "feat.",
/// sin remaster/deluxe y sin el "Artista" repetido delante.
pub fn clean_title(title: &str) -> String {
    let without_groups = strip_noise_groups(title);
    let without_suffix = cut_noise_suffix(&without_groups);
    let without_collab = cut_collab(&without_suffix);
    tidy(&cut_trailing_noise(&without_collab))
}

/// Recorta ruido pegado al final que no lleva separador ni paréntesis.
fn cut_trailing_noise(text: &str) -> String {
    let mut out = text.to_string();
    loop {
        let lower = out.to_lowercase();
        let mut cut = None;
        for noise in TRAILING_NOISE {
            if !lower.ends_with(noise) {
                continue;
            }
            let index = lower.len() - noise.len();
            // Solo si queda título delante y la palabra empieza limpia.
            if index == 0 || !out.is_char_boundary(index) || lower.len() != out.len() {
                continue;
            }
            let boundary = out[..index].chars().next_back();
            if boundary.is_none_or(|ch| ch.is_whitespace() || ch == '-' || ch == '(' || ch == '[') {
                cut = Some(index);
                break;
            }
        }
        let Some(index) = cut else { return out };
        out = tidy(&out[..index]);
    }
}

/// Actor canónico: sin sufijos de canal de YouTube ("- Topic", "VEVO") ni
/// ruido de vídeo, pero conservando colaboraciones.
pub fn clean_artist(artist: &str) -> String {
    let mut text = strip_noise_groups(artist);
    for suffix in [" - topic", " - official", " - music", " official"] {
        if let Some(index) = find_lower(&text, suffix) {
            if text[index..].trim().eq_ignore_ascii_case(suffix.trim()) {
                text.truncate(index);
            }
        }
    }
    if let Some(index) = find_lower(&text, "vevo") {
        let tail = text[index..].trim();
        if tail.eq_ignore_ascii_case("vevo") && index > 0 {
            text.truncate(index);
        }
    }
    tidy(&text)
}

/// Primer artista de una colaboración: "A, B feat. C" → "A".
pub fn primary_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let mut cut = artist.len();
    for marker in [
        ",",
        " & ",
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " x ",
        " y ",
        " vs. ",
    ] {
        if let Some(index) = lower.find(marker) {
            cut = cut.min(index);
        }
    }
    tidy(&artist[..cut])
}

/// Parte "Artista - Canción" de un título de YouTube. Solo devuelve el corte si
/// el lado izquierdo es corto (los títulos con guion suelen ser "Artista - Tema").
pub fn split_artist_title(title: &str) -> Option<(&str, &str)> {
    for separator in [" - ", " – ", " — "] {
        if let Some(index) = title.find(separator) {
            let left = title[..index].trim();
            let right = title[index + separator.len()..].trim();
            if !left.is_empty()
                && left.chars().count() <= 40
                && right.chars().count() >= 2
                && !left.to_lowercase().contains("feat")
            {
                return Some((left, right));
            }
        }
    }
    None
}

/// Quita los grupos `(…)`, `[…]` y `{…}` que solo contienen ruido de vídeo o de
/// versión. El resto de paréntesis se respeta.
fn strip_noise_groups(text: &str) -> String {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let (start, ch) = chars[index];
        let close = match ch {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => {
                out.push(ch);
                index += 1;
                continue;
            }
        };
        let mut scan = index + 1;
        let mut end = None;
        while scan < chars.len() {
            if chars[scan].1 == close {
                end = Some(chars[scan].0);
                break;
            }
            scan += 1;
        }
        match end {
            Some(end) if is_noise_group(&text[start + ch.len_utf8()..end]) => index = scan + 1,
            _ => {
                out.push(ch);
                index += 1;
            }
        }
    }
    out
}

fn is_noise_group(inner: &str) -> bool {
    let words = words_of(&inner.to_lowercase());
    let tokens = words.split_whitespace().count();
    NOISE_WORDS
        .iter()
        .any(|needle| has_phrase(&words, needle) && tokens <= 8)
}

/// Recorta el título en el primer separador seguido de ruido: "Tema | Official
/// Video" → "Tema".
fn cut_noise_suffix(text: &str) -> String {
    let mut cut = text.len();
    for separator in SEPARATORS {
        let mut from = 0;
        while let Some(found) = text[from..].find(separator) {
            let start = from + found;
            let rest = &text[start + separator.len()..];
            if starts_noisy(rest) {
                cut = cut.min(start);
            }
            from = start + separator.len();
        }
    }
    text[..cut].to_string()
}

fn starts_noisy(rest: &str) -> bool {
    let words = words_of(&rest.to_lowercase());
    if words.trim().is_empty() {
        return false;
    }
    if STRONG_NOISE.iter().any(|needle| has_phrase(&words, needle)) {
        return true;
    }
    // Las marcas débiles solo cuentan si el sufijo entero es ruido.
    let all_noise = words
        .split_whitespace()
        .all(|token| token.chars().all(|ch| ch.is_ascii_digit()) || WEAK_NOISE.contains(&token));
    all_noise && WEAK_NOISE.iter().any(|needle| has_phrase(&words, needle))
}

/// Corta "feat./ft./prod." y la colaboración que arrastran.
fn cut_collab(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut cut = text.len();
    for marker in [
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " prod.",
        " prod ",
    ] {
        if let Some(index) = lower.find(marker) {
            if text.is_char_boundary(index) && lower.len() == text.len() {
                cut = cut.min(index);
            }
        }
    }
    text[..cut].to_string()
}

/// Colapsa espacios y quita separadores y paréntesis huérfanos que deja el recorte.
fn tidy(text: &str) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let before = out.clone();
        out = out.trim().to_string();
        for suffix in [
            "-", "|", "–", "—", "//", "/", ",", ":", ";", "(", "[", "{", "&",
        ] {
            if let Some(stripped) = out.strip_suffix(suffix) {
                out = stripped.trim_end().to_string();
            }
        }
        if out == before {
            return out;
        }
    }
}

// ---------------------------------------------------------------------------
// Similitud
// ---------------------------------------------------------------------------

/// Similitud `0..=1` entre dos títulos (o álbumes), ya limpios.
pub fn similarity(a: &str, b: &str) -> f64 {
    let left = canonical(&clean_title(a));
    let right = canonical(&clean_title(b));
    score_tokens(&left, &right)
}

/// Similitud entre artistas. Una colaboración donde el artista pedido aparece
/// en un lado ("Calvin Harris, Dua Lipa" vs "Dua Lipa") cuenta casi como exacta.
pub fn artist_similarity(a: &str, b: &str) -> f64 {
    let left = canonical(&clean_artist(a));
    let right = canonical(&clean_artist(b));
    let base = score_tokens(&left, &right);
    let left_tokens = keywords(&left);
    let right_tokens = keywords(&right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return base;
    }
    if left_tokens.is_subset(&right_tokens) || right_tokens.is_subset(&left_tokens) {
        return base.max(0.92);
    }
    base
}

fn score_tokens(left: &str, right: &str) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_tokens = keywords(left);
    let right_tokens = keywords(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let common = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count();
    let jaccard = if union == 0 {
        0.0
    } else {
        common as f64 / union as f64
    };
    // Los bigramas dan margen ante erratas y puntuación ("wanna" / "wana").
    (0.65 * jaccard + 0.35 * bigram_dice(left, right)).clamp(0.0, 1.0)
}

fn canonical(text: &str) -> String {
    normalize(text)
}

/// Palabras significativas de un texto ya normalizado (sin colaboraciones).
fn keywords(normalized: &str) -> HashSet<&str> {
    normalized
        .split_whitespace()
        .take_while(|token| !matches!(*token, "feat" | "ft" | "featuring"))
        .collect()
}

/// Coeficiente de Dice sobre bigramas de caracteres, sin espacios.
fn bigram_dice(a: &str, b: &str) -> f64 {
    let left = bigrams(a);
    let right = bigrams(b);
    if left.is_empty() || right.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let common = left.intersection(&right).count();
    2.0 * common as f64 / (left.len() + right.len()) as f64
}

fn bigrams(text: &str) -> HashSet<(char, char)> {
    let chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    chars.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

// ---------------------------------------------------------------------------
// Helpers de texto
// ---------------------------------------------------------------------------

/// Texto en minúsculas rodeado de espacios: permite buscar palabras completas.
fn words_of(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(' ');
    for token in text.split(|ch: char| !ch.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        out.push_str(token);
        out.push(' ');
    }
    out
}

fn has_phrase(words: &str, phrase: &str) -> bool {
    words.contains(&format!(" {phrase} "))
}

/// `find` en minúsculas, solo cuando los índices siguen cuadrando con el
/// original (`to_lowercase` puede cambiar el número de bytes).
fn find_lower(text: &str, needle: &str) -> Option<usize> {
    let lower = text.to_lowercase();
    let index = lower.find(needle)?;
    if lower.len() == text.len() && text.is_char_boundary(index) {
        Some(index)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_strips_video_noise() {
        let cases = [
            ("Do I Wanna Know? (Official Video)", "Do I Wanna Know?"),
            ("Do I Wanna Know?", "Do I Wanna Know?"),
            ("Some Song | Official Music Video", "Some Song"),
            ("Some Song - Lyric Video", "Some Song"),
            ("Some Song [Official Audio]", "Some Song"),
            ("Some Song (Remastered 2011)", "Some Song"),
            ("Some Song (Deluxe Edition)", "Some Song"),
            ("Some Song (Visualizer)", "Some Song"),
            ("Some Song HD", "Some Song"),
            ("Some Song Official Video", "Some Song"),
        ];
        for (input, expected) in cases {
            assert_eq!(clean_title(input), expected, "entrada: {input}");
        }
    }

    #[test]
    fn clean_title_drops_collaborations() {
        assert_eq!(clean_title("Song feat. Someone"), "Song");
        assert_eq!(clean_title("Song (feat. Someone)"), "Song");
        assert_eq!(clean_title("Song ft. A & B"), "Song");
        assert_eq!(clean_title("Song featuring X"), "Song");
    }

    #[test]
    fn clean_title_keeps_meaningful_parentheses() {
        assert_eq!(clean_title("Song (I Love You)"), "Song (I Love You)");
        assert_eq!(clean_title("Song - Live Forever"), "Song - Live Forever");
    }

    #[test]
    fn clean_artist_strips_channel_suffixes() {
        assert_eq!(clean_artist("Arctic Monkeys - Topic"), "Arctic Monkeys");
        assert_eq!(clean_artist("Arctic MonkeysVEVO"), "Arctic Monkeys");
        assert_eq!(clean_artist("Arctic Monkeys"), "Arctic Monkeys");
        assert_eq!(primary_artist("Calvin Harris, Dua Lipa"), "Calvin Harris");
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon");
    }

    #[test]
    fn split_artist_title_needs_a_short_left_side() {
        let (left, right) = split_artist_title("Arctic Monkeys - Do I Wanna Know?").unwrap();
        assert_eq!(left, "Arctic Monkeys");
        assert_eq!(right, "Do I Wanna Know?");
        assert!(split_artist_title("Do I Wanna Know?").is_none());
    }

    #[test]
    fn version_kind_detects_alternates() {
        assert_eq!(
            version_kind("Do I Wanna Know?", Some("AM")),
            VersionKind::Studio
        );
        assert_eq!(
            version_kind("Do I Wanna Know? (Live)", None),
            VersionKind::Live
        );
        assert_eq!(version_kind("Song - Remix", None), VersionKind::Remix);
        assert_eq!(
            version_kind("Song (Karaoke Version)", None),
            VersionKind::Karaoke
        );
        assert_eq!(version_kind("Song (Sped Up)", None), VersionKind::SpedUp);
        assert_eq!(version_kind("Stayin' Alive", None), VersionKind::Studio);
        assert!(VersionKind::Karaoke.is_incompatible());
        assert!(!VersionKind::Live.is_incompatible());
    }

    #[test]
    fn similarity_is_one_for_noisy_variants_of_the_same_title() {
        assert_eq!(
            similarity("Do I Wanna Know?", "Do I Wanna Know? (Official Video)"),
            1.0
        );
        assert_eq!(similarity("Song | Official Video", "Song"), 1.0);
        assert!(similarity("Do I Wanna Know?", "R U Mine?") < 0.2);
    }

    #[test]
    fn similarity_tolerates_typos() {
        let exact = similarity("Do I Wanna Know?", "Do I Wanna Know?");
        let typo = similarity("Do I Wanna Know?", "Do I Wana Know");
        assert!(typo > 0.7 && typo < exact, "{typo}");
    }

    #[test]
    fn artist_similarity_rewards_collaborations() {
        let collab = artist_similarity("Calvin Harris, Dua Lipa", "Dua Lipa");
        let stranger = artist_similarity("Calvin Harris", "Dua Lipa");
        assert!(collab > 0.9, "{collab}");
        assert!(stranger < 0.2, "{stranger}");
    }
}
