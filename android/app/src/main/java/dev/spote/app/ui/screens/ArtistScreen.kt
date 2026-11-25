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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.Album
import dev.spote.app.model.PlayerState
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AlbumCard
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun ArtistScreen(
    name: String,
    tracks: List<Track>,
    albums: List<Album>,
    player: PlayerState,
    onBack: () -> Unit,
    onPlayTrack: (Track) -> Unit,
    onOpenAlbum: (Album) -> Unit,
    onMore: (Track) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val sorted = tracks.sortedByDescending { it.durationMs }
    val cover = tracks.firstOrNull()?.cover ?: albums.firstOrNull()?.cover
    val totalMs = tracks.sumOf { it.durationMs }

    AppBackground(accentStart = cover?.start ?: colors.coverStart) {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 28.dp),
        ) {
            item(key = "topbar") { TopBar(onBack = onBack) }
            item(key = "header") {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    if (cover != null) {
                        CoverArt(cover = cover, size = 150.dp, corner = 75.dp, labelSize = 42.sp)
                    }
                    Spacer(Modifier.height(16.dp))
                    Text(
                        text = name,
                        style = MaterialTheme.typography.headlineLarge,
                        color = MaterialTheme.colorScheme.onBackground,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(5.dp))
                    Text(
                        text = "${tracks.size} canciones · ${albums.size} álbumes · ${formatTotal(totalMs)}",
                        style = MaterialTheme.typography.labelMedium,
                        color = colors.accent,
                    )
                    Spacer(Modifier.height(16.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        PlayButton(
                            isPlaying = false,
                            onClick = { sorted.firstOrNull()?.let(onPlayTrack) },
                            size = 56.dp,
                        )
                    }
                }
            }

            if (tracks.isEmpty()) {
                item(key = "empty") {
                    EmptyState(
                        icon = Icons.Rounded.Search,
                        title = "Sin canciones",
                        text = "Este artista no tiene pistas en tu biblioteca.",
                    )
                }
            } else {
                if (albums.isNotEmpty()) {
                    item(key = "albums-title") { SectionHeader(title = "Álbumes") }
                    item(key = "albums") {
                        LazyRow(
                            contentPadding = PaddingValues(horizontal = 20.dp),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            items(albums, key = { it.id }) { album ->
                                AlbumCard(album = album, onClick = { onOpenAlbum(album) }, width = 148.dp)
                            }
                        }
                    }
                }

                item(key = "popular-title") { SectionHeader(title = "Populares") }
                items(sorted.take(5), key = { it.key }) { track ->
                    TrackRow(
                        track = track,
                        onClick = { onPlayTrack(track) },
                        onMore = { onMore(track) },
                        isPlaying = player.track?.key == track.key,
                    )
                }

                if (sorted.size > 5) {
                    item(key = "all-title") { SectionHeader(title = "Todas las canciones") }
                    items(sorted.drop(5), key = { it.key }) { track ->
                        TrackRow(
                            track = track,
                            onClick = { onPlayTrack(track) },
                            onMore = { onMore(track) },
                            isPlaying = player.track?.key == track.key,
                        )
                    }
                }
            }
        }
    }
}

private fun formatTotal(ms: Long): String {
    val minutes = (ms / 60_000L).coerceAtLeast(0L)
    return if (minutes >= 60) "${minutes / 60} h ${minutes % 60} min" else "$minutes min"
}
