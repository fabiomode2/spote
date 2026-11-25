package dev.spote.app.update

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Cliente mínimo para la API pública de GitHub. No usa librerías HTTP externas:
 * sólo [HttpURLConnection] y `org.json`, ambos presentes en Android.
 */
class GitHubReleaseClient {

    /**
     * Devuelve la última release estable del repositorio, o `null` si no hay
     * ninguna (404) o si la release está marcada como borrador/pre-release.
     *
     * Lanza [IOException] ante errores de red o respuestas HTTP inesperadas,
     * para que la capa de UI pueda mostrar un mensaje.
     */
    suspend fun latest(repo: String): ReleaseInfo? = withContext(Dispatchers.IO) {
        val slug = repo.trim().trim('/')
        require(slug.isNotEmpty()) { "El repositorio de GitHub no está configurado" }

        val connection = openFollowingRedirects(
            url = "https://api.github.com/repos/$slug/releases/latest",
            accept = ACCEPT_JSON,
        )
        try {
            when (val status = connection.responseCode) {
                HttpURLConnection.HTTP_NOT_FOUND -> null

                in 200..299 -> {
                    val body = connection.inputStream
                        .bufferedReader(Charsets.UTF_8)
                        .use { it.readText() }
                    parseRelease(body)
                }

                else -> {
                    val detail = connection.errorStream
                        ?.bufferedReader(Charsets.UTF_8)
                        ?.use { it.readText() }
                        ?.trim()
                        ?.takeIf { it.isNotEmpty() }
                        ?.take(200)
                    throw IOException(
                        buildString {
                            append("GitHub respondió HTTP $status")
                            if (detail != null) append(": $detail")
                        },
                    )
                }
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun open(url: String, accept: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            instanceFollowRedirects = false
            useCaches = false
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            setRequestProperty("User-Agent", USER_AGENT)
            setRequestProperty("Accept", accept)
            setRequestProperty("Accept-Encoding", "identity")
        }

    /** Sigue redirecciones a mano para conservar las cabeceras en cada salto. */
    private fun openFollowingRedirects(url: String, accept: String): HttpURLConnection {
        var connection = open(url, accept)
        var hops = 0
        while (hops < MAX_REDIRECTS) {
            val status = connection.responseCode
            if (status !in 300..399) return connection
            val location = connection.getHeaderField("Location") ?: return connection
            val next = URL(connection.url, location).toString()
            connection.disconnect()
            connection = open(next, accept)
            hops++
        }
        return connection
    }

    private companion object {
        const val USER_AGENT = "spote-android"
        const val ACCEPT_JSON = "application/vnd.github+json"
        const val CONNECT_TIMEOUT_MS = 15_000
        const val READ_TIMEOUT_MS = 30_000
        const val MAX_REDIRECTS = 5
    }
}

/** Asset de una release, ya normalizado a los datos que usa el updater. */
internal data class ApkAsset(
    val name: String,
    val url: String,
    val sha256: String?,
)

/**
 * Parsea el JSON de `releases/latest`. Es `internal` y sin dependencias de
 * Android para poder ejercitarlo desde tests JVM.
 */
internal fun parseRelease(json: String): ReleaseInfo? {
    val root = JSONObject(json)
    if (root.optBoolean("draft", false) || root.optBoolean("prerelease", false)) return null

    val tag = root.text("tag_name")
    if (tag.isEmpty()) return null

    val assetsJson = root.optJSONArray("assets") ?: JSONArray()
    val assets = ArrayList<ApkAsset>(assetsJson.length())
    for (index in 0 until assetsJson.length()) {
        val asset = assetsJson.optJSONObject(index) ?: continue
        val name = asset.text("name")
        val url = asset.text("browser_download_url")
        if (name.isEmpty() || url.isEmpty()) continue
        assets += ApkAsset(
            name = name,
            url = url,
            sha256 = parseSha256Digest(asset.text("digest")),
        )
    }

    val apk = selectApkAsset(assets) ?: return null

    return ReleaseInfo(
        tag = tag,
        version = normalizeVersion(tag),
        notes = root.text("body"),
        apkUrl = apk.url,
        apkName = apk.name,
        publishedAt = root.text("published_at"),
        sha256 = apk.sha256,
    )
}

/**
 * Elige el APK adecuado: primero uno cuyo nombre mencione `arm64` o
 * `universal`; si no hay, el primer `.apk` de la lista.
 */
internal fun selectApkAsset(assets: List<ApkAsset>): ApkAsset? {
    val apks = assets.filter { it.name.endsWith(".apk", ignoreCase = true) }
    if (apks.isEmpty()) return null
    return apks.firstOrNull { asset ->
        val name = asset.name.lowercase()
        "arm64" in name || "universal" in name
    } ?: apks.first()
}

/** Acepta `sha256:<hex>` y devuelve el hex en minúsculas; otro digest → `null`. */
internal fun parseSha256Digest(digest: String?): String? {
    val value = digest?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    val lower = value.lowercase()
    if (!lower.startsWith("sha256:")) return null
    val hex = lower.substring("sha256:".length).trim()
    val valid = hex.length == 64 && hex.all { it in '0'..'9' || it in 'a'..'f' }
    return if (valid) hex else null
}

/** `v0.2.0` → `0.2.0`; los tags sin prefijo se dejan tal cual. */
internal fun normalizeVersion(raw: String): String {
    val trimmed = raw.trim()
    return if (trimmed.length > 1 && (trimmed[0] == 'v' || trimmed[0] == 'V') && trimmed[1].isDigit()) {
        trimmed.substring(1)
    } else {
        trimmed
    }
}

internal data class SemVer(
    val major: Int,
    val minor: Int,
    val patch: Int,
    val preRelease: List<String>?,
)

/**
 * Parsea versiones `X`, `X.Y` o `X.Y.Z`, con prefijo `v`, sufijo de
 * pre-release (`-debug`, `-beta.1`) y metadatos (`+build`). Devuelve `null`
 * si no hay una parte numérica válida.
 */
internal fun parseSemVer(raw: String): SemVer? {
    val normalized = normalizeVersion(raw)
    if (normalized.isEmpty()) return null

    val withoutBuild = normalized.substringBefore('+').trim()
    if (withoutBuild.isEmpty()) return null

    val hasPreRelease = withoutBuild.contains('-')
    val core = withoutBuild.substringBefore('-')
    val preRelease = if (hasPreRelease) {
        withoutBuild.substringAfter('-')
            .split('.')
            .filter { it.isNotEmpty() }
            .takeIf { it.isNotEmpty() }
    } else {
        null
    }

    val parts = core.split('.')
    if (parts.size !in 1..3) return null
    val numbers = parts.map { part ->
        if (part.isEmpty() || part.any { !it.isDigit() }) return null
        part.toIntOrNull() ?: return null
    }

    return SemVer(
        major = numbers.getOrElse(0) { 0 },
        minor = numbers.getOrElse(1) { 0 },
        patch = numbers.getOrElse(2) { 0 },
        preRelease = preRelease,
    )
}

/**
 * Compara dos versiones con precedencia semver. Las versiones no parseables
 * se consideran equivalentes (0) para no ofrecer actualizaciones dudosas.
 */
internal fun compareVersions(left: String, right: String): Int {
    val a = parseSemVer(left) ?: return 0
    val b = parseSemVer(right) ?: return 0
    return compareSemVer(a, b)
}

/** `true` sólo si ambas versiones son válidas y `remote` es estrictamente mayor. */
internal fun isNewerVersion(remote: String, current: String): Boolean {
    val remoteVersion = parseSemVer(remote) ?: return false
    val currentVersion = parseSemVer(current) ?: return false
    return compareSemVer(remoteVersion, currentVersion) > 0
}

private fun compareSemVer(left: SemVer, right: SemVer): Int {
    val major = left.major.compareTo(right.major)
    if (major != 0) return major
    val minor = left.minor.compareTo(right.minor)
    if (minor != 0) return minor
    val patch = left.patch.compareTo(right.patch)
    if (patch != 0) return patch
    return comparePreRelease(left.preRelease, right.preRelease)
}

private fun comparePreRelease(left: List<String>?, right: List<String>?): Int {
    if (left == null && right == null) return 0
    if (left == null) return 1
    if (right == null) return -1

    val shared = minOf(left.size, right.size)
    for (index in 0 until shared) {
        val a = left[index]
        val b = right[index]
        val aNumber = a.toIntOrNull()
        val bNumber = b.toIntOrNull()
        val result = when {
            aNumber != null && bNumber != null -> aNumber.compareTo(bNumber)
            aNumber != null -> -1
            bNumber != null -> 1
            else -> a.compareTo(b, ignoreCase = true)
        }
        if (result != 0) return result
    }
    return left.size.compareTo(right.size)
}

/** `optString` devuelve "null" para valores JSON nulos; esto lo evita. */
private fun JSONObject.text(key: String): String {
    if (!has(key) || isNull(key)) return ""
    return optString(key, "").trim()
}
