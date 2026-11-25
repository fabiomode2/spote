package dev.spote.app.ui.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.QueueMusic
import androidx.compose.material.icons.rounded.Repeat
import androidx.compose.material.icons.rounded.RepeatOne
import androidx.compose.material.icons.rounded.Shuffle
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Subtitles
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.PlayerState
import dev.spote.app.model.RepeatMode
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.SpectrumStrip
import dev.spote.app.ui.components.LabelledAction
import dev.spote.app.ui.components.LikeButton
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.util.asTrackDuration

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun NowPlayingScreen(
    player: PlayerState,
    onClose: () -> Unit,
    onTogglePlay: () -> Unit,
    onNext: () -> Unit,
    onPrevious: () -> Unit,
    onSeek: (Long) -> Unit,
    onShuffle: () -> Unit,
    onRepeat: () -> Unit,
    onLike: () -> Unit,
    onDownload: () -> Unit,
    onLyrics: () -> Unit,
    onQueue: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val track = player.track ?: return
    val colors = LocalSpoteColors.current
    val activeLine = player.lyrics.indexOfLast { it.startMs <= player.positionMs }
    val currentLyric = player.lyrics.getOrNull(activeLine)
    val nextLyric = player.lyrics.getOrNull(activeLine + 1)
    val currentIndex = player.queue.indexOfFirst { it.key == track.key }
    val nextTrack = player.queue.getOrNull(currentIndex + 1)

    AppBackground(accentStart = track.cover.start) {
        BoxWithConstraints(modifier = modifier.fillMaxSize()) {
            val coverSize = minOf(maxWidth - 72.dp, 330.dp)
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState()),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .statusBarsPadding()
                        .height(56.dp)
                        .padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    CircleIconButton(
                        icon = Icons.Rounded.KeyboardArrowDown,
                        contentDescription = "Cerrar",
                        onClick = onClose,
                        size = 40.dp,
                        iconSize = 26.dp,
                        tint = MaterialTheme.colorScheme.onBackground,
                    )
                    Column(
                        modifier = Modifier.weight(1f),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(
                            text = "REPRODUCIENDO",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = track.album ?: "Cola",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onBackground,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Spacer(Modifier.size(40.dp))
                }

                Spacer(Modifier.height(8.dp))
                SpectrumStrip(
                    playing = player.isPlaying,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 48.dp)
                        .height(26.dp),
                )
                Spacer(Modifier.height(10.dp))
                Box(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    CoverArt(
                        cover = track.cover,
                        size = coverSize,
                        corner = 16.dp,
                        labelSize = 72.sp,
                    )
                }
                Spacer(Modifier.height(24.dp))

                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 28.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = track.title,
                            style = MaterialTheme.typography.headlineMedium,
                            color = MaterialTheme.colorScheme.onBackground,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Spacer(Modifier.height(3.dp))
                        Text(
                            text = track.artist,
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    LikeButton(favorite = track.favorite, onClick = onLike)
                }

                Spacer(Modifier.height(14.dp))
                Slider(
                    value = if (player.durationMs > 0L) {
                        (player.positionMs.toFloat() / player.durationMs.toFloat()).coerceIn(0f, 1f)
                    } else {
                        0f
                    },
                    onValueChange = { fraction ->
                        onSeek((fraction * player.durationMs).toLong())
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 18.dp),
                    colors = SliderDefaults.colors(
                        thumbColor = colors.accent,
                        activeTrackColor = colors.accent,
                        inactiveTrackColor = MaterialTheme.colorScheme.outline,
                    ),
                    thumb = {
                        Box(
                            modifier = Modifier
                                .size(10.dp)
                                .clip(CircleShape)
                                .background(colors.accent),
                        )
                    },
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 28.dp),
                ) {
                    Text(
                        text = player.positionMs.asTrackDuration(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.weight(1f))
                    Text(
                        text = player.durationMs.asTrackDuration(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                Spacer(Modifier.height(14.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceEvenly,
                ) {
                    CircleIconButton(
                        icon = Icons.Rounded.Shuffle,
                        contentDescription = "Aleatorio",
                        onClick = onShuffle,
                        size = 44.dp,
                        iconSize = 22.dp,
                        tint = if (player.shuffle) colors.accent else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    CircleIconButton(
                        icon = Icons.Rounded.SkipPrevious,
                        contentDescription = "Anterior",
                        onClick = onPrevious,
                        size = 52.dp,
                        iconSize = 32.dp,
                        tint = MaterialTheme.colorScheme.onBackground,
                    )
                    PlayButton(isPlaying = player.isPlaying, onClick = onTogglePlay, size = 68.dp)
                    CircleIconButton(
                        icon = Icons.Rounded.SkipNext,
                        contentDescription = "Siguiente",
                        onClick = onNext,
                        size = 52.dp,
                        iconSize = 32.dp,
                        tint = MaterialTheme.colorScheme.onBackground,
                    )
                    CircleIconButton(
                        icon = if (player.repeat == RepeatMode.One) Icons.Rounded.RepeatOne else Icons.Rounded.Repeat,
                        contentDescription = "Repetir",
                        onClick = onRepeat,
                        size = 44.dp,
                        iconSize = 22.dp,
                        tint = if (player.repeat != RepeatMode.Off) colors.accent else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                if (nextTrack != null) {
                    Spacer(Modifier.height(16.dp))
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Text(
                            text = "A CONTINUACIÓN",
                            style = MaterialTheme.typography.labelSmall,
                            color = colors.onSurfaceVariant,
                        )
                        CoverArt(cover = nextTrack.cover, size = 26.dp, corner = 5.dp, labelSize = 9.sp)
                        Text(
                            text = nextTrack.title,
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onBackground,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }

                if (currentLyric != null) {
                    Spacer(Modifier.height(18.dp))
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp)
                            .clip(RoundedCornerShape(14.dp)),
                        color = colors.surface.copy(alpha = 0.72f),
                        shape = RoundedCornerShape(14.dp),
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 13.dp),
                            verticalArrangement = Arrangement.spacedBy(5.dp),
                        ) {
                            Text(
                                text = "LETRA",
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.accent,
                            )
                            Text(
                                text = currentLyric.text,
                                style = MaterialTheme.typography.titleLarge,
                                color = MaterialTheme.colorScheme.onBackground,
                            )
                            if (nextLyric != null) {
                                Text(
                                    text = nextLyric.text,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                    }
                }

                Spacer(Modifier.height(14.dp))
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                ) {
                    LabelledAction(
                        icon = Icons.Rounded.Subtitles,
                        label = "Letra",
                        onClick = onLyrics,
                        tint = MaterialTheme.colorScheme.onBackground,
                    )
                    LabelledAction(
                        icon = Icons.Rounded.QueueMusic,
                        label = "Cola",
                        onClick = onQueue,
                        tint = MaterialTheme.colorScheme.onBackground,
                    )
                    LabelledAction(
                        icon = if (track.downloaded) Icons.Rounded.DownloadDone else Icons.Rounded.Download,
                        label = if (track.downloaded) "Guardada" else "Guardar",
                        onClick = onDownload,
                        tint = if (track.downloaded) colors.accent else MaterialTheme.colorScheme.onBackground,
                    )
                }
                Spacer(Modifier.height(26.dp))
            }
        }
    }
}
