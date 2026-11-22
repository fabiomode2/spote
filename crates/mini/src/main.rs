//! `spote-mini`: mini reproductor de escritorio conectado por socket Unix.
//!
//! El crate solo implementa el cliente y la ventana. El servidor (la app
//! principal) debe escuchar en `spote_protocol::mini::mini_socket_path()` y
//! hablar NDJSON tal como se documenta en el módulo `mini` del protocolo.

mod app;
mod net;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("spote mini")
        .with_inner_size([420.0, 220.0])
        .with_min_inner_size([420.0, 220.0])
        .with_max_inner_size([420.0, 220.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top()
        .with_transparent(false);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "spote mini",
        options,
        Box::new(|cc| Ok(Box::new(app::MiniApp::new(cc)))),
    )
}
