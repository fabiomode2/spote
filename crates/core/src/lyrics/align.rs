//! Alineación forzada de la letra conocida contra las palabras que detecta el
//! ASR.
//!
//! El texto de la letra ya se conoce, así que no se compara una transcripción
//! literal (el modelo se equivoca mucho cantando) sino que se hace un
//! alineamiento global fonético. Las palabras que casan anclan su línea; las que no, se interpolan
//! entre las anclas vecinas.
//!
//! El módulo es puro: recibe líneas y palabras con tiempos y no toca audio,
//! red ni caché. La orquestación de lecturas vive en [`super::asr`].

use std::collections::HashMap;

use super::{mark_optional_words, word_times, LyricLine, Word};

/// Las palabras muy cortas se emparejan con más tolerancia: un "mrs" oído como
/// "mr" sigue siendo un ancla útil.
const SHORT_WORDS: &[&str] = &[
    "a", "al", "con", "de", "del", "e", "el", "en", "es", "la", "las", "lo", "los", "me", "mi",
    "no", "oh", "pa", "para", "por", "que", "se", "si", "son", "su", "te", "tu", "un", "una",
    "unas", "unos", "y", "ya", "yo",
];

const GAP: f64 = -0.5;
const MIN_CONFIDENCE: f64 = 0.30;

/// Una palabra detectada por el modelo, en segundos.
#[derive(Debug, Clone, PartialEq)]
pub struct AsrWord {
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
    pub probability: f32,
}

#[derive(Debug, Clone, Default)]
pub struct AlignStats {
    /// Qué líneas consiguieron un ancla real en el ASR.
    pub anchored: Vec<bool>,
    pub matched: usize,
    pub median_shift: f64,
}

/// Convierte un texto en el token que se compara: minúsculas, sin acentos y
/// solo `[a-z0-9]`. El original usaba NFKD; aquí se pliegan los acentos que
/// aparecen en español, inglés, francés, alemán y portugués.
pub fn normalize_word(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for char in value.chars() {
        let lower = char.to_lowercase().next().unwrap_or(char);
        if let Some(folded) = fold_accent(lower) {
            if folded.is_ascii_alphanumeric() {
                out.push(folded);
            }
        }
    }
    out
}

fn fold_accent(char: char) -> Option<char> {
    let folded = match char {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ñ' | 'ń' | 'ň' => 'n',
        'ç' | 'ć' | 'č' => 'c',
        'ý' | 'ÿ' => 'y',
        'ś' | 'š' => 's',
        'ź' | 'ż' | 'ž' => 'z',
        'ł' => 'l',
        'đ' | 'ð' => 'd',
        'þ' => 't',
        'æ' => 'e',
        'œ' => 'o',
        other => other,
    };
    Some(folded)
}

fn is_short_word(token: &str) -> bool {
    SHORT_WORDS.contains(&token)
}

/// Español aproximado: `qu`→`k`, `ll`→`y`, `v`→`b`, `z`→`s`, `h` muda y las
/// palatales `ce/ci`/`ge/gi`. El modelo confunde esas grafías al cantar.
fn phonetic(word: &str) -> String {
    let value = normalize_word(word);
    let mut value = value
        .replace("qu", "k")
        .replace("ll", "y")
        .replace('v', "b")
        .replace('z', "s")
        .replace('h', "");
    value = replace_soft_c(&value);
    value = replace_soft_g(&value);
    collapse_repeats(&value)
}

fn replace_soft_c(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        let char = chars[index];
        if char == 'c' && index + 1 < chars.len() && "eiy".contains(chars[index + 1]) {
            out.push('s');
        } else {
            out.push(char);
        }
        index += 1;
    }
    out
}

fn replace_soft_g(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        let char = chars[index];
        if char == 'g' && index + 1 < chars.len() && "eiy".contains(chars[index + 1]) {
            out.push('j');
        } else {
            out.push(char);
        }
        index += 1;
    }
    out
}

fn collapse_repeats(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous = None;
    for char in value.chars() {
        if Some(char) != previous {
            out.push(char);
        }
        previous = Some(char);
    }
    out
}

/// Puntuación de similitud entre dos tokens ya normalizados.
fn similarity(left: &str, phonetic_left: &str, right: &str, phonetic_right: &str) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    if left.chars().count() < 3 || right.chars().count() < 3 {
        return if sequence_ratio(left, right) >= 0.66 {
            0.8
        } else {
            0.0
        };
    }
    let ratio = sequence_ratio(left, right);
    if !phonetic_left.is_empty() && phonetic_left == phonetic_right {
        return 0.9;
    }
    let phonetic_ratio = sequence_ratio(phonetic_left, phonetic_right);
    let best = ratio.max(phonetic_ratio);
    if best >= 0.75 {
        0.8
    } else if best >= 0.62 {
        0.5
    } else {
        0.0
    }
}

/// Equivalente a `difflib.SequenceMatcher.ratio()`: caracteres que casan en
/// común / total. Se usa una búsqueda recursiva del bloque común más largo.
fn sequence_ratio(left: &str, right: &str) -> f64 {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let matched = matching_chars(&a, &b, 0, a.len(), 0, b.len());
    2.0 * matched as f64 / (a.len() + b.len()) as f64
}

fn matching_chars(
    a: &[char],
    b: &[char],
    a_begin: usize,
    a_end: usize,
    b_begin: usize,
    b_end: usize,
) -> usize {
    let (a_start, b_start, size) = longest_match(a, b, a_begin, a_end, b_begin, b_end);
    if size == 0 {
        return 0;
    }
    size + matching_chars(a, b, a_begin, a_start, b_begin, b_start)
        + matching_chars(a, b, a_start + size, a_end, b_start + size, b_end)
}

fn longest_match(
    a: &[char],
    b: &[char],
    a_begin: usize,
    a_end: usize,
    b_begin: usize,
    b_end: usize,
) -> (usize, usize, usize) {
    let width = b_end.saturating_sub(b_begin);
    if width == 0 || a_begin >= a_end {
        return (a_begin, b_begin, 0);
    }
    let mut best = (a_begin, b_begin, 0);
    let mut previous = vec![0usize; width + 1];
    let mut current = vec![0usize; width + 1];
    for i in a_begin..a_end {
        current.fill(0);
        for j in b_begin..b_end {
            if a[i] == b[j] {
                let run = previous[j - b_begin] + 1;
                current[j - b_begin + 1] = run;
                if run > best.2 {
                    best = (i + 1 - run, j + 1 - run, run);
                }
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }
    best
}

/// Tabla de tokens con su forma fonética para no recalcularla en cada celda.
struct Tokens {
    raw: Vec<String>,
    phonetic: Vec<String>,
}

impl Tokens {
    fn new<I: IntoIterator<Item = String>>(tokens: I) -> (Self, Vec<usize>) {
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut table = Tokens {
            raw: Vec::new(),
            phonetic: Vec::new(),
        };
        let mut ids = Vec::new();
        for token in tokens {
            let id = *index.entry(token.clone()).or_insert_with(|| {
                table.raw.push(token);
                let id = table.raw.len() - 1;
                let raw = table.raw[id].clone();
                table.phonetic.push(phonetic(&raw));
                id
            });
            ids.push(id);
        }
        (table, ids)
    }
}

struct Similarity<'a> {
    tokens: &'a Tokens,
    cache: HashMap<(usize, usize), f64>,
}

impl<'a> Similarity<'a> {
    fn get(&mut self, left: usize, right: usize) -> f64 {
        if let Some(value) = self.cache.get(&(left, right)) {
            return *value;
        }
        let value = similarity(
            &self.tokens.raw[left],
            &self.tokens.phonetic[left],
            &self.tokens.raw[right],
            &self.tokens.phonetic[right],
        );
        self.cache.insert((left, right), value);
        value
    }
}

/// Needleman-Wunsch sobre similitud difusa. Devuelve `lyric_index -> asr_index`.
fn mapping(lyric_ids: &[usize], asr_ids: &[usize], tokens: &Tokens) -> HashMap<usize, usize> {
    let rows = lyric_ids.len();
    let columns = asr_ids.len();
    let stride = columns + 1;
    let mut scores = vec![0.0f64; (rows + 1) * stride];
    for row in 1..=rows {
        scores[row * stride] = scores[(row - 1) * stride] + GAP;
    }
    for column in 1..=columns {
        scores[column] = scores[column - 1] + GAP;
    }

    let mut similarity_cache = Similarity {
        tokens,
        cache: HashMap::new(),
    };
    for row in 1..=rows {
        for column in 1..=columns {
            let match_score = similarity_cache.get(lyric_ids[row - 1], asr_ids[column - 1]);
            let diagonal = scores[(row - 1) * stride + column - 1] + match_score;
            let up = scores[(row - 1) * stride + column] + GAP;
            let left = scores[row * stride + column - 1] + GAP;
            scores[row * stride + column] = diagonal.max(up).max(left);
        }
    }

    let mut mapping = HashMap::new();
    let (mut row, mut column) = (rows, columns);
    while row > 0 && column > 0 {
        let match_score = similarity_cache.get(lyric_ids[row - 1], asr_ids[column - 1]);
        let current = scores[row * stride + column];
        if (current - (scores[(row - 1) * stride + column - 1] + match_score)).abs() < 1e-9 {
            if match_score > 0.0 {
                mapping.insert(row - 1, column - 1);
            }
            row -= 1;
            column -= 1;
        } else if (current - (scores[(row - 1) * stride + column] + GAP)).abs() < 1e-9 {
            row -= 1;
        } else {
            column -= 1;
        }
    }
    mapping
}

pub fn count_syllables(text: &str) -> usize {
    let mut clean = String::new();
    for char in text.chars() {
        let lower = char.to_lowercase().next().unwrap_or(char);
        let folded = fold_accent(lower).unwrap_or(lower);
        if folded.is_ascii_alphabetic() {
            clean.push(folded);
        } else {
            clean.push(' ');
        }
    }
    let clean = clean
        .replace("gue", "ge")
        .replace("gui", "gi")
        .replace("que", "qe")
        .replace("qui", "qi");
    let mut total = 0usize;
    for word in clean.split_whitespace() {
        let mut in_vowel = false;
        for char in word.chars() {
            let vowel = "aeiou".contains(char);
            if vowel && !in_vowel {
                total += 1;
            }
            in_vowel = vowel;
        }
    }
    total.max(1)
}

fn estimated_end(text: &str, start_ms: u64) -> u64 {
    let syllables = count_syllables(text);
    start_ms + ((0.35 + syllables as f64 * 0.21).clamp(0.8, 7.0) * 1000.0) as u64
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(|a, b| a.total_cmp(b));
    ordered[ordered.len() / 2]
}

/// Reparte una tirada sin ancla entre dos anclas. Se programa hacia atrás
/// (lo normal es que las líneas perdidas estén justo antes de la siguiente) y
/// se comprime si no caben. Antes se apilaban contra el borde y el recorte
/// monótono las dejaba separadas 30 ms.
fn fit_run(durations: &[f64], begin: f64, end: f64) -> Vec<f64> {
    let count = durations.len();
    if count == 0 {
        return Vec::new();
    }
    let span = (end - begin).max(0.0);
    if span <= 0.0 {
        return (0..count)
            .map(|index| begin + index as f64 * 0.15)
            .collect();
    }
    let gap = 0.20;
    let usable = (span - gap * (count.saturating_sub(1)) as f64).max(0.0);
    let total: f64 = durations.iter().sum();
    let scale = if total <= 0.0 {
        1.0
    } else {
        (usable / total).min(1.0)
    };
    // Menos de ~0.35 s por línea se lee como un salto: reparto uniforme.
    if count > 1 && usable / (count as f64) < 0.35 {
        let step = span / (count - 1) as f64;
        return (0..count)
            .map(|index| begin + step * index as f64)
            .collect();
    }
    let mut starts = vec![0.0; count];
    let mut cursor = end;
    for index in (0..count).rev() {
        cursor = begin.max(cursor - durations[index] * scale);
        starts[index] = cursor;
        cursor -= gap;
    }
    starts
}

/// Construye los tiempos de palabra de una línea alineada.
///
/// Usa el reparto por sílabas como base y sobrescribe las palabras que casaron
/// con el ASR. Antes `align_lines` devolvía `words: Vec::new()`: la app se salta
/// el refinado cuando `aligned` es true, así que el karaoke se quedaba sin
/// progreso por palabra justo en las letras mejor alineadas.
fn aligned_words(
    words: &[String],
    token_words: &[usize],
    range: (usize, usize),
    mapping: &HashMap<usize, usize>,
    asr_tokens: &[(f64, f64, String)],
    start_ms: u64,
    end_ms: u64,
) -> Vec<Word> {
    if words.is_empty() {
        return Vec::new();
    }
    let end_ms = end_ms.max(start_ms + 1);
    let mut out = if words.len() >= 2 {
        word_times(start_ms, end_ms, &words.join(" ")).unwrap_or_else(|| {
            vec![Word {
                start_ms,
                end_ms,
                text: words.join(" "),
                optional: false,
            }]
        })
    } else {
        vec![Word {
            start_ms,
            end_ms,
            text: words[0].clone(),
            optional: false,
        }]
    };
    if out.len() != words.len() {
        out = words
            .iter()
            .map(|text| Word {
                start_ms,
                end_ms,
                text: text.clone(),
                optional: false,
            })
            .collect();
    }

    // Ancla de cada palabra de la línea, si su token casó con el ASR.
    let begin = range.0;
    for global in range.0..range.1 {
        let Some(&asr_index) = mapping.get(&global) else {
            continue;
        };
        let Some(local) = global.checked_sub(begin) else {
            continue;
        };
        let Some(&word_index) = token_words.get(local) else {
            continue;
        };
        let Some(word) = out.get_mut(word_index) else {
            continue;
        };
        let Some((start_s, end_s, _)) = asr_tokens.get(asr_index) else {
            continue;
        };
        let last_usable = end_ms.saturating_sub(1).max(start_ms);
        let anchor_start =
            ((*start_s * 1000.0).round() as i64).clamp(start_ms as i64, last_usable as i64) as u64;
        let anchor_end =
            ((*end_s * 1000.0).round() as i64).clamp(anchor_start as i64 + 1, end_ms as i64) as u64;
        word.start_ms = anchor_start;
        word.end_ms = anchor_end;
    }

    // Monótono y dentro de la línea: una ancla mala no puede adelantar la
    // palabra siguiente ni salirse del verso.
    let last_usable = end_ms.saturating_sub(1).max(start_ms);
    let mut previous = start_ms;
    for word in &mut out {
        let floor = previous.min(last_usable);
        word.start_ms = word.start_ms.clamp(floor, last_usable);
        word.end_ms = word.end_ms.max(word.start_ms + 1).min(end_ms);
        previous = word.start_ms;
    }
    let count = out.len();
    for index in 0..count {
        let next_start = out
            .get(index + 1)
            .map(|word| word.start_ms)
            .unwrap_or(end_ms);
        let word_start = out[index].start_ms;
        out[index].end_ms = next_start.max(word_start + 1).min(end_ms);
    }
    if let Some(last) = out.last_mut() {
        last.end_ms = end_ms.max(last.start_ms + 1);
    }
    out
}

/// Alinea las líneas contra las palabras del ASR.
///
/// Las líneas deben venir sin marcas instrumentales (`♪`): el llamante las
/// quita. Devuelve las líneas con tiempos nuevos, la confianza y qué líneas
/// consiguieron ancla (para decidir qué zonas releer).
pub fn align_lines(
    lines: &[LyricLine],
    words: &[AsrWord],
    duration_s: f64,
) -> Option<(Vec<LyricLine>, f64, AlignStats)> {
    if lines.len() < 2 || words.len() < 6 {
        return None;
    }

    // 1. Tokens de la letra, guardando a qué línea y a qué palabra pertenece
    //    cada token. Así luego se reconstruyen tiempos por palabra con las
    //    anclas del ASR.
    let mut lyric_tokens: Vec<String> = Vec::new();
    let mut line_ranges: Vec<(usize, usize)> = Vec::with_capacity(lines.len());
    let mut line_words: Vec<Vec<String>> = Vec::with_capacity(lines.len());
    let mut token_words: Vec<Vec<usize>> = Vec::with_capacity(lines.len());
    for line in lines {
        let begin = lyric_tokens.len();
        let mut words_here: Vec<String> = Vec::new();
        let mut token_word_here: Vec<usize> = Vec::new();
        for word in line.text.split_whitespace() {
            let word_index = words_here.len();
            words_here.push(word.to_string());
            let normalized = normalize_word(word);
            if !normalized.is_empty() {
                lyric_tokens.push(normalized);
                token_word_here.push(word_index);
            }
        }
        line_ranges.push((begin, lyric_tokens.len()));
        line_words.push(words_here);
        token_words.push(token_word_here);
    }
    if lyric_tokens.len() < 6 {
        return None;
    }

    // 2. Tokens del ASR. El modelo parte etiquetas ("M" + "&R"): se unen las
    // piezas de una o dos letras que no forman una palabra corta real.
    let mut raw_words: Vec<(f64, f64, String)> = Vec::new();
    for word in words {
        let normalized = normalize_word(&word.text);
        if !normalized.is_empty() {
            raw_words.push((word.start_s, word.end_s, normalized));
        }
    }
    let mut asr_tokens: Vec<(f64, f64, String)> = Vec::new();
    let mut index = 0;
    while index < raw_words.len() {
        let (start, end, token) = &raw_words[index];
        let next = raw_words.get(index + 1);
        if token.chars().count() <= 2
            && !is_short_word(token)
            && next.is_some_and(|(_, _, next)| next.chars().count() <= 2 && !is_short_word(next))
        {
            let next = &raw_words[index + 1];
            asr_tokens.push((*start, next.1, format!("{token}{}", next.2)));
            index += 2;
        } else {
            asr_tokens.push((*start, *end, token.clone()));
            index += 1;
        }
    }
    if asr_tokens.len() < 6 {
        return None;
    }

    let (tokens, lyric_ids) = Tokens::new(lyric_tokens.iter().cloned());
    let (asr_table, asr_ids) = Tokens::new(asr_tokens.iter().map(|(_, _, token)| token.clone()));
    // Las dos tablas comparten la forma de los tokens; para la similitud hace
    // falta una sola tabla con ambos vocabularios. Se reconstruye con los ids
    // del ASR mapeados a la tabla de la letra.
    let mut text_tokens = tokens;
    let mut asr_ids = asr_ids;
    {
        let mut index: HashMap<String, usize> = text_tokens
            .raw
            .iter()
            .enumerate()
            .map(|(id, token)| (token.clone(), id))
            .collect();
        for id in &mut asr_ids {
            let token = &asr_table.raw[*id];
            let mapped = *index.entry(token.clone()).or_insert_with(|| {
                text_tokens.raw.push(token.clone());
                let new_id = text_tokens.raw.len() - 1;
                let raw = text_tokens.raw[new_id].clone();
                text_tokens.phonetic.push(phonetic(&raw));
                new_id
            });
            *id = mapped;
        }
    }

    let mapping = mapping(&lyric_ids, &asr_ids, &text_tokens);
    if mapping.is_empty() {
        return None;
    }
    let token_ratio = mapping.len() as f64 / lyric_tokens.len() as f64;
    if token_ratio < 0.20 {
        return None;
    }

    // 3. Ancla de cada línea: primera y última palabra casada.
    let mut starts: Vec<Option<f64>> = Vec::with_capacity(lines.len());
    let mut ends: Vec<Option<f64>> = Vec::with_capacity(lines.len());
    let mut matched_lines = 0usize;
    let mut line_token_last: Vec<Option<usize>> = Vec::with_capacity(lines.len());
    for &(begin, end) in &line_ranges {
        let indices: Vec<usize> = (begin..end)
            .filter_map(|index| mapping.get(&index).copied())
            .collect();
        if indices.is_empty() {
            starts.push(None);
            ends.push(None);
            line_token_last.push(None);
            continue;
        }
        let first = *indices.iter().min().unwrap();
        let last = *indices.iter().max().unwrap();
        starts.push(Some(asr_tokens[first].0));
        ends.push(Some(asr_tokens[last].1));
        line_token_last.push(Some(end.saturating_sub(1)));
        matched_lines += 1;
    }
    if matched_lines < 2 {
        return None;
    }

    // 4. Un ancla casi encima de la anterior cuando la letra original dejaba
    // un hueco largo suele ser una palabra repetida mal casada. Si además hay
    // líneas sin anclar en medio, se suelta para que quepan en vez de aplastar
    // la estrofa entera.
    let mut previous_anchor: Option<usize> = None;
    for index in 0..lines.len() {
        let Some(start) = starts[index] else { continue };
        if let Some(previous) = previous_anchor {
            let original_gap =
                (lines[index].start_ms as f64 - lines[previous].start_ms as f64) / 1000.0;
            let asr_gap = start - starts[previous].unwrap_or(start);
            let skipped = (previous + 1..index).any(|inside| starts[inside].is_none());
            if skipped && original_gap > 4.0 && asr_gap < 1.2f64.min(original_gap * 0.20) {
                starts[index] = None;
                ends[index] = None;
                matched_lines -= 1;
                continue;
            }
        }
        previous_anchor = Some(index);
    }
    if matched_lines < 2 {
        return None;
    }

    let max_asr = asr_tokens
        .iter()
        .map(|(_, end, _)| *end)
        .fold(0.0, f64::max);
    let shifts: Vec<f64> = (0..lines.len())
        .filter_map(|index| {
            starts[index].map(|start| start - lines[index].start_ms as f64 / 1000.0)
        })
        .collect();
    let median_shift = median(&shifts);

    // 5. Tiempo de línea: ancla si la hay; si no, posición estimada + el
    // desplazamiento global. Las líneas que quedan más allá de lo transcrito
    // conservan su tiempo estimado si el ASR no llegó.
    let mut final_starts: Vec<f64> = Vec::with_capacity(lines.len());
    let mut final_ends: Vec<Option<f64>> = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let original = line.start_ms as f64 / 1000.0;
        let previous_start = final_starts.last().copied().unwrap_or(-1.0);
        let mut start = starts[index].unwrap_or(original + median_shift);
        if !(0.0..=max_asr + 2.0).contains(&start) {
            start = original + median_shift;
        }
        let tail_fallback = original > max_asr + 5.0
            && (starts[index].is_none()
                || starts[index].is_some_and(|anchor| {
                    anchor > max_asr + 2.0 || anchor <= previous_start + 0.05
                }))
            && (duration_s <= 0.0 || original <= duration_s + 5.0);
        if tail_fallback {
            start = original;
            final_ends.push(None);
        } else {
            final_ends.push(ends[index]);
        }
        final_starts.push(start);
    }

    // 6. Reparto de las líneas que el ASR no oyó entre sus anclas vecinas.
    let mut index = 0;
    while index < lines.len() {
        if starts[index].is_some() {
            index += 1;
            continue;
        }
        let mut run_end = index;
        while run_end + 1 < lines.len() && starts[run_end + 1].is_none() {
            run_end += 1;
        }
        let all_inside =
            (index..=run_end).all(|line| lines[line].start_ms as f64 / 1000.0 <= max_asr + 5.0);
        if index > 0
            && run_end + 1 < lines.len()
            && starts[run_end + 1].is_some()
            && starts[index - 1].is_some()
            && all_inside
        {
            let previous_end =
                final_ends[index - 1].unwrap_or_else(|| final_starts[index - 1] + 1.0);
            let next_start = final_starts[run_end + 1];
            let durations: Vec<f64> = (index..=run_end)
                .map(|line| {
                    (0.35 + count_syllables(&lines[line].text) as f64 * 0.21).clamp(0.9, 5.0)
                })
                .collect();
            let fitted = fit_run(&durations, previous_end + 0.10, next_start - 0.20);
            final_starts.splice(index..=run_end, fitted);
        }
        index = run_end + 1;
    }

    // 7. Salida monótona y duraciones razonables.
    let mut refined = Vec::with_capacity(lines.len());
    let mut previous_ms: Option<u64> = None;
    for (index, line) in lines.iter().enumerate() {
        let mut start_ms = (final_starts[index] * 1000.0).round().max(0.0) as u64;
        if let Some(previous) = previous_ms {
            // 30 ms dejaba pasar ráfagas; 120 ms mantiene la frase legible.
            start_ms = start_ms.max(previous + 120);
        }
        if duration_s > 0.0 {
            start_ms = start_ms.min((duration_s * 1000.0) as u64 + 2000);
        }
        let next_ms = if index + 1 < lines.len() {
            (start_ms + 50).max((final_starts[index + 1] * 1000.0).round() as u64)
        } else {
            start_ms + 8000
        };
        let last_word = line_token_last[index];
        let mut end_ms = if final_ends[index].is_some()
            && last_word.is_some_and(|last_word| mapping.contains_key(&last_word))
        {
            (final_ends[index].unwrap() * 1000.0).round().max(0.0) as u64 + 300
        } else {
            estimated_end(&line.text, start_ms)
        };
        end_ms = end_ms.min(next_ms.saturating_sub(40)).max(start_ms + 250);
        let words = aligned_words(
            &line_words[index],
            &token_words[index],
            line_ranges[index],
            &mapping,
            &asr_tokens,
            start_ms,
            end_ms,
        );
        let mut refined_line = LyricLine {
            start_ms,
            end_ms,
            text: line.text.clone(),
            words,
            // Los tiempos por palabra vienen del ASR, no del mapa de energía:
            // que la UI no los vuelva a repartir.
            words_exact: true,
        };
        mark_optional_words(&mut refined_line);
        refined.push(refined_line);
        previous_ms = Some(start_ms);
    }

    let coverage = matched_lines as f64 / lines.len() as f64;
    let confidence = (0.60 * token_ratio + 0.40 * coverage).min(1.0);
    if confidence < MIN_CONFIDENCE {
        return None;
    }
    let stats = AlignStats {
        anchored: (0..lines.len())
            .map(|index| starts[index].is_some())
            .collect(),
        matched: matched_lines,
        median_shift,
    };
    Some((refined, (confidence * 100.0).round() / 100.0, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start_ms: u64, text: &str) -> LyricLine {
        LyricLine {
            start_ms,
            text: text.to_string(),
            ..LyricLine::default()
        }
    }

    fn word(start: f64, end: f64, text: &str, probability: f32) -> AsrWord {
        AsrWord {
            start_s: start,
            end_s: end,
            text: text.to_string(),
            probability,
        }
    }

    #[test]
    fn matches_misheard_spanish_words() {
        let lines = vec![
            line(1_000, "Fumando grasa en el Golf"),
            line(5_000, "Tapa'o de diseñador"),
        ];
        let words = vec![
            word(20.0, 20.5, "Fomando", 0.7),
            word(20.5, 20.9, "grasa", 0.8),
            word(20.9, 21.1, "en", 0.9),
            word(21.1, 21.3, "el", 0.9),
            word(21.3, 21.7, "Golf", 0.8),
            word(23.0, 23.4, "Tapau", 0.6),
            word(23.4, 23.7, "de", 0.9),
            word(23.7, 24.2, "diseñador", 0.9),
        ];

        let (refined, confidence, _) = align_lines(&lines, &words, 30.0).unwrap();

        assert!(confidence > 0.5);
        assert_eq!(refined[0].start_ms, 20_000);
        assert_eq!(refined[1].start_ms, 23_000);
        assert!(refined[0].end_ms <= refined[1].start_ms);
    }

    #[test]
    fn interpolates_a_skipped_line() {
        let lines = vec![
            line(0, "one two three"),
            line(4_000, "middle"),
            line(8_000, "four five six"),
        ];
        let words = vec![
            word(0.0, 0.2, "one", 0.9),
            word(0.2, 0.4, "two", 0.9),
            word(0.4, 0.6, "three", 0.9),
            word(6.0, 6.2, "four", 0.9),
            word(6.2, 6.4, "five", 0.9),
            word(6.4, 6.6, "six", 0.9),
        ];

        let (refined, _, _) = align_lines(&lines, &words, 10.0).unwrap();

        assert!(refined[1].start_ms > 4_000);
        assert!(refined[1].start_ms < refined[2].start_ms);
    }

    #[test]
    fn never_crams_a_skipped_run() {
        let lines = vec![
            line(0, "one two three"),
            line(2_000, "extraordinariamente sobrevalorado"),
            line(3_000, "incomprensiblemente alargado"),
            line(4_000, "desproporcionadamente extenso"),
            line(5_000, "interminablemente prolongado"),
            line(20_000, "six seven eight"),
        ];
        let words = vec![
            word(0.2, 0.4, "one", 0.9),
            word(0.4, 0.6, "two", 0.9),
            word(0.6, 0.8, "three", 0.9),
            word(6.5, 6.7, "six", 0.9),
            word(6.7, 6.9, "seven", 0.9),
            word(6.9, 7.1, "eight", 0.9),
        ];

        let (refined, _, _) = align_lines(&lines, &words, 30.0).unwrap();
        let starts: Vec<u64> = refined.iter().map(|line| line.start_ms).collect();
        for pair in starts.windows(2) {
            assert!(pair[1] - pair[0] >= 250, "{starts:?}");
        }
        assert!(
            starts[1..5].iter().all(|start| *start < 6_500),
            "{starts:?}"
        );
    }

    #[test]
    fn stray_anchor_does_not_cram_a_skipped_run() {
        let lines = vec![
            line(0, "one two three"),
            line(9_000, "these words are not heard"),
            line(10_000, "tengo la imagen grabada"),
            line(11_000, "other invented verse"),
            line(13_000, "six seven eight"),
            line(20_000, "nine ten eleven twelve thirteen"),
        ];
        let words = vec![
            word(0.2, 0.4, "one", 0.9),
            word(0.4, 0.6, "two", 0.9),
            word(0.6, 0.8, "three", 0.9),
            word(0.5, 0.7, "tengo", 0.4),
            word(8.0, 8.2, "nine", 0.9),
            word(8.2, 8.4, "ten", 0.9),
            word(8.4, 8.6, "eleven", 0.9),
            word(8.6, 8.8, "twelve", 0.9),
            word(8.8, 9.0, "thirteen", 0.9),
        ];

        let (refined, _, _) = align_lines(&lines, &words, 30.0).unwrap();
        let starts: Vec<u64> = refined.iter().map(|line| line.start_ms).collect();
        for pair in starts.windows(2) {
            assert!(pair[1] - pair[0] >= 250, "{starts:?}");
        }
        assert!(starts[2] > 1_000, "{starts:?}");
    }

    #[test]
    fn aligned_lines_get_word_timings_from_the_asr() {
        let lines = vec![line(0, "one two three"), line(4_000, "four five six")];
        let words = vec![
            word(0.0, 0.2, "one", 0.9),
            word(0.2, 0.4, "two", 0.9),
            word(0.4, 0.6, "three", 0.9),
            word(6.0, 6.2, "four", 0.9),
            word(6.2, 6.4, "five", 0.9),
            word(6.4, 6.6, "six", 0.9),
        ];
        let (refined, _, _) = align_lines(&lines, &words, 10.0).unwrap();
        for line in &refined {
            assert!(line.words_exact, "la UI no debe repartir estas palabras");
            assert!(!line.words.is_empty());
            let mut previous = line.start_ms;
            for word in &line.words {
                assert!(word.start_ms >= previous, "{word:?} en {line:?}");
                assert!(word.end_ms >= word.start_ms, "{word:?}");
                assert!(word.end_ms <= line.end_ms, "{word:?} en {line:?}");
                previous = word.start_ms;
            }
        }
        // La primera palabra se ancla al inicio real de la voz (6 s), no al
        // reparto proporcional de la línea.
        assert!((refined[1].words[0].start_ms as i64 - 6_000).abs() < 50);
    }

    #[test]
    fn normalize_strips_accents_and_punctuation() {
        assert_eq!(normalize_word("Diseñador,"), "disenador");
        assert_eq!(normalize_word("Tapa'o"), "tapao");
        assert_eq!(normalize_word("¿Qué?"), "que");
    }

    #[test]
    fn syllables_follow_spanish_groups() {
        assert!(count_syllables("extraordinariamente") >= 5);
        assert_eq!(count_syllables("que"), 1);
    }
}
