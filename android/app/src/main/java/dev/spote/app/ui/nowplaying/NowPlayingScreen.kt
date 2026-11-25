package dev.spote.app.ui.nowplaying

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.PlayerState
import dev.spote.app.model.RepeatMode
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.LabelledAction
import dev.spote.app.ui.components.LikeButton
import dev.spote.app.ui.components.LocalArtistNavigation
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.util.asTrackDuration
import kotlinx.coroutines.launch

private const val LyricsHeaderIndex = 4
private const val LyricsStartIndex = 5

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
    onQueue: () -> Unit,
    onRetryLyrics: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val track = player.track ?: return
    val colors = LocalSpoteColors.current
    val openArtist = LocalArtistNavigation.current
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    var lyricsPinned by remember { mutableStateOf(false) }
    val activeIndex = remember(player.positionMs, player.lyrics, player.lyricsSynced) {
        if (!player.lyricsSynced) -1
        else player.lyrics.indexOfLast { it.startMs <= player.positionMs }
    }

    LaunchedEffect(listState.firstVisibleItemIndex) {
        if (listState.firstVisibleItemIndex >= LyricsHeaderIndex) lyricsPinned = true
    }
    LaunchedEffect(activeIndex, lyricsPinned, player.lyricsSynced) {
        if (lyricsPinned && player.lyricsSynced && activeIndex >= 0) {
            listState.animateScrollToItem(
                (LyricsStartIndex + activeIndex - 2).coerceAtLeast(LyricsStartIndex),
            )
        }
    }

    AppBackground(accentStart = track.cover.start) {
        BoxWithConstraints(modifier = modifier.fillMaxSize()) {
            val coverSize = minOf(maxWidth - 64.dp, 300.dp).coerceAtLeast(170.dp)

            Column(modifier = Modifier.fillMaxSize()) {
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
                            text = "SONANDO AHORA",
                            style = MaterialTheme.typography.labelSmall,
                            color = colors.accent,
                        )
                        Text(
                            text = track.album ?: "Tu biblioteca",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Spacer(Modifier.size(40.dp))
                }

                LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f),
                    contentPadding = PaddingValues(bottom = 16.dp),
                ) {
                    item(key = "cover") {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 8.dp),
                            horizontalArrangement = Arrangement.Center,
                        ) {
                            CoverArt(
                                cover = track.cover,
                                size = coverSize,
                                corner = 18.dp,
                                labelSize = 64.sp,
                            )
                        }
                    }
                    item(key = "meta") {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(start = 26.dp, end = 20.dp, top = 18.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = track.title,
                                    style = MaterialTheme.typography.headlineSmall,
                                    color = MaterialTheme.colorScheme.onBackground,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                Spacer(Modifier.height(4.dp))
                                Text(
                                    text = track.artist,
                                    style = MaterialTheme.typography.titleMedium,
                                    color = if (openArtist != null) {
                                        colors.accent
                                    } else {
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    },
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = if (openArtist != null) {
                                        Modifier.clickable { openArtist(track.artist) }
                                    } else {
                                        Modifier
                                    },
                                )
                            }
                            LikeButton(favorite = track.favorite, onClick = onLike)
                        }
                    }
                    item(key = "seek") {
                        Column(modifier = Modifier.padding(top = 12.dp)) {
                            SeekBar(
                                positionMs = player.positionMs,
                                durationMs = player.durationMs,
                                onSeek = onSeek,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 26.dp),
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
                        }
                    }
                    item(key = "controls") {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 24.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            CircleIconButton(
                                icon = Icons.Rounded.Shuffle,
                                contentDescription = "Aleatorio",
                                onClick = onShuffle,
                                size = 42.dp,
                                iconSize = 22.dp,
                                tint = if (player.shuffle) colors.accent else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            CircleIconButton(
                                icon = Icons.Rounded.SkipPrevious,
                                contentDescription = "Anterior",
                                onClick = onPrevious,
                                size = 50.dp,
                                iconSize = 32.dp,
                                tint = MaterialTheme.colorScheme.onBackground,
                            )
                            PlayButton(isPlaying = player.isPlaying, onClick = onTogglePlay, size = 72.dp)
                            CircleIconButton(
                                icon = Icons.Rounded.SkipNext,
                                contentDescription = "Siguiente",
                                onClick = onNext,
                                size = 50.dp,
                                iconSize = 32.dp,
                                tint = MaterialTheme.colorScheme.onBackground,
                            )
                            CircleIconButton(
                                icon = if (player.repeat == RepeatMode.One) {
                                    Icons.Rounded.RepeatOne
                                } else {
                                    Icons.Rounded.Repeat
                                },
                                contentDescription = "Repetir",
                                onClick = onRepeat,
                                size = 42.dp,
                                iconSize = 22.dp,
                                tint = if (player.repeat != RepeatMode.Off) {
                                    colors.accent
                                } else {
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                },
                            )
                        }
                    }
                    item(key = "lyrics-header") {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(start = 26.dp, end = 20.dp, top = 14.dp, bottom = 2.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = "LETRA",
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.accent,
                            )
                            Spacer(Modifier.weight(1f))
                            Text(
                                text = when {
                                    player.lyrics.isEmpty() -> ""
                                    player.lyricsSource == "genius" -> "Genius · sin tiempos"
                                    player.lyricsSynced -> "Sincronizada"
                                    else -> player.lyricsSource
                                        ?.replaceFirstChar { it.uppercase() }
                                        .orEmpty()
                                },
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                    if (player.lyrics.isEmpty()) {
                        item(key = "lyrics-empty") {
                            Column(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 26.dp, vertical = 16.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                            ) {
                                Text(
                                    text = if (player.lyricsLoading) "Buscando letra…" else "Sin letra",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                TextButton(onClick = onRetryLyrics) {
                                    Text("Buscar otra vez")
                                }
                            }
                        }
                    } else {
                        itemsIndexed(
                            player.lyrics,
                            key = { index, line -> "$index-${line.startMs}" },
                        ) { index, line ->
                            val active = player.lyricsSynced && index == activeIndex
                            Text(
                                text = line.text,
                                style = if (active) {
                                    MaterialTheme.typography.titleLarge
                                } else {
                                    MaterialTheme.typography.bodyLarge
                                },
                                color = if (active) {
                                    colors.accent
                                } else {
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 26.dp, vertical = 5.dp)
                                    .then(
                                        if (player.lyricsSynced) {
                                            Modifier.clickable { onSeek(line.startMs) }
                                        } else {
                                            Modifier
                                        },
                                    ),
                            )
                        }
                    }
                    item(key = "actions") {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 10.dp, vertical = 14.dp),
                            horizontalArrangement = Arrangement.SpaceEvenly,
                        ) {
                            LabelledAction(
                                icon = Icons.Rounded.Subtitles,
                                label = "Letra",
                                onClick = {
                                    lyricsPinned = true
                                    scope.launch {
                                        listState.animateScrollToItem(LyricsHeaderIndex)
                                    }
                                },
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
                    }
                }
            }
        }
    }
}

@Composable
private fun SeekBar(
    positionMs: Long,
    durationMs: Long,
    onSeek: (Long) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    var draggingFraction by remember { mutableStateOf<Float?>(null) }
    val actualFraction = if (durationMs > 0L) {
        (positionMs.toFloat() / durationMs.toFloat()).coerceIn(0f, 1f)
    } else {
        0f
    }
    val fraction = draggingFraction ?: actualFraction

    Canvas(
        modifier = modifier
            .height(30.dp)
            .pointerInput(durationMs) {
                if (durationMs <= 0L) return@pointerInput
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    var value = (down.position.x / size.width.toFloat()).coerceIn(0f, 1f)
                    draggingFraction = value
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        value = (change.position.x / size.width.toFloat()).coerceIn(0f, 1f)
                        draggingFraction = value
                        if (!change.pressed) {
                            change.consume()
                            break
                        }
                        change.consume()
                    }
                    onSeek((value * durationMs).toLong())
                    draggingFraction = null
                }
            },
    ) {
        val trackHeight = 3.dp.toPx()
        val centerY = size.height / 2f
        val radius = 6.dp.toPx()
        drawRoundRect(
            color = colors.outline.copy(alpha = 0.55f),
            topLeft = Offset(0f, centerY - trackHeight / 2f),
            size = Size(size.width, trackHeight),
            cornerRadius = CornerRadius(trackHeight / 2f, trackHeight / 2f),
        )
        drawRoundRect(
            color = colors.accent,
            topLeft = Offset(0f, centerY - trackHeight / 2f),
            size = Size((size.width * fraction).coerceAtLeast(0f), trackHeight),
            cornerRadius = CornerRadius(trackHeight / 2f, trackHeight / 2f),
        )
        drawCircle(
            color = colors.accent,
            radius = if (draggingFraction != null) radius * 1.25f else radius,
            center = Offset((size.width * fraction).coerceIn(0f, size.width), centerY),
        )
    }
}
