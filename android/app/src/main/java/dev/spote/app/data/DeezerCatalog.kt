package dev.spote.app.data

import dev.spote.app.model.Album
import dev.spote.app.model.Track
import dev.spote.app.rust.coverFor
import java.io.IOException
import java.util.Locale
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

data class CatalogResult(
    val tracks: List<Track> = emptyList(),
    val albums: List<Album> = emptyList(),
    val artists: List<String> = emptyList(),
)

/**
 * Catálogo de canciones (Deezer, API pública sin clave), como la búsqueda de
 * la TUI. YouTube/NewPipe queda solo para resolver el audio de la pista
 * elegida: así los resultados traen artista, álbum, duración y portada reales.
 */
object DeezerCatalog {

    private const val SEARCH = "https://api.deezer.com/search"
    private const val USER_AGENT = "spote-android/0.4 (catalog)"

    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .callTimeout(20, TimeUnit.SECONDS)
            .build()
    }

    suspend fun search(query: String, limit: Int = 25): CatalogResult = withContext(Dispatchers.IO) {
        val trimmed = query.trim()
        if (trimmed.isEmpty()) return@withContext CatalogResult()
        val url = SEARCH.toHttpUrl().newBuilder()
            .addQueryParameter("q", trimmed)
            .addQueryParameter("limit", limit.coerceIn(1, 50).toString())
            .build()
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", USER_AGENT)
            .get()
            .build()
        val body = client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw IOException("Deezer HTTP ${response.code}")
            response.body?.string().orEmpty()
        }
        val root = JSONObject(body)
        if (root.has("error")) {
            throw IOException(root.optJSONObject("error")?.optString("message") ?: "Deezer error")
        }
        val data = root.optJSONArray("data") ?: return@withContext CatalogResult()

        val tracks = buildList {
            for (index in 0 until data.length()) {
                val item = data.optJSONObject(index) ?: continue
                val id = item.optLong("id", 0L)
                val title = item.optString("title").trim()
                if (id <= 0L || title.isEmpty()) continue
                val artist = item.optJSONObject("artist")?.optString("name").orEmpty().trim()
                if (artist.isEmpty()) continue
                val albumObj = item.optJSONObject("album")
                val album = albumObj?.optString("title")?.trim()?.takeIf { it.isNotEmpty() }
                val cover = albumObj?.optString("cover_xl")
                    ?.takeIf { it.isNotBlank() }
                    ?: albumObj?.optString("cover_big")?.takeIf { it.isNotBlank() }
                val durationMs = item.optLong("duration", 0L) * 1000L
                add(
                    Track(
                        key = "deezer:$id",
                        title = title,
                        artist = artist,
                        album = album,
                        durationMs = durationMs,
                        cover = coverFor(title, null, cover),
                        sourceLabel = "Deezer",
                        remoteQuery = "$artist $title",
                    ),
                )
            }
        }.distinctBy { it.key }

        val albums = tracks
            .filter { !it.album.isNullOrBlank() }
            .groupBy { it.album.orEmpty().lowercase(Locale.ROOT) to it.artist.lowercase(Locale.ROOT) }
            .map { (pair, rows) ->
                val first = rows.first()
                Album(
                    id = "dcat:${pair.first}:${pair.second}",
                    title = first.album.orEmpty(),
                    artist = first.artist,
                    year = 0,
                    cover = first.cover,
                    tracks = rows,
                )
            }
        val artists = tracks.map { it.artist }.distinct()
        CatalogResult(tracks = tracks, albums = albums, artists = artists)
    }
}
