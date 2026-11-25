package dev.spote.app.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.spote.app.model.Album
import dev.spote.app.model.Cover
import dev.spote.app.model.Genre
import dev.spote.app.model.Playlist
import dev.spote.app.model.Room
import dev.spote.app.model.Track
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.util.asTrackDuration

@Composable
fun TrackRow(
    track: Track,
    onClick: () -> Unit,
    onMore: () -> Unit,
    modifier: Modifier = Modifier,
    isPlaying: Boolean = false,
    showCover: Boolean = true,
) {
    val colors = LocalSpoteColors.current
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (showCover) {
            CoverArt(cover = track.cover, size = 48.dp, corner = 8.dp, labelSize = 14.sp)
        }
        Column(modifier = Modifier.weight(1f)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (isPlaying) {
                    Icon(
                        imageVector = Icons.Rounded.GraphicEq,
                        contentDescription = "Sonando",
                        tint = colors.accent,
                        modifier = Modifier.size(15.dp),
                    )
                }
                Text(
                    text = track.title,
                    style = MaterialTheme.typography.titleMedium,
                    color = if (isPlaying) colors.accent else MaterialTheme.colorScheme.onBackground,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
            }
            Text(
                text = buildString {
                    append(track.artist)
                    if (track.explicit) append("  ·  E")
                    track.sourceLabel?.let { append("  ·  $it") }
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (track.downloaded) {
            Icon(
                imageVector = Icons.Rounded.DownloadDone,
                contentDescription = "Descargada",
                tint = colors.accent,
                modifier = Modifier.size(16.dp),
            )
        }
        Text(
            text = track.durationMs.asTrackDuration(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        CircleIconButton(
            icon = Icons.Rounded.MoreHoriz,
            contentDescription = "Más opciones",
            onClick = onMore,
            size = 32.dp,
            iconSize = 18.dp,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun AlbumTrackRow(
    track: Track,
    index: Int,
    onClick: () -> Unit,
    onMore: () -> Unit,
    modifier: Modifier = Modifier,
    isPlaying: Boolean = false,
) {
    val colors = LocalSpoteColors.current
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(modifier = Modifier.width(24.dp), contentAlignment = Alignment.CenterStart) {
            if (isPlaying) {
                Icon(
                    imageVector = Icons.Rounded.GraphicEq,
                    contentDescription = "Sonando",
                    tint = colors.accent,
                    modifier = Modifier.size(16.dp),
                )
            } else {
                Text(
                    text = index.toString().padStart(2, '0'),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = track.title,
                style = MaterialTheme.typography.titleMedium,
                color = if (isPlaying) colors.accent else MaterialTheme.colorScheme.onBackground,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = track.artist,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (track.downloaded) {
            Icon(
                imageVector = Icons.Rounded.CheckCircle,
                contentDescription = "Descargada",
                tint = colors.success,
                modifier = Modifier.size(15.dp),
            )
        }
        Text(
            text = track.durationMs.asTrackDuration(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        CircleIconButton(
            icon = Icons.Rounded.MoreHoriz,
            contentDescription = "Más opciones",
            onClick = onMore,
            size = 30.dp,
            iconSize = 17.dp,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun CollectionRow(
    cover: Cover,
    title: String,
    subtitle: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    badge: String? = null,
) {
    val colors = LocalSpoteColors.current
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        CoverArt(cover = cover, size = 54.dp, corner = 6.dp, labelSize = 15.sp)
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onBackground,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (badge != null) {
            Surface(
                color = colors.surfaceHigh,
                contentColor = colors.accent,
                shape = RoundedCornerShape(50),
            ) {
                Text(
                    text = badge,
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
    }
}

@Composable
fun RecentCard(track: Track, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = 148.dp) {
    Column(
        modifier = modifier
            .width(width)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .padding(bottom = 4.dp),
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        CoverArt(cover = track.cover, size = width, corner = 12.dp, labelSize = 30.sp)
        Text(
            text = track.title,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onBackground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = track.artist,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
fun AlbumCard(album: Album, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = 156.dp) {
    Column(
        modifier = modifier
            .width(width)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .padding(bottom = 4.dp),
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        CoverArt(cover = album.cover, size = width, corner = 12.dp, labelSize = 28.sp)
        Text(
            text = album.title,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onBackground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = album.artist,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
fun PlaylistCard(playlist: Playlist, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = 156.dp) {
    Column(
        modifier = modifier
            .width(width)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .padding(bottom = 4.dp),
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        CoverArt(cover = playlist.cover, size = width, corner = 12.dp, labelSize = 28.sp)
        Text(
            text = playlist.name,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onBackground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = "${playlist.trackCount} canciones",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
        )
    }
}

@Composable
fun GenreCard(genre: Genre, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .height(82.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(Brush.linearGradient(listOf(genre.cover.start, genre.cover.end)))
            .clickable(onClick = onClick)
            .padding(12.dp),
    ) {
        Box(
            modifier = Modifier
                .size(44.dp)
                .align(Alignment.BottomEnd)
                .clip(CircleShape)
                .background(Color.White.copy(alpha = 0.12f)),
        )
        Text(
            text = genre.name,
            style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold),
            color = Color.White,
        )
    }
}

@Composable
fun RoomCard(room: Room, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val colors = LocalSpoteColors.current
    Surface(
        modifier = modifier
            .width(286.dp)
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick),
        color = colors.surface,
        shape = RoundedCornerShape(16.dp),
    ) {
        Column(modifier = Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (room.live) LiveDot()
                Spacer(Modifier.width(7.dp))
                Text(
                    text = room.name,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onBackground,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    room.members.take(3).forEach { member ->
                        Avatar(
                            name = member,
                            size = 24.dp,
                            imageUri = room.memberAvatars[member],
                        )
                    }
                }
            }
            room.nowPlaying?.let { track ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    CoverArt(cover = track.cover, size = 38.dp, corner = 6.dp, labelSize = 12.sp)
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = track.title,
                            style = MaterialTheme.typography.titleSmall,
                            color = MaterialTheme.colorScheme.onBackground,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = track.artist,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                        )
                    }
                    Icon(
                        imageVector = Icons.Rounded.GraphicEq,
                        contentDescription = null,
                        tint = colors.accent,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }
        }
    }
}

@Composable
fun ArtistCircle(name: String, cover: Cover, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .width(112.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Box(
            modifier = Modifier
                .size(96.dp)
                .clip(CircleShape),
        ) {
            CoverArt(cover = cover, size = 96.dp, corner = 48.dp, labelSize = 26.sp)
        }
        Text(
            text = name,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onBackground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
