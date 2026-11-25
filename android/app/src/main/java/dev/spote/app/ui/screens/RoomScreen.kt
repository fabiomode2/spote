package dev.spote.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Send
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.ChatMessage
import dev.spote.app.model.LyricLine
import dev.spote.app.model.Room
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.Avatar
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.CoverArt
import dev.spote.app.ui.components.LiveDot
import dev.spote.app.ui.components.PlayButton
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun RoomScreen(
    room: Room,
    queue: List<Track>,
    onBack: () -> Unit,
    onPlayTrack: (Track) -> Unit,
    onMore: (Track) -> Unit,
    messages: List<ChatMessage> = emptyList(),
    connected: Boolean = false,
    playing: Boolean = true,
    onTogglePlay: () -> Unit = {},
    onSendChat: (String) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    var draft by remember { mutableStateOf("") }

    AppBackground(accentStart = room.nowPlaying?.cover?.start ?: colors.coverStart) {
        Column(modifier = modifier.fillMaxSize()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                CircleIconButton(
                    icon = Icons.Rounded.KeyboardArrowDown,
                    contentDescription = "Cerrar sala",
                    onClick = onBack,
                    tint = MaterialTheme.colorScheme.onBackground,
                )
                Column(modifier = Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        LiveDot()
                        Spacer(Modifier.width(7.dp))
                        Text(
                            text = room.name,
                            style = MaterialTheme.typography.titleLarge,
                            color = MaterialTheme.colorScheme.onBackground,
                        )
                    }
                    Text(
                        text = if (connected) {
                            "${room.members.size} escuchando juntos"
                        } else {
                            "Conectando con la sala…"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    room.members.take(3).forEach { member ->
                        Avatar(
                            name = member,
                            size = 26.dp,
                            imageUri = room.memberAvatars[member],
                        )
                    }
                }
            }

            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(bottom = 24.dp),
            ) {
                item(key = "now") {
                    room.nowPlaying?.let { track ->
                        Surface(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 20.dp, vertical = 10.dp),
                            color = colors.surface,
                            shape = RoundedCornerShape(18.dp),
                        ) {
                            Column(
                                modifier = Modifier.padding(16.dp),
                                verticalArrangement = Arrangement.spacedBy(14.dp),
                            ) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    CoverArt(cover = track.cover, size = 64.dp, corner = 10.dp, labelSize = 17.sp)
                                    Spacer(Modifier.width(12.dp))
                                    Column(modifier = Modifier.weight(1f)) {
                                        Text(
                                            text = "EN LA SALA",
                                            style = MaterialTheme.typography.labelSmall,
                                            color = colors.accent,
                                        )
                                        Text(
                                            text = track.title,
                                            style = MaterialTheme.typography.titleLarge,
                                            color = MaterialTheme.colorScheme.onBackground,
                                            maxLines = 1,
                                        )
                                        Text(
                                            text = track.artist,
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                ) {
                                    PlayButton(isPlaying = playing, onClick = onTogglePlay, size = 46.dp)
                                    Text(
                                        text = "El tiempo lo manda el servidor. Todos escuchan su copia local.",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                            }
                        }
                    }
                }

                item(key = "queue-title") { QueueTitle() }
                items(queue.take(4), key = { it.key }) { track ->
                    TrackRow(
                        track = track,
                        onClick = { onPlayTrack(track) },
                        onMore = { onMore(track) },
                    )
                }

                item(key = "chat-title") { QueueTitle("CHAT") }
                if (messages.isEmpty()) {
                    item(key = "chat-empty") {
                        Text(
                            text = if (connected) {
                                "Aún no hay mensajes. Rompe el hielo."
                            } else {
                                "Conecta para ver el chat de la sala."
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                        )
                    }
                }
                items(messages.size, key = { "chat-$it" }) { index ->
                    val line = messages[index]
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp, vertical = 5.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.Top,
                    ) {
                        Text(
                            text = line.author,
                            style = MaterialTheme.typography.labelMedium,
                            color = colors.accent,
                            modifier = Modifier.width(60.dp),
                            maxLines = 1,
                        )
                        Text(
                            text = line.text,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onBackground,
                        )
                    }
                }
                item(key = "chat-input") {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp, vertical = 10.dp)
                            .background(colors.surfaceHigh, RoundedCornerShape(12.dp))
                            .padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        BasicTextField(
                            value = draft,
                            onValueChange = { draft = it },
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodyMedium.copy(
                                color = MaterialTheme.colorScheme.onBackground,
                            ),
                            cursorBrush = SolidColor(colors.accent),
                            modifier = Modifier.weight(1f),
                            decorationBox = { inner ->
                                if (draft.isEmpty()) {
                                    Text(
                                        text = "Escribe a la sala…",
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                inner()
                            },
                        )
                        CircleIconButton(
                            icon = Icons.Rounded.Send,
                            contentDescription = "Enviar",
                            onClick = {
                                onSendChat(draft)
                                draft = ""
                            },
                            size = 36.dp,
                            iconSize = 18.dp,
                            tint = colors.accent,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun QueueTitle(title: String = "COLA COMPARTIDA") {
    Text(
        text = title,
        style = MaterialTheme.typography.labelSmall,
        color = LocalSpoteColors.current.accent,
        modifier = Modifier.padding(start = 20.dp, top = 18.dp, bottom = 4.dp),
    )
}
