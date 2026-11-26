package dev.spote.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.SearchOff
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.spote.app.model.Album
import dev.spote.app.model.SearchState
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AlbumCard
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.ArtistCircle
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.GenreCard
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.SpoteChip
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun SearchScreen(
    state: SearchState,
    onQueryChange: (String) -> Unit,
    onPlayTrack: (Track) -> Unit,
    onOpenAlbum: (Album) -> Unit,
    onMore: (Track) -> Unit,
    onOpenArtist: (String) -> Unit = { onQueryChange(it) },
    onClearSearch: () -> Unit = { onQueryChange("") },
    onClearHistory: () -> Unit = {},
    onSearchCommit: (String) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val focusManager = LocalFocusManager.current
    val hasQuery = state.query.isNotBlank()
    val allAlbums = remember(state.albums, state.remoteAlbums) {
        (state.albums + state.remoteAlbums).distinctBy { it.id }
    }
    val allArtists = remember(state.artists, state.remoteArtists) {
        (state.artists + state.remoteArtists).distinctBy { it.lowercase() }
    }

    AppBackground {
        Column(modifier = modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(top = 10.dp, bottom = 10.dp),
            ) {
                Text(
                    text = "Buscar",
                    style = MaterialTheme.typography.displayMedium,
                    color = MaterialTheme.colorScheme.onBackground,
                    modifier = Modifier.padding(horizontal = 20.dp),
                )
                Spacer(Modifier.height(12.dp))
                BasicTextField(
                    value = state.query,
                    onValueChange = onQueryChange,
                    singleLine = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(
                        color = MaterialTheme.colorScheme.onBackground,
                    ),
                    cursorBrush = SolidColor(colors.accent),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(
                        onSearch = {
                            onSearchCommit(state.query)
                            focusManager.clearFocus()
                        },
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp),
                    decorationBox = { inner ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .height(48.dp)
                                .background(colors.surfaceHigh, RoundedCornerShape(12.dp))
                                .padding(horizontal = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon(
                                imageVector = Icons.Rounded.Search,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.width(22.dp),
                            )
                            Spacer(Modifier.width(10.dp))
                            Box(modifier = Modifier.weight(1f)) {
                                if (state.query.isEmpty()) {
                                    Text(
                                        text = "Artistas, canciones, álbumes…",
                                        style = MaterialTheme.typography.bodyLarge,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                                inner()
                            }
                            if (state.query.isNotEmpty()) {
                                CircleIconButton(
                                    icon = Icons.Rounded.Close,
                                    contentDescription = "Limpiar",
                                    onClick = onClearSearch,
                                    size = 28.dp,
                                    iconSize = 18.dp,
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    },
                )
            }

            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(bottom = 24.dp),
            ) {
                if (!hasQuery) {
                    if (state.recentQueries.isNotEmpty()) {
                        item(key = "recent-title") {
                            SectionHeader(
                                title = "Búsquedas recientes",
                                actionLabel = "Limpiar",
                                onAction = onClearHistory,
                            )
                        }
                        item(key = "recent") {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .horizontalScroll(rememberScrollState())
                                    .padding(horizontal = 20.dp),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                state.recentQueries.forEach { query ->
                                    SpoteChip(
                                        text = query,
                                        selected = false,
                                        onClick = { onQueryChange(query) },
                                    )
                                }
                            }
                        }
                    }
                    if (state.genres.isNotEmpty()) {
                        item(key = "genres-title") {
                            SectionHeader(title = "Explorar")
                        }
                        items(
                            state.genres.chunked(2),
                            key = { row -> row.joinToString("|") { it.name } },
                        ) { row ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 20.dp, vertical = 4.dp),
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                row.forEach { genre ->
                                    GenreCard(
                                        genre = genre,
                                        onClick = { onQueryChange(genre.name) },
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                            }
                        }
                    }
                    if (state.recentQueries.isEmpty() && state.genres.isEmpty()) {
                        item(key = "search-hint") {
                            EmptyState(
                                icon = Icons.Rounded.Search,
                                title = "Busca en todas partes",
                                text = "Tu biblioteca y el catálogo, en el mismo sitio. Escribe un título o artista.",
                            )
                        }
                    }
                } else {
                    if (state.results.isNotEmpty()) {
                        item(key = "results-title") {
                            SectionHeader(title = "Canciones")
                        }
                        items(state.results, key = { it.key }) { track ->
                            TrackRow(
                                track = track,
                                onClick = { onPlayTrack(track) },
                                onMore = { onMore(track) },
                            )
                        }
                    }
                    if (state.remoteLoading) {
                        item(key = "remote-loading") {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 20.dp, vertical = 10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                CircularProgressIndicator(
                                    modifier = Modifier.size(16.dp),
                                    strokeWidth = 2.dp,
                                    color = colors.accent,
                                )
                                Text(
                                    text = "Buscando en el catálogo…",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    if (state.remoteResults.isNotEmpty()) {
                        item(key = "remote-title") {
                            SectionHeader(title = "Canciones")
                        }
                        items(state.remoteResults, key = { "remote-${it.key}" }) { track ->
                            TrackRow(
                                track = track,
                                onClick = { onPlayTrack(track) },
                                onMore = { onMore(track) },
                            )
                        }
                    }
                    if (state.remoteError != null && state.remoteResults.isEmpty() && !state.remoteLoading) {
                        item(key = "remote-error") {
                            Text(
                                text = "La búsqueda remota falló: ${state.remoteError}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                            )
                        }
                    }
                    if (allAlbums.isNotEmpty()) {
                        item(key = "albums-title") {
                            SectionHeader(title = "Álbumes")
                        }
                        item(key = "albums") {
                            LazyRow(
                                contentPadding = PaddingValues(horizontal = 20.dp),
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                items(allAlbums, key = { it.id }) { album ->
                                    AlbumCard(album = album, onClick = { onOpenAlbum(album) })
                                }
                            }
                        }
                    }
                    if (allArtists.isNotEmpty()) {
                        item(key = "artists-title") {
                            SectionHeader(title = "Artistas")
                        }
                        item(key = "artists") {
                            LazyRow(
                                contentPadding = PaddingValues(horizontal = 20.dp),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                items(allArtists, key = { it }) { artist ->
                                    val cover = allAlbums.firstOrNull { it.artist == artist }?.cover
                                        ?: state.results.firstOrNull { it.artist == artist }?.cover
                                        ?: state.remoteResults.firstOrNull { it.artist == artist }?.cover
                                        ?: dev.spote.app.model.Cover(
                                            start = Color(0xFF3A3A42),
                                            end = Color(0xFF15151A),
                                            label = artist.take(1).uppercase(),
                                        )
                                    ArtistCircle(name = artist, cover = cover, onClick = {
                                        onOpenArtist(artist)
                                    })
                                }
                            }
                        }
                    }
                    if (
                        state.results.isEmpty() &&
                        allAlbums.isEmpty() &&
                        allArtists.isEmpty() &&
                        state.remoteResults.isEmpty() &&
                        !state.remoteLoading
                    ) {
                        item(key = "no-results") {
                            EmptyState(
                                icon = Icons.Rounded.SearchOff,
                                title = "Nada por aquí",
                                text = "Prueba con otro título, artista o álbum.",
                            )
                        }
                    }
                }
            }
        }
    }
}
