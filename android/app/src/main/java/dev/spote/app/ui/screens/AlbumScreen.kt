package dev.spote.app.ui.screens

import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.Shuffle
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
import dev.spote.app.ui.components.AlbumTrackRow
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.LocalArtistNavigation
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.util.asAlbumDuration

@Composable
fun AlbumScreen(
    album: Album,
    player: PlayerState,
    onBack: () -> Unit,
    onPlay: () -> Unit,
    onShuffle: () -> Unit,
    onPlayTrack: (Track) -> Unit,
    onMore: (Track) -> Unit,
    onDownload: () -> Unit,
    onLike: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val openArtist = LocalArtistNavigation.current
    val albumPlaying = player.isPlaying && player.track?.let { current ->
        album.tracks.any { it.key == current.key }
    } == true

    AppBackground(
        accentStart = album.cover.start,
        cover = album.cover,
    ) {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 28.dp),
        ) {
            item(key = "topbar") {
                TopBar(onBack = onBack)
            }
            item(key = "header") {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CoverArt(
                        cover = album.cover,
                        size = 220.dp,
                        corner = 18.dp,
                        labelSize = 58.sp,
                    )
                    Spacer(Modifier.height(20.dp))
                    Text(
                        text = album.title,
                        style = MaterialTheme.typography.headlineLarge,
                        color = MaterialTheme.colorScheme.onBackground,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = album.artist,
                        style = MaterialTheme.typography.titleMedium,
                        color = colors.accent,
                        textAlign = TextAlign.Center,
                        modifier = if (openArtist != null) {
                            Modifier.clickable { openArtist(album.artist) }
                        } else {
                            Modifier
                        },
                    )
                    Spacer(Modifier.height(6.dp))
                    Text(
                        text = buildString {
                            if (album.year > 0) append("${album.year}  ·  ")
                            append("${album.tracks.size} canciones")
                            if (album.durationMs > 0L) {
                                append("  ·  ${album.durationMs.asAlbumDuration()}")
                            }
                        },
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(18.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(18.dp),
                    ) {
                        PlayButton(isPlaying = albumPlaying, onClick = onPlay, size = 56.dp)
                        CircleIconButton(
                            icon = Icons.Rounded.Shuffle,
                            contentDescription = "Aleatorio",
                            onClick = onShuffle,
                            size = 44.dp,
                            iconSize = 24.dp,
                            tint = MaterialTheme.colorScheme.onBackground,
                        )
                        CircleIconButton(
                            icon = Icons.Rounded.Download,
                            contentDescription = "Descargar álbum",
                            onClick = onDownload,
                            size = 44.dp,
                            iconSize = 23.dp,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        CircleIconButton(
                            icon = Icons.Rounded.FavoriteBorder,
                            contentDescription = "Favorito",
                            onClick = onLike,
                            size = 44.dp,
                            iconSize = 23.dp,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            item(key = "tracks-title") {
                SectionHeader(title = "Canciones")
            }
            items(album.tracks, key = { it.key }) { track ->
                AlbumTrackRow(
                    track = track,
                    index = album.tracks.indexOf(track) + 1,
                    onClick = { onPlayTrack(track) },
                    onMore = { onMore(track) },
                    isPlaying = player.isPlaying && player.track?.key == track.key,
                )
            }
            item(key = "footer") {
                Text(
                    text = "Las portadas, la paleta y el ambiente salen de cada disco.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 40.dp, vertical = 28.dp),
                )
            }
        }
    }
}
