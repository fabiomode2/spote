//! Enlace `spote://setup` que imprime `spoted qr`.
//!
//! El formato es el mismo que entiende `spote setup "<enlace>"` en la TUI:
//! `server`, `key` e `invite` obligatorios y `name` opcional, con los valores
//! codificados en porcentaje (RFC 3986). El enlace lleva la clave de cliente:
//! solo se imprime por stdout cuando se pide explícitamente.

use anyhow::{bail, Context, Result};
use qrcode::{EcLevel, QrCode};

pub const SETUP_PREFIX: &str = "spote://setup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupLink {
    pub server: String,
    pub key: String,
    pub invite: String,
    pub name: Option<String>,
}

impl SetupLink {
    pub fn new(server: &str, key: &str, invite: &str, name: Option<&str>) -> Result<Self> {
        reject_controls("la URL del servidor", server)?;
        reject_controls("la clave de cliente", key)?;
        reject_controls("la invitación", invite)?;
        if let Some(name) = name {
            reject_controls("el nombre", name)?;
        }
        let server = server.trim().trim_end_matches('/');
        let key = key.trim();
        let invite = invite.trim();
        if server.is_empty() {
            bail!("falta la URL del servidor");
        }
        if !(server.starts_with("https://") || server.starts_with("http://")) {
            bail!("la URL del servidor debe empezar por http:// o https://");
        }
        if key.is_empty() {
            bail!("falta la clave de cliente");
        }
        if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
            bail!("la clave de cliente no es válida");
        }
        if invite.is_empty() {
            bail!("falta la invitación");
        }
        if invite.chars().any(|c| c.is_control() || c.is_whitespace()) {
            bail!("la invitación no es válida");
        }
        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        Ok(Self {
            server: server.to_string(),
            key: key.to_string(),
            invite: invite.to_string(),
            name,
        })
    }

    pub fn to_link(&self) -> String {
        let mut link = format!(
            "{SETUP_PREFIX}?server={}&key={}&invite={}",
            encode(&self.server),
            encode(&self.key),
            encode(&self.invite)
        );
        if let Some(name) = &self.name {
            link.push_str("&name=");
            link.push_str(&encode(name));
        }
        link
    }
}

fn reject_controls(field: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        bail!("{field} contiene caracteres de control");
    }
    Ok(())
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// QR en bloques Unicode (`▀`, `▄`, `█`) con margen de 4 módulos.
pub fn render_qr(data: &str) -> Result<String> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M)
        .context("el enlace no cabe en un QR")?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .module_dimensions(1, 1)
        .build())
}

/// Imprime el enlace y su QR por stdout.
pub fn print_link(link: &SetupLink) -> Result<()> {
    let data = link.to_link();
    println!("{data}");
    println!();
    println!("{}", render_qr(&data)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const INVITE: &str = "invite-0123456789abcdef";

    #[test]
    fn link_has_the_expected_format() {
        let link =
            SetupLink::new("https://spote.example.com/", KEY, INVITE, Some("ana")).expect("enlace");
        assert_eq!(
            link.to_link(),
            format!(
                "spote://setup?server=https%3A%2F%2Fspote.example.com&key={KEY}&invite={INVITE}&name=ana"
            )
        );
    }

    #[test]
    fn name_is_optional_and_reserved_characters_are_encoded() {
        let no_name = SetupLink::new("http://127.0.0.1:8787", KEY, INVITE, None).expect("enlace");
        assert!(!no_name.to_link().contains("name="));

        let with_name = SetupLink::new(
            "https://spote.example.com/spote",
            KEY,
            INVITE,
            Some("Ana Pérez & cía + %"),
        )
        .expect("enlace");
        let link = with_name.to_link();
        assert!(link.contains("name=Ana%20P%C3%A9rez%20%26%20c%C3%ADa%20%2B%20%25"));
        assert!(!link.contains(' '));
    }

    #[test]
    fn rejects_invalid_values() {
        let bad = [
            ("", KEY, INVITE),
            ("spote.example.com", KEY, INVITE),
            ("ftp://spote.example.com", KEY, INVITE),
            ("https://spote.example.com", "", INVITE),
            ("https://spote.example.com", "clave con espacios", INVITE),
            ("https://spote.example.com", KEY, ""),
            ("https://spote.example.com", KEY, "invite\n"),
        ];
        for (server, key, invite) in bad {
            assert!(
                SetupLink::new(server, key, invite, None).is_err(),
                "debería rechazar {server:?} {key:?} {invite:?}"
            );
        }
    }

    #[test]
    fn qr_uses_unicode_half_blocks() {
        let link = SetupLink::new("https://spote.example.com", KEY, INVITE, None).expect("enlace");
        let qr = render_qr(&link.to_link()).expect("qr");
        assert!(qr.contains('▀') || qr.contains('▄') || qr.contains('█'));
        assert!(qr.lines().count() > 10);
    }
}
