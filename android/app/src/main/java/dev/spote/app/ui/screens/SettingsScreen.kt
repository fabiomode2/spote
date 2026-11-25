package dev.spote.app.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.StatFs
import android.text.format.Formatter
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.Group
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Palette
import androidx.compose.material.icons.rounded.Security
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import dev.spote.app.logging.ActionLog
import dev.spote.app.setup.QrCodeImage
import dev.spote.app.setup.SetupLink
import dev.spote.app.setup.SetupLinkBus
import dev.spote.app.setup.rememberQrScanner
import dev.spote.app.ui.components.AppBackground
import dev.spote.app.ui.components.Avatar
import dev.spote.app.ui.components.SettingsRow
import dev.spote.app.ui.components.TopBar
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.ui.theme.SpoteThemeMode
import dev.spote.app.ui.theme.SpoteUiStyle

@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onOpenDownloads: () -> Unit,
    updateContent: @Composable () -> Unit,
    themeMode: SpoteThemeMode = SpoteThemeMode.Midnight,
    onThemeModeChange: (SpoteThemeMode) -> Unit = {},
    uiStyle: SpoteUiStyle = SpoteUiStyle.Medium,
    onUiStyleChange: (SpoteUiStyle) -> Unit = {},
    downloadedCount: Int = 0,
    storageLabel: String = "—",
    profileName: String = "",
    avatarPath: String? = null,
    onProfileNameChange: (String) -> Unit = {},
    onAvatarSelected: (Uri) -> Unit = {},
    socialLoggedIn: Boolean = false,
    socialBusy: Boolean = false,
    socialConnecting: Boolean = false,
    socialUser: String = "",
    socialServer: String = "",
    socialClientKey: String = "",
    socialInvite: String = "",
    socialError: String? = null,
    socialStorageEncrypted: Boolean = true,
    onSocialLogin: (url: String, key: String, name: String, invite: String) -> Unit = { _, _, _, _ -> },
    onSocialLogout: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val uriHandler = LocalUriHandler.current
    val context = LocalContext.current
    var showThemePicker by remember { mutableStateOf(false) }
    var showStylePicker by remember { mutableStateOf(false) }
    var showSocialDialog by remember { mutableStateOf(false) }
    var showProfileDialog by remember { mutableStateOf(false) }
    var draftName by remember { mutableStateOf(profileName) }
    var qrLink by remember { mutableStateOf<SetupLink?>(null) }
    val scanQr = rememberQrScanner(
        onResult = { raw ->
            if (!SetupLinkBus.publish(raw)) {
                Toast.makeText(context, "Ese QR no es de Spote", Toast.LENGTH_SHORT).show()
            }
        },
    )
    fun currentSetupLink(): SetupLink? = if (
        socialServer.isNotBlank() &&
        socialClientKey.isNotBlank() &&
        socialInvite.isNotBlank()
    ) {
        SetupLink(
            serverUrl = socialServer,
            clientKey = socialClientKey,
            invite = socialInvite,
            name = socialUser.takeIf { it.isNotBlank() },
        )
    } else {
        null
    }
    val avatarLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.GetContent(),
    ) { uri -> uri?.let(onAvatarSelected) }
    var cacheSize by remember { mutableLongStateOf(dirSize(context.cacheDir)) }
    val freeSpace = remember {
        runCatching { StatFs(context.filesDir.path).availableBytes }.getOrDefault(0L)
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
                    Avatar(
                        name = socialUser.ifBlank { profileName.ifBlank { "Tú" } },
                        size = 56.dp,
                        imageUri = avatarPath,
                        modifier = Modifier.clip(RoundedCornerShape(50)).clickable {
                            draftName = profileName
                            showProfileDialog = true
                        },
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = socialUser.ifBlank { profileName.ifBlank { "Tú" } },
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
            item(key = "ui-style") {
                SettingsRow(
                    title = "Estilo TUI",
                    subtitle = uiStyle.subtitle,
                    icon = Icons.Rounded.Code,
                    onClick = { showStylePicker = true },
                    trailing = {
                        Text(
                            text = uiStyle.label,
                            style = MaterialTheme.typography.labelLarge,
                            color = colors.accent,
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
            item(key = "social-scan") {
                SettingsRow(
                    title = "Escanear QR",
                    subtitle = "Abre la app con el servidor ya configurado",
                    icon = Icons.Rounded.Group,
                    onClick = scanQr,
                )
            }
            if (currentSetupLink() != null) {
                item(key = "social-qr") {
                    SettingsRow(
                        title = "Mostrar QR y enlace",
                        subtitle = "Para configurar otro móvil o el desktop",
                        icon = Icons.Rounded.Code,
                        onClick = { qrLink = currentSetupLink() },
                    )
                }
            }
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

            item(key = "logs-title") { SettingsGroup("DIAGNÓSTICO") }
            item(key = "share-log") {
                SettingsRow(
                    title = "Compartir registro",
                    subtitle = "Acciones, navegación, errores y tiempos",
                    icon = Icons.Rounded.Info,
                    onClick = {
                        ActionLog.log("log.share")
                        val intent = ActionLog.shareIntent()
                        if (intent != null) {
                            context.startActivity(
                                Intent.createChooser(intent, "Compartir registro de Spote"),
                            )
                        } else {
                            Toast.makeText(context, "Todavía no hay registro", Toast.LENGTH_SHORT).show()
                        }
                    },
                )
            }
            item(key = "clear-log") {
                SettingsRow(
                    title = "Borrar registro",
                    subtitle = "Elimina los logs locales",
                    icon = Icons.Rounded.DeleteSweep,
                    onClick = {
                        ActionLog.clear()
                        Toast.makeText(context, "Registro borrado", Toast.LENGTH_SHORT).show()
                    },
                )
            }

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

        qrLink?.let { link ->
            val encoded = link.encode()
            AlertDialog(
                onDismissRequest = { qrLink = null },
                title = { Text("Configurar Spote") },
                text = {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        QrCodeImage(text = encoded, size = 240.dp)
                        Text(
                            text = "El QR contiene la clave de cliente y la invitación. Compártelo solo con quien deba entrar.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = encoded,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 3,
                        )
                    }
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            val share = Intent(Intent.ACTION_SEND).apply {
                                type = "text/plain"
                                putExtra(Intent.EXTRA_SUBJECT, "Spote")
                                putExtra(Intent.EXTRA_TEXT, encoded)
                            }
                            context.startActivity(
                                Intent.createChooser(share, "Compartir configuración"),
                            )
                        },
                    ) { Text("Compartir") }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            val clipboard = context.getSystemService(ClipboardManager::class.java)
                            clipboard?.setPrimaryClip(
                                ClipData.newPlainText("Spote", encoded),
                            )
                            Toast.makeText(context, "Enlace copiado", Toast.LENGTH_SHORT).show()
                        },
                    ) { Text("Copiar enlace") }
                },
                containerColor = MaterialTheme.colorScheme.surface,
            )
        }

        if (showProfileDialog) {
            AlertDialog(
                onDismissRequest = { showProfileDialog = false },
                title = { Text("Tu perfil") },
                text = {
                    Column(
                        verticalArrangement = Arrangement.spacedBy(14.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Avatar(
                            name = draftName.ifBlank { "Tú" },
                            size = 88.dp,
                            imageUri = avatarPath,
                        )
                        TextButton(onClick = { avatarLauncher.launch("image/*") }) {
                            Text("Cambiar foto")
                        }
                        OutlinedTextField(
                            value = draftName,
                            onValueChange = { draftName = it.take(32) },
                            label = { Text("Nombre") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            onProfileNameChange(draftName.trim())
                            showProfileDialog = false
                        },
                    ) { Text("Guardar") }
                },
                dismissButton = {
                    TextButton(onClick = { showProfileDialog = false }) { Text("Cancelar") }
                },
                containerColor = MaterialTheme.colorScheme.surface,
            )
        }

        if (showStylePicker) {
            StylePickerDialog(
                current = uiStyle,
                onSelect = { style ->
                    onUiStyleChange(style)
                    showStylePicker = false
                },
                onDismiss = { showStylePicker = false },
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
private fun StylePickerDialog(
    current: SpoteUiStyle,
    onSelect: (SpoteUiStyle) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Estilo TUI", style = MaterialTheme.typography.headlineSmall) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                SpoteUiStyle.entries.forEach { style ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(14.dp))
                            .clickable { onSelect(style) }
                            .padding(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = "\$",
                            style = MaterialTheme.typography.titleLarge.copy(
                                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                            ),
                            color = LocalSpoteColors.current.accent,
                        )
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = style.label,
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onBackground,
                            )
                            Text(
                                text = style.subtitle,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (style == current) {
                            Icon(
                                imageVector = Icons.Rounded.Check,
                                contentDescription = null,
                                tint = LocalSpoteColors.current.accent,
                            )
                        }
                    }
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
