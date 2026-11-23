//! Punto de entrada del binario `spoted`.
//!
//! `spoted keygen [etiqueta…]` imprime secretos listos para pegar en `.env`.

use anyhow::Context;
use spote_server::{config::random_secret, AppState, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // SQLite crea la base de datos (y WAL/SHM) con esta máscara. En Docker el
    // volumen solo lo lee el usuario 10001 del contenedor.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }

    if std::env::args().nth(1).as_deref() == Some("keygen") {
        print_keygen();
        return Ok(());
    }

    init_tracing();

    let config = Config::from_env().context("configuración inválida")?;
    tracing::info!(
        bind = %config.bind,
        db = %config.db_path.display(),
        clients = config.client_keys.len(),
        session_days = config.session_days,
        "spoted arrancando"
    );
    if !config.bind.ip().is_loopback() {
        tracing::warn!(
            bind = %config.bind,
            "spoted escucha fuera de loopback; asegúrate de que nginx/TLS está delante"
        );
    }

    let state = AppState::new(config).context("no se pudo abrir la base de datos")?;
    spote_server::serve(state).await
}

fn print_keygen() {
    let labels: Vec<String> = std::env::args().skip(2).collect();
    let labels = if labels.is_empty() {
        vec!["ana".to_string(), "bea".to_string()]
    } else {
        labels
    };
    let keys = labels
        .iter()
        .map(|label| format!("{label}:{}", random_secret(32)))
        .collect::<Vec<_>>()
        .join(",");
    println!("# Invitación compartida por el grupo (no la clave de cada cliente)");
    println!("SPOTE_INVITE={}", random_secret(24));
    println!("# Una clave por persona; cada usuario queda atado a la suya");
    println!("SPOTE_CLIENT_KEYS={keys}");
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,spote_server=debug,tower_http=info")
    });
    // `try_init` para no petar si el entorno ya instaló un subscriber.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
