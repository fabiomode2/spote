//! Pestañas de documentos e historial de navegación.
//!
//! Las secciones fijas siguen viviendo en `Tab`; artista, álbum, playlist y
//! las listas abiertas se apilan en `DocTab`. Cada pestaña guarda su propia
//! pila, así que volver atrás nunca cierra la app ni pierde la selección.

use super::*;

impl App {
    pub fn snapshot(&self) -> ViewSnapshot {
        ViewSnapshot {
            tab: self.tab,
            lib_filter: self.lib_filter,
            selected: self.selected,
            list_start: self.list_start,
            lib_query: self.lib_query.clone(),
            lib_search_focus: self.lib_search_focus,
            album_query: self.album_query.clone(),
            album_search_focus: self.album_search_focus,
            album_selected: self.album_selected,
            detail: self.detail,
            artist: self.artist.clone(),
            artist_art: self.artist_art.clone(),
            artist_selected: self.artist_selected,
            artist_album_scroll: self.artist_album_scroll,
            album_page: self.album_page.clone(),
            album_open: self.album_open.clone(),
            center_focus: self.center_focus,
            focus: self.focus,
            search: self.search.clone(),
        }
    }

    pub fn restore_snapshot(&mut self, s: ViewSnapshot) {
        self.tab = s.tab;
        self.lib_filter = s.lib_filter;
        self.selected = s.selected;
        self.list_start = s.list_start;
        self.lib_query = s.lib_query;
        self.lib_search_focus = s.lib_search_focus;
        self.album_query = s.album_query;
        self.album_search_focus = s.album_search_focus;
        self.album_selected = s.album_selected;
        self.detail = s.detail;
        self.artist = s.artist;
        self.artist_art = s.artist_art;
        self.artist_selected = s.artist_selected;
        self.artist_album_scroll = s.artist_album_scroll;
        self.album_page = s.album_page;
        self.album_open = s.album_open;
        self.center_focus = s.center_focus;
        self.focus = s.focus;
        self.search = s.search;
    }

    /// Empuja una página de documento. `previous` se captura antes de mutar
    /// el estado y solo se usa cuando hay que crear una pestaña nueva.
    pub fn push_doc_page(&mut self, kind: DocKind, title: String, previous: ViewSnapshot) {
        let current = self.snapshot();
        if let Some(index) = self.active_doc {
            if let Some(tab) = self.doc_tabs.get_mut(index) {
                tab.kind = kind;
                tab.title = title;
                tab.pages.push(current);
                tab.forward.clear();
                return;
            }
        }
        self.doc_tabs.push(DocTab {
            kind,
            title,
            base: previous,
            pages: vec![current],
            forward: Vec::new(),
        });
        self.active_doc = Some(self.doc_tabs.len() - 1);
    }

    pub fn current_doc(&self) -> Option<&DocTab> {
        self.active_doc.and_then(|index| self.doc_tabs.get(index))
    }

    fn refresh_active_doc_title(&mut self) {
        let title = if self.detail == Some(Detail::Artist) {
            self.artist
                .as_ref()
                .map(|artist| artist.name.clone())
                .unwrap_or_else(|| "artista".to_string())
        } else if self.detail == Some(Detail::Album) {
            self.album_page
                .as_ref()
                .map(|album| album.title.clone())
                .unwrap_or_else(|| "álbum".to_string())
        } else {
            self.current_doc()
                .map(|tab| tab.title.clone())
                .unwrap_or_default()
        };
        if let Some(index) = self.active_doc {
            if let Some(tab) = self.doc_tabs.get_mut(index) {
                tab.title = title;
            }
        }
    }

    /// Guarda el estado visible en la pestaña activa antes de cambiar de
    /// pestaña o de sección.
    pub(crate) fn save_active_page(&mut self) {
        if let Some(index) = self.active_doc {
            let snapshot = self.snapshot();
            if let Some(tab) = self.doc_tabs.get_mut(index) {
                if let Some(page) = tab.pages.last_mut() {
                    *page = snapshot;
                }
            }
        }
    }

    pub fn activate_doc(&mut self, index: usize) -> bool {
        if index >= self.doc_tabs.len() {
            return false;
        }
        if self.active_doc == Some(index) {
            return true;
        }
        self.save_active_page();
        let snapshot = self
            .doc_tabs
            .get(index)
            .and_then(|tab| tab.pages.last().cloned());
        let Some(snapshot) = snapshot else {
            return false;
        };
        self.active_doc = Some(index);
        self.restore_snapshot(snapshot);
        self.refresh_active_doc_title();
        self.request_redraw();
        true
    }

    pub fn close_doc(&mut self, index: usize) -> bool {
        if index >= self.doc_tabs.len() {
            return false;
        }
        let was_active = self.active_doc == Some(index);
        let tab = self.doc_tabs.remove(index);
        if self.doc_tabs.is_empty() {
            self.active_doc = None;
            if was_active {
                self.restore_snapshot(tab.base);
                self.refresh_view();
            }
            self.request_redraw();
            return true;
        }
        if was_active {
            let next = index.min(self.doc_tabs.len() - 1);
            self.active_doc = Some(next);
            let snapshot = self.doc_tabs[next].pages.last().cloned();
            if let Some(snapshot) = snapshot {
                self.restore_snapshot(snapshot);
                self.refresh_active_doc_title();
            }
            self.request_redraw();
        } else if let Some(active) = self.active_doc {
            // Al cerrar una pestaña anterior, el índice activo se desplaza.
            self.active_doc = Some(if index < active { active - 1 } else { active });
        }
        true
    }

    pub fn close_active_doc(&mut self) {
        if let Some(index) = self.active_doc {
            self.close_doc(index);
        }
    }

    /// Duplica la página actual en una pestaña nueva. Es el `ctrl+enter`
    /// genérico; las vistas pueden ofrecer abrir un destino concreto.
    pub fn duplicate_doc(&mut self) {
        let snapshot = self.snapshot();
        let (kind, title) = self
            .current_doc()
            .map(|tab| (tab.kind, tab.title.clone()))
            .unwrap_or((DocKind::Other, "documento".to_string()));
        self.doc_tabs.push(DocTab {
            kind,
            title,
            base: snapshot.clone(),
            pages: vec![snapshot],
            forward: Vec::new(),
        });
        self.active_doc = Some(self.doc_tabs.len() - 1);
        self.request_redraw();
    }

    pub fn cycle_doc(&mut self, forward: bool) {
        if self.doc_tabs.is_empty() {
            return;
        }
        let current = self.active_doc.unwrap_or(0);
        let next = if forward {
            (current + 1) % self.doc_tabs.len()
        } else {
            (current + self.doc_tabs.len() - 1) % self.doc_tabs.len()
        };
        self.activate_doc(next);
    }

    pub fn nav_back(&mut self) -> bool {
        if let Some(index) = self.active_doc {
            let mut restore = None;
            let mut close = false;
            if let Some(tab) = self.doc_tabs.get_mut(index) {
                if tab.pages.len() > 1 {
                    if let Some(current) = tab.pages.pop() {
                        tab.forward.push(current);
                    }
                    restore = tab.pages.last().cloned();
                } else {
                    close = true;
                }
            }
            if let Some(snapshot) = restore {
                self.restore_snapshot(snapshot);
                self.refresh_active_doc_title();
                self.request_redraw();
                return true;
            }
            if close {
                self.close_doc(index);
                return true;
            }
        }
        if let Some(snapshot) = self.section_back.pop() {
            let current = self.snapshot();
            self.section_forward.push(current);
            self.restore_snapshot(snapshot);
            self.refresh_view();
            self.request_redraw();
            return true;
        }
        false
    }

    pub fn nav_forward(&mut self) -> bool {
        if let Some(index) = self.active_doc {
            let mut target = None;
            if let Some(tab) = self.doc_tabs.get_mut(index) {
                if let Some(next) = tab.forward.pop() {
                    tab.pages.push(next.clone());
                    target = Some(next);
                }
            }
            if let Some(snapshot) = target {
                self.restore_snapshot(snapshot);
                self.refresh_active_doc_title();
                self.request_redraw();
                return true;
            }
        }
        if let Some(snapshot) = self.section_forward.pop() {
            let current = self.snapshot();
            self.section_back.push(current);
            self.restore_snapshot(snapshot);
            self.refresh_view();
            self.request_redraw();
            return true;
        }
        false
    }

    /// Cambia de sección guardando el historial. Si hay un documento abierto,
    /// se guarda su página y se vuelve a la sección sin perder la pestaña.
    pub fn goto_section(&mut self, tab: Tab) {
        if self.active_doc.is_some() {
            self.save_active_page();
            self.active_doc = None;
        } else if self.tab != tab {
            let current = self.snapshot();
            self.section_back.push(current);
            self.section_forward.clear();
        }
        self.tab = tab;
        self.detail = None;
        self.artist = None;
        self.artist_art = None;
        self.album_page = None;
        self.album_open = None;
        self.center_focus = CenterFocus::Normal;
        self.fullscreen = Fullscreen::None;
        self.focus = Pane::Center;
        if tab == Tab::Library {
            self.selected = self.selected.min(self.view.len().saturating_sub(1));
        }
        self.refresh_view();
        self.request_redraw();
    }
}
