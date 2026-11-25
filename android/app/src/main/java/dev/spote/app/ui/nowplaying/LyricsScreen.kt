package dev.spote.app.ui.nowplaying

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Subtitles
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.PlayerState
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun LyricsScreen(
    player: PlayerState,
    onBack: () -> Unit,
    onSeekTo: (Long) -> Unit,
    modifier: Modifier = Modifier,
) {
    val track = player.track ?: return
    val colors = LocalSpoteColors.current
    val listState = rememberLazyListState()
    val activeIndex = remember(player.positionMs, player.lyrics) {
        player.lyrics.indexOfLast { it.startMs <= player.positionMs }
    }

    LaunchedEffect(activeIndex) {
        if (activeIndex >= 0) {
            listState.animateScrollToItem((activeIndex - 2).coerceAtLeast(0))
        }
    }

    AppBackground(accentStart = track.cover.start) {
        Column(modifier = modifier.fillMaxSize()) {
            TopBar(title = "Letra", onBack = onBack)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                CoverArt(cover = track.cover, size = 48.dp, corner = 8.dp, labelSize = 13.sp)
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = track.title,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onBackground,
                    )
                    Text(
                        text = track.artist,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    text = if (player.lyrics.isNotEmpty()) "Sincronizada" else "Sin letra",
                    style = MaterialTheme.typography.labelSmall,
                    color = colors.accent,
                )
            }

            if (player.lyrics.isEmpty()) {
                EmptyState(
                    icon = Icons.Rounded.Subtitles,
                    title = "Sin letra para esta pista",
                    text = "spote buscará en LRCLIB, Genius y los subtítulos de YouTube la próxima vez que suene.",
                )
            } else {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 24.dp, vertical = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    itemsIndexed(player.lyrics, key = { index, line -> "$index-${line.startMs}" }) { index, line ->
                        val active = index == activeIndex
                        Text(
                            text = line.text,
                            style = if (active) {
                                MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
                            } else {
                                MaterialTheme.typography.titleLarge
                            },
                            color = if (active) colors.accent else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onSeekTo(line.startMs) }
                                .padding(vertical = 2.dp),
                        )
                    }
                    item(key = "footer") {
                        Text(
                            text = "Toca una línea para saltar. La letra se afina con el audio cuando hace falta.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 18.dp, bottom = 28.dp),
                        )
                    }
                }
            }
        }
    }
}
