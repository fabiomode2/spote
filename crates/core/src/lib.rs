//! Núcleo de spote: todo lo que no es UI.
//!
//! Regla de oro: este crate no sabe que existe una terminal. La TUI consume
//! estas APIs, pero cualquier cliente futuro (web, Android) puede hacer lo mismo.

pub mod artwork;
pub mod audio;
pub mod audio_fx;
pub mod audio_map;
pub mod catalog;
pub mod config;
pub mod crypto;
pub mod discord;
pub mod downloader;
#[cfg(feature = "desktop")]
pub mod harvest;
pub mod library;
pub mod lyrics;
#[cfg(feature = "desktop")]
pub mod mpris;
#[cfg(feature = "desktop")]
pub mod mpris_server;
pub mod playback;
pub mod player;
#[cfg(feature = "desktop")]
pub mod record;
#[cfg(feature = "desktop")]
pub mod spotify;
pub mod stream;
pub mod spotify_api;
#[cfg(feature = "desktop")]
pub mod system_media;
pub mod tunebat;
pub mod util;

pub use spote_protocol as protocol;

/// Comprueba que los binarios externos que necesita el núcleo están presentes.
#[derive(Debug, Clone, Default)]
pub struct ToolStatus {
    pub mpv: bool,
    pub ffmpeg: bool,
    pub yt_dlp: bool,
    pub parec: bool,
}

impl ToolStatus {
    pub fn detect() -> Self {
        Self {
            mpv: util::which("mpv").is_some(),
            ffmpeg: util::which("ffmpeg").is_some(),
            yt_dlp: crate::stream::ytdlp_available(),
            parec: util::which("parec").is_some(),
        }
    }

    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.mpv {
            out.push("mpv");
        }
        if !self.ffmpeg {
            out.push("ffmpeg");
        }
        if !self.yt_dlp {
            out.push("yt-dlp");
        }
        out
    }
}
