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
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.Album
import dev.spote.app.model.ArtistProfile
import dev.spote.app.model.PlayerState
import dev.spote.app.model.Track
import dev.spote.app.rust.coverFor
import dev.spote.app.ui.components.AlbumCard
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.SpoteChip
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
    profile: ArtistProfile? = null,
    loading: Boolean = false,
    error: String? = null,
    onRetry: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val cover = profile?.artist?.imageUrl?.let {
        coverFor(name, null, it)
    } ?: albums.firstOrNull()?.cover ?: tracks.firstOrNull()?.cover
    val sorted = tracks
    val totalMs = tracks.sumOf { it.durationMs }
    val followers = profile?.artist?.followers ?: 0L
    val genres = profile?.genres.orEmpty()

    AppBackground(
        accentStart = cover?.start ?: colors.coverStart,
        cover = cover,
    ) {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 28.dp),
        ) {
            item(key = "topbar") { TopBar(onBack = onBack) }
            item(key = "header") {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 22.dp, vertical = 14.dp),
                    verticalAlignment = Alignment.Bottom,
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    if (cover != null) {
                        CoverArt(
                            cover = cover,
                            size = 132.dp,
                            corner = 66.dp,
                            labelSize = 34.sp,
                        )
                    }
                    Column(
                        modifier = Modifier.weight(1f),
                        verticalArrangement = Arrangement.spacedBy(5.dp),
                    ) {
                        Text(
                            text = "ARTISTA",
                            style = MaterialTheme.typography.labelSmall,
                            color = colors.accent,
                        )
                        Text(
                            text = name,
                            style = MaterialTheme.typography.headlineLarge,
                            color = MaterialTheme.colorScheme.onBackground,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = buildString {
                                if (followers > 0L) {
                                    append("${formatFollowers(followers)} seguidores")
                                }
                                if (tracks.isNotEmpty()) {
                                    if (isNotEmpty()) append(" · ")
                                    append("${tracks.size} canciones")
                                }
                                if (albums.isNotEmpty()) {
                                    if (isNotEmpty()) append(" · ")
                                    append("${albums.size} álbumes")
                                }
                                if (totalMs > 0L) {
                                    if (isNotEmpty()) append(" · ")
                                    append(formatTotal(totalMs))
                                }
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (genres.isNotEmpty()) {
                            Text(
                                text = genres.take(3).joinToString(" · "),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Spacer(Modifier.height(6.dp))
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            PlayButton(
                                isPlaying = player.track != null && tracks.any {
                                    it.key == player.track?.key
                                } && player.isPlaying,
                                onClick = { sorted.firstOrNull()?.let(onPlayTrack) },
                                size = 46.dp,
                            )
                            if (loading) {
                                CircularProgressIndicator(
                                    strokeWidth = 2.dp,
                                    color = colors.accent,
                                    modifier = Modifier.height(18.dp),
                                )
                            }
                        }
                    }
                }
            }

            if (error != null && tracks.isEmpty() && albums.isEmpty()) {
                item(key = "error") {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 22.dp, vertical = 10.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            text = "No se pudo cargar el perfil",
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onBackground,
                        )
                        Text(
                            text = error,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        TextButton(onClick = onRetry) {
                            androidx.compose.material3.Icon(
                                imageVector = Icons.Rounded.Refresh,
                                contentDescription = null,
                            )
                            Spacer(Modifier.height(4.dp))
                            Text("Reintentar")
                        }
                    }
                }
            }

            if (profile?.bio?.isNotBlank() == true) {
                item(key = "bio") {
                    Text(
                        text = profile.bio.orEmpty(),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 4,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(horizontal = 22.dp, vertical = 6.dp),
                    )
                }
            }

            if (albums.isNotEmpty()) {
                item(key = "albums-title") { SectionHeader(title = "Discografía") }
                item(key = "albums") {
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        items(albums, key = { it.id }) { album ->
                            AlbumCard(
                                album = album,
                                onClick = { onOpenAlbum(album) },
                                width = 148.dp,
                            )
                        }
                    }
                }
            }

            if (sorted.isNotEmpty()) {
                item(key = "tracks-title") { SectionHeader(title = "Populares") }
                items(sorted, key = { it.key }) { track ->
                    TrackRow(
                        track = track,
                        onClick = { onPlayTrack(track) },
                        onMore = { onMore(track) },
                        isPlaying = player.track?.key == track.key,
                        showAlbum = true,
                    )
                }
            } else if (!loading) {
                item(key = "empty") {
                    EmptyState(
                        icon = Icons.Rounded.Refresh,
                        title = "Sin canciones",
                        text = "Este artista no tiene pistas en tu biblioteca ni en el catálogo.",
                    )
                }
            }
        }
    }
}

private fun formatFollowers(value: Long): String = when {
    value >= 1_000_000L -> "%.1f M".format(value / 1_000_000.0)
    value >= 1_000L -> "%.1f mil".format(value / 1_000.0)
    else -> value.toString()
}

private fun formatTotal(ms: Long): String {
    val minutes = (ms / 60_000L).coerceAtLeast(0L)
    return if (minutes >= 60) "${minutes / 60} h ${minutes % 60} min" else "$minutes min"
}
