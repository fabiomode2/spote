package dev.spote.app.remote

/*
 * NewPipeExtractor-backed resolver for Spote.
 *
 * NewPipeExtractor v0.27.1 is free software licensed under the GNU General Public License
 * v3.0 or later (https://github.com/TeamNewPipe/NewPipeExtractor) and is consumed as an
 * unmodified library dependency. A binary that links it must be distributed under GPL-3.0
 * terms. See also NewPipeDownloader.kt.
 */

import android.content.Context
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import org.schabi.newpipe.extractor.Image
import org.schabi.newpipe.extractor.MediaFormat
import org.schabi.newpipe.extractor.NewPipe
import org.schabi.newpipe.extractor.ServiceList
import org.schabi.newpipe.extractor.StreamingService
import org.schabi.newpipe.extractor.exceptions.ContentNotAvailableException
import org.schabi.newpipe.extractor.exceptions.PaidContentException
import org.schabi.newpipe.extractor.exceptions.ReCaptchaException
import org.schabi.newpipe.extractor.exceptions.SignInConfirmNotBotException
import org.schabi.newpipe.extractor.exceptions.SoundCloudGoPlusContentException
import org.schabi.newpipe.extractor.localization.Localization
import org.schabi.newpipe.extractor.search.SearchExtractor
import org.schabi.newpipe.extractor.stream.AudioStream
import org.schabi.newpipe.extractor.stream.AudioTrackType
import org.schabi.newpipe.extractor.stream.DeliveryMethod
import org.schabi.newpipe.extractor.stream.StreamInfo
import org.schabi.newpipe.extractor.stream.StreamInfoItem
import org.schabi.newpipe.extractor.stream.StreamType
import java.io.IOException
import java.util.Locale

/** Thrown by [RemoteResolver] with a user/UI-friendly message and the original cause. */
class RemoteResolverException(message: String, cause: Throwable? = null) : IOException(message, cause)

/**
 * On-device YouTube/SoundCloud search and audio resolution.
 *
 * YouTube stream URLs are resolved to direct progressive/DASH audio when possible. For
 * SoundCloud, v0.27.1 only exposes HLS transcodings: Media3 plays them, but they are not
 * downloadable as a single file.
 *
 * Call [init] once (e.g. from `Application.onCreate`) before using [search], [resolve] or
 * [resolveUrl]. All network work runs on [Dispatchers.IO]; exceptions are wrapped in
 * [RemoteResolverException] (cancellation is always rethrown untouched).
 */
object RemoteResolver {

    private const val MAX_PAGES_PER_SERVICE = 2
    private const val UNKNOWN_ARTIST = "Unknown artist"
    private const val UNKNOWN_TITLE = "Unknown title"

    // Resolved lazily so ServiceList is not touched before NewPipe.init().
    // SoundCloud se busca, pero v0.27.1 solo expone transcodings HLS: se reproducen con
    // Media3 y se marcan como no descargables. YouTube sigue prefiriendo audio directo.
    private val searchSources: List<Pair<StreamingService, String>> by lazy {
        listOf(
            ServiceList.YouTube to RemoteTrack.SOURCE_YOUTUBE,
            ServiceList.SoundCloud to RemoteTrack.SOURCE_SOUNDCLOUD,
        )
    }

    private val YOUTUBE_ID_REGEX = Regex(
        """(?:[?&]v=|youtu\.be/|/shorts/|/embed/|/live/|/v/)([A-Za-z0-9_-]{11})""",
    )

    @Volatile
    private var initialized = false

    /**
     * Initializes NewPipeExtractor once. Idempotent and thread-safe.
     * The device language/country is used so titles and search results are localized.
     */
    @Synchronized
    fun init(context: Context) {
        if (initialized) return
        val applicationContext = context.applicationContext ?: context
        NewPipe.init(NewPipeDownloader(), deviceLocalization(applicationContext))
        initialized = true
    }

    /**
     * Searches YouTube. Live streams, ended live streams and zero-duration items are filtered
     * out.
     *
     * SoundCloud is not searched: NewPipeExtractor v0.27.1 currently exposes only HLS
     * transcodings for it, which are not direct downloadable audio files. Use [resolveUrl] with
     * a SoundCloud URL only if HLS playback is handled by the caller.
     *
     * @param limit maximum number of tracks to return.
     */
    suspend fun search(query: String, limit: Int = 20): List<RemoteTrack> {
        requireInitialized()
        val trimmed = query.trim()
        if (trimmed.isEmpty() || limit <= 0) return emptyList()

        return try {
            withContext(Dispatchers.IO) { searchInternal(trimmed, limit) }
        } catch (cancellation: CancellationException) {
            throw cancellation
        } catch (alreadyWrapped: RemoteResolverException) {
            throw alreadyWrapped
        } catch (t: Throwable) {
            throw asRemoteError("Search for \"$trimmed\"", t)
        }
    }

    /** Resolves a search result to a direct audio URL. */
    suspend fun resolve(track: RemoteTrack): ResolvedAudio {
        requireInitialized()
        return resolveInternal(
            url = track.webUrl,
            sourceHint = track.source,
            label = track.title.ifBlank { track.webUrl },
        )
    }

    /** Resolves a watch/track URL (YouTube or SoundCloud) to a direct audio URL. */
    suspend fun resolveUrl(url: String): ResolvedAudio {
        requireInitialized()
        return resolveInternal(url = url, sourceHint = null, label = url)
    }

    // ---------------------------------------------------------------------------------------
    // Search
    // ---------------------------------------------------------------------------------------

    private suspend fun searchInternal(query: String, limit: Int): List<RemoteTrack> {
        val merged = LinkedHashMap<String, RemoteTrack>(limit)
        var firstFailure: Throwable? = null

        for ((service, source) in searchSources) {
            if (merged.size >= limit) break
            try {
                val found = searchService(
                    service = service,
                    query = query,
                    wanted = limit - merged.size,
                    source = source,
                )
                for (track in found) {
                    merged.putIfAbsent(track.key, track)
                }
            } catch (cancellation: CancellationException) {
                throw cancellation
            } catch (_: SearchExtractor.NothingFoundException) {
                // No results from this service; try the next one.
            } catch (t: Throwable) {
                if (firstFailure == null) firstFailure = t
            }
        }

        if (merged.isEmpty() && firstFailure != null) {
            throw asRemoteError("Search for \"$query\"", firstFailure)
        }
        return merged.values.take(limit)
    }

    private suspend fun searchService(
        service: StreamingService,
        query: String,
        wanted: Int,
        source: String,
    ): List<RemoteTrack> {
        val extractor = service.getSearchExtractor(query)
        extractor.fetchPage()
        var page = extractor.initialPage
        val found = LinkedHashMap<String, RemoteTrack>()
        var fetchedPages = 1

        while (true) {
            currentCoroutineContext().ensureActive()
            for (item in page.items) {
                if (item !is StreamInfoItem) continue
                val track = item.toRemoteTrack(source) ?: continue
                found.putIfAbsent(track.key, track)
                if (found.size >= wanted) break
            }

            if (found.size >= wanted ||
                !page.hasNextPage() ||
                fetchedPages >= MAX_PAGES_PER_SERVICE
            ) {
                break
            }

            page = extractor.getPage(page.nextPage)
            fetchedPages++
        }

        return found.values.toList()
    }

    private fun StreamInfoItem.toRemoteTrack(source: String): RemoteTrack? {
        val title = name?.trim().orEmpty()
        val webUrl = url?.trim().orEmpty()
        if (title.isEmpty() || webUrl.isEmpty()) return null

        // Exclude running/ended live streams and unknown-duration items.
        if (streamType != StreamType.VIDEO_STREAM && streamType != StreamType.AUDIO_STREAM) return null
        if (duration <= 0L) return null

        val uploader = uploaderName?.trim().orEmpty()
        return RemoteTrack(
            key = trackKey(source, webUrl),
            title = title,
            artist = cleanArtist(uploader).ifBlank { UNKNOWN_ARTIST },
            album = null,
            durationMs = duration * 1000L,
            thumbnailUrl = bestThumbnailUrl(thumbnails),
            webUrl = webUrl,
            source = source,
        )
    }

    // ---------------------------------------------------------------------------------------
    // Resolution
    // ---------------------------------------------------------------------------------------

    private suspend fun resolveInternal(
        url: String,
        sourceHint: String?,
        label: String,
    ): ResolvedAudio {
        val normalized = normalizeUrl(url)
        return try {
            withContext(Dispatchers.IO) {
                val service = serviceFor(sourceHint)
                val info = if (service != null) {
                    StreamInfo.getInfo(service, normalized)
                } else {
                    StreamInfo.getInfo(normalized)
                }
                info.toResolvedAudio(normalized)
            }
        } catch (cancellation: CancellationException) {
            throw cancellation
        } catch (alreadyWrapped: RemoteResolverException) {
            throw alreadyWrapped
        } catch (t: Throwable) {
            throw asRemoteError("Resolve of \"$label\"", t)
        }
    }

    private fun StreamInfo.toResolvedAudio(requestedUrl: String): ResolvedAudio {
        val audio = AudioStreamSelector.select(audioStreams)
            ?: throw RemoteResolverException(noAudioStreamMessage())

        val uploader = uploaderName?.trim().orEmpty()
        val isHls = audio.deliveryMethod == DeliveryMethod.HLS ||
            audio.content.contains(".m3u8", ignoreCase = true)
        return ResolvedAudio(
            url = audio.content,
            mimeType = if (isHls) {
                "application/vnd.apple.mpegurl"
            } else {
                audio.format?.mimeType ?: mimeTypeForCodec(audio.codec)
            },
            bitrate = AudioStreamSelector.effectiveBitrate(audio),
            durationMs = duration.coerceAtLeast(0L) * 1000L,
            title = name?.trim().orEmpty().ifBlank { UNKNOWN_TITLE },
            artist = cleanArtist(uploader).ifBlank { UNKNOWN_ARTIST },
            album = null,
            thumbnailUrl = bestThumbnailUrl(thumbnails),
            webUrl = url?.takeIf { it.isNotBlank() } ?: requestedUrl,
            isHls = isHls,
        )
    }

    private fun StreamInfo.noAudioStreamMessage(): String {
        if (audioStreams.isEmpty()) {
            return "No audio streams found for \"$name\" (stream type: $streamType). " +
                "It may be a live stream, or the extractor was blocked by the service."
        }
        val available = audioStreams.joinToString(limit = 4) {
            "${it.format ?: "unknown"}/${it.deliveryMethod}"
        }
        return "No playable audio stream for \"$name\" (stream type: $streamType; " +
            "available: $available)."
    }

    private fun serviceFor(source: String?): StreamingService? = when (source) {
        RemoteTrack.SOURCE_YOUTUBE -> ServiceList.YouTube
        RemoteTrack.SOURCE_SOUNDCLOUD -> ServiceList.SoundCloud
        else -> null
    }

    private fun normalizeUrl(raw: String): String {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) throw RemoteResolverException("Cannot resolve an empty URL")
        return when {
            trimmed.startsWith("https://", ignoreCase = true) ||
                trimmed.startsWith("http://", ignoreCase = true) -> trimmed
            trimmed.startsWith("//") -> "https:$trimmed"
            else -> "https://$trimmed"
        }
    }

    // ---------------------------------------------------------------------------------------
    // Metadata helpers
    // ---------------------------------------------------------------------------------------

    private fun trackKey(source: String, webUrl: String): String {
        if (source == RemoteTrack.SOURCE_YOUTUBE) {
            YOUTUBE_ID_REGEX.find(webUrl)?.groupValues?.getOrNull(1)?.let { return "yt:$it" }
        }
        if (source == RemoteTrack.SOURCE_SOUNDCLOUD) {
            soundCloudPath(webUrl)?.let { return "sc:$it" }
        }
        return "$source:$webUrl"
    }

    private fun soundCloudPath(url: String): String? {
        val path = url.substringAfter("://", missingDelimiterValue = "")
            .substringAfter('/', missingDelimiterValue = "")
            .substringBefore('?')
            .substringBefore('#')
            .trim('/')
        if (path.isBlank() || !path.contains('/')) return null
        return path
    }

    private fun cleanArtist(uploader: String): String =
        uploader.removeSuffix(" - Topic").trim()

    private fun bestThumbnailUrl(images: List<Image>): String? = images
        .asSequence()
        .filter { it.url.isNotBlank() }
        .maxByOrNull { image ->
            val pixels = if (image.width > 0 && image.height > 0) {
                image.width.toLong() * image.height.toLong()
            } else {
                0L
            }
            val level = when (image.estimatedResolutionLevel) {
                Image.ResolutionLevel.HIGH -> 3L
                Image.ResolutionLevel.MEDIUM -> 2L
                Image.ResolutionLevel.LOW -> 1L
                else -> 0L
            }
            pixels * 10L + level
        }
        ?.url

    private fun mimeTypeForCodec(codec: String?): String? {
        val normalized = codec?.lowercase(Locale.ROOT) ?: return null
        return when {
            normalized.contains("mp4a") || normalized.contains("aac") -> "audio/mp4"
            normalized.contains("opus") -> "audio/webm"
            normalized.contains("vorbis") -> "audio/ogg"
            normalized.contains("mpeg") || normalized.contains("mp3") -> "audio/mpeg"
            normalized.contains("flac") -> "audio/flac"
            else -> null
        }
    }

    private fun deviceLocalization(context: Context): Localization {
        val locale = runCatching {
            context.resources.configuration.locales[0]
        }.getOrNull() ?: Locale.ENGLISH

        val language = locale.language.takeIf { it.isNotBlank() } ?: "en"
        val country = locale.country.takeIf { it.isNotBlank() }
        return Localization(language, country)
    }

    // ---------------------------------------------------------------------------------------
    // Errors
    // ---------------------------------------------------------------------------------------

    private fun requireInitialized() {
        if (!initialized) {
            throw RemoteResolverException(
                "RemoteResolver.init(context) must be called before using the remote resolver",
            )
        }
    }

    private fun asRemoteError(action: String, failure: Throwable): RemoteResolverException {
        val reason = when (failure) {
            is ReCaptchaException ->
                "the service requested a CAPTCHA (HTTP 429 rate limit); try again later"
            is SignInConfirmNotBotException ->
                "YouTube requires sign-in to confirm this client is not a bot; " +
                    "the network/IP is likely rate-limited"
            is SoundCloudGoPlusContentException ->
                "this SoundCloud track requires a Go+ subscription"
            is PaidContentException ->
                "this content requires a paid subscription"
            is ContentNotAvailableException ->
                "content is not available (private, removed, age-restricted or blocked in " +
                    "this region)"
            is SearchExtractor.NothingFoundException ->
                "the service returned no results"
            is IOException ->
                "network error: ${failure.message ?: failure.javaClass.simpleName}"
            else ->
                failure.message?.takeIf { it.isNotBlank() }
                    ?: "extractor error: ${failure.javaClass.simpleName}"
        }
        return RemoteResolverException("$action failed: $reason", failure)
    }
}

/**
 * Pure audio-stream ranking, kept separate so it can be unit-tested without network access.
 *
 * Priority: original audio track, then m4a/AAC at an adequate bitrate, then Opus/WebM, then
 * any other audio format; bitrate breaks ties with a cap around 160-192 kbps.
 */
internal object AudioStreamSelector {

    fun select(streams: List<AudioStream>): AudioStream? {
        val direct = streams
            .asSequence()
            .filter { it.isPlayableDirectAudio() }
            .maxByOrNull { score(it) }
        if (direct != null) return direct
        // SoundCloud solo ofrece HLS en v0.27.1: se usa como último recurso.
        return streams
            .asSequence()
            .filter { it.isPlayableHls() }
            .maxByOrNull { score(it) }
    }

    /** Average bitrate when available, otherwise the declared one; 0 when unknown. */
    fun effectiveBitrate(stream: AudioStream): Int {
        val average = stream.averageBitrate
        if (average > 0) return average
        val declared = stream.bitrate
        return if (declared > 0) declared else 0
    }

    private fun AudioStream.isPlayableDirectAudio(): Boolean {
        if (!isUrl) return false
        if (content.isBlank()) return false
        if (content.contains(".m3u8", ignoreCase = true)) return false
        if (deliveryMethod != DeliveryMethod.PROGRESSIVE_HTTP &&
            deliveryMethod != DeliveryMethod.DASH
        ) {
            return false
        }
        return isAudioFormat(format)
    }

    private fun AudioStream.isPlayableHls(): Boolean {
        if (!isUrl) return false
        if (content.isBlank()) return false
        val isManifest = deliveryMethod == DeliveryMethod.HLS ||
            content.contains(".m3u8", ignoreCase = true)
        return isManifest && isAudioFormat(format)
    }

    private fun isAudioFormat(format: MediaFormat?): Boolean = when (format) {
        null,
        MediaFormat.M4A,
        MediaFormat.WEBMA,
        MediaFormat.MP3,
        MediaFormat.MP2,
        MediaFormat.OPUS,
        MediaFormat.OGG,
        MediaFormat.WEBMA_OPUS,
        MediaFormat.AIFF,
        MediaFormat.AIF,
        MediaFormat.WAV,
        MediaFormat.FLAC,
        MediaFormat.ALAC -> true

        else -> false
    }

    private fun score(stream: AudioStream): Long {
        val formatScore = formatScore(stream) * 10_000_000L
        val trackScore = trackScore(stream)
        val bitrateScore = effectiveBitrate(stream).coerceIn(0, 192).toLong() * 1_000L
        return formatScore + trackScore + bitrateScore
    }

    private fun formatScore(stream: AudioStream): Long {
        val format = stream.format
        val bitrate = effectiveBitrate(stream)
        val adequateBitrate = bitrate <= 0 || bitrate >= 96
        val isAac = format == MediaFormat.M4A
        val isOpusWebm = format == MediaFormat.OPUS ||
            format == MediaFormat.WEBMA_OPUS ||
            format == MediaFormat.WEBMA

        return when {
            isAac && adequateBitrate -> 4L
            isOpusWebm && adequateBitrate -> 3L
            isAac -> 2L
            isOpusWebm -> 1L
            else -> 0L
        }
    }

    private fun trackScore(stream: AudioStream): Long = when (stream.audioTrackType) {
        AudioTrackType.ORIGINAL, null -> 100_000_000L
        AudioTrackType.SECONDARY -> 50_000_000L
        AudioTrackType.DUBBED -> 20_000_000L
        AudioTrackType.DESCRIPTIVE -> 0L
        else -> 100_000_000L
    }
}
