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
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.spote.app.model.Album
import dev.spote.app.model.HomeState
import dev.spote.app.model.Playlist
import dev.spote.app.model.Room
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AlbumCard
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.Avatar
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.FilterChips
import dev.spote.app.ui.components.PlaylistCard
import dev.spote.app.ui.components.RecentCard
import dev.spote.app.ui.components.RoomCard
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun HomeScreen(
    state: HomeState,
    onPlayTrack: (Track) -> Unit,
    onOpenAlbum: (Album) -> Unit,
    onOpenPlaylist: (Playlist) -> Unit,
    onOpenRoom: (Room) -> Unit,
    onOpenSettings: () -> Unit,
    onImport: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    var selectedChip by rememberSaveable { mutableStateOf(state.chips.firstOrNull() ?: "Todo") }
    AppBackground(accentStart = state.recent.firstOrNull()?.cover?.start ?: colors.coverStart) {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 24.dp),
        ) {
            item(key = "header") {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .statusBarsPadding()
                        .padding(top = 10.dp, bottom = 4.dp),
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = "spote",
                                style = MaterialTheme.typography.labelMedium,
                                color = colors.accent,
                            )
                            Text(
                                text = state.greeting,
                                style = MaterialTheme.typography.displayMedium,
                                color = MaterialTheme.colorScheme.onBackground,
                            )
                        }
                        CircleIconButton(
                            icon = Icons.Rounded.Settings,
                            contentDescription = "Ajustes",
                            onClick = onOpenSettings,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(Modifier.width(6.dp))
                        Avatar(name = "Tú", size = 34.dp)
                    }
                    Spacer(Modifier.height(14.dp))
                    FilterChips(
                        labels = state.chips,
                        selected = selectedChip,
                        onSelect = { selectedChip = it },
                    )
                }
            }

            item(key = "recent-title") {
                SectionHeader(title = "Sigue escuchando")
            }
            if (state.recent.isEmpty()) {
                item(key = "empty-library") {
                    EmptyLibraryCard(onImport = onImport)
                }
            } else {
                item(key = "recent") {
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        items(state.recent, key = { it.key }) { track ->
                            RecentCard(track = track, onClick = { onPlayTrack(track) })
                        }
                    }
                }
            }

            if (state.rooms.isNotEmpty()) {
                item(key = "rooms-title") {
                    SectionHeader(title = "Tus salas")
                }
                item(key = "rooms") {
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        items(state.rooms, key = { it.id }) { room ->
                            RoomCard(room = room, onClick = { onOpenRoom(room) })
                        }
                    }
                }
            }

            if (state.playlists.isNotEmpty()) {
                item(key = "playlists-title") {
                    SectionHeader(title = "Tus playlists")
                }
                item(key = "playlists") {
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        items(state.playlists, key = { it.id }) { playlist ->
                            PlaylistCard(playlist = playlist, onClick = { onOpenPlaylist(playlist) })
                        }
                    }
                }
            }

            if (state.albums.isNotEmpty()) {
                item(key = "albums-title") {
                    SectionHeader(title = "Añadido recientemente")
                }
                items(
                    state.albums.chunked(2),
                    key = { row -> row.joinToString("|") { it.id } },
                ) { row ->
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
        }
    }
}

@Composable
private fun EmptyLibraryCard(onImport: () -> Unit) {
    val colors = LocalSpoteColors.current
    androidx.compose.material3.Surface(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 6.dp),
        color = colors.surface,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(16.dp),
    ) {
        Column(
            modifier = Modifier.padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                text = "Tu biblioteca está vacía",
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onBackground,
            )
            Text(
                text = "Importa audio del teléfono y aparecerá aquí, listo para sonar sin conexión.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            dev.spote.app.ui.components.AccentPill(
                text = "Importar música",
                onClick = onImport,
                icon = Icons.Rounded.Add,
            )
        }
    }
}
