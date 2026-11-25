package dev.spote.app.playback

/**
 * A single playable item for the Media3 layer. [uri] may be a `file://`, `content://`
 * or `http(s)://` URI; Media3 chooses the right data source automatically.
 */
data class PlaybackTrack(
    val key: String,
    val uri: String,
    val title: String,
    val artist: String,
    val album: String? = null,
    val artUri: String? = null,
    val durationMs: Long = 0L,
    /** Necesario para HLS sin extensión `.m3u8` (SoundCloud). */
    val mimeType: String? = null,
)

/**
 * Snapshot of the real player state exposed to the UI.
 *
 * [repeatMode] uses the app-level mapping: 0 = off, 1 = all, 2 = one.
 */
data class PlaybackUiState(
    val current: PlaybackTrack? = null,
    val isPlaying: Boolean = false,
    val positionMs: Long = 0L,
    val durationMs: Long = 0L,
    val shuffle: Boolean = false,
    val repeatMode: Int = 0,
    val queue: List<PlaybackTrack> = emptyList(),
)
