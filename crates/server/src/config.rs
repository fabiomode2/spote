//! Configuración por entorno de `spoted`.

use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::Context;

pub const DEFAULT_BIND: &str = "127.0.0.1:7070";
pub const DEFAULT_DB: &str = "./spote-server.db";
pub const DEFAULT_INVITE: &str = "spote";

#[derive(Debug, Clone)]
pub struct Config {
    /// Dirección de escucha (`SPOTE_BIND`).
    pub bind: SocketAddr,
    /// Fichero SQLite (`SPOTE_DB`).
    pub db_path: PathBuf,
    /// Código de invitación global (`SPOTE_INVITE`).
    pub invite: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Igual que [`Config::from_env`] pero con un lookup inyectable (tests).
    pub fn from_lookup(mut get: impl FnMut(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind_raw = get("SPOTE_BIND").unwrap_or_else(|| DEFAULT_BIND.to_string());
        let bind = bind_raw
            .parse::<SocketAddr>()
            .with_context(|| format!("SPOTE_BIND inválido: {bind_raw}"))?;
        let db_path = PathBuf::from(get("SPOTE_DB").unwrap_or_else(|| DEFAULT_DB.to_string()));
        let invite = get("SPOTE_INVITE")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INVITE.to_string());
        Ok(Self {
            bind,
            db_path,
            invite,
        })
    }
}
