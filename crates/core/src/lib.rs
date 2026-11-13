//! Núcleo de spote: todo lo que no es UI.
//!
//! Regla de oro: este crate no sabe que existe una terminal. La TUI consume
//! estas APIs, pero cualquier cliente futuro (web, Android) puede hacer lo mismo.

pub mod artwork;
pub mod audio;
pub mod audio_map;
pub mod catalog;
pub mod config;
pub mod discord;
pub mod downloader;
pub mod harvest;
pub mod library;
pub mod lyrics;
pub mod mpris;
pub mod player;
pub mod record;
pub mod spotify;
pub mod spotify_api;
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
            yt_dlp: util::which("yt-dlp").is_some(),
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
