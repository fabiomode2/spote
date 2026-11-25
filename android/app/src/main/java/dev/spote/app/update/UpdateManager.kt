package dev.spote.app.update

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/** Resultado de una comprobación, ya sin detalles de red. */
sealed interface UpdateCheckResult {
    /** Ya se comprobó hace menos de 24 h y no se pidió `force`. */
    data object Throttled : UpdateCheckResult

    /** El repositorio no tiene releases (o no hay ninguna estable). */
    data object NoRelease : UpdateCheckResult

    data class UpToDate(val currentVersion: String) : UpdateCheckResult

    data class Available(val release: ReleaseInfo) : UpdateCheckResult
}

/**
 * Comprueba releases, descarga el APK y lanza el instalador del sistema.
 *
 * La instalación nunca es silenciosa: Android muestra su propia confirmación.
 * Si la app no tiene permiso para instalar paquetes desconocidos, se abre la
 * pantalla de ajustes correspondiente y el usuario debe volver a pulsar
 * "Instalar".
 */
class UpdateManager(
    context: Context,
    private val repo: String,
    private val currentVersion: String,
) {
    private val appContext = context.applicationContext
    private val prefs = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val client = GitHubReleaseClient()

    suspend fun check(force: Boolean = false): UpdateCheckResult {
        val now = System.currentTimeMillis()
        if (!force) {
            val lastCheck = prefs.getLong(KEY_LAST_CHECK, 0L)
            if (lastCheck > 0L && now - lastCheck < CHECK_INTERVAL_MS) {
                return UpdateCheckResult.Throttled
            }
        }

        val release = client.latest(repo)
        prefs.edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis()).apply()

        if (release == null) return UpdateCheckResult.NoRelease

        return if (isNewerVersion(release.version, currentVersion)) {
            UpdateCheckResult.Available(release)
        } else {
            UpdateCheckResult.UpToDate(currentVersion)
        }
    }

    /**
     * Descarga el APK en `filesDir/updates/`, informa del progreso (0..1) y
     * verifica el SHA-256 cuando GitHub publica el digest. Ante cualquier
     * fallo elimina el archivo parcial.
     */
    suspend fun download(
        release: ReleaseInfo,
        onProgress: (Float) -> Unit = {},
    ): File = withContext(Dispatchers.IO) {
        val updatesDir = File(appContext.filesDir, UPDATES_DIR)
        if (!updatesDir.exists() && !updatesDir.mkdirs()) {
            throw IOException("No se pudo preparar la carpeta de actualizaciones")
        }
        updatesDir.listFiles()?.forEach { stale ->
            if (stale.isFile && (stale.name.endsWith(".apk", true) || stale.name.endsWith(".part", true))) {
                stale.delete()
            }
        }

        val target = File(updatesDir, safeApkName(release))
        val partial = File(updatesDir, "${target.name}.part")
        partial.delete()

        try {
            val connection = openFollowingRedirects(release.apkUrl)
            try {
                val status = connection.responseCode
                if (status !in 200..299) {
                    throw IOException("La descarga del APK falló (HTTP $status)")
                }

                val totalBytes = connection.contentLengthLong.takeIf { it > 0L }
                onProgress(0f)
                var downloaded = 0L

                connection.inputStream.use { input ->
                    FileOutputStream(partial).use { output ->
                        val buffer = ByteArray(BUFFER_SIZE)
                        while (true) {
                            if (!isActive) throw CancellationException("Descarga cancelada")
                            val read = input.read(buffer)
                            if (read < 0) break
                            output.write(buffer, 0, read)
                            downloaded += read
                            if (totalBytes != null) {
                                onProgress(
                                    (downloaded.toDouble() / totalBytes.toDouble())
                                        .toFloat()
                                        .coerceIn(0f, 1f),
                                )
                            }
                        }
                        output.flush()
                    }
                }

                if (totalBytes != null && downloaded != totalBytes) {
                    throw IOException("La descarga del APK quedó incompleta")
                }

                release.sha256?.let { expected ->
                    val actual = sha256Hex(partial)
                    if (!actual.equals(expected, ignoreCase = true)) {
                        throw IOException("La firma SHA-256 del APK no coincide")
                    }
                }

                if (target.exists() && !target.delete()) {
                    throw IOException("No se pudo reemplazar el APK anterior")
                }
                if (!partial.renameTo(target)) {
                    partial.copyTo(target, overwrite = true)
                    partial.delete()
                }

                onProgress(1f)
                target
            } finally {
                connection.disconnect()
            }
        } catch (t: Throwable) {
            partial.delete()
            target.delete()
            throw t
        }
    }

    /**
     * Abre el instalador de Android para [apk]. Si falta el permiso de
     * "instalar apps desconocidas", abre los ajustes de la app y no instala.
     */
    fun install(apk: File) {
        if (!apk.isFile) throw IOException("El APK descargado ya no existe")

        if (!canInstallPackages()) {
            val settings = Intent(
                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:${appContext.packageName}"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            appContext.startActivity(settings)
            return
        }

        val uri = FileProvider.getUriForFile(
            appContext,
            "${appContext.packageName}.fileprovider",
            apk,
        )
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        appContext.startActivity(intent)
    }

    private fun canInstallPackages(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.O ||
            appContext.packageManager.canRequestPackageInstalls()

    private fun safeApkName(release: ReleaseInfo): String {
        val raw = release.apkName
            .substringAfterLast('/')
            .substringAfterLast('\\')
            .ifBlank { "spote-${release.version}-arm64-v8a.apk" }
        val sanitized = raw
            .replace(Regex("[^A-Za-z0-9._-]"), "_")
            .trim('.', '_')
            .ifBlank { "spote-${release.version}-arm64-v8a.apk" }
        return if (sanitized.endsWith(".apk", ignoreCase = true)) sanitized else "$sanitized.apk"
    }

    private fun openFollowingRedirects(url: String): HttpURLConnection {
        var connection = open(url)
        var hops = 0
        while (hops < MAX_REDIRECTS) {
            val status = connection.responseCode
            if (status !in 300..399) return connection
            val location = connection.getHeaderField("Location") ?: return connection
            val next = URL(connection.url, location).toString()
            connection.disconnect()
            connection = open(next)
            hops++
        }
        return connection
    }

    private fun open(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            instanceFollowRedirects = false
            useCaches = false
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            setRequestProperty("User-Agent", "spote-android")
            setRequestProperty("Accept", "application/octet-stream")
            setRequestProperty("Accept-Encoding", "identity")
        }

    private fun sha256Hex(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(BUFFER_SIZE)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        val bytes = digest.digest()
        val digits = "0123456789abcdef"
        val hex = CharArray(bytes.size * 2)
        bytes.forEachIndexed { index, byte ->
            val value = byte.toInt() and 0xFF
            hex[index * 2] = digits[value ushr 4]
            hex[index * 2 + 1] = digits[value and 0x0F]
        }
        return String(hex)
    }

    private companion object {
        const val PREFS_NAME = "spote_updates"
        const val KEY_LAST_CHECK = "last_check_at"
        const val CHECK_INTERVAL_MS = 24L * 60L * 60L * 1_000L
        const val UPDATES_DIR = "updates"
        const val CONNECT_TIMEOUT_MS = 20_000
        const val READ_TIMEOUT_MS = 60_000
        const val MAX_REDIRECTS = 5
        const val BUFFER_SIZE = 64 * 1024
    }
}
