//! Proveedor criptográfico de rustls.
//!
//! reqwest 0.13 se compila con `rustls-no-provider` para poder elegir ring en
//! todas las plataformas. En Android aws-lc-rs puede ejecutar trampolines en
//! memoria no ejecutable con páginas de 16 KB y tumbar el proceso, así que
//! instalamos ring una sola vez antes de construir cualquier cliente HTTPS.

use std::sync::OnceLock;

pub fn install() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
