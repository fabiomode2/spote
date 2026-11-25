package dev.spote.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import dev.spote.app.model.DownloadStatus
import dev.spote.app.model.DownloadUi
import dev.spote.app.model.Track
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.CircleIconButton
import dev.spote.app.ui.components.EmptyState
import dev.spote.app.ui.components.SectionHeader
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.components.TrackRow
import dev.spote.app.ui.theme.LocalSpoteColors
import java.io.File

@Composable
fun DownloadsScreen(
    downloaded: List<Track>,
    onBack: () -> Unit,
    onPlayTrack: (Track) -> Unit,
    onMore: (Track) -> Unit,
    downloads: List<DownloadUi> = emptyList(),
    onCancelDownload: (String) -> Unit = {},
    onDismissDownload: (String) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val totalBytes = remember(downloaded) {
        downloaded.sumOf { track ->
            track.localPath
                ?.let { path -> File(path).takeIf(File::isFile)?.length() }
                ?: 0L
        }
    }

    AppBackground {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 28.dp),
        ) {
            item(key = "topbar") { TopBar(title = "Descargas", onBack = onBack) }
            item(key = "summary") {
                Surface(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 8.dp),
                    color = colors.surface,
                    shape = RoundedCornerShape(16.dp),
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                imageVector = Icons.Rounded.DownloadDone,
                                contentDescription = null,
                                tint = colors.accent,
                                modifier = Modifier.size(20.dp),
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(
                                text = "Disponible sin conexión",
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onBackground,
                            )
                            Spacer(Modifier.weight(1f))
                            Text(
                                text = formatSize(totalBytes),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Text(
                            text = if (downloaded.isEmpty()) {
                                "Todavía no has guardado ninguna canción."
                            } else {
                                if (downloaded.size == 1) {
                                    "1 canción en el almacenamiento privado de la app."
                                } else {
                                    "${downloaded.size} canciones en el almacenamiento privado de la app."
                                }
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            if (downloads.isNotEmpty()) {
                item(key = "active-title") { SectionHeader(title = "En curso") }
                items(downloads, key = { "dl-${it.key}" }) { download ->
                    ActiveDownloadRow(
                        download = download,
                        onCancel = {
                            if (download.status == DownloadStatus.Failed) {
                                onDismissDownload(download.key)
                            } else {
                                onCancelDownload(download.key)
                            }
                        },
                    )
                }
            }

            item(key = "done-title") { SectionHeader(title = "Completadas") }
            if (downloaded.isEmpty()) {
                item(key = "empty") {
                    EmptyState(
                        icon = Icons.Rounded.Download,
                        title = "Todavía no hay nada",
                        text = "Busca en YouTube y descarga una canción para oírla sin cobertura.",
                    )
                }
            } else {
                items(downloaded, key = { it.key }) { track ->
                    TrackRow(
                        track = track,
                        onClick = { onPlayTrack(track) },
                        onMore = { onMore(track) },
                        showAlbum = true,
                    )
                }
            }
        }
    }
}

@Composable
private fun ActiveDownloadRow(download: DownloadUi, onCancel: () -> Unit) {
    val colors = LocalSpoteColors.current
    val status = when (download.status) {
        DownloadStatus.Resolving -> "Resolviendo el stream…"
        DownloadStatus.Downloading -> "Descargando · ${(download.progress * 100).toInt()} %"
        DownloadStatus.Importing -> "Guardando en la biblioteca…"
        DownloadStatus.Failed -> download.error ?: "Error de descarga"
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                text = download.title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onBackground,
                maxLines = 1,
            )
            Text(
                text = status,
                style = MaterialTheme.typography.labelSmall,
                color = if (download.status == DownloadStatus.Failed) {
                    colors.accent
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                maxLines = 2,
            )
            if (download.status == DownloadStatus.Downloading ||
                download.status == DownloadStatus.Importing
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp)
                        .clip(RoundedCornerShape(50))
                        .background(colors.surfaceHigh),
                ) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth(
                                if (download.status == DownloadStatus.Importing) 1f
                                else download.progress.coerceIn(0f, 1f),
                            )
                            .height(4.dp)
                            .clip(RoundedCornerShape(50))
                            .background(colors.accent),
                    )
                }
            }
        }
        CircleIconButton(
            icon = Icons.Rounded.Close,
            contentDescription = if (download.status == DownloadStatus.Failed) "Descartar" else "Cancelar",
            onClick = onCancel,
            size = 32.dp,
            iconSize = 18.dp,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private fun formatSize(bytes: Long): String = when {
    bytes >= 1_000_000_000 -> String.format("%.1f GB", bytes / 1_000_000_000.0)
    bytes >= 1_000_000 -> String.format("%.0f MB", bytes / 1_000_000.0)
    bytes >= 1_000 -> String.format("%.0f KB", bytes / 1_000.0)
    else -> "$bytes B"
}
