package dev.spote.app.lyrics

/**
 * A single timed word inside a lyric line.
 *
 * [endMs] is `null` when the end cannot be inferred (e.g. the word is the last one and the
 * next line never starts).
 */
data class LyricWord(
    val startMs: Long,
    val endMs: Long?,
    val text: String,
)

/**
 * One lyric line.
 *
 * [words] is non-empty only for enhanced LRC lines that carry `<mm:ss.xx>` word tags.
 * Plain lines have their text available in [text] and no word timing.
 */
data class LyricLine(
    val startMs: Long,
    val text: String,
    val words: List<LyricWord> = emptyList(),
)

/**
 * Parsed lyrics for a track.
 *
 * [synced] is `true` when [lines] have real timestamps. Unsynced documents use `startMs = 0`
 * for every line and keep the original text in [plain].
 */
data class LyricsDoc(
    val lines: List<LyricLine>,
    val synced: Boolean,
    val source: String,
    val plain: String? = null,
) {
    val isEmpty: Boolean get() = lines.isEmpty()

    /**
     * Index of the line that should be highlighted at [positionMs], or `-1` before the first
     * line. [lines] must be sorted by start time (the parser guarantees this).
     */
    fun lineIndexAt(positionMs: Long): Int {
        var low = 0
        var high = lines.lastIndex
        var found = -1
        while (low <= high) {
            val mid = (low + high) ushr 1
            if (lines[mid].startMs <= positionMs) {
                found = mid
                low = mid + 1
            } else {
                high = mid - 1
            }
        }
        return found
    }
}
