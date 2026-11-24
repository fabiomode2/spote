//! Búsqueda manual de letras: candidatos, previsualización y aceptación.
use super::*;

impl App {
    pub fn open_lyrics_search(&mut self) {
        let Some(track) = self.now.track.clone() else {
            self.toast("no hay nada sonando", ToastKind::Warn);
            return;
        };
        let query = format!("{} {}", track.title, track.artist);
        self.tab = Tab::Now;
        self.center_focus = CenterFocus::Lyrics;
        self.lyrics_search = Some(LyricsSearchState {
            query,
            ..LyricsSearchState::default()
        });
        self.overlay = Overlay::None;
        self.needs_redraw = true;
        self.run_lyrics_search();
    }

    pub fn run_lyrics_search(&mut self) {
        let Some(state) = self.lyrics_search.as_mut() else {
            return;
        };
        let query = state.query.trim().to_string();
        if query.is_empty() {
            state.status = "escribe una búsqueda".into();
            return;
        }
        state.query = query.clone();
        state.last_search_query = query.clone();
        state.loading = true;
        state.status = "buscando posibles letras…".into();
        state.results.clear();
        state.selected = 0;
        state.preview_scroll = 0;
        state.query_id = self.lyrics_search_worker.request(query);
    }

    pub fn close_lyrics_search(&mut self) {
        self.lyrics_search = None;
        self.needs_redraw = true;
    }

    pub fn select_lyrics_candidate(&mut self, index: usize) {
        if let Some(state) = self.lyrics_search.as_mut() {
            if state.results.is_empty() {
                return;
            }
            state.selected = index.min(state.results.len() - 1);
            state.preview_scroll = 0;
        }
    }

    /// Teclado de la búsqueda. Mientras está abierta captura todo para que el
    /// texto de la consulta no dispare atajos globales.
    pub fn handle_lyrics_search_key(&mut self, key: KeyEvent) -> bool {
        let Some(state) = self.lyrics_search.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => self.close_lyrics_search(),
            KeyCode::Backspace => {
                state.query.pop();
                state.status.clear();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.query.clear();
                state.status.clear();
            }
            KeyCode::Enter => {
                if state.query.trim() != state.last_search_query || state.results.is_empty() {
                    self.run_lyrics_search();
                } else {
                    self.accept_lyrics_candidate();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.selected > 0 {
                    state.selected -= 1;
                    state.preview_scroll = 0;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !state.results.is_empty() {
                    state.selected = (state.selected + 1).min(state.results.len() - 1);
                    state.preview_scroll = 0;
                }
            }
            KeyCode::PageUp => state.preview_scroll = state.preview_scroll.saturating_sub(8),
            KeyCode::PageDown => state.preview_scroll += 8,
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                if !c.is_control() {
                    state.query.push(c);
                    state.status.clear();
                }
            }
            _ => {}
        }
        self.needs_redraw = true;
        true
    }

    pub fn accept_lyrics_candidate(&mut self) {
        let Some(track) = self.now.track.clone() else {
            return;
        };
        let Some(state) = self.lyrics_search.as_ref() else {
            return;
        };
        let Some(candidate) = state.results.get(state.selected).cloned() else {
            self.toast("no hay ninguna letra seleccionada", ToastKind::Warn);
            return;
        };
        let (lrc, synced) =
            if let Some(lrc) = candidate.lrc.clone().filter(|lrc| !lrc.trim().is_empty()) {
                (lrc, true)
            } else if let Some(plain) = candidate
                .plain
                .clone()
                .filter(|plain| !plain.trim().is_empty())
            {
                let duration = if self.now.duration_ms > 0 {
                    self.now.duration_ms
                } else {
                    candidate.duration_ms
                };
                (plain_to_lrc(&plain, duration), false)
            } else {
                self.toast("esa candidata no tiene texto", ToastKind::Warn);
                return;
            };
        let mut doc = LyricsDoc::from_lrc(&lrc, synced);
        if doc.is_empty() {
            self.toast("la letra elegida está vacía", ToastKind::Warn);
            return;
        }
        doc.synced = synced;
        doc.aligned = false;
        doc.offset_ms = 0;
        doc.source = spote_core::lyrics::LyricsSource::Manual;
        self.lib
            .save_lyrics(&track.key, &lrc, synced, "manual", false, false)
            .ok();
        self.lib.set_lyrics_offset(&track.key, 0).ok();
        self.lyrics = Some(doc);
        self.lyrics_key = track.key.clone();
        self.lyrics_status = "manual".into();
        self.lyrics_offset_ms = 0;
        self.lyrics_auto_offset = None;
        self.lyrics_retry.remove(&track.key);
        self.reset_lyrics_view();
        self.lyrics_search = None;
        self.toast("letra aceptada a mano", ToastKind::Ok);
    }

    pub(super) fn poll_lyrics_search(&mut self) {
        let mut latest = None;
        while let Some(message) = self.lyrics_search_worker.try_recv() {
            latest = Some(message);
        }
        let Some(crate::fetch::LyricsSearchMsg::Results {
            query_id,
            query,
            result,
        }) = latest
        else {
            return;
        };
        let Some(state) = self.lyrics_search.as_mut() else {
            return;
        };
        if state.query_id != query_id {
            return;
        }
        state.loading = false;
        if !state.query.trim().eq_ignore_ascii_case(&query) {
            // El usuario siguió escribiendo: no pisar la consulta nueva.
            return;
        }
        match result {
            Ok(results) if results.is_empty() => {
                state.results.clear();
                state.selected = 0;
                state.status = "sin resultados · prueba con otro título o artista".into();
            }
            Ok(results) => {
                state.status = format!("{} versiones encontradas", results.len());
                state.results = results;
                state.selected = 0;
                state.preview_scroll = 0;
            }
            Err(error) => {
                state.results.clear();
                state.status = format!("no pude buscar: {error}");
            }
        }
    }
}

/// Convierte una letra plana en LRC repartiendo las líneas sobre la duración.
fn plain_to_lrc(text: &str, duration_ms: u64) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let count = lines.len() as u64;
    let total = if duration_ms > 0 {
        duration_ms
    } else {
        count * 4_000
    };
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        let start = index as u64 * total / count;
        out.push_str(&format!("[{}] {line}\n", crate::fetch::lrc_stamp(start)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lrc_never_writes_sixty_seconds() {
        // 59,999 ms / 2 líneas => la primera empieza en 0 y la segunda en
        // 29,999; el redondeo de la última línea no debe producir 60 s.
        let lrc = plain_to_lrc("uno\ndos", 119_999);
        assert!(!lrc.contains(":60."), "{lrc}");
        assert!(lrc.contains("[01:00.00]"), "{lrc}");
        let doc = spote_core::lyrics::parse_lrc(&lrc);
        assert!(doc.lines.len() >= 2, "{lrc}");
    }
}
