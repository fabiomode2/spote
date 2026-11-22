//! Procesado de audio ajustable sobre mpv: preamp, EQ gráfico de 10 bandas,
//! normalizador y limitador.
//!
//! La cadena se construye como un string de filtros de libavfilter y se aplica
//! en caliente a la propiedad `af` de mpv. No hay que reiniciar mpv ni recargar
//! la pista: `set_property af "<cadena>"` reconstruye el grafo al instante y
//! `set_property af ""` lo vacía. La cadena sobrevive a los cambios de pista
//! porque `af` es una opción del reproductor, no del fichero.
//!
//! Este módulo es puro: no habla con mpv. `AudioCfg::filter_chain` devuelve el
//! string y `Player::set_audio_filters` lo envía por IPC.

use serde::{Deserialize, Deserializer, Serialize};

/// Frecuencias centrales del EQ, en Hz. Índice alineado con `AudioCfg::bands`.
pub const BAND_FREQS: [u32; 10] = [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];

/// Etiquetas compactas para la UI (mismo orden que `BAND_FREQS`).
pub const BAND_LABELS: [&str; 10] = [
    "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];

/// Presets planos para que la UI los aplique sin conocer el motor.
pub const PRESET_FLAT: [f32; 10] = [0.0; 10];
pub const PRESET_BASS: [f32; 10] = [7.0, 6.0, 5.0, 3.0, 1.0, 0.0, -1.0, -1.0, 0.0, 0.0];
pub const PRESET_VOCAL: [f32; 10] = [-4.0, -3.0, -2.0, 0.0, 2.0, 4.0, 5.0, 4.0, 2.0, 0.0];
pub const PRESET_ROCK: [f32; 10] = [5.0, 4.0, 2.0, 0.0, -2.0, -1.0, 1.0, 3.0, 4.0, 4.0];
pub const PRESET_ELECTRONIC: [f32; 10] = [6.0, 5.0, 3.0, 0.0, -2.0, 1.0, 2.0, 4.0, 5.0, 4.0];

/// Presets con nombre, listos para pintar un selector.
pub const EQ_PRESETS: [(&str, [f32; 10]); 5] = [
    ("Flat", PRESET_FLAT),
    ("Bass", PRESET_BASS),
    ("Vocal", PRESET_VOCAL),
    ("Rock", PRESET_ROCK),
    ("Electronic", PRESET_ELECTRONIC),
];

/// Normalizador de volumen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NormalizerMode {
    /// Sin normalización.
    #[default]
    Off,
    /// Compresión dinámica adaptativa (`dynaudnorm`): mantiene el nivel entre
    /// pistas y sube las partes flojas sin tocar los picos.
    Dynamic,
    /// Normalización EBU R128 (`loudnorm`): target de emisión, más lenta.
    Loudnorm,
}

impl NormalizerMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Dynamic, Self::Loudnorm];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Dynamic => "dynamic",
            Self::Loudnorm => "loudnorm",
        }
    }

    /// Siguiente modo, para ciclar desde la UI.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Dynamic,
            Self::Dynamic => Self::Loudnorm,
            Self::Loudnorm => Self::Off,
        }
    }
}

/// Cadena de procesado persistida en `[audio]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioCfg {
    /// Interruptor maestro. Si está apagado no se envía ninguna cadena.
    pub enabled: bool,
    /// Permite apagar solo el EQ y conservar preamp/normalizador/limitador.
    pub eq_enabled: bool,
    /// Ganancia por banda en dB, en el orden de `BAND_FREQS`. Rango -12..+12.
    /// El deserializador tolera listas de otra longitud (configs viejas o
    /// editadas a mano) en vez de tirar toda la configuración.
    #[serde(default = "default_bands", deserialize_with = "deserialize_bands")]
    pub bands: [f32; 10],
    /// Ganancia de entrada en dB. Rango -12..+12.
    pub preamp_db: f32,
    /// Normalizador activo. Un valor desconocido cae en `Off` sin invalidar
    /// el resto de `config.toml`.
    #[serde(default, deserialize_with = "deserialize_normalizer")]
    pub normalizer: NormalizerMode,
    /// Intensidad del normalizador dinámico. Rango 0..1.
    pub normalizer_strength: f32,
    /// Iguala el volumen entre pistas. Para ficheros locales se usa la
    /// ganancia medida en el análisis completo de la canción; para streams o
    /// mientras se mide, un nivelador en directo como respaldo.
    #[serde(default = "default_true")]
    pub normalize_tracks: bool,
    /// Limitador de picos al final de la cadena.
    pub limiter: bool,
}

impl Default for AudioCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            eq_enabled: true,
            bands: [0.0; 10],
            preamp_db: 0.0,
            normalizer: NormalizerMode::Off,
            normalizer_strength: 0.5,
            normalize_tracks: true,
            // Neutro por defecto: el limitador es opt-in, no se aplica a todo
            // el audio sin que el usuario lo pida.
            limiter: false,
        }
    }
}

impl AudioCfg {
    /// Copia con los valores fuera de rango saneados. Los NaN e infinitos se
    /// convierten en el neutro (0 dB, 0 de intensidad) en vez de propagarse.
    pub fn sanitized(&self) -> Self {
        let mut out = self.clone();
        out.bands = self.bands.map(clamp_db);
        out.preamp_db = clamp_db(self.preamp_db);
        out.normalizer_strength = clamp01(self.normalizer_strength);
        out
    }

    /// Cadena de filtros para la propiedad `af` de mpv, o `None` si no hay
    /// nada que aplicar.
    ///
    /// Orden: preamp/ganancia de pista → EQ → nivelador en directo →
    /// normalizador manual → limitador. Las ganancias usan punto decimal
    /// (nunca coma, que separa filtros) y los valores no finitos se neutralizan.
    pub fn filter_chain(&self) -> Option<String> {
        self.filter_chain_with_gain(0.0, true)
    }

    /// Variante para la pista actual: `track_gain_db` es la compensación
    /// medida por el analizador (0 si aún no hay medida) y `live_fallback`
    /// añade un nivelador en directo mientras no existe esa medida (streams o
    /// primeros segundos de una pista local).
    pub fn filter_chain_with_gain(&self, track_gain_db: f32, live_fallback: bool) -> Option<String> {
        if !self.enabled {
            return None;
        }

        let mut filters: Vec<String> = Vec::with_capacity(16);

        let track_gain = if track_gain_db.is_finite() {
            track_gain_db.clamp(-18.0, 18.0)
        } else {
            0.0
        };
        let trim = round2(clamp_db(self.preamp_db) + track_gain);
        if trim != 0.0 {
            filters.push(format!(
                "volume=volume={}dB:precision=double",
                fmt_num(trim, 2)
            ));
        }

        if self.eq_enabled {
            for (freq, gain) in BAND_FREQS.iter().zip(self.bands.iter()) {
                let gain = round2(clamp_db(*gain));
                if gain == 0.0 {
                    continue;
                }
                filters.push(format!(
                    "equalizer=f={freq}:t=q:w=1.0:g={}",
                    fmt_num(gain, 2)
                ));
            }
        }

        let strength = round2(clamp01(self.normalizer_strength));
        if self.normalize_tracks && live_fallback && self.normalizer == NormalizerMode::Off {
            // Mismo objetivo para todas las pistas mientras no hay ganancia
            // medida. Ventana amplia para no bombear con cada golpe.
            let g = 15.0 + strength * 20.0;
            let p = 0.86 + strength * 0.08;
            filters.push(format!(
                "dynaudnorm=f=500:g={}:p={}:m=20",
                fmt_num(g, 2),
                fmt_num(p, 3)
            ));
        }
        match self.normalizer {
            NormalizerMode::Off => {}
            // `g` (ventana gaussiana) y `p` (pico) crecen con la intensidad.
            // A 0.5 queda g=15, p=0.875, m=10: nivel estable sin bombear.
            NormalizerMode::Dynamic => {
                let g = 5.0 + strength * 20.0;
                let p = 0.8 + strength * 0.15;
                filters.push(format!(
                    "dynaudnorm=f=250:g={}:p={}:m=10",
                    fmt_num(g, 2),
                    fmt_num(p, 3)
                ));
            }
            // Target fijo de emisión. `strength` no lo altera: loudnorm no es
            // un mando gradual y mover el target cambiaría el máster.
            NormalizerMode::Loudnorm => {
                filters.push("loudnorm=I=-16:TP=-1.5:LRA=11".to_string());
            }
        }

        if self.limiter {
            filters.push("alimiter=limit=0.95:attack=5:release=50".to_string());
        }

        if filters.is_empty() {
            None
        } else {
            Some(filters.join(","))
        }
    }
}

fn default_bands() -> [f32; 10] {
    [0.0; 10]
}

fn default_true() -> bool {
    true
}

fn deserialize_bands<'de, D>(deserializer: D) -> Result<[f32; 10], D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<f32>::deserialize(deserializer)?;
    let mut bands = [0.0; 10];
    for (slot, value) in bands.iter_mut().zip(values) {
        *slot = if value.is_finite() {
            value.clamp(-12.0, 12.0)
        } else {
            0.0
        };
    }
    Ok(bands)
}

fn deserialize_normalizer<'de, D>(deserializer: D) -> Result<NormalizerMode, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(match value.as_deref() {
        Some("dynamic") => NormalizerMode::Dynamic,
        Some("loudnorm") => NormalizerMode::Loudnorm,
        _ => NormalizerMode::Off,
    })
}

fn clamp_db(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-12.0, 12.0)
    } else {
        0.0
    }
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Redondea a dos decimales para que la cadena no arrastre `-0.0000001` ni
/// colas binarias del tipo `5.300000190734863`.
fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

/// Número con `decimals` decimales, sin ceros sobrantes y sin `-0`.
fn fmt_num(value: f32, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" || text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(cfg: &AudioCfg) -> String {
        cfg.filter_chain().expect("la configuración produce cadena")
    }

    #[test]
    fn tolerates_short_band_lists_and_unknown_normalizer() {
        let cfg: AudioCfg = toml::from_str("bands = [1.0, 2.0]\nnormalizer = \"weird\"").unwrap();
        assert_eq!(cfg.bands[0], 1.0);
        assert_eq!(cfg.bands[1], 2.0);
        assert_eq!(cfg.bands[9], 0.0);
        assert_eq!(cfg.normalizer, NormalizerMode::Off);
    }

    #[test]
    fn defaults_level_every_track() {
        assert_eq!(
            AudioCfg::default().filter_chain().as_deref(),
            Some("dynaudnorm=f=500:g=25:p=0.9:m=20")
        );
    }

    #[test]
    fn normalization_can_be_turned_off() {
        let cfg = AudioCfg {
            normalize_tracks: false,
            ..AudioCfg::default()
        };
        assert_eq!(cfg.filter_chain(), None);
    }

    #[test]
    fn flat_without_extras_has_no_chain() {
        let cfg = AudioCfg {
            limiter: false,
            normalize_tracks: false,
            ..AudioCfg::default()
        };
        assert_eq!(cfg.filter_chain(), None);
    }

    #[test]
    fn disabled_master_has_no_chain() {
        let cfg = AudioCfg {
            enabled: false,
            preamp_db: 6.0,
            normalizer: NormalizerMode::Dynamic,
            ..AudioCfg::default()
        };
        assert_eq!(cfg.filter_chain(), None);
    }

    #[test]
    fn one_band() {
        let mut cfg = AudioCfg {
            limiter: false,
            normalize_tracks: false,
            ..AudioCfg::default()
        };
        cfg.bands[5] = 6.0;
        assert_eq!(chain(&cfg), "equalizer=f=1000:t=q:w=1.0:g=6".to_string());
    }

    #[test]
    fn several_bands_keep_frequency_order() {
        let mut cfg = AudioCfg {
            limiter: false,
            normalize_tracks: false,
            ..AudioCfg::default()
        };
        cfg.bands[0] = 4.0;
        cfg.bands[9] = -3.5;
        cfg.bands[4] = 0.5;
        assert_eq!(
            chain(&cfg),
            "equalizer=f=31:t=q:w=1.0:g=4,\
             equalizer=f=500:t=q:w=1.0:g=0.5,\
             equalizer=f=16000:t=q:w=1.0:g=-3.5"
        );
    }

    #[test]
    fn eq_can_be_disabled_while_keeping_preamp() {
        let cfg = AudioCfg {
            eq_enabled: false,
            limiter: false,
            normalize_tracks: false,
            preamp_db: -2.5,
            ..AudioCfg::default()
        };
        assert_eq!(chain(&cfg), "volume=volume=-2.5dB:precision=double");
    }

    #[test]
    fn dynamic_normalizer_scales_with_strength() {
        let cfg = AudioCfg {
            limiter: false,
            normalizer: NormalizerMode::Dynamic,
            normalizer_strength: 0.5,
            ..AudioCfg::default()
        };
        assert_eq!(
            chain(&cfg),
            "dynaudnorm=f=250:g=15:p=0.875:m=10".to_string()
        );

        let full = AudioCfg {
            normalizer_strength: 1.0,
            ..cfg.clone()
        };
        assert_eq!(
            chain(&full),
            "dynaudnorm=f=250:g=25:p=0.95:m=10".to_string()
        );

        let zero = AudioCfg {
            normalizer_strength: 0.0,
            ..cfg
        };
        assert_eq!(chain(&zero), "dynaudnorm=f=250:g=5:p=0.8:m=10".to_string());
    }

    #[test]
    fn loudnorm_is_fixed_target() {
        let cfg = AudioCfg {
            limiter: false,
            normalizer: NormalizerMode::Loudnorm,
            normalizer_strength: 0.1,
            ..AudioCfg::default()
        };
        assert_eq!(chain(&cfg), "loudnorm=I=-16:TP=-1.5:LRA=11");
    }

    #[test]
    fn limiter_closes_the_chain() {
        let mut cfg = AudioCfg {
            limiter: true,
            normalizer: NormalizerMode::Dynamic,
            ..AudioCfg::default()
        };
        cfg.bands[3] = 2.0;
        assert_eq!(
            chain(&cfg),
            "equalizer=f=250:t=q:w=1.0:g=2,\
             dynaudnorm=f=250:g=15:p=0.875:m=10,\
             alimiter=limit=0.95:attack=5:release=50"
        );
    }

    #[test]
    fn preamp_comes_before_eq() {
        let mut cfg = AudioCfg {
            limiter: false,
            normalize_tracks: false,
            preamp_db: 3.0,
            ..AudioCfg::default()
        };
        cfg.bands[0] = 1.0;
        assert_eq!(
            chain(&cfg),
            "volume=volume=3dB:precision=double,\
             equalizer=f=31:t=q:w=1.0:g=1"
        );
    }

    #[test]
    fn nan_and_overflow_are_sanitized() {
        let cfg = AudioCfg {
            bands: [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                40.0,
                -40.0,
                0.001,
                f32::NAN,
                0.0,
                0.0,
                0.0,
            ],
            preamp_db: f32::INFINITY,
            normalizer: NormalizerMode::Dynamic,
            normalizer_strength: f32::NAN,
            limiter: false,
            ..AudioCfg::default()
        };
        // 40 → 12, -40 → -12, 0.001 se redondea a 0 y desaparece; NaN/inf → 0.
        assert_eq!(
            chain(&cfg),
            "equalizer=f=250:t=q:w=1.0:g=12,\
             equalizer=f=500:t=q:w=1.0:g=-12,\
             dynaudnorm=f=250:g=5:p=0.8:m=10"
        );
        assert_eq!(cfg.sanitized().preamp_db, 0.0);
        assert_eq!(cfg.sanitized().normalizer_strength, 0.0);
    }

    #[test]
    fn presets_are_complete_and_in_range() {
        for (name, bands) in EQ_PRESETS {
            assert_eq!(bands.len(), 10, "preset {name}");
            for gain in bands {
                assert!(gain.is_finite(), "preset {name}");
                assert!((-12.0..=12.0).contains(&gain), "preset {name}");
            }
        }
        assert_eq!(EQ_PRESETS.len(), 5);
        assert_eq!(PRESET_FLAT, [0.0; 10]);
        assert_eq!(BAND_FREQS.len(), BAND_LABELS.len());
        assert_eq!(BAND_FREQS[0], 31);
        assert_eq!(BAND_FREQS[9], 16_000);
    }

    #[test]
    fn config_roundtrip_with_audio_section() {
        let cfg = AudioCfg {
            preamp_db: -3.0,
            bands: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
            normalizer: NormalizerMode::Loudnorm,
            ..AudioCfg::default()
        };
        let text = toml::to_string(&cfg).unwrap();
        assert!(text.contains("normalizer = \"loudnorm\""));
        let back: AudioCfg = toml::from_str(&text).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn partial_audio_section_uses_defaults() {
        let cfg: AudioCfg = toml::from_str("enabled = false\n").unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.eq_enabled);
        assert_eq!(cfg.bands, [0.0; 10]);
        assert_eq!(cfg.normalizer, NormalizerMode::Off);
        assert_eq!(cfg.normalizer_strength, 0.5);
        assert!(cfg.normalize_tracks);
        assert!(!cfg.limiter);
    }
}
