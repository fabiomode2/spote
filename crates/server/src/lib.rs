//! `spoted`: backend social de spote.
//!
//! Solo coordina usuarios, salas, presencia y chat. Nunca toca audio ni disco
//! más allá de su SQLite. La API pública vive bajo `/api/v1` (REST + WebSocket).

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod service;
pub mod state;
pub mod sync;
pub mod ws;

pub use api::router;
pub use config::Config;
pub use state::AppState;

use anyhow::Context;

/// Arranca el servidor HTTP/WS en la dirección configurada.
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    state.spawn_background();
    let addr = state.config.bind;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("no se pudo escuchar en {addr}"))?;
    tracing::info!(%addr, "spoted escuchando");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("el servidor HTTP terminó con error")?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("apagando spoted");
}
