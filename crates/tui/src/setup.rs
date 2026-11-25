//! Enlace `spote://setup` y su QR para configurar clientes.
//!
//! El enlace lleva la URL del servidor, la clave de cliente de esa persona y la
//! invitación del grupo. Es un secreto: por eso el comando que lo imprime lo
//! hace explícitamente (y el QR también), pero nada de esto se registra en los
//! logs de tracing.

use anyhow::{bail, Context, Result};
use qrcode::{EcLevel, QrCode};
use spote_core::config::Config;

/// Esquema y host del enlace. La query va detrás.
pub const SETUP_PREFIX: &str = "spote://setup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupLink {
    pub server: String,
    pub key: String,
    pub invite: String,
    pub name: Option<String>,
}

impl SetupLink {
    /// Valida y normaliza los cuatro campos.
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

    /// Construye el enlace a partir de la config, permitiendo que los
    /// argumentos de la CLI pisen cada campo.
    pub fn from_config(
        config: &Config,
        server: Option<&str>,
        key: Option<&str>,
        invite: Option<&str>,
        name: Option<&str>,
    ) -> Result<Self> {
        let social = &config.server;
        let pick = |flag: Option<&str>, current: &str| {
            flag.map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(current)
                .to_string()
        };
        let server = pick(server, &social.url);
        let key = pick(key, &social.client_key);
        let invite = pick(invite, &social.invite);
        let name = match name.map(str::trim).filter(|value| !value.is_empty()) {
            Some(name) => Some(name),
            None if social.name.trim().is_empty() => None,
            None => Some(social.name.trim()),
        };
        Self::new(&server, &key, &invite, name)
    }

    /// `spote://setup?server=…&key=…&invite=…[&name=…]`.
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

    /// Acepta `spote://setup?...` (y `spote://setup/?...`). Los parámetros
    /// desconocidos se ignoran para que un cliente viejo no rompa enlaces
    /// nuevos; los obligatorios deben venir una sola vez y no vacíos.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let rest = raw
            .strip_prefix(SETUP_PREFIX)
            .with_context(|| format!("el enlace debe empezar por {SETUP_PREFIX}"))?;
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        let query = rest
            .strip_prefix('?')
            .context("al enlace le falta la query de configuración")?;
        let query = query.split('#').next().unwrap_or(query);
        if query.is_empty() {
            bail!("la query del enlace está vacía");
        }

        let mut server = None;
        let mut key = None;
        let mut invite = None;
        let mut name = None;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (raw_key, raw_value) = pair
                .split_once('=')
                .with_context(|| format!("parámetro sin valor en «{pair}»"))?;
            let field = decode(raw_key).context("nombre de parámetro mal codificado")?;
            match field.as_str() {
                "server" => set_once(
                    &mut server,
                    "server",
                    decode(raw_value).context("valor de «server» mal codificado")?,
                )?,
                "key" => set_once(
                    &mut key,
                    "key",
                    decode(raw_value).context("valor de «key» mal codificado")?,
                )?,
                "invite" => set_once(
                    &mut invite,
                    "invite",
                    decode(raw_value).context("valor de «invite» mal codificado")?,
                )?,
                "name" => set_once(
                    &mut name,
                    "name",
                    decode(raw_value).context("valor de «name» mal codificado")?,
                )?,
                _ => {}
            }
        }

        let server = server.context("el enlace no lleva server")?;
        let key = key.context("el enlace no lleva key")?;
        let invite = invite.context("el enlace no lleva invite")?;
        Self::new(&server, &key, &invite, name.as_deref())
    }
}

fn reject_controls(field: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        bail!("{field} contiene caracteres de control");
    }
    Ok(())
}

fn set_once(slot: &mut Option<String>, field: &str, value: String) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("el enlace repite el parámetro «{field}»");
    }
    Ok(())
}

/// Porcentaje estricto: solo quedan tal cual los caracteres no reservados de
/// RFC 3986. El `+` se codifica como `%2B` y al decodificar se acepta como
/// espacio (compatibilidad con formularios), sin ambigüedad.
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

fn decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hi = bytes
                    .get(index + 1)
                    .copied()
                    .and_then(hex_value)
                    .context("porcentaje incompleto o inválido")?;
                let lo = bytes
                    .get(index + 2)
                    .copied()
                    .and_then(hex_value)
                    .context("porcentaje incompleto o inválido")?;
                out.push(hi << 4 | lo);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).context("el enlace no es UTF-8 válido")
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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

/// Imprime el enlace y su QR. Es la única salida que lleva la clave en claro.
pub fn print_link(setup: &SetupLink) -> Result<()> {
    let link = setup.to_link();
    println!("{link}");
    println!();
    println!("{}", render_qr(&link)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spote_core::config::Config;

    const KEY: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const INVITE: &str = "invite-0123456789abcdef";

    #[test]
    fn link_has_the_expected_format() {
        let setup =
            SetupLink::new("https://spote.example.com/", KEY, INVITE, Some("ana")).expect("enlace");
        assert_eq!(
            setup.to_link(),
            format!(
                "spote://setup?server=https%3A%2F%2Fspote.example.com&key={KEY}&invite={INVITE}&name=ana"
            )
        );
    }

    #[test]
    fn parser_and_serializer_roundtrip_with_reserved_characters() {
        let setup = SetupLink::new(
            "https://spote.example.com/spote",
            KEY,
            INVITE,
            Some("Ana Pérez & cía + % ✓"),
        )
        .expect("enlace");
        let link = setup.to_link();
        assert!(!link.contains(' '), "los espacios van codificados");
        assert!(
            link.contains("name=Ana%20P%C3%A9rez%20%26%20c%C3%ADa%20%2B%20%25%20%E2%9C%93"),
            "los valores reservados van codificados: {link}"
        );
        assert_eq!(SetupLink::parse(&link).expect("parse"), setup);
    }

    #[test]
    fn name_is_optional_and_empty_name_is_omitted() {
        let setup = SetupLink::new("http://127.0.0.1:8787", KEY, INVITE, None).expect("enlace");
        let link = setup.to_link();
        assert!(!link.contains("name="));
        assert_eq!(SetupLink::parse(&link).expect("parse").name, None);

        let blank =
            SetupLink::new("http://127.0.0.1:8787", KEY, INVITE, Some("   ")).expect("enlace");
        assert_eq!(blank.name, None);
    }

    #[test]
    fn parse_accepts_trailing_slash_and_ignores_unknown_fields() {
        let link = "spote://setup/?server=https%3A%2F%2Fspote.example.com&key=abc&invite=def&future=%ZZ#frag";
        let setup = SetupLink::parse(link).expect("parse");
        assert_eq!(setup.server, "https://spote.example.com");
        assert_eq!(setup.key, "abc");
        assert_eq!(setup.invite, "def");
    }

    #[test]
    fn parse_rejects_invalid_links() {
        let bad = [
            "",
            "https://spote.example.com",
            "spote://other?server=https%3A%2F%2Fx&key=k&invite=i",
            "spote://setup",
            "spote://setup?server=https%3A%2F%2Fx&invite=i",
            "spote://setup?server=https%3A%2F%2Fx&key=k",
            "spote://setup?key=k&invite=i",
            "spote://setup?server=&key=k&invite=i",
            "spote://setup?server=https%3A%2F%2Fx&key=k&key=j&invite=i",
            "spote://setup?server=https%3A%2F%2Fx&key=%ZZ&invite=i",
            "spote://setup?server=spote.example.com&key=k&invite=i",
            "spote://setup?server=https%3A%2F%2Fx&key=k%0A&invite=i",
            "spote://setup?server=https%3A%2F%2Fx&key=k&invite=i&name=ana%1B%5D",
        ];
        for link in bad {
            assert!(SetupLink::parse(link).is_err(), "debería rechazar «{link}»");
        }
    }

    #[test]
    fn config_is_the_default_source_and_flags_override_fields() {
        let mut config = Config::default();
        config.server.url = "https://spote.example.com".into();
        config.server.client_key = KEY.into();
        config.server.invite = INVITE.into();
        config.server.name = "ana".into();

        let from_config =
            SetupLink::from_config(&config, None, None, None, None).expect("config completa");
        assert_eq!(from_config.name.as_deref(), Some("ana"));
        assert_eq!(from_config.server, "https://spote.example.com");

        let overridden = SetupLink::from_config(
            &config,
            Some("https://otro.example.com"),
            Some("otra-clave"),
            None,
            Some("bea"),
        )
        .expect("flags");
        assert_eq!(overridden.server, "https://otro.example.com");
        assert_eq!(overridden.key, "otra-clave");
        assert_eq!(overridden.invite, INVITE);
        assert_eq!(overridden.name.as_deref(), Some("bea"));
    }

    #[test]
    fn missing_config_fields_are_reported() {
        let config = Config::default();
        let error = SetupLink::from_config(&config, None, None, None, None).expect_err("vacía");
        assert!(error.to_string().contains("servidor"));
    }

    #[test]
    fn qr_uses_unicode_half_blocks_and_a_margin() {
        let setup = SetupLink::new("https://spote.example.com", KEY, INVITE, None).expect("enlace");
        let qr = render_qr(&setup.to_link()).expect("qr");
        assert!(qr.contains('▀') || qr.contains('▄') || qr.contains('█'));
        assert!(qr.lines().count() > 10);
        // El margen: la primera y la última fila solo llevan espacios.
        assert!(qr.lines().next().expect("primera fila").trim().is_empty());
        assert!(qr.lines().last().expect("última fila").trim().is_empty());
    }
}
