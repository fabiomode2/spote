//! Pantalla de álbumes: agrupa la biblioteca por artista+álbum, abre el
//! tracklist y permite bajar el álbum entero o guardarlo como lista.

use super::*;
use spote_core::library::AlbumRow;

impl App {
    /// `O`: abrir el álbum de la fila seleccionada (biblioteca, búsqueda,
    /// discos o canción actual).
    pub(super) fn open_album_from_selection(&mut self) {
        if self.tab == Tab::Search {
            if let Some(SearchEntry::Album(index)) = self.search_entry(self.search.selected) {
                if let Some(album) = self.search.albums.get(index).cloned() {
                    self.open_album_summary(&album);
                    return;
                }
            }
        }
        if self.tab == Tab::Albums {
            self.open_album_selected();
            return;
        }
        if let Some(row) = self.selected_row().cloned() {
            if let Some(album) = row.track.album.clone() {
                let artist = row
                    .album_artist
                    .clone()
                    .unwrap_or_else(|| crate::app::credits::row_primary_artist(&row));
                self.open_album_by_name(&album, &artist);
                return;
            }
        }
        if let Some(track) = self.now.track.clone() {
            if let Some(album) = track.album {
                self.open_album_by_name(&album, &track.artist);
            }
        }
    }

    pub(super) fn open_album_selected(&mut self) {
        let Some(album) = self.albums.get(self.album_selected).cloned() else {
            return;
        };
        // La ficha se abre como página del historial, no como un estado
        // aparte de la pestaña `discos`: así “atrás” vuelve a la lista.
        self.open_album_by_name(&album.album, &album.artist);
    }

    /// Pide confirmación para quitar el disco seleccionado de la biblioteca.
    pub(super) fn open_delete_album(&mut self) {
        let album = self
            .album_open
            .clone()
            .or_else(|| self.albums.get(self.album_selected).cloned());
        let Some(album) = album else {
            self.toast("elige un disco para quitarlo", ToastKind::Warn);
            return;
        };
        self.open_delete_album_named(&album.album, &album.artist);
    }

    pub(super) fn open_delete_album_named(&mut self, album: &str, artist: &str) {
        let tracks = self
            .lib
            .album_tracks(album, artist)
            .map(|rows| rows.len())
            .unwrap_or(0);
        if tracks == 0 {
            self.toast("ese disco ya no está en la biblioteca", ToastKind::Warn);
            return;
        }
        self.overlay = Overlay::ConfirmDeleteAlbum {
            album: album.to_string(),
            artist: artist.to_string(),
            tracks,
        };
    }

    /// `x` en el tracklist: quita la pista seleccionada de la biblioteca.
    pub(super) fn remove_album_track_selected(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            self.toast("no hay una pista seleccionada", ToastKind::Warn);
            return;
        };
        if self.hide_library_keys(std::slice::from_ref(&row.track.key)) > 0 {
            self.toast(
                format!("«{}» fuera de la biblioteca", row.track.title),
                ToastKind::Ok,
            );
        } else {
            self.toast("esa pista ya no estaba en la biblioteca", ToastKind::Warn);
        }
    }

    /// `x` en la ficha remota de un álbum: quita la pista si está en la
    /// biblioteca. Si solo es un resultado de catálogo, no hay nada que hacer.
    pub(super) fn remove_album_page_track(&mut self) {
        let Some(page) = self.album_page.clone() else {
            return;
        };
        let Some(track) = page.tracks.get(page.selected).cloned() else {
            return;
        };
        if self.lib.track_by_key(&track.key).ok().flatten().is_none() {
            self.toast("esa pista aún no está en la biblioteca", ToastKind::Warn);
            return;
        }
        if self.hide_library_keys(std::slice::from_ref(&track.key)) > 0 {
            self.toast(
                format!("«{}» fuera de la biblioteca", track.title),
                ToastKind::Ok,
            );
        } else {
            self.toast("esa pista ya no estaba en la biblioteca", ToastKind::Warn);
        }
    }

    /// Abre el álbum por nombre desde cualquier nombre clicable de la UI.
    pub(super) fn open_album_by_name(&mut self, album: &str, artist: &str) {
        let previous = self.snapshot();
        let title = album.to_string();
        // Se consulta la biblioteca completa: `self.albums` puede estar
        // filtrado por el buscador de discos y dejar fuera justo el que se
        // acaba de pulsar.
        let all = self.lib.albums().unwrap_or_else(|_| self.albums.clone());
        let wanted_album = spote_core::util::normalize(album);
        let wanted_artist = spote_core::util::normalize(artist);
        // El mismo disco aparece a veces con el artista escrito de dos
        // formas («Biberon» y «Bíberon»): para agrupar se compara sin
        // acentos ni mayúsculas. Si no se hiciera, abrir la pista descargada
        // enseñaba solo esa y escondía el resto del álbum.
        let matching: Vec<AlbumRow> = all
            .into_iter()
            .filter(|row| {
                spote_core::util::normalize(&row.album) == wanted_album
                    && artists_match(
                        &wanted_artist,
                        &spote_core::util::normalize(&row.artist),
                    )
            })
            .collect();
        let primary = matching
            .iter()
            .max_by_key(|row| (row.track_count, row.ready_count))
            .cloned();
        if let Some(row) = primary {
            let album_name = row.album.clone();
            let artist_name = row.artist.clone();
            let mut rows = Vec::new();
            for variant in &matching {
                rows.extend(
                    self.lib
                        .album_tracks(&variant.album, &variant.artist)
                        .unwrap_or_default(),
                );
            }
            rows.sort_by_key(|track| track.id);
            let rows = dedupe_album_rows(rows);
            let tracks = rows.into_iter().map(|track| track.track).collect();
            self.album_page = Some(AlbumPage {
                provider: "local".into(),
                id: format!("{artist_name}:{album_name}"),
                title: album_name,
                artist: artist_name,
                cover_url: row.art_url,
                cover_path: row.art_path,
                release_date: None,
                tracks,
                selected: 0,
            });
            self.album_page_loading = false;
            self.album_page_status.clear();
            self.detail = Some(Detail::Album);
            self.overlay = Overlay::None;
            // La biblioteca local solo conoce lo descargado. Se pide la ficha
            // al catálogo para completar el tracklist: si hay una sola pista
            // bajada de un disco de diez, el panel no debe enseñar solo esa.
            // Si no hay red, la lista local se queda como está (sin errores).
            // Se usa el artista agrupado en la biblioteca, no el del enlace:
            // el disco local ya está identificado y no hace falta adivinar.
            self.meta_worker.album_search(row.album.clone(), row.artist.clone());
        } else {
            // No está descargado: se busca la ficha en el catálogo, igual que
            // al pulsar un álbum en los resultados de búsqueda.
            self.album_page = Some(AlbumPage {
                provider: "search".into(),
                id: album.to_string(),
                title: album.to_string(),
                artist: artist.to_string(),
                cover_url: None,
                cover_path: None,
                release_date: None,
                tracks: Vec::new(),
                selected: 0,
            });
            self.album_page_loading = true;
            self.album_page_status = format!("buscando «{album}»…");
            self.meta_worker
                .album_search(album.to_string(), artist.to_string());
            self.detail = Some(Detail::Album);
            self.overlay = Overlay::None;
        }
        self.push_doc_page(DocKind::Album, title, previous);
    }

    /// Abre un álbum del catálogo remoto (resultado de búsqueda o artista).
    pub(super) fn open_album_summary(&mut self, album: &spote_core::spotify_api::AlbumSummary) {
        let previous = self.snapshot();
        let title = album.title.clone();
        self.album_page = Some(AlbumPage {
            provider: album.provider.clone(),
            id: album.id.clone(),
            title: album.title.clone(),
            artist: album.artist.clone(),
            cover_url: album.cover_url.clone(),
            cover_path: None,
            release_date: album.release_year.map(|year| year.to_string()),
            tracks: Vec::new(),
            selected: 0,
        });
        self.album_page_loading = true;
        self.album_page_status = format!("cargando {}…", album.title);
        self.detail = Some(Detail::Album);
        self.overlay = Overlay::None;
        self.meta_worker
            .album(album.provider.clone(), album.id.clone());
        self.push_doc_page(DocKind::Album, title, previous);
    }

    pub(super) fn open_album_info(&mut self, info: spote_core::spotify_api::AlbumInfo) {
        let tracks = info
            .tracks
            .iter()
            .map(super::artist::spotify_track_ref)
            .map(|track| self.localize_track(track))
            .collect();
        self.album_page = Some(AlbumPage {
            provider: info.provider,
            id: info.id,
            title: info.title,
            artist: info.artist,
            cover_url: info.cover_url,
            cover_path: None,
            release_date: info.release_date,
            tracks,
            selected: 0,
        });
        self.album_page_loading = false;
        self.album_page_status.clear();
        self.detail = Some(Detail::Album);
        self.overlay = Overlay::None;
        self.save_active_page();
    }

    /// Reemplaza una ficha local por la del catálogo conservando las pistas
    /// que ya estaban descargadas. El catálogo manda en el orden, la portada y
    /// los metadatos; lo que exista en disco se reconoce por clave o por
    /// artista+título para que el ✓ de descargada siga saliendo.
    pub(super) fn complete_local_album(
        &self,
        mut page: AlbumPage,
        info: spote_core::spotify_api::AlbumInfo,
    ) -> AlbumPage {
        let local_tracks = std::mem::take(&mut page.tracks);
        let remote: Vec<TrackRef> = info
            .tracks
            .iter()
            .map(super::artist::spotify_track_ref)
            .collect();
        // El catálogo manda en el orden. Las pistas locales que no aparezcan
        // en él se añaden al final en vez de desaparecer.
        let tracks = merge_album_tracks(&remote, &local_tracks);
        page.provider = info.provider;
        page.id = info.id;
        page.title = info.title;
        page.artist = info.artist;
        page.cover_url = info.cover_url.or(page.cover_url);
        page.release_date = info.release_date.or(page.release_date);
        page.selected = page.selected.min(tracks.len().saturating_sub(1));
        page.tracks = tracks;
        page
    }

    /// Cambia la clave de una pista del catálogo por la de su copia local
    /// descargada (si existe). Así el tracklist marca ✓ y reproduce el
    /// fichero de disco en vez de resolver otro stream.
    fn localize_track(&self, track: TrackRef) -> TrackRef {
        let local = self
            .lib
            .track_by_key(&track.key)
            .ok()
            .flatten()
            .filter(TrackRow::playable)
            .or_else(|| {
                self.lib
                    .find_by_meta(&track.artist, &track.title, track.duration_ms)
                    .ok()
                    .flatten()
                    .filter(TrackRow::playable)
            });
        match local {
            Some(row) => TrackRef {
                key: row.track.key,
                ..track
            },
            None => track,
        }
    }

    pub(crate) fn request_album_page_art(&mut self, page: &AlbumPage, w: u16, h: u16) {
        let key = format!("albumpage:{}:{}", page.provider, page.id);
        let slot = (key.clone(), w, h);
        if self.thumbs.contains_key(&slot) || self.thumb_requests.contains(&slot) {
            return;
        }
        self.thumb_requests.insert(slot);
        self.art_worker.request(crate::fetch::ArtRequest {
            key,
            url: page.cover_url.clone(),
            local: page.cover_path.clone(),
            prefer_url: true,
            w,
            h,
            mode: ArtMode::Plain,
            kind: crate::fetch::ArtKind::Thumb,
        });
    }

    pub(super) fn close_album_page(&mut self) {
        self.close_detail();
    }

    pub(super) fn handle_album_key(&mut self, key: KeyEvent) -> bool {
        let len = self
            .album_page
            .as_ref()
            .map(|page| page.tracks.len())
            .unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => self.close_detail(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(page) = &mut self.album_page {
                    page.selected = page.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(page) = &mut self.album_page {
                    if len > 0 {
                        page.selected = (page.selected + 1).min(len - 1);
                    }
                }
            }
            KeyCode::Enter => self.play_album_page_track(),
            KeyCode::Char('a') => {
                if let Some(track) = self
                    .album_page
                    .as_ref()
                    .and_then(|page| page.tracks.get(page.selected))
                    .cloned()
                {
                    self.queue_track(track, None);
                }
            }
            KeyCode::Char('g') => {
                if let Some(artist) = self.album_page.as_ref().map(|page| page.artist.clone()) {
                    self.open_artist_by_name(&artist);
                }
            }
            KeyCode::Char('d') => self.download_album_page(),
            KeyCode::Char('s') => self.save_album_page_playlist(),
            KeyCode::Char('x') => self.remove_album_page_track(),
            KeyCode::Char('X') => {
                if let Some(page) = &self.album_page {
                    if page.provider == "local" {
                        let (album, artist) = (page.title.clone(), page.artist.clone());
                        self.open_delete_album_named(&album, &artist);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn play_album_page_track(&mut self) {
        let Some(page) = self.album_page.clone() else {
            return;
        };
        let Some(track) = page.tracks.get(page.selected).cloned() else {
            return;
        };
        let entries: Vec<QueueEntry> = page
            .tracks
            .iter()
            .map(|track| {
                let path = self.lib.local_path(&track.key).filter(|path| path.exists());
                QueueEntry {
                    track: track.clone(),
                    path,
                }
            })
            .collect();
        self.set_context(entries, page.selected);
        self.play_external_track(track);
    }

    /// Reproduce una pista que puede no estar descargada. Es el mismo camino
    /// que el top de artistas: si hay fichero local suena ya; si no, se resuelve.
    pub(super) fn play_external_track(&mut self, track: TrackRef) {
        self.lib.upsert_track(&track).ok();
        if let Some(url) = &track.art_url {
            self.lib.set_art(&track.key, None, Some(url)).ok();
        }
        // Si no hay fichero local, `play_local` resuelve un stream o descarga.
        self.play_local(track, None);
    }

    pub(super) fn download_album_page(&mut self) {
        let Some(page) = self.album_page.clone() else {
            return;
        };
        let mut queued = 0;
        for track in page.tracks {
            if self
                .lib
                .local_path(&track.key)
                .is_some_and(|path| path.exists())
            {
                continue;
            }
            self.lib.upsert_track(&track).ok();
            if let Some(url) = &track.art_url {
                self.lib.set_art(&track.key, None, Some(url)).ok();
            }
            self.download_track(&track);
            queued += 1;
        }
        self.toast(
            if queued == 0 {
                "el álbum ya está descargado".to_string()
            } else {
                format!("{queued} pistas en descarga")
            },
            ToastKind::Info,
        );
    }

    pub(super) fn save_album_page_playlist(&mut self) {
        let Some(page) = self.album_page.clone() else {
            return;
        };
        if page.tracks.is_empty() {
            self.toast("el álbum no tiene pistas", ToastKind::Warn);
            return;
        }
        let name = format!("{} — {}", page.artist, page.title);
        let id = match self.lib.create_playlist_full(
            &name,
            &format!("álbum · {} pistas", page.tracks.len()),
            None,
            page.cover_url.as_deref(),
        ) {
            Ok(id) => id,
            Err(err) => {
                self.toast(format!("no pude crear la lista: {err}"), ToastKind::Error);
                return;
            }
        };
        for track in &page.tracks {
            self.lib.upsert_track(track).ok();
            self.lib.add_to_playlist(id, &track.key).ok();
        }
        self.toast(format!("«{name}» guardado como lista"), ToastKind::Ok);
    }

    pub(super) fn close_album(&mut self) {
        self.album_open = None;
        self.view.clear();
        self.selected = 0;
        self.refresh_view();
    }

    pub(super) fn move_album_selection(&mut self, delta: i32) {
        if self.album_open.is_some() {
            self.move_selection(delta);
            return;
        }
        if self.albums.is_empty() {
            return;
        }
        let next =
            (self.album_selected as i32 + delta).clamp(0, self.albums.len() as i32 - 1) as usize;
        self.album_selected = next;
    }

    /// Baja todas las pistas que falten del álbum seleccionado.
    pub(super) fn download_album(&mut self) {
        let album = self
            .album_open
            .clone()
            .or_else(|| self.albums.get(self.album_selected).cloned());
        let Some(album) = album else { return };
        let tracks = self
            .lib
            .album_tracks(&album.album, &album.artist)
            .unwrap_or_default();
        let mut queued = 0usize;
        for row in &tracks {
            if row.playable() {
                continue;
            }
            self.lib.upsert_track(&row.track).ok();
            if let Some(url) = &row.track.art_url {
                self.lib.set_art(&row.track.key, None, Some(url)).ok();
            }
            self.download_track(&row.track);
            queued += 1;
        }
        if queued == 0 {
            self.toast("el álbum ya está descargado", ToastKind::Info);
        } else {
            self.toast(
                format!("{} · {} pistas en descarga", album.album, queued),
                ToastKind::Ok,
            );
        }
    }

    /// Guarda el álbum como una lista local con todas sus pistas.
    pub(super) fn save_album_playlist(&mut self) {
        let album = self
            .album_open
            .clone()
            .or_else(|| self.albums.get(self.album_selected).cloned());
        let Some(album) = album else { return };
        let tracks = self
            .lib
            .album_tracks(&album.album, &album.artist)
            .unwrap_or_default();
        if tracks.is_empty() {
            self.toast("el álbum no tiene pistas", ToastKind::Warn);
            return;
        }
        let name = format!("{} — {}", album.artist, album.album);
        let description = format!("álbum · {} pistas", tracks.len());
        let cover_url = album.art_url.as_deref();
        let id = match self
            .lib
            .create_playlist_full(&name, &description, None, cover_url)
        {
            Ok(id) => id,
            Err(err) => {
                self.toast(format!("no pude crear la lista: {err}"), ToastKind::Error);
                return;
            }
        };
        for row in &tracks {
            self.lib.add_to_playlist(id, &row.track.key).ok();
        }
        self.lib_filter = LibFilter::Playlist(id);
        self.tab = Tab::Library;
        self.selected = 0;
        self.refresh_view();
        self.toast(format!("«{name}» guardado como lista"), ToastKind::Ok);
    }

    /// Enter en la lista de álbumes o en el tracklist.
    pub(super) fn album_enter(&mut self) {
        if self.album_open.is_some() {
            if let Some(row) = self.selected_row().cloned() {
                self.play_local(row.track, row.path);
            }
        } else {
            self.open_album_selected();
        }
    }

    /// Portada sintética del álbum para pedir la miniatura al worker.
    /// Pide la miniatura del álbum usando la portada local si existe. Evita
    /// depender de la red para álbumes ya ingestados.
    pub(crate) fn request_album_thumb(&mut self, album: &AlbumRow, w: u16, h: u16) {
        let key = format!("album:{}:{}", album.artist, album.album);
        let slot = (key.clone(), w, h);
        if self.thumbs.contains_key(&slot) || self.thumb_requests.contains(&slot) {
            return;
        }
        self.thumb_requests.insert(slot);
        self.art_worker.request(crate::fetch::ArtRequest {
            key,
            url: album.art_url.clone(),
            local: album.art_path.clone(),
            prefer_url: true,
            w,
            h,
            mode: ArtMode::Plain,
            kind: crate::fetch::ArtKind::Thumb,
        });
    }

    /// Si el álbum no tiene portada, pide metadatos de su primera pista (una
    /// vez por álbum) para que la miniatura se rellene sola.
    pub(crate) fn ensure_album_art(&mut self, album: &AlbumRow) {
        if album.art_path.is_some() || album.art_url.is_some() {
            return;
        }
        if self.metadata_requests.len() >= 3 {
            return;
        }
        if let Ok(tracks) = self.lib.album_tracks(&album.album, &album.artist) {
            if let Some(row) = tracks.first() {
                self.ensure_missing_art(row);
            }
        }
    }

    pub(crate) fn album_thumb_track(album: &AlbumRow) -> TrackRef {
        TrackRef {
            key: format!("album:{}:{}", album.artist, album.album),
            title: album.album.clone(),
            artist: album.artist.clone(),
            album: Some(album.album.clone()),
            duration_ms: album.duration_ms,
            art_url: album.art_url.clone(),
            source_hint: None,
        }
    }
}

/// El mismo disco puede venir con el artista escrito de dos formas
/// («Biberon» / «Bíberon») según el proveedor. Se comparan ya normalizados.
fn artists_match(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || a == b || a.contains(b) || b.contains(a)
}

/// La biblioteca puede tener la misma canción dos veces (una clave ISRC y
/// otra de Spotify/YouTube). En el tracklist del álbum se enseña una sola,
/// prefiriendo la copia que tiene fichero local.
fn dedupe_album_rows(rows: Vec<TrackRow>) -> Vec<TrackRow> {
    let mut out: Vec<TrackRow> = Vec::new();
    for row in rows {
        if let Some(index) = out
            .iter()
            .position(|kept| same_song(&kept.track, &row.track))
        {
            if row.playable() && !out[index].playable() {
                out[index] = row;
            }
        } else {
            out.push(row);
        }
    }
    out
}

/// Mezcla el tracklist del catálogo con las copias locales. Las pistas que
/// existen en disco conservan su clave (para que `local_path` acierte) pero
/// toman los metadatos del catálogo. Lo local que no esté en el remoto se
/// queda al final para no esconder descargas.
fn merge_album_tracks(remote: &[TrackRef], local: &[TrackRef]) -> Vec<TrackRef> {
    let mut used_local: Vec<&str> = Vec::new();
    let mut out = Vec::with_capacity(remote.len().max(local.len()));
    for track in remote {
        let matched = local.iter().find(|candidate| {
            !used_local.iter().any(|key| *key == candidate.key) && same_song(candidate, track)
        });
        match matched {
            Some(candidate) => {
                used_local.push(&candidate.key);
                out.push(TrackRef {
                    key: candidate.key.clone(),
                    ..track.clone()
                });
            }
            None => out.push(track.clone()),
        }
    }
    for track in local {
        // No se añade si ya salió en el catálogo con otra clave (mismo
        // título/artista/duración): era la copia duplicada de la biblioteca.
        if !used_local.iter().any(|key| *key == track.key)
            && !out
                .iter()
                .any(|other| other.key == track.key || same_song(other, track))
        {
            out.push(track.clone());
        }
    }
    out
}

/// ¿Dos pistas son la misma canción aunque vengan de proveedores distintos?
/// El título manda; la duración evita confundir una versión de directo o un
/// remix, y el artista se compara de forma laxa porque YouTube suele añadir
/// colaboraciones al nombre.
fn same_song(a: &TrackRef, b: &TrackRef) -> bool {
    if a.key == b.key {
        return true;
    }
    let title_a = spote_core::util::normalize(&a.title);
    let title_b = spote_core::util::normalize(&b.title);
    if title_a.is_empty() || title_b.is_empty() {
        return false;
    }
    if title_a != title_b && !title_a.contains(&title_b) && !title_b.contains(&title_a) {
        return false;
    }
    if a.duration_ms > 0
        && b.duration_ms > 0
        && a.duration_ms.abs_diff(b.duration_ms) > 8_000
    {
        return false;
    }
    let artist_a = spote_core::util::normalize(&a.artist);
    let artist_b = spote_core::util::normalize(&b.artist);
    artist_a.is_empty()
        || artist_b.is_empty()
        || artist_a == artist_b
        || artist_a.contains(&artist_b)
        || artist_b.contains(&artist_a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str, title: &str, duration_ms: u64) -> TrackRef {
        TrackRef {
            key: key.into(),
            title: title.into(),
            artist: "Biberon".into(),
            album: Some("¡Se cayó la bocina!".into()),
            duration_ms,
            art_url: None,
            source_hint: None,
        }
    }

    #[test]
    fn catalog_album_keeps_the_downloaded_key_for_the_same_song() {
        let local = vec![track("youtube:abc", "Regular", 139_000)];
        let remote = vec![
            track("isrc:uno", "Otra", 120_000),
            track("isrc:dos", "Regular", 139_500),
        ];
        let merged = merge_album_tracks(&remote, &local);
        assert_eq!(merged.len(), 2, "no debe duplicar la descarga local");
        assert_eq!(merged[1].key, "youtube:abc");
        assert_eq!(merged[1].title, "Regular");
    }

    #[test]
    fn local_bonus_tracks_survive_after_the_catalog_list() {
        let local = vec![
            track("youtube:abc", "Regular", 139_000),
            track("youtube:directo", "Regular (directo)", 150_000),
        ];
        let remote = vec![track("isrc:dos", "Regular", 139_500)];
        let merged = merge_album_tracks(&remote, &local);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].key, "youtube:abc");
        assert_eq!(merged[1].key, "youtube:directo");
    }

    #[test]
    fn mismatched_duration_is_not_the_same_song() {
        let local = track("youtube:abc", "Regular", 139_000);
        let remote = track("isrc:dos", "Regular", 190_000);
        assert!(!same_song(&local, &remote));
    }

    #[test]
    fn accented_artist_is_the_same_album_group() {
        let mut accented = track("isrc:bk4", "Regular", 115_000);
        accented.artist = "Bíberon".into();
        let plain = track("spotify:track:6nn", "Regular", 115_336);
        assert!(artists_match(
            &spote_core::util::normalize("Biberon"),
            &spote_core::util::normalize("Bíberon")
        ));
        assert!(same_song(&plain, &accented));
    }

    #[test]
    fn duplicate_local_copies_do_not_repeat_after_the_catalog_merge() {
        let local = vec![
            track("spotify:track:6nn", "Regular", 115_336),
            track("isrc:bk4", "Regular", 115_000),
        ];
        let remote = vec![track("isrc:remote", "Regular", 115_000)];
        let merged = merge_album_tracks(&remote, &local);
        assert_eq!(merged.len(), 1, "la copia local duplicada no se repite");
    }
}
