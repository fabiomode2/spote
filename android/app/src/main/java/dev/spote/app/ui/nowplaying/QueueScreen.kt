package dev.spote.app.ui.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.QueueMusic
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.PlayerState
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun QueueScreen(
    player: PlayerState,
    onBack: () -> Unit,
    onPlayTrack: (Int) -> Unit,
    onMore: (Track, Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val track = player.track
    val queue = player.queue

    AppBackground(accentStart = track?.cover?.start ?: colors.coverStart) {
        Column(modifier = modifier.fillMaxSize()) {
            TopBar(title = "Cola", onBack = onBack)
            if (track != null) {
                Surface(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 8.dp),
                    color = colors.surface,
                    shape = RoundedCornerShape(14.dp),
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        CoverArt(cover = track.cover, size = 52.dp, corner = 8.dp, labelSize = 14.sp)
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = "SONANDO AHORA",
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.accent,
                            )
                            Text(
                                text = track.title,
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onBackground,
                                maxLines = 1,
                            )
                            Text(
                                text = track.artist,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                            )
                        }
                        Icon(
                            imageVector = Icons.Rounded.QueueMusic,
                            contentDescription = null,
                            tint = colors.accent,
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }

            if (queue.isEmpty()) {
                EmptyState(
                    icon = Icons.Rounded.QueueMusic,
                    title = "La cola está vacía",
                    text = "Encola algo desde la biblioteca o desde una sala.",
                )
            } else {
                SectionHeader(title = "En la cola")
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(bottom = 24.dp),
                ) {
                    itemsIndexed(queue, key = { _, item -> item.key }) { index, item ->
                        TrackRow(
                            track = item,
                            onClick = { onPlayTrack(index) },
                            onMore = { onMore(item, index) },
                            isPlaying = item.key == track?.key,
                        )
                    }
                }
            }
        }
    }
}
