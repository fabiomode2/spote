package dev.spote.app.ui.screens

import android.content.Context
import android.os.StatFs
import android.text.format.Formatter
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.CloudDownload
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.Group
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Palette
import androidx.compose.material.icons.rounded.Security
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.spote.app.BuildConfig
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.Avatar
import dev.spote.app.ui.components.SettingsRow
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.ui.theme.SpoteThemeMode

@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onOpenDownloads: () -> Unit,
    updateContent: @Composable () -> Unit,
    themeMode: SpoteThemeMode = SpoteThemeMode.Midnight,
    onThemeModeChange: (SpoteThemeMode) -> Unit = {},
    downloadedCount: Int = 0,
    storageLabel: String = "—",
    socialLoggedIn: Boolean = false,
    socialBusy: Boolean = false,
    socialConnecting: Boolean = false,
    socialUser: String = "",
    socialServer: String = "",
    socialError: String? = null,
    socialStorageEncrypted: Boolean = true,
    onSocialLogin: (url: String, key: String, name: String, invite: String) -> Unit = { _, _, _, _ -> },
    onSocialLogout: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val uriHandler = LocalUriHandler.current
    val context = LocalContext.current
    val prefs = remember {
        context.getSharedPreferences("spote.settings", Context.MODE_PRIVATE)
    }
    var wifiOnly by remember { mutableStateOf(prefs.getBoolean("wifi_only", true)) }
    var normalize by remember { mutableStateOf(prefs.getBoolean("normalize", true)) }
    var effects by remember { mutableStateOf(prefs.getBoolean("effects", true)) }
    var animatedLyrics by remember { mutableStateOf(prefs.getBoolean("lyrics", true)) }
    var showThemePicker by remember { mutableStateOf(false) }
    var showSocialDialog by remember { mutableStateOf(false) }
    var cacheSize by remember { mutableLongStateOf(dirSize(context.cacheDir)) }
    val freeSpace = remember {
        runCatching { StatFs(context.filesDir.path).availableBytes }.getOrDefault(0L)
    }

    fun persist(key: String, value: Boolean) {
        prefs.edit().putBoolean(key, value).apply()
    }

    AppBackground {
        LazyColumn(
            modifier = modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = 32.dp),
        ) {
            item(key = "topbar") {
                TopBar(title = "Ajustes", onBack = onBack)
            }
            item(key = "profile") {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Avatar(name = socialUser.ifBlank { "Tú" }, size = 56.dp)
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = socialUser.ifBlank { "Tú" },
                            style = MaterialTheme.typography.titleLarge,
                            color = MaterialTheme.colorScheme.onBackground,
                        )
                        Text(
                            text = if (socialLoggedIn) {
                                if (socialConnecting) "Sala social · conectando…" else "Sala social · en línea"
                            } else {
                                "Sesión local · sin servidor"
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        text = "Local",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            item(key = "playback-title") { SettingsGroup("REPRODUCCIÓN") }
            item(key = "wifi") {
                SettingsRow(
                    title = "Descargar solo por WiFi",
                    subtitle = "Ahorra datos y batería",
                    icon = Icons.Rounded.Wifi,
                    trailing = {
                        Switch(
                            checked = wifiOnly,
                            onCheckedChange = {
                                wifiOnly = it
                                persist("wifi_only", it)
                            },
                            colors = spoteSwitchColors(),
                        )
                    },
                )
            }
            item(key = "normalize") {
                SettingsRow(
                    title = "Normalizar volumen",
                    subtitle = "Todas las pistas al mismo nivel",
                    icon = Icons.Rounded.VolumeUp,
                    trailing = {
                        Switch(
                            checked = normalize,
                            onCheckedChange = {
                                normalize = it
                                persist("normalize", it)
                            },
                            colors = spoteSwitchColors(),
                        )
                    },
                )
            }
            item(key = "appearance-title") { SettingsGroup("APARIENCIA") }
            item(key = "theme") {
                SettingsRow(
                    title = "Tema",
                    subtitle = themeMode.subtitle,
                    icon = Icons.Rounded.Palette,
                    onClick = { showThemePicker = true },
                    trailing = {
                        Text(
                            text = themeMode.label,
                            style = MaterialTheme.typography.labelLarge,
                            color = colors.accent,
                        )
                    },
                )
            }
            item(key = "effects") {
                SettingsRow(
                    title = "Efectos de fondo",
                    subtitle = "Mosaico y ambiente a 30 fps",
                    icon = Icons.Rounded.GraphicEq,
                    trailing = {
                        Switch(
                            checked = effects,
                            onCheckedChange = {
                                effects = it
                                persist("effects", it)
                            },
                            colors = spoteSwitchColors(),
                        )
                    },
                )
            }
            item(key = "lyrics") {
                SettingsRow(
                    title = "Letras animadas",
                    subtitle = "Karaoke palabra a palabra",
                    icon = Icons.Rounded.GraphicEq,
                    trailing = {
                        Switch(
                            checked = animatedLyrics,
                            onCheckedChange = {
                                animatedLyrics = it
                                persist("lyrics", it)
                            },
                            colors = spoteSwitchColors(),
                        )
                    },
                )
            }

            item(key = "storage-title") { SettingsGroup("ALMACENAMIENTO") }
            item(key = "downloads") {
                SettingsRow(
                    title = "Descargas",
                    subtitle = "$storageLabel · $downloadedCount canciones",
                    icon = Icons.Rounded.DownloadDone,
                    onClick = onOpenDownloads,
                )
            }
            item(key = "cache") {
                SettingsRow(
                    title = "Borrar caché",
                    subtitle = "Portadas y letras temporales · ${formatBytes(cacheSize)}",
                    icon = Icons.Rounded.DeleteSweep,
                    onClick = {
                        context.cacheDir.listFiles()?.forEach { it.deleteRecursively() }
                        cacheSize = 0L
                    },
                )
            }
            item(key = "storage") {
                SettingsRow(
                    title = "Espacio libre",
                    subtitle = "${formatBytes(freeSpace)} disponibles",
                    icon = Icons.Rounded.Storage,
                )
            }

            item(key = "social-title") { SettingsGroup("SALAS") }
            if (socialLoggedIn) {
                item(key = "social-session") {
                    SettingsRow(
                        title = "Sala social",
                        subtitle = "${socialUser.ifBlank { "Tú" }} · $socialServer",
                        icon = Icons.Rounded.Group,
                    )
                }
                item(key = "social-logout") {
                    SettingsRow(
                        title = "Cerrar sesión",
                        subtitle = "Revoca el token y lo borra del dispositivo",
                        icon = Icons.Rounded.Security,
                        onClick = onSocialLogout,
                    )
                }
            } else {
                item(key = "social-connect") {
                    SettingsRow(
                        title = "Conectar sala",
                        subtitle = "Servidor, clave de cliente e invitación",
                        icon = Icons.Rounded.Group,
                        onClick = { showSocialDialog = true },
                    )
                }
            }
            if (!socialStorageEncrypted) {
                item(key = "social-storage") {
                    Text(
                        text = "El token se guarda en almacenamiento privado sin cifrar: el Keystore no está disponible.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
                    )
                }
            }
            if (socialError != null) {
                item(key = "social-error") {
                    Text(
                        text = socialError,
                        style = MaterialTheme.typography.bodySmall,
                        color = colors.accent,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
                    )
                }
            }

            item(key = "updates-title") { SettingsGroup("ACTUALIZACIONES") }
            item(key = "updates") { updateContent() }

            item(key = "about-title") { SettingsGroup("ACERCA DE") }
            item(key = "version") {
                SettingsRow(
                    title = "Versión",
                    subtitle = BuildConfig.VERSION_NAME,
                    icon = Icons.Rounded.Info,
                )
            }
            item(key = "github") {
                SettingsRow(
                    title = "GitHub",
                    subtitle = BuildConfig.GITHUB_REPO,
                    icon = Icons.Rounded.CloudDownload,
                    onClick = {
                        uriHandler.openUri("https://github.com/${BuildConfig.GITHUB_REPO}")
                    },
                )
            }
            item(key = "licenses") {
                SettingsRow(
                    title = "Licencias",
                    subtitle = "Rust core MIT · app GPL-3.0",
                    icon = Icons.Rounded.Info,
                )
            }
        }

        if (showThemePicker) {
            ThemePickerDialog(
                current = themeMode,
                onSelect = { mode ->
                    onThemeModeChange(mode)
                    showThemePicker = false
                },
                onDismiss = { showThemePicker = false },
            )
        }

        if (showSocialDialog) {
            SocialLoginDialog(
                initialServer = socialServer,
                initialName = socialUser,
                busy = socialBusy,
                error = socialError,
                onDismiss = { showSocialDialog = false },
                onSubmit = { url, key, name, invite ->
                    onSocialLogin(url, key, name, invite)
                    showSocialDialog = false
                },
            )
        }
    }
}

@Composable
private fun SocialLoginDialog(
    initialServer: String,
    initialName: String,
    busy: Boolean,
    error: String?,
    onDismiss: () -> Unit,
    onSubmit: (url: String, key: String, name: String, invite: String) -> Unit,
) {
    var server by remember { mutableStateOf(initialServer) }
    var clientKey by remember { mutableStateOf("") }
    var name by remember { mutableStateOf(initialName) }
    var invite by remember { mutableStateOf("") }
    val canSubmit = !busy &&
        server.isNotBlank() &&
        clientKey.isNotBlank() &&
        name.isNotBlank() &&
        invite.isNotBlank()

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Conectar sala", style = MaterialTheme.typography.headlineSmall) },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    text = "Los datos se guardan cifrados con el Keystore. Usa un servidor HTTPS y la clave que te haya dado el anfitrión.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedTextField(
                    value = server,
                    onValueChange = { server = it },
                    label = { Text("Servidor") },
                    placeholder = { Text("https://spote.example.com") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Tu nombre") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = clientKey,
                    onValueChange = { clientKey = it },
                    label = { Text("Clave de cliente") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = invite,
                    onValueChange = { invite = it },
                    label = { Text("Invitación") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                if (error != null) {
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodySmall,
                        color = LocalSpoteColors.current.accent,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = canSubmit,
                onClick = { onSubmit(server.trim(), clientKey.trim(), name.trim(), invite.trim()) },
            ) {
                Text(if (busy) "Entrando…" else "Entrar")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancelar") }
        },
        containerColor = MaterialTheme.colorScheme.surface,
    )
}

@Composable
fun ThemePickerDialog(
    current: SpoteThemeMode,
    onSelect: (SpoteThemeMode) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "Elige tema",
                style = MaterialTheme.typography.headlineSmall,
            )
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                SpoteThemeMode.entries.forEach { mode ->
                    ThemeOption(
                        mode = mode,
                        selected = mode == current,
                        onClick = { onSelect(mode) },
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Cerrar") }
        },
        containerColor = MaterialTheme.colorScheme.surface,
    )
}

@Composable
private fun ThemeOption(
    mode: SpoteThemeMode,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val palette = mode.colors()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .clickable(onClick = onClick)
            .padding(8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            modifier = Modifier
                .size(46.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(
                    Brush.linearGradient(
                        listOf(palette.background, palette.surfaceHigh, palette.accent),
                    ),
                ),
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = mode.label,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onBackground,
            )
            Text(
                text = mode.subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (selected) {
            Icon(
                imageVector = Icons.Rounded.Check,
                contentDescription = null,
                tint = palette.accent,
            )
        }
    }
}

private fun dirSize(file: java.io.File): Long =
    if (!file.exists()) 0L
    else file.walkTopDown().filter { it.isFile }.sumOf { it.length() }

private fun formatBytes(bytes: Long): String = when {
    bytes >= 1_000_000_000 -> String.format("%.1f GB", bytes / 1_000_000_000.0)
    bytes >= 1_000_000 -> String.format("%.0f MB", bytes / 1_000_000.0)
    bytes >= 1_000 -> String.format("%.0f KB", bytes / 1_000.0)
    else -> "$bytes B"
}

@Composable
private fun SettingsGroup(title: String) {
    Text(
        text = title,
        style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold),
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp),
    )
}

@Composable
private fun spoteSwitchColors() = SwitchDefaults.colors(
    checkedThumbColor = LocalSpoteColors.current.accentInk,
    checkedTrackColor = LocalSpoteColors.current.accent,
    checkedBorderColor = LocalSpoteColors.current.accent,
    uncheckedThumbColor = MaterialTheme.colorScheme.onSurfaceVariant,
    uncheckedTrackColor = LocalSpoteColors.current.surfaceHigh,
    uncheckedBorderColor = MaterialTheme.colorScheme.outline,
)
