//! Configuración por entorno de `spoted`.
//!
//! Los secretos no tienen valores por defecto: si faltan, el servidor no
//! arranca. Es preferible un fallo ruidoso en el despliegue a una API abierta.

use std::{
    collections::{HashMap, HashSet},
    env, fmt, fs,
    net::SocketAddr,
    path::PathBuf,
};

use anyhow::{bail, Context};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const DEFAULT_BIND: &str = "127.0.0.1:7070";
pub const DEFAULT_DB: &str = "./spote-server.db";
pub const MIN_INVITE_CHARS: usize = 16;
pub const MIN_CLIENT_KEY_CHARS: usize = 32;
pub const DEFAULT_SESSION_DAYS: i64 = 30;
const MAX_SECRET_CHARS: usize = 512;

/// Clave de cliente ya procesada. Solo se guarda el SHA-256: el valor en claro
/// vive en el entorno del servidor y en el `config.toml` de cada cliente.
#[derive(Clone)]
pub struct ClientKey {
    pub label: String,
    pub hash: [u8; 32],
}

impl ClientKey {
    /// Identidad estable de la clave. Es lo que se guarda en SQLite para atar
    /// usuarios y sesiones a una clave concreta.
    pub fn fingerprint(&self) -> String {
        hex::encode(self.hash)
    }
}

impl fmt::Debug for ClientKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Nunca imprimir el hash completo en logs de configuración.
        f.debug_struct("ClientKey")
            .field("label", &self.label)
            .field(
                "fingerprint",
                &format_args!("{}…", &self.fingerprint()[..12]),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Dirección de escucha (`SPOTE_BIND`).
    pub bind: SocketAddr,
    /// Fichero SQLite (`SPOTE_DB`).
    pub db_path: PathBuf,
    /// SHA-256 del código de invitación (`SPOTE_INVITE`).
    pub invite_hash: [u8; 32],
    /// Claves de cliente aceptadas (`SPOTE_CLIENT_KEYS`).
    pub client_keys: Vec<ClientKey>,
    /// Duración de una sesión en días (`SPOTE_SESSION_DAYS`).
    pub session_days: i64,
    /// Origen permitido para CORS (`SPOTE_CORS_ORIGIN`). Sin valor, CORS cerrado.
    pub cors_origin: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut values: HashMap<String, String> = env::vars().collect();

        // Docker secrets / ficheros montados: `SPOTE_INVITE_FILE` y
        // `SPOTE_CLIENT_KEYS_FILE`. La variable directa tiene prioridad.
        for (target, file_var) in [
            ("SPOTE_INVITE", "SPOTE_INVITE_FILE"),
            ("SPOTE_CLIENT_KEYS", "SPOTE_CLIENT_KEYS_FILE"),
        ] {
            if values
                .get(target)
                .is_none_or(|value| value.trim().is_empty())
            {
                if let Some(path) = values.get(file_var).cloned() {
                    let raw = fs::read_to_string(&path)
                        .with_context(|| format!("no se pudo leer {file_var}={path}"))?;
                    values.insert(target.to_string(), raw.trim().to_string());
                }
            }
        }

        Self::from_lookup(|key| values.get(key).cloned())
    }

    /// Igual que [`Config::from_env`] pero con un lookup inyectable (tests).
    pub fn from_lookup(mut get: impl FnMut(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind_raw = get("SPOTE_BIND")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BIND.to_string());
        let bind = bind_raw
            .parse::<SocketAddr>()
            .with_context(|| format!("SPOTE_BIND inválido: {bind_raw}"))?;
        let db_path = PathBuf::from(
            get("SPOTE_DB")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_DB.to_string()),
        );

        let invite_raw = get("SPOTE_INVITE")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .context(
                "falta SPOTE_INVITE: define un código de invitación aleatorio de al menos 16 caracteres",
            )?;
        validate_secret("SPOTE_INVITE", &invite_raw, MIN_INVITE_CHARS)?;
        if invite_raw.starts_with("cambia-esto") {
            bail!("SPOTE_INVITE sigue siendo el valor de ejemplo; genera uno aleatorio");
        }

        let keys_raw = get("SPOTE_CLIENT_KEYS")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .context(
                "falta SPOTE_CLIENT_KEYS: define al menos una clave de cliente (formato etiqueta:clave)",
            )?;
        let client_keys = parse_client_keys(&keys_raw)?;
        let invite_hash = sha256(invite_raw.as_bytes());
        if client_keys.iter().any(|key| key.hash == invite_hash) {
            bail!("SPOTE_INVITE no puede ser igual que una clave de cliente");
        }

        let session_days = match get("SPOTE_SESSION_DAYS") {
            Some(value) if !value.trim().is_empty() => value
                .trim()
                .parse::<i64>()
                .with_context(|| format!("SPOTE_SESSION_DAYS inválido: {value}"))?,
            _ => DEFAULT_SESSION_DAYS,
        };
        if !(1..=3650).contains(&session_days) {
            bail!("SPOTE_SESSION_DAYS debe estar entre 1 y 3650");
        }

        let cors_origin = get("SPOTE_CORS_ORIGIN")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(origin) = &cors_origin {
            if !(origin.starts_with("https://") || origin.starts_with("http://"))
                || axum::http::HeaderValue::from_str(origin).is_err()
            {
                bail!("SPOTE_CORS_ORIGIN debe ser un origen http(s) válido");
            }
        }

        Ok(Self {
            bind,
            db_path,
            invite_hash,
            client_keys,
            session_days,
            cors_origin,
        })
    }

    /// Comparación en tiempo constante del código de invitación.
    pub fn verify_invite(&self, candidate: &str) -> bool {
        let candidate = sha256(candidate.trim().as_bytes());
        self.invite_hash.ct_eq(&candidate).into()
    }

    /// Devuelve la huella de la clave si está autorizada.
    pub fn client_key_fingerprint(&self, candidate: &str) -> Option<String> {
        let candidate = sha256(candidate.as_bytes());
        let mut found = None;
        for key in &self.client_keys {
            if key.hash.ct_eq(&candidate).into() {
                found = Some(key.fingerprint());
            }
        }
        found
    }

    pub fn client_key_label(&self, fingerprint: &str) -> Option<&str> {
        self.client_keys
            .iter()
            .find(|key| key.fingerprint() == fingerprint)
            .map(|key| key.label.as_str())
    }
}

fn parse_client_keys(raw: &str) -> anyhow::Result<Vec<ClientKey>> {
    let mut keys = Vec::new();
    for (index, entry) in raw
        .split([',', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .enumerate()
    {
        let (label, value) = match entry.split_once(':') {
            Some((label, value)) => (label.trim(), value.trim()),
            None => ("", entry),
        };
        let label = if label.is_empty() {
            format!("client-{}", index + 1)
        } else {
            label.to_string()
        };
        if label.len() > 32
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("etiqueta de cliente inválida: {label:?} (usa [a-z0-9_-], máx. 32)");
        }
        validate_secret(
            &format!("SPOTE_CLIENT_KEYS[{label}]"),
            value,
            MIN_CLIENT_KEY_CHARS,
        )?;

        let hash = sha256(value.as_bytes());
        if keys.iter().any(|key: &ClientKey| key.hash == hash) {
            bail!("SPOTE_CLIENT_KEYS contiene la misma clave dos veces");
        }
        if keys.iter().any(|key: &ClientKey| key.label == label) {
            bail!("SPOTE_CLIENT_KEYS contiene la etiqueta {label:?} dos veces");
        }
        keys.push(ClientKey { label, hash });
    }
    if keys.is_empty() {
        bail!("SPOTE_CLIENT_KEYS no contiene ninguna clave válida");
    }
    Ok(keys)
}

fn validate_secret(name: &str, value: &str, min_chars: usize) -> anyhow::Result<()> {
    if value.chars().count() < min_chars {
        bail!("{name} debe tener al menos {min_chars} caracteres");
    }
    if value.chars().count() > MAX_SECRET_CHARS {
        bail!("{name} es demasiado largo (máx. {MAX_SECRET_CHARS})");
    }
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("{name} no puede contener espacios ni caracteres de control");
    }
    let unique: HashSet<char> = value.chars().collect();
    if unique.len() < 6 {
        bail!("{name} parece débil: usa un secreto aleatorio, no un patrón repetido");
    }
    Ok(())
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Secreto aleatorio de `bytes` bytes en hexadecimal.
pub fn random_secret(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    hex::encode(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: &str = "invite-de-prueba-0123456789";
    const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const KEY_B: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1";

    fn config(extra: impl Fn(&str) -> Option<String>) -> anyhow::Result<Config> {
        Config::from_lookup(|key| match key {
            "SPOTE_INVITE" => Some(INVITE.to_string()),
            "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_B}")),
            other => extra(other),
        })
    }

    #[test]
    fn requires_secrets() {
        assert!(Config::from_lookup(|_| None).is_err());
        assert!(Config::from_lookup(|key| match key {
            "SPOTE_INVITE" => Some(INVITE.to_string()),
            _ => None,
        })
        .is_err());
    }

    #[test]
    fn parses_keys_and_checks_them() {
        let cfg = config(|_| None).expect("config");
        assert_eq!(cfg.client_keys.len(), 2);
        assert_eq!(
            cfg.client_key_label(&cfg.client_key_fingerprint(KEY_A).unwrap()),
            Some("ana")
        );
        assert!(cfg.verify_invite(INVITE));
        assert!(!cfg.verify_invite("otra-invitacion-larga"));
        assert!(cfg.client_key_fingerprint(KEY_B).is_some());
        assert!(cfg.client_key_fingerprint("cccc").is_none());
    }

    #[test]
    fn rejects_weak_or_duplicate_keys() {
        let weak = Config::from_lookup(|key| match key {
            "SPOTE_INVITE" => Some(INVITE.to_string()),
            "SPOTE_CLIENT_KEYS" => Some("ana:corta".to_string()),
            _ => None,
        });
        assert!(weak.is_err());

        let duplicate = Config::from_lookup(|key| match key {
            "SPOTE_INVITE" => Some(INVITE.to_string()),
            "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_A}")),
            _ => None,
        });
        assert!(duplicate.is_err());
    }
}
