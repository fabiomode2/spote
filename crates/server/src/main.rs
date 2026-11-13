//! Punto de entrada del binario `spoted`.

use anyhow::Context;
use spote_server::{AppState, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::from_env().context("configuración inválida")?;
    tracing::info!(
        bind = %config.bind,
        db = %config.db_path.display(),
        "spoted arrancando"
    );

    let state = AppState::new(config).context("no se pudo abrir la base de datos")?;
    spote_server::serve(state).await
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
