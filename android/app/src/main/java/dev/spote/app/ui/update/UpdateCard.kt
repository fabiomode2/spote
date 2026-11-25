package dev.spote.app.ui.update

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SystemUpdate
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.spote.app.BuildConfig
import dev.spote.app.update.UpdateUiState
import dev.spote.app.update.UpdateViewModel
import kotlin.math.roundToInt

/**
 * Tarjeta de actualizaciones lista para colocar en Ajustes o en la home.
 *
 * El [UpdateViewModel] se puede obtener con `viewModel()`: su constructor
 * `(Application)` lo resuelve la factory por defecto de AndroidX.
 */
@Composable
fun UpdateCard(viewModel: UpdateViewModel, modifier: Modifier = Modifier) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    UpdateCardContent(
        state = state,
        onCheck = { viewModel.check(force = true) },
        onDownload = viewModel::download,
        onInstall = viewModel::install,
        onDismiss = viewModel::dismiss,
        modifier = modifier,
    )
}

@Composable
internal fun UpdateCardContent(
    state: UpdateUiState,
    onCheck: () -> Unit,
    onDownload: () -> Unit,
    onInstall: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val currentVersion = when (state) {
        is UpdateUiState.UpToDate -> state.current
        is UpdateUiState.Available -> state.current
        else -> BuildConfig.VERSION_NAME
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            contentColor = MaterialTheme.colorScheme.onSurface,
        ),
        elevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(36.dp)
                        .background(
                            color = MaterialTheme.colorScheme.primaryContainer,
                            shape = CircleShape,
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Rounded.SystemUpdate,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(20.dp),
                    )
                }

                Spacer(Modifier.width(12.dp))

                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "Actualizaciones",
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = "Versión actual $currentVersion",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                when (state) {
                    is UpdateUiState.Checking -> CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                    )

                    is UpdateUiState.UpToDate -> Icon(
                        imageVector = Icons.Rounded.CheckCircle,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(20.dp),
                    )

                    else -> Unit
                }
            }

            Spacer(Modifier.height(12.dp))

            when (state) {
                UpdateUiState.Idle -> IdleContent(onCheck)
                UpdateUiState.Checking -> CheckingContent()
                is UpdateUiState.UpToDate -> UpToDateContent(onCheck)
                is UpdateUiState.Available -> AvailableContent(state, onDownload, onDismiss)
                is UpdateUiState.Downloading -> DownloadingContent(state, onDismiss)
                is UpdateUiState.Ready -> ReadyContent(state, onInstall)
                is UpdateUiState.Error -> ErrorContent(state, onCheck)
            }
        }
    }
}

@Composable
private fun IdleContent(onCheck: () -> Unit) {
    Text(
        text = "Comprueba si hay una versión nueva publicada.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(10.dp))
    UpdateActionButton(text = "Buscar", icon = Icons.Rounded.Refresh, onClick = onCheck)
}

@Composable
private fun CheckingContent() {
    Text(
        text = "Buscando actualizaciones…",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun UpToDateContent(onCheck: () -> Unit) {
    Text(
        text = "Estás al día",
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = FontWeight.Medium,
        color = MaterialTheme.colorScheme.primary,
    )
    Spacer(Modifier.height(10.dp))
    UpdateActionButton(text = "Buscar", icon = Icons.Rounded.Refresh, onClick = onCheck)
}

@Composable
private fun AvailableContent(
    state: UpdateUiState.Available,
    onDownload: () -> Unit,
    onDismiss: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            text = "Nueva versión",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(8.dp))
        VersionBadge(state.latest)
    }

    if (state.notes.isNotBlank()) {
        Spacer(Modifier.height(8.dp))
        Text(
            text = state.notes.trim(),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
        )
    }

    Spacer(Modifier.height(12.dp))
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        UpdateActionButton(text = "Descargar", icon = Icons.Rounded.Download, onClick = onDownload)
        TextButton(
            onClick = onDismiss,
            contentPadding = PaddingValues(horizontal = 8.dp),
        ) {
            Text(text = "Ahora no", style = MaterialTheme.typography.labelLarge)
        }
    }
}

@Composable
private fun DownloadingContent(
    state: UpdateUiState.Downloading,
    onDismiss: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            text = "Descargando ${state.latest}…",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
        )
        Text(
            text = "${(state.progress * 100).roundToInt()}%",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.primary,
        )
    }
    Spacer(Modifier.height(8.dp))
    LinearProgressIndicator(
        progress = { state.progress.coerceIn(0f, 1f) },
        modifier = Modifier
            .fillMaxWidth()
            .height(4.dp),
    )
    Spacer(Modifier.height(4.dp))
    TextButton(
        onClick = onDismiss,
        contentPadding = PaddingValues(horizontal = 0.dp, vertical = 4.dp),
    ) {
        Text(text = "Cancelar", style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun ReadyContent(
    state: UpdateUiState.Ready,
    onInstall: () -> Unit,
) {
    Text(
        text = "La versión ${state.latest} está descargada. El sistema pedirá confirmación para instalarla.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(12.dp))
    UpdateActionButton(
        text = "Instalar",
        icon = Icons.Rounded.SystemUpdate,
        onClick = onInstall,
    )
}

@Composable
private fun ErrorContent(
    state: UpdateUiState.Error,
    onCheck: () -> Unit,
) {
    Text(
        text = state.message,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
    )
    Spacer(Modifier.height(10.dp))
    UpdateActionButton(text = "Buscar", icon = Icons.Rounded.Refresh, onClick = onCheck)
}

@Composable
private fun VersionBadge(version: String) {
    Surface(
        color = MaterialTheme.colorScheme.primaryContainer,
        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
        shape = RoundedCornerShape(6.dp),
    ) {
        Text(
            text = version,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
        )
    }
}

@Composable
private fun UpdateActionButton(
    text: String,
    icon: ImageVector,
    onClick: () -> Unit,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.heightIn(min = 40.dp),
        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(18.dp),
        )
        Spacer(Modifier.width(6.dp))
        Text(text = text, style = MaterialTheme.typography.labelLarge)
    }
}
