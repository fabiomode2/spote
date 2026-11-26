package dev.spote.app.remote

/*
 * On-device remote music models.
 *
 * This package talks to NewPipeExtractor v0.27.1, which is free software licensed under the
 * GNU General Public License v3.0 or later (https://github.com/TeamNewPipe/NewPipeExtractor).
 * NewPipeExtractor is consumed as an unmodified library dependency; no extractor code is copied
 * here. A binary that links it is a combined work and must be distributed under GPL-3.0 terms.
 */

/** A track found through the on-device search, before its audio stream is resolved. */
data class RemoteTrack(
    /** Stable key: `yt:<videoId>`, `sc:<artist>/<track>` or `youtube:<url>` as a fallback. */
    val key: String,
    val title: String,
    val artist: String,
    val album: String?,
    val durationMs: Long,
    val thumbnailUrl: String?,
    val webUrl: String,
    /** [SOURCE_YOUTUBE] or [SOURCE_SOUNDCLOUD]. */
    val source: String,
) {
    val isYouTube: Boolean get() = source == SOURCE_YOUTUBE

    companion object {
        const val SOURCE_YOUTUBE = "youtube"
        const val SOURCE_SOUNDCLOUD = "soundcloud"
    }
}

/** A resolved audio stream plus display metadata. */
data class ResolvedAudio(
    /** Direct media URL (googlevideo / SoundCloud CDN) or an HLS manifest. */
    val url: String,
    val mimeType: String?,
    /** Bitrate in kbps, or 0 when unknown. */
    val bitrate: Int,
    val durationMs: Long,
    val title: String,
    val artist: String,
    val album: String?,
    val thumbnailUrl: String?,
    val webUrl: String,
    /** HLS se reproduce en streaming; no se descarga como archivo único. */
    val isHls: Boolean = false,
)
