package dev.spote.app.lyrics

/**
 * Parser for LRC and enhanced LRC (word-level) lyrics.
 *
 * Supported syntax:
 *  - `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]` (and the less common `[mm:ss:xx]`).
 *  - Several timestamps on the same line, e.g. `[00:10.00][00:30.00]Chorus`.
 *  - Metadata tags such as `[ar:Artist]`, `[ti:Title]`, `[length:03:21]`; they are dropped.
 *  - `[offset:+250]` / `[offset:-250]` (milliseconds; positive moves lyrics later).
 *  - Word tags `<00:12.50>word` for karaoke-style highlighting.
 *
 * The parser is pure JVM code: no Android or networking dependencies.
 */
object LrcParser {
    /** Fallback duration given to the last line when nothing else is known. */
    private const val DEFAULT_LINE_DURATION_MS = 4_000L

    private val LEADING_TAG = Regex("^\\[([^\\]]*)]")
    private val TIMESTAMP_TAG = Regex("^(\\d{1,4}):(\\d{1,2})(?:[.:](\\d{1,3}))?$")
    private val METADATA_TAG = Regex("^[A-Za-z][A-Za-z0-9_#-]*\\s*:.*$")
    private val OFFSET_TAG = Regex("^offset\\s*:\\s*([+-]?\\d+)$", RegexOption.IGNORE_CASE)
    private val WORD_TAG = Regex("<(\\d{1,4}):(\\d{1,2})(?:[.:](\\d{1,3}))?>")

    /**
     * Parses [text] as timed LRC. Returns a document with `synced = false` and no lines when
     * the text contains no timestamps at all.
     */
    fun parse(text: String, source: String = "lrc"): LyricsDoc {
        val lines = parseLines(text)
        return LyricsDoc(lines = lines, synced = lines.isNotEmpty(), source = source)
    }

    /**
     * Splits plain (unsynced) lyrics into one [LyricLine] per non-empty line.
     * All lines get `startMs = 0`; callers must check [LyricsDoc.synced] before scrolling.
     */
    fun parsePlain(text: String, source: String = "plain"): LyricsDoc {
        val lines = text.lineSequence()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .map { LyricLine(startMs = 0L, text = it) }
            .toList()
        return LyricsDoc(lines = lines, synced = false, source = source, plain = text)
    }

    /**
     * Parses timed lines only, sorted by start time, with word ends inferred.
     * Metadata, empty and untimed lines are ignored.
     */
    fun parseLines(text: String): List<LyricLine> {
        val rawLines = mutableListOf<RawLine>()
        var offsetMs = 0L

        text.removePrefix("\uFEFF").lineSequence().forEach { rawLine ->
            val scan = scanLeadingTags(rawLine)
            offsetMs += scan.offsetDelta

            if (scan.content.isBlank() || scan.timestamps.isEmpty()) return@forEach

            val times = scan.timestamps.sorted()
            val (lineText, wordMarks) = parseWordMarks(scan.content, times.first())
            if (lineText.isEmpty() && wordMarks.isEmpty()) return@forEach

            rawLines += RawLine(times = times, text = lineText, words = wordMarks)
        }

        if (rawLines.isEmpty()) return emptyList()

        // Expand lines that carry multiple timestamps. Word tags are absolute; when a line is
        // repeated at a later time, shift its word marks by the same delta.
        val emitted = mutableListOf<EmittedLine>()
        for (raw in rawLines) {
            val first = raw.times.first()
            for (time in raw.times) {
                val shift = time - first
                val lineStart = (time + offsetMs).coerceAtLeast(0L)
                val words = raw.words.map { mark ->
                    LyricWord(
                        startMs = (mark.startMs + shift + offsetMs).coerceAtLeast(lineStart),
                        endMs = null,
                        text = mark.text,
                    )
                }
                emitted += EmittedLine(startMs = lineStart, text = raw.text, words = words)
            }
        }

        emitted.sortBy { it.startMs }

        return emitted.mapIndexed { index, line ->
            val nextLineStart = emitted.getOrNull(index + 1)?.startMs
            val lineEnd = if (nextLineStart != null && nextLineStart > line.startMs) {
                nextLineStart
            } else {
                val lastWordStart = line.words.lastOrNull()?.startMs ?: line.startMs
                maxOf(lastWordStart, line.startMs) + DEFAULT_LINE_DURATION_MS
            }

            val words = line.words.mapIndexed { wordIndex, word ->
                val nextWordStart = line.words.getOrNull(wordIndex + 1)?.startMs
                val end = nextWordStart ?: lineEnd
                word.copy(endMs = end.takeIf { it > word.startMs })
            }
            LyricLine(startMs = line.startMs, text = line.text, words = words)
        }
    }

    /** Consumes leading `[...]` tags, returning their timestamps and the remaining text. */
    private fun scanLeadingTags(rawLine: String): TagScan {
        var rest = rawLine.trimStart()
        val timestamps = mutableListOf<Long>()
        var offsetDelta = 0L

        while (true) {
            val match = LEADING_TAG.find(rest) ?: break
            if (match.range.first != 0) break

            val tag = match.groupValues[1].trim()
            val time = parseTimestamp(tag)
            if (time != null) {
                timestamps += time
            } else {
                OFFSET_TAG.matchEntire(tag)?.let { offsetMatch ->
                    offsetDelta += offsetMatch.groupValues[1].toLongOrNull() ?: 0L
                }
                // Anything that looks like "key:value" is metadata; a bare "[Chorus]" is text.
                if (!METADATA_TAG.matches(tag)) break
            }

            rest = rest.substring(match.range.last + 1).trimStart()
        }

        return TagScan(timestamps = timestamps, content = rest, offsetDelta = offsetDelta)
    }

    private fun parseTimestamp(tag: String): Long? {
        val match = TIMESTAMP_TAG.matchEntire(tag) ?: return null
        return timeMs(
            minutes = match.groupValues[1].toLong(),
            seconds = match.groupValues[2].toLong(),
            fraction = match.groupValues[3],
        )
    }

    /**
     * Strips `<mm:ss.xx>` tags from [content] and returns the clean line plus its word marks.
     * Text before the first tag is anchored to the line start.
     */
    private fun parseWordMarks(content: String, lineStartMs: Long): Pair<String, List<WordMark>> {
        val matches = WORD_TAG.findAll(content).toList()
        if (matches.isEmpty()) return content.trim() to emptyList()

        val lineText = WORD_TAG.replace(content, "").trim()
        val marks = mutableListOf<WordMark>()

        val leadingText = content.substring(0, matches.first().range.first).trim()
        if (leadingText.isNotEmpty()) marks += WordMark(startMs = lineStartMs, text = leadingText)

        matches.forEachIndexed { index, match ->
            val start = timeMs(
                minutes = match.groupValues[1].toLong(),
                seconds = match.groupValues[2].toLong(),
                fraction = match.groupValues[3],
            )
            val textStart = match.range.last + 1
            val textEnd = matches.getOrNull(index + 1)?.range?.first ?: content.length
            val wordText = content.substring(textStart, textEnd).trim()
            if (wordText.isNotEmpty()) marks += WordMark(startMs = start, text = wordText)
        }

        return lineText to marks
    }

    private fun timeMs(minutes: Long, seconds: Long, fraction: String?): Long {
        val fractionMs = when {
            fraction.isNullOrEmpty() -> 0L
            fraction.length == 1 -> fraction.toLong() * 100L
            fraction.length == 2 -> fraction.toLong() * 10L
            else -> fraction.toLong()
        }
        return minutes * 60_000L + seconds * 1_000L + fractionMs
    }

    private data class TagScan(
        val timestamps: List<Long>,
        val content: String,
        val offsetDelta: Long,
    )

    private data class WordMark(
        val startMs: Long,
        val text: String,
    )

    private data class RawLine(
        val times: List<Long>,
        val text: String,
        val words: List<WordMark>,
    )

    private data class EmittedLine(
        val startMs: Long,
        val text: String,
        val words: List<LyricWord>,
    )
}
