package dev.spote.app.lyrics

import android.content.Context
import dev.spote.app.logging.ActionLog
import java.io.File
import java.io.IOException
import java.text.Normalizer
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlin.math.roundToLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
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

    private data class LyricsBundle(
        val genius: LyricsDoc?,
        val lrclib: LyricsDoc?,
        val lrcmux: LyricsDoc?,
        val plus: LyricsDoc?,
    )

    private suspend fun <T> safe(block: suspend () -> T): T? = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
        null
    }

    /**
     * Conserva el texto de Genius y solo acepta los tiempos del proveedor si
     * su letra se parece lo suficiente. Es la regla de la TUI: antes una letra
     * plana correcta que un karaoke de otra canción.
     */
    private fun resolveGenius(genius: LyricsDoc, timing: LyricsDoc): LyricsDoc {
        if (lyricsSimilarity(genius, timing) < GENIUS_TEXT_MATCH) return genius
        return if (genius.lines.size == timing.lines.size) {
            timing.copy(
                lines = timing.lines.mapIndexed { index, line ->
                    line.copy(text = genius.lines[index].text)
                },
                source = "genius",
            )
        } else {
            timing.copy(source = "genius")
        }
    }

    private fun lyricsSimilarity(a: LyricsDoc, b: LyricsDoc): Double {
        val left = lyricWords(a)
        val right = lyricWords(b)
        if (left.isEmpty() || right.isEmpty()) return 0.0
        val intersection = left.intersect(right).size
        return (2.0 * intersection) / (left.size + right.size)
    }

    private fun lyricWords(doc: LyricsDoc): Set<String> =
        normalizeText(doc.lines.joinToString(" ") { it.text })
            .split(' ')
            .filter { it.length >= 3 }
            .toSet()

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
            // Genius es la fuente autoritativa del texto (como en la TUI).
            // LRCLIB/lrcmux/LyricsPlus solo pueden prestar tiempos si su
            // letra coincide con la de Genius.
            val bundle = kotlinx.coroutines.coroutineScope {
                val genius = async {
                    safe { fetchGenius(title = title, artist = artist) }
                }
                val lrclib = async {
                    safe {
                        fetchLrclib(
                            title = title,
                            artist = artist,
                            album = album,
                            durationMs = durationMs,
                        )
                    }
                }
                val lrcmux = async {
                    safe {
                        fetchLrcmux(
                            title = title,
                            artist = artist,
                            album = album,
                            durationMs = durationMs,
                        )
                    }
                }
                val plus = async {
                    safe {
                        fetchLyricsPlus(
                            title = title,
                            artist = artist,
                            album = album,
                            durationMs = durationMs,
                        )
                    }
                }
                LyricsBundle(
                    genius = genius.await(),
                    lrclib = lrclib.await(),
                    lrcmux = lrcmux.await(),
                    plus = plus.await(),
                )
            }
            val timing = listOf(bundle.lrclib, bundle.lrcmux, bundle.plus)
                .firstOrNull { it?.synced == true }
            val plain = bundle.lrclib?.takeIf { !it.synced }
                ?: bundle.lrcmux?.takeIf { !it.synced }
                ?: bundle.plus?.takeIf { !it.synced }
            when {
                bundle.genius != null && timing != null ->
                    resolveGenius(bundle.genius, timing)
                bundle.genius != null -> bundle.genius
                timing != null -> timing
                else -> plain
            }
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

    private suspend fun fetchLrclib(title: String, artist: String, album: String?, durationMs: Long): LyricsDoc? {
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
        val queryTitle = nfc(title)
        val queryArtist = nfc(artist)
        val builder = API_BASE.newBuilder()
            .addPathSegment("api")
            .addPathSegment("get")
            .addQueryParameter("artist_name", queryArtist)
            .addQueryParameter("track_name", queryTitle)
        if (!album.isNullOrBlank()) builder.addQueryParameter("album_name", nfc(album))
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
            .addQueryParameter("track_name", nfc(title))
        if (artist.isNotBlank()) builder.addQueryParameter("artist_name", nfc(artist))

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
     * Proveedor con líneas sincronizadas (Apple/Spotify/QQ vía LyricsPlus).
     * Si no hay tiempos reales, se descarta y se prueba Genius como texto plano.
     */
    private suspend fun fetchLyricsPlus(
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
    ): LyricsDoc? {
        if (title.isBlank() || artist.isBlank()) return null
        val url = LYRICS_PLUS_API.toHttpUrl().newBuilder()
            .addQueryParameter("title", nfc(title))
            .addQueryParameter("artist", nfc(artist))
            .apply {
                if (!album.isNullOrBlank()) addQueryParameter("album", nfc(album))
                if (durationMs > 0L) {
                    addQueryParameter("duration", "%.3f".format(Locale.ROOT, durationMs / 1000.0))
                }
                addQueryParameter("source", "apple,spotify,qq")
            }
            .build()
        val body = executeJson(url) ?: return null
        val root = runCatching { JSONObject(body) }.getOrNull() ?: return null
        val linesJson = root.optJSONArray("lyrics") ?: return null
        val lines = buildList {
            for (index in 0 until linesJson.length()) {
                val item = linesJson.optJSONObject(index) ?: continue
                val text = item.optString("text").trim()
                if (text.isEmpty()) continue
                val time = item.optLong("time", 0L)
                add(LyricLine(startMs = time.coerceAtLeast(0L), text = text))
            }
        }
        if (lines.size < 2) return null
        val timed = lines.any { it.startMs > 0L }
        if (!timed) {
            return LrcParser.parsePlain(
                lines.joinToString("\n") { it.text },
                source = "apple",
            )
        }
        val lastStart = lines.maxOf { it.startMs }
        if (durationMs > 0L && lastStart > durationMs + 20_000L) return null
        return LyricsDoc(
            lines = lines.sortedBy { it.startMs },
            synced = true,
            source = "lyricsplus",
        )
    }

    /**
     * Fallback de texto cuando LRCLIB no conoce la pista. Genius no da tiempos,
     * así que se devuelve como letra plana (la UI no finge sincronización).
     */
    /**
     * lrcmux es el agregador que usa la TUI (Apple/Spotify/QQ). Su endpoint
     * compatible devuelve el mismo formato LRC que LRCLIB; se valida título,
     * artista y duración antes de aceptar nada.
     */
    private suspend fun fetchLrcmux(
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
    ): LyricsDoc? {
        if (title.isBlank() || artist.isBlank()) return null
        val compat = LRC_MUX_COMPAT.toHttpUrl().newBuilder()
            .addQueryParameter("track_name", nfc(title))
            .addQueryParameter("artist_name", nfc(artist))
            .apply {
                if (!album.isNullOrBlank()) addQueryParameter("album_name", nfc(album))
                if (durationMs > 0L) addQueryParameter("duration", (durationMs / 1000L).toString())
            }
            .build()
        val compatBody = safe { executeJson(compat) }
        if (compatBody != null) {
            val root = runCatching { JSONObject(compatBody) }.getOrNull()
            if (root != null) {
                val providerTitle = root.optString("trackName")
                val providerArtist = root.optString("artistName")
                val providerAlbum = root.optString("albumName")
                val providerDuration = root.optDouble("duration", 0.0)
                if (
                    providerMetadataMatches(
                        wantedTitle = title,
                        wantedArtist = artist,
                        wantedAlbum = album,
                        wantedDurationMs = durationMs,
                        providerTitle = providerTitle,
                        providerArtist = providerArtist,
                        providerAlbum = providerAlbum,
                        providerDurationSec = providerDuration,
                    )
                ) {
                    val synced = root.optString("syncedLyrics").trim()
                    if (synced.isNotEmpty()) {
                        val doc = LrcParser.parse(synced, source = "lrcmux")
                        if (doc.synced && doc.lines.size >= 2) return doc
                    }
                    val plain = root.optString("plainLyrics").trim()
                    if (plain.isNotEmpty()) return LrcParser.parsePlain(plain, source = "lrcmux")
                }
            }
        }

        // Endpoint nativo: líneas separadas con `start` opcional en segundos.
        val native = LRC_MUX_NATIVE.toHttpUrl().newBuilder()
            .addQueryParameter("artist", nfc(artist))
            .addQueryParameter("title", nfc(title))
            .apply { if (durationMs > 0L) addQueryParameter("duration", (durationMs / 1000L).toString()) }
            .addQueryParameter("format", "json")
            .build()
        val nativeBody = safe { executeJson(native) } ?: return null
        val root = runCatching { JSONObject(nativeBody) }.getOrNull() ?: return null
        val track = root.optJSONObject("track") ?: return null
        if (
            !providerMetadataMatches(
                wantedTitle = title,
                wantedArtist = artist,
                wantedAlbum = album,
                wantedDurationMs = durationMs,
                providerTitle = track.optString("title"),
                providerArtist = track.optString("artist"),
                providerAlbum = track.optString("album"),
                providerDurationSec = track.optDouble("duration", 0.0),
            )
        ) {
            return null
        }
        val rows = root.optJSONArray("lines") ?: return null
        val timed = mutableListOf<LyricLine>()
        val plain = mutableListOf<String>()
        for (index in 0 until rows.length()) {
            val item = rows.optJSONObject(index) ?: continue
            val text = item.optString("text").trim()
            if (text.isEmpty()) continue
            plain += text
            if (item.has("start") && !item.isNull("start")) {
                val seconds = item.optDouble("start", -1.0)
                if (seconds.isFinite() && seconds >= 0.0) {
                    timed += LyricLine((seconds * 1000.0).toLong(), text)
                }
            }
        }
        if (timed.size >= 2 && timed.size * 100 >= plain.size * 60) {
            return LyricsDoc(
                lines = timed.sortedBy { it.startMs },
                synced = true,
                source = "lrcmux",
            )
        }
        if (plain.isEmpty()) return null
        return LrcParser.parsePlain(plain.joinToString("\n"), source = "lrcmux")
    }

    private fun providerMetadataMatches(
        wantedTitle: String,
        wantedArtist: String,
        wantedAlbum: String?,
        wantedDurationMs: Long,
        providerTitle: String,
        providerArtist: String,
        providerAlbum: String?,
        providerDurationSec: Double,
    ): Boolean {
        val wanted = normalizeText(wantedTitle)
        val got = normalizeText(providerTitle)
        if (wanted.isEmpty() || got.isEmpty()) return false
        if (got != wanted && !got.contains(wanted) && !wanted.contains(got)) return false

        val wantedPerformer = normalizeText(wantedArtist.substringBefore(","))
        val gotPerformer = normalizeText(providerArtist.substringBefore(","))
        if (
            wantedPerformer.isNotEmpty() && gotPerformer.isNotEmpty() &&
            !gotPerformer.contains(wantedPerformer) &&
            !wantedPerformer.contains(gotPerformer)
        ) {
            return false
        }
        if (
            wantedDurationMs > 0L && providerDurationSec.isFinite() && providerDurationSec > 0.0 &&
            kotlin.math.abs(providerDurationSec * 1000.0 - wantedDurationMs) > 8_000.0
        ) {
            return false
        }
        if (!wantedAlbum.isNullOrBlank() && !providerAlbum.isNullOrBlank()) {
            val wantedA = normalizeText(wantedAlbum)
            val gotA = normalizeText(providerAlbum)
            if (wantedA.isNotEmpty() && gotA.isNotEmpty() &&
                !gotA.contains(wantedA) && !wantedA.contains(gotA)
            ) {
                return false
            }
        }
        return true
    }

    private suspend fun fetchGenius(title: String, artist: String): LyricsDoc? {        val query = "${nfc(title)} ${nfc(artist)}".trim()
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
        var bestPath: String? = null
        var bestScore = 0

        for (sectionIndex in 0 until sections.length()) {
            val section = sections.optJSONObject(sectionIndex) ?: continue
            // Las secciones "lyric" son snippets de canciones que mencionan la
            // consulta; nunca son la ficha de la canción.
            val sectionType = section.optString("type")
            if (sectionType != "top_hit" && sectionType != "song") continue
            val hits = section.optJSONArray("hits") ?: continue
            for (hitIndex in 0 until hits.length()) {
                val result = hits.optJSONObject(hitIndex)
                    ?.optJSONObject("result") ?: continue
                if (result.optString("_type") != "song") continue
                if (result.optString("lyrics_state") != "complete") continue
                val resultTitle = normalizeText(result.optString("title"))
                val resultArtist = normalizeText(result.optString("artist_names"))
                val exactTitle = resultTitle == wantedTitle
                val titleOk = exactTitle ||
                    resultTitle.contains(wantedTitle) ||
                    wantedTitle.contains(resultTitle)
                val exactArtist = wantedArtist.isNotBlank() && resultArtist == wantedArtist
                val artistOk = wantedArtist.isBlank() || exactArtist ||
                    resultArtist.contains(wantedArtist) ||
                    wantedArtist.contains(resultArtist)
                if (!titleOk || !artistOk) continue
                val score = (if (exactTitle) 100 else 40) + (if (exactArtist) 100 else 20)
                if (score > bestScore) {
                    bestScore = score
                    bestPath = result.optString("path").takeIf { it.isNotBlank() }
                }
            }
        }
        val path = bestPath ?: return null
        val pageUrl = if (path.startsWith("http")) path else "https://genius.com$path"
        val page = executeGenius(pageUrl.toHttpUrl(), acceptHtml = true) ?: return null
        val text = extractGeniusLyrics(page)
        if (text.isBlank()) return null
        val header = normalizeText("$title lyrics")
        val cleaned = text.lineSequence()
            .filterNot { normalizeText(it) == header }
            .joinToString("\n")
        if (cleaned.isBlank()) return null
        return LrcParser.parsePlain(cleaned, source = "genius")
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

    private fun normalizeText(value: String): String =
        Normalizer.normalize(value, Normalizer.Form.NFD)
            .replace(Regex("\\p{M}+"), "")
            .lowercase(Locale.ROOT)
            .replace(Regex("[^a-z0-9]+"), " ")
            .trim()

    private fun nfc(value: String): String =
        Normalizer.normalize(value, Normalizer.Form.NFC).trim()

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
        private const val LYRICS_PLUS_API = "https://lyricsplus.prjktla.my.id/v2/lyrics/get"
        private const val LRC_MUX_COMPAT = "https://api.lrcmux.dev/compat/lrclib/api/get"
        private const val LRC_MUX_NATIVE = "https://api.lrcmux.dev/get"
        private const val GENIUS_TEXT_MATCH = 0.55
        private const val GENIUS_USER_AGENT =
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) " +
                "Chrome/124.0 Mobile Safari/537.36"
        private const val CACHE_DIR = "lyrics"
        private const val CACHE_VERSION = 3
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
