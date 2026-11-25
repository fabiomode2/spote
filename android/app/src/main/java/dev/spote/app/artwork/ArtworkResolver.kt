package dev.spote.app.artwork

import android.content.Context
import java.io.File
import java.io.IOException
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.text.Normalizer
import java.util.Locale
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

data class Artwork(
    val localPath: String?,
    val remoteUrl: String?,
    val album: String? = null,
)

/**
 * Portadas para pistas que no traen arte embebido.
 *
 * Usa la iTunes Search API (sin clave) y guarda el JPEG en el almacenamiento
 * privado para que también funcione sin conexión. Es un backfill best-effort:
 * un fallo de red o una búsqueda sin resultados no rompe la biblioteca.
 */
object ArtworkResolver {

    private const val SEARCH = "https://itunes.apple.com/search"
    private const val USER_AGENT = "spote-android/0.3 (artwork backfill)"
    private val ARTWORK_DIR = "artwork"

    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(12, TimeUnit.SECONDS)
            .callTimeout(18, TimeUnit.SECONDS)
            .build()
    }

    suspend fun resolve(
        context: Context,
        key: String,
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
    ): Artwork? = withContext(Dispatchers.IO) {
        val query = listOf(artist, title, album.orEmpty())
            .filter { it.isNotBlank() }
            .joinToString(" ")
        if (query.isBlank()) return@withContext null

        val url = SEARCH.toHttpUrl().newBuilder()
            .addQueryParameter("term", query)
            .addQueryParameter("entity", "song")
            .addQueryParameter("limit", "8")
            .addQueryParameter("country", "ES")
            .build()
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .get()
            .build()

        val body = client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw IOException("iTunes HTTP ${response.code}")
            response.body?.string().orEmpty()
        }
        val best = runCatching { pickBest(JSONObject(body), title, artist, durationMs) }
            .getOrNull() ?: return@withContext null
        val remote = best.optString("artworkUrl100")
            .takeIf { it.isNotBlank() }
            ?.replace("100x100bb", "600x600bb")
            ?: return@withContext null
        val local = download(context, remote, key)
        Artwork(
            localPath = local,
            remoteUrl = remote,
            album = best.optString("collectionName").takeIf { it.isNotBlank() },
        )
    }

    /** Descarga la miniatura de un stream remoto para usarla al importarlo. */
    suspend fun downloadThumbnail(
        context: Context,
        key: String,
        url: String?,
    ): String? = withContext(Dispatchers.IO) {
        val target = url?.trim()?.takeIf { it.startsWith("http") } ?: return@withContext null
        download(context, target.replace("100x100bb", "600x600bb"), key)
    }

    private fun pickBest(
        root: JSONObject,
        title: String,
        artist: String,
        durationMs: Long,
    ): JSONObject? {
        val results = root.optJSONArray("results") ?: return null
        val wantedTitle = normalize(title)
        val wantedArtist = normalize(artist)
        var fallback: JSONObject? = null

        for (index in 0 until results.length()) {
            val item = results.optJSONObject(index) ?: continue
            if (item.optString("artworkUrl100").isBlank()) continue
            val resultTitle = normalize(item.optString("trackName"))
            val resultArtist = normalize(item.optString("artistName"))
            if (resultTitle.isBlank()) continue
            val titleMatch = resultTitle == wantedTitle ||
                resultTitle.contains(wantedTitle) ||
                wantedTitle.contains(resultTitle)
            val artistMatch = wantedArtist.isBlank() ||
                resultArtist.contains(wantedArtist) ||
                wantedArtist.contains(resultArtist)
            if (!titleMatch || !artistMatch) continue

            if (
                resultTitle == wantedTitle &&
                (wantedArtist.isBlank() || resultArtist.contains(wantedArtist))
            ) {
                return item
            }
            val trackMs = item.optLong("trackTimeMillis", 0L)
            if (durationMs > 0L && trackMs > 0L && kotlin.math.abs(trackMs - durationMs) > 15_000L) {
                continue
            }
            if (fallback == null) fallback = item
        }
        return fallback
    }

    private fun download(context: Context, url: String, key: String): String? = runCatching {
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", USER_AGENT)
            .get()
            .build()
        val bytes = client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw IOException("art HTTP ${response.code}")
            response.body?.bytes() ?: throw IOException("art without body")
        }
        if (bytes.isEmpty()) return@runCatching null
        val dir = File(context.filesDir, ARTWORK_DIR).apply { mkdirs() }
        val file = File(dir, "${safeKey(key)}.jpg")
        file.writeBytes(bytes)
        file.absolutePath
    }.getOrNull()

    private fun safeKey(key: String): String {
        val digest = MessageDigest.getInstance("SHA-1")
            .digest(key.toByteArray(StandardCharsets.UTF_8))
        return digest.joinToString("") { "%02x".format(it) }.take(24)
    }

    private fun normalize(value: String): String = Normalizer.normalize(value, Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(Locale.ROOT)
        .replace(Regex("[^a-z0-9]+"), " ")
        .trim()
}
