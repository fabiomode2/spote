package dev.spote.app.lyrics

import android.content.Context
import dev.spote.app.logging.ActionLog
import java.io.File
import java.io.IOException
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlin.math.roundToLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import org.json.JSONArray
import org.json.JSONObject
import org.jsoup.Jsoup
import org.jsoup.parser.Parser

/**
 * Fetches lyrics from [LRCLIB](https://lrclib.net) and caches them on disk.
 *
 * Lookup order:
 *  1. In-memory cache.
 *  2. JSON file in `filesDir/lyrics/<safe-key>.json` (30 day TTL).
 *  3. `GET /api/get` with the exact track metadata.
 *  4. `GET /api/search` plus [LrclibMatcher] when the exact lookup misses.
 *
 * All failures (network, HTTP, malformed JSON) are swallowed and reported as `null`;
 * cancellation is always rethrown.
 */
class LyricsClient(context: Context) {

    private val appContext: Context = context.applicationContext ?: context
    private val cacheDir = File(appContext.filesDir, CACHE_DIR)
    private val memoryCache = ConcurrentHashMap<String, CacheEntry>()
    private val diskLock = Any()

    private val httpClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .callTimeout(25, TimeUnit.SECONDS)
            .build()
    }

    /**
     * Returns lyrics for the requested track, using the cache when possible.
     *
     * @param durationMs track duration in milliseconds (sent to LRCLIB in seconds).
     * @param cacheKey stable per-track key; it is sanitized before being used as a file name.
     */
    suspend fun get(
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
        cacheKey: String,
    ): LyricsDoc? = withContext(Dispatchers.IO) {
        val key = sanitizeFileKey(cacheKey.ifBlank { "$artist-$title" })
        cached(key)?.let { return@withContext it }

        val doc = try {
            fetch(title = title, artist = artist, album = album, durationMs = durationMs)
                ?: fetchGenius(title = title, artist = artist)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            ActionLog.log("lyrics.error", "title" to title, "artist" to artist, "error" to e.message)
            null
        } ?: run {
            ActionLog.log("lyrics.miss", "title" to title, "artist" to artist)
            return@withContext null
        }
        ActionLog.log(
            "lyrics.hit",
            "title" to title,
            "artist" to artist,
            "source" to doc.source,
            "lines" to doc.lines.size,
            "synced" to doc.synced,
        )

        val entry = CacheEntry(doc = doc, savedAt = System.currentTimeMillis())
        memoryCache[key] = entry
        writeDisk(key, entry)
        doc
    }

    private fun cached(key: String): LyricsDoc? {
        memoryCache[key]?.let { entry ->
            if (isFresh(entry.savedAt)) return entry.doc
            memoryCache.remove(key)
        }

        val file = File(cacheDir, "$key.json")
        val entry = readDisk(file)
        if (entry == null) {
            if (file.exists()) file.delete()
            return null
        }
        if (!isFresh(entry.savedAt)) {
            file.delete()
            return null
        }

        memoryCache[key] = entry
        return entry.doc
    }

    private fun isFresh(savedAt: Long): Boolean {
        val age = System.currentTimeMillis() - savedAt
        return age in -FUTURE_SKEW_MS..CACHE_TTL_MS
    }

    private suspend fun fetch(title: String, artist: String, album: String?, durationMs: Long): LyricsDoc? {
        if (title.isBlank()) return null

        exactLookup(title, artist, album, durationMs)?.toLyricsDoc()?.let { return it }

        val best = LrclibMatcher.selectBest(
            candidates = search(title, artist),
            title = title,
            artist = artist,
            album = album,
            durationMs = durationMs,
        ) ?: return null
        return best.toLyricsDoc()
    }

    private suspend fun exactLookup(
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
    ): LrclibTrack? {
        val builder = API_BASE.newBuilder()
            .addPathSegment("api")
            .addPathSegment("get")
            .addQueryParameter("artist_name", artist)
            .addQueryParameter("track_name", title)
        if (!album.isNullOrBlank()) builder.addQueryParameter("album_name", album)
        if (durationMs > 0L) {
            builder.addQueryParameter("duration", (durationMs / 1000.0).roundToLong().toString())
        }

        val body = executeJson(builder.build()) ?: return null
        return runCatching { parseTrack(JSONObject(body)) }.getOrNull()
    }

    private suspend fun search(title: String, artist: String): List<LrclibTrack> {
        val builder = API_BASE.newBuilder()
            .addPathSegment("api")
            .addPathSegment("search")
            .addQueryParameter("track_name", title)
        if (artist.isNotBlank()) builder.addQueryParameter("artist_name", artist)

        val body = executeJson(builder.build()) ?: return emptyList()
        return runCatching {
            val array = JSONArray(body)
            buildList {
                for (index in 0 until array.length()) {
                    array.optJSONObject(index)?.let { obj ->
                        parseTrack(obj)?.let(::add)
                    }
                }
            }
        }.getOrDefault(emptyList())
    }

    /**
     * Fallback de texto cuando LRCLIB no conoce la pista. Genius no da tiempos,
     * así que se devuelve como letra plana (la UI no finge sincronización).
     */
    private suspend fun fetchGenius(title: String, artist: String): LyricsDoc? {
        val query = "$title $artist".trim()
        if (query.isEmpty()) return null
        val url = GENIUS_SEARCH.toHttpUrl().newBuilder()
            .addQueryParameter("per_page", "5")
            .addQueryParameter("q", query)
            .build()
        val body = executeGenius(url) ?: return null
        val root = runCatching { JSONObject(body) }.getOrNull() ?: return null
        val sections = root.optJSONObject("response")?.optJSONArray("sections") ?: return null
        val wantedTitle = normalizeText(title)
        val wantedArtist = normalizeText(artist)
        var candidatePath: String? = null

        for (sectionIndex in 0 until sections.length()) {
            val hits = sections.optJSONObject(sectionIndex)?.optJSONArray("hits") ?: continue
            for (hitIndex in 0 until hits.length()) {
                val result = hits.optJSONObject(hitIndex)
                    ?.optJSONObject("result") ?: continue
                if (result.optString("_type") != "song") continue
                if (result.optString("lyrics_state") != "complete") continue
                val resultTitle = normalizeText(result.optString("title"))
                val resultArtist = normalizeText(result.optString("artist_names"))
                val titleOk = resultTitle == wantedTitle ||
                    resultTitle.contains(wantedTitle) ||
                    wantedTitle.contains(resultTitle)
                val artistOk = wantedArtist.isBlank() ||
                    resultArtist.contains(wantedArtist) ||
                    wantedArtist.contains(resultArtist)
                if (titleOk && artistOk) {
                    candidatePath = result.optString("path").takeIf { it.isNotBlank() }
                    break
                }
            }
            if (candidatePath != null) break
        }
        val path = candidatePath ?: return null
        val pageUrl = if (path.startsWith("http")) path else "https://genius.com$path"
        val page = executeGenius(pageUrl.toHttpUrl(), acceptHtml = true) ?: return null
        val text = extractGeniusLyrics(page)
        if (text.isBlank()) return null
        return LrcParser.parsePlain(text, source = "genius")
    }

    private fun extractGeniusLyrics(page: String): String {
        val document = runCatching { Jsoup.parse(page) }.getOrNull() ?: return ""
        val lines = mutableListOf<String>()
        document.select("div[data-lyrics-container=true]").forEach { container ->
            val html = container.html()
                .replace(Regex("(?i)<br\\s*/?>"), "\n")
                .replace(Regex("(?i)</p>"), "\n")
            val text = Parser.unescapeEntities(html.replace(Regex("<[^>]+>"), ""), false)
            text.lineSequence()
                .map { it.trim() }
                .filter { it.isNotEmpty() }
                .filterNot { it.startsWith("[") && it.endsWith("]") }
                .filterNot { it.contains("Lyrics[Letra de") }
                .filterNot { it.contains("Letra de \"") && it.endsWith("Lyrics") }
                .forEach(lines::add)
        }
        return lines.joinToString("\n")
    }

    private fun normalizeText(value: String): String = value
        .lowercase(Locale.ROOT)
        .replace(Regex("[^a-z0-9áéíóúüñ]+"), " ")
        .trim()

    private suspend fun executeGenius(
        url: HttpUrl,
        acceptHtml: Boolean = false,
    ): String? = suspendCancellableCoroutine { continuation ->
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", GENIUS_USER_AGENT)
            .header(
                "Accept",
                if (acceptHtml) {
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                } else {
                    "application/json"
                },
            )
            .get()
            .build()
        val call = httpClient.newCall(request)
        continuation.invokeOnCancellation { call.cancel() }
        call.enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (continuation.isActive) continuation.resumeWith(Result.failure(e))
            }

            override fun onResponse(call: Call, response: Response) {
                val body = try {
                    response.use { if (it.isSuccessful) it.body?.string() else null }
                } catch (e: Exception) {
                    if (continuation.isActive) continuation.resumeWith(Result.failure(e))
                    return
                }
                if (continuation.isActive) continuation.resumeWith(Result.success(body))
            }
        })
    }

    private suspend fun executeJson(url: HttpUrl): String? = suspendCancellableCoroutine { continuation ->
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .get()
            .build()

        val call = httpClient.newCall(request)
        continuation.invokeOnCancellation { call.cancel() }

        call.enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (continuation.isActive) continuation.resumeWith(Result.failure(e))
            }

            override fun onResponse(call: Call, response: Response) {
                val body = try {
                    response.use { if (it.isSuccessful) it.body?.string() else null }
                } catch (e: Exception) {
                    if (continuation.isActive) continuation.resumeWith(Result.failure(e))
                    return
                }
                if (continuation.isActive) continuation.resumeWith(Result.success(body))
            }
        })
    }

    private fun parseTrack(obj: JSONObject): LrclibTrack? {
        val trackName = obj.stringOrNull("trackName") ?: return null
        val artistName = obj.stringOrNull("artistName") ?: return null

        return LrclibTrack(
            id = if (obj.has("id") && !obj.isNull("id")) obj.optLong("id") else null,
            trackName = trackName,
            artistName = artistName,
            albumName = obj.stringOrNull("albumName"),
            durationSec = if (obj.has("duration") && !obj.isNull("duration")) {
                obj.optDouble("duration").takeIf { it.isFinite() && it > 0.0 }
            } else {
                null
            },
            instrumental = obj.optBoolean("instrumental", false),
            syncedLyrics = obj.stringOrNull("syncedLyrics"),
            plainLyrics = obj.stringOrNull("plainLyrics"),
        )
    }

    private fun LrclibTrack.toLyricsDoc(): LyricsDoc? {
        if (instrumental) return null

        val synced = syncedLyrics?.trim().orEmpty()
        if (synced.isNotEmpty()) {
            val doc = LrcParser.parse(synced, source = SOURCE)
            if (doc.lines.isNotEmpty()) return doc
        }

        val plain = plainLyrics?.trim().orEmpty()
        if (plain.isEmpty()) return null
        return LrcParser.parsePlain(plain, source = SOURCE)
    }

    private fun readDisk(file: File): CacheEntry? {
        return try {
            if (!file.isFile) return null
            val root = JSONObject(file.readText(Charsets.UTF_8))
            if (root.optInt("version", 0) != CACHE_VERSION) return null

            val savedAt = root.optLong("savedAt", 0L)
            if (savedAt <= 0L) return null

            val plain = root.stringOrNull("plain")
            val linesArray = root.optJSONArray("lines") ?: JSONArray()
            val lines = buildList {
                for (index in 0 until linesArray.length()) {
                    val lineObject = linesArray.optJSONObject(index) ?: continue
                    val lineText = lineObject.stringOrNull("text") ?: continue
                    val wordsArray = lineObject.optJSONArray("words") ?: JSONArray()
                    val words = buildList {
                        for (wordIndex in 0 until wordsArray.length()) {
                            val wordObject = wordsArray.optJSONObject(wordIndex) ?: continue
                            val wordText = wordObject.stringOrNull("text") ?: continue
                            add(
                                LyricWord(
                                    startMs = wordObject.optLong("startMs", 0L),
                                    endMs = if (wordObject.has("endMs") && !wordObject.isNull("endMs")) {
                                        wordObject.optLong("endMs")
                                    } else {
                                        null
                                    },
                                    text = wordText,
                                ),
                            )
                        }
                    }
                    add(
                        LyricLine(
                            startMs = lineObject.optLong("startMs", 0L),
                            text = lineText,
                            words = words,
                        ),
                    )
                }
            }

            if (lines.isEmpty() && plain.isNullOrBlank()) return null

            CacheEntry(
                doc = LyricsDoc(
                    lines = lines,
                    synced = root.optBoolean("synced", false),
                    source = root.stringOrNull("source") ?: SOURCE,
                    plain = plain,
                ),
                savedAt = savedAt,
            )
        } catch (_: Exception) {
            null
        }
    }

    private fun writeDisk(key: String, entry: CacheEntry) {
        try {
            synchronized(diskLock) {
                cacheDir.mkdirs()
                val file = File(cacheDir, "$key.json")
                val tmp = File(cacheDir, "$key.json.tmp")
                tmp.writeText(entry.toJson().toString(), Charsets.UTF_8)
                if (!tmp.renameTo(file)) {
                    file.delete()
                    if (!tmp.renameTo(file)) {
                        tmp.delete()
                    }
                }
            }
        } catch (_: Exception) {
            // Cache writes are best effort: a failed write must never break playback.
        }
    }

    private fun CacheEntry.toJson(): JSONObject {
        val root = JSONObject()
        root.put("version", CACHE_VERSION)
        root.put("savedAt", savedAt)
        root.put("synced", doc.synced)
        root.put("source", doc.source)
        doc.plain?.let { root.put("plain", it) }

        val lines = JSONArray()
        doc.lines.forEach { line ->
            val lineObject = JSONObject()
            lineObject.put("startMs", line.startMs)
            lineObject.put("text", line.text)
            if (line.words.isNotEmpty()) {
                val words = JSONArray()
                line.words.forEach { word ->
                    val wordObject = JSONObject()
                    wordObject.put("startMs", word.startMs)
                    word.endMs?.let { wordObject.put("endMs", it) }
                    wordObject.put("text", word.text)
                    words.put(wordObject)
                }
                lineObject.put("words", words)
            }
            lines.put(lineObject)
        }
        root.put("lines", lines)
        return root
    }

    private fun JSONObject.stringOrNull(name: String): String? =
        if (!has(name) || isNull(name)) null else optString(name).takeIf { it.isNotBlank() }

    private data class CacheEntry(
        val doc: LyricsDoc,
        val savedAt: Long,
    )

    companion object {
        private const val SOURCE = "lrclib"
        private const val USER_AGENT = "spote-android"
        private const val GENIUS_SEARCH = "https://genius.com/api/search/multi"
        private const val GENIUS_USER_AGENT =
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) " +
                "Chrome/124.0 Mobile Safari/537.36"
        private const val CACHE_DIR = "lyrics"
        private const val CACHE_VERSION = 1
        private const val CACHE_TTL_MS = 30L * 24L * 60L * 60L * 1000L
        private const val FUTURE_SKEW_MS = 5L * 60L * 1000L
        private const val MAX_FILE_KEY_LENGTH = 100

        private val API_BASE = "https://lrclib.net".toHttpUrl()

        /**
         * Converts an arbitrary cache key into a safe file name. When characters are replaced
         * or the key is truncated, a short hash is appended to keep different keys distinct.
         */
        fun sanitizeFileKey(key: String): String {
            val value = key.trim()
            if (value.isEmpty()) return "unknown"

            val builder = StringBuilder(value.length)
            for (ch in value) {
                val safe = (ch.isLetterOrDigit() && ch.code < 128) ||
                    ch == '-' || ch == '_' || ch == '.'
                builder.append(if (safe) ch else '_')
            }

            var base = builder.toString().trim('.', '_')
            if (base.isEmpty()) base = "lyrics"
            if (base.length > MAX_FILE_KEY_LENGTH) base = base.take(80)

            return if (base == value) base else "$base-${Integer.toHexString(value.hashCode())}"
        }
    }
}
