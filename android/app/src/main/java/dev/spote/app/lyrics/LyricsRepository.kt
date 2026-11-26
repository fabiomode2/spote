package dev.spote.app.lyrics

import android.content.Context

/**
 * Thin facade over [LyricsClient] for callers that do not want to deal with the client's
 * cache-key parameter directly.
 *
 * Create one instance per application scope (it keeps an in-memory cache) and call
 * [forTrack] from a coroutine.
 */
class LyricsRepository(
    context: Context,
    private val client: LyricsClient = LyricsClient(context),
) {
    suspend fun forTrack(
        key: String,
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
    ): LyricsDoc? = client.get(
        title = title,
        artist = artist,
        album = album,
        durationMs = durationMs,
        cacheKey = key,
    )

    companion object {
        /** Makes an arbitrary track key safe to use as a file name. */
        fun sanitizeFileKey(key: String): String = LyricsClient.sanitizeFileKey(key)
    }
}
