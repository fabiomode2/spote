package dev.spote.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.GridView
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.ViewList
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.spote.app.model.Album
import dev.spote.app.model.Cover
import dev.spote.app.model.LibraryFilter
import dev.spote.app.model.LibraryState
import dev.spote.app.model.Playlist
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AlbumCard
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.CollectionRow
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.FilterChips
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.SpoteHeader
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors
import androidx.compose.material.icons.rounded.LibraryMusic

@Composable
fun LibraryScreen(
    state: LibraryState,
    onFilter: (LibraryFilter) -> Unit,
    onToggleGrid: () -> Unit,
    onPlayTrack: (Track) -> Unit,
    onMore: (Track) -> Unit,
    onOpenAlbum: (Album) -> Unit,
    onOpenPlaylist: (Playlist) -> Unit,
    onOpenDownloads: () -> Unit,
    onOpenSearch: () -> Unit,
    onOpenArtist: (String) -> Unit,
    onCreatePlaylist: () -> Unit = {},
    onImport: () -> Unit = {},
    userName: String = "",
    avatarUri: String? = null,
    onOpenSettings: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val filters = LibraryFilter.entries
    val artists = state.tracks.map { it.artist }.distinct()
    val downloaded = state.tracks.filter { it.downloaded }

    AppBackground(
        cover = state.albums.firstOrNull()?.cover ?: state.tracks.firstOrNull()?.cover,
    ) {
        Column(modifier = modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 4.dp),
            ) {
                SpoteHeader(
                    title = "Tu biblioteca",
                    userName = userName,
                    avatarUri = avatarUri,
                    onOpenSettings = onOpenSettings,
                    actions = {
                        CircleIconButton(
                            icon = Icons.Rounded.Search,
                            contentDescription = "Buscar",
                            onClick = onOpenSearch,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        CircleIconButton(
                            icon = Icons.Rounded.Add,
                            contentDescription = "Importar música",
                            onClick = onImport,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    },
                )
                Spacer(Modifier.height(8.dp))
                FilterChips(
                    labels = filters.map { it.label },
                    selected = state.filter.label,
                    onSelect = { label -> filters.firstOrNull { it.label == label }?.let(onFilter) },
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = when (state.filter) {
                            LibraryFilter.All -> "${state.playlists.size + state.albums.size} colecciones"
                            LibraryFilter.Playlists -> "${state.playlists.size} playlists"
                            LibraryFilter.Albums -> "${state.albums.size} álbumes"
                            LibraryFilter.Artists -> "${artists.size} artistas"
                            LibraryFilter.Downloaded -> "${downloaded.size} canciones offline"
                            LibraryFilter.Favorites -> "${state.tracks.count { it.favorite }} canciones"
                        },
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    CircleIconButton(
                        icon = if (state.isGrid) Icons.Rounded.ViewList else Icons.Rounded.GridView,
                        contentDescription = if (state.isGrid) "Ver lista" else "Ver cuadrícula",
                        onClick = onToggleGrid,
                        size = 32.dp,
                        iconSize = 18.dp,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(bottom = 24.dp),
            ) {
                if (state.filter == LibraryFilter.All) {
                    item(key = "liked") {
                        CollectionRow(
                            cover = Cover(
                                start = Color(0xFF8A5AC8),
                                end = Color(0xFF2A124A),
                                label = "FAV",
                            ),
                            title = "Canciones que te gustan",
                            subtitle = "Todas tus favoritas",
                            onClick = { onFilter(LibraryFilter.Favorites) },
                        )
                    }
                    item(key = "downloads") {
                        CollectionRow(
                            cover = Cover(
                                start = colors.accent,
                                end = Color(0xFF2A3A0E),
                                label = "DL",
                            ),
                            title = "Descargas",
                            subtitle = "${state.downloadedCount} canciones · ${state.storageLabel}",
                            badge = "Offline",
                            onClick = onOpenDownloads,
                        )
                    }
                    item(key = "new-playlist") {
                        CollectionRow(
                            cover = Cover(
                                start = colors.accent.copy(alpha = 0.75f),
                                end = colors.surfaceHigh,
                                label = "+",
                            ),
                            title = "Nueva playlist",
                            subtitle = "Agrupa canciones de tu biblioteca",
                            onClick = onCreatePlaylist,
                        )
                    }
                    item(key = "playlists-title") {
                        SectionHeader(title = "Playlists")
                    }
                    items(state.playlists, key = { it.id }) { playlist ->
                        CollectionRow(
                            cover = playlist.cover,
                            title = playlist.name,
                            subtitle = "${playlist.trackCount} canciones",
                            onClick = { onOpenPlaylist(playlist) },
                        )
                    }
                    item(key = "albums-title") {
                        SectionHeader(title = "Álbumes")
                    }
                    items(state.albums.chunked(2), key = { row -> row.joinToString("|") { it.id } }) { row ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 20.dp, vertical = 4.dp),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            row.forEach { album ->
                                AlbumCard(
                                    album = album,
                                    onClick = { onOpenAlbum(album) },
                                    width = 148.dp,
                                )
                            }
                        }
                    }
                }

                if (state.filter == LibraryFilter.Playlists) {
                    items(state.playlists, key = { it.id }) { playlist ->
                        CollectionRow(
                            cover = playlist.cover,
                            title = playlist.name,
                            subtitle = "${playlist.trackCount} canciones",
                            onClick = { onOpenPlaylist(playlist) },
                        )
                    }
                }

                if (state.filter == LibraryFilter.Albums) {
                    if (state.isGrid) {
                        items(state.albums.chunked(2), key = { row -> row.joinToString("|") { it.id } }) { row ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 20.dp, vertical = 4.dp),
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                row.forEach { album ->
                                    AlbumCard(
                                        album = album,
                                        onClick = { onOpenAlbum(album) },
                                        width = 148.dp,
                                    )
                                }
                            }
                        }
                    } else {
                        items(state.albums, key = { it.id }) { album ->
                            CollectionRow(
                                cover = album.cover,
                                title = album.title,
                                subtitle = album.artist,
                                onClick = { onOpenAlbum(album) },
                            )
                        }
                    }
                }

                if (state.filter == LibraryFilter.Artists) {
                    items(artists, key = { it }) { artist ->
                        val cover = state.tracks.firstOrNull { it.artist == artist }?.cover
                            ?: Cover(Color(0xFF3A3A42), Color(0xFF15151A), artist.take(1))
                        CollectionRow(
                            cover = cover,
                            title = artist,
                            subtitle = "Artista",
                            onClick = { onOpenArtist(artist) },
                        )
                    }
                }

                if (state.filter == LibraryFilter.Favorites) {
                    val favorites = state.tracks.filter { it.favorite }
                    if (favorites.isEmpty()) {
                        item(key = "no-favorites") {
                            EmptyState(
                                icon = Icons.Rounded.FavoriteBorder,
                                title = "Sin favoritas",
                                text = "Marca el corazón en una canción y la encontrarás aquí.",
                            )
                        }
                    } else {
                        items(favorites, key = { it.key }) { track ->
                            TrackRow(
                                track = track,
                                onClick = { onPlayTrack(track) },
                                onMore = { onMore(track) },
                            )
                        }
                    }
                }

                if (state.filter == LibraryFilter.Downloaded) {
                    if (downloaded.isEmpty()) {
                        item(key = "no-downloads") {
                            EmptyState(
                                icon = Icons.Rounded.LibraryMusic,
                                title = "Sin descargas",
                                text = "Las canciones que guardes aparecerán aquí, listas para sonar sin conexión.",
                            )
                        }
                    } else {
                        items(downloaded, key = { it.key }) { track ->
                            TrackRow(
                                track = track,
                                onClick = { onPlayTrack(track) },
                                onMore = { onMore(track) },
                            )
                        }
                    }
                }
            }
        }
    }
}
