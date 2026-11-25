package dev.spote.app.lyrics

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LrcParserTest {

    @Test
    fun `parses centisecond timestamps`() {
        val lines = LrcParser.parseLines("[00:12.34]Hello")

        assertEquals(1, lines.size)
        assertEquals(12_340L, lines[0].startMs)
        assertEquals("Hello", lines[0].text)
        assertTrue(lines[0].words.isEmpty())
    }

    @Test
    fun `parses millisecond timestamps`() {
        val lines = LrcParser.parseLines("[01:02.345]Hi")

        assertEquals(62_345L, lines[0].startMs)
    }

    @Test
    fun `parses timestamps without fractions`() {
        val lines = LrcParser.parseLines("[01:02]Hi")

        assertEquals(62_000L, lines[0].startMs)
    }

    @Test
    fun `line with several timestamps produces several lines`() {
        val lines = LrcParser.parseLines("[00:10.00][00:30.50]Chorus")

        assertEquals(2, lines.size)
        assertEquals(10_000L, lines[0].startMs)
        assertEquals(30_500L, lines[1].startMs)
        assertEquals("Chorus", lines[0].text)
        assertEquals("Chorus", lines[1].text)
    }

    @Test
    fun `unordered timestamps are sorted`() {
        val lines = LrcParser.parseLines(
            """
            [00:20.00]B
            [00:10.00]A
            """.trimIndent(),
        )

        assertEquals(listOf("A", "B"), lines.map { it.text })
        assertEquals(listOf(10_000L, 20_000L), lines.map { it.startMs })
    }

    @Test
    fun `metadata tags are ignored`() {
        val lines = LrcParser.parseLines(
            """
            [ar:Some Artist]
            [ti:Some Title]
            [al:Some Album]
            [by:uploader]
            [length:03:21]
            [00:05.00]First line
            """.trimIndent(),
        )

        assertEquals(1, lines.size)
        assertEquals("First line", lines[0].text)
        assertEquals(5_000L, lines[0].startMs)
    }

    @Test
    fun `empty and untimed lines are ignored`() {
        val lines = LrcParser.parseLines(
            """
            Just a header without timing

            [00:01.00]
            [00:02.00]Real line
            """.trimIndent(),
        )

        assertEquals(1, lines.size)
        assertEquals("Real line", lines[0].text)
    }

    @Test
    fun `positive offset moves lines later`() {
        val doc = LrcParser.parse(
            """
            [offset:+250]
            [00:10.00]Late
            """.trimIndent(),
        )

        assertEquals(10_250L, doc.lines.single().startMs)
    }

    @Test
    fun `negative offset moves lines earlier and clamps at zero`() {
        val shifted = LrcParser.parse(
            """
            [offset:-250]
            [00:10.50]Early
            """.trimIndent(),
        )
        assertEquals(10_250L, shifted.lines.single().startMs)

        val clamped = LrcParser.parse(
            """
            [offset:-15000]
            [00:10.00]Too early
            """.trimIndent(),
        )
        assertEquals(0L, clamped.lines.single().startMs)
    }

    @Test
    fun `word tags are removed from line text and timed`() {
        val lines = LrcParser.parseLines("[00:10.00]<00:10.00>Hello <00:11.50>world")
        val line = lines.single()

        assertEquals("Hello world", line.text)
        assertFalse(line.text.contains('<'))
        assertEquals(2, line.words.size)

        assertEquals("Hello", line.words[0].text)
        assertEquals(10_000L, line.words[0].startMs)
        assertEquals(11_500L, line.words[0].endMs)

        assertEquals("world", line.words[1].text)
        assertEquals(11_500L, line.words[1].startMs)
        // Last line: end falls back to a default tail after its last word.
        assertEquals(15_500L, line.words[1].endMs)
    }

    @Test
    fun `word end is inferred from the next line`() {
        val lines = LrcParser.parseLines(
            """
            [00:10.00]<00:10.00>Hello <00:11.00>world
            [00:15.00]Next
            """.trimIndent(),
        )

        assertEquals(11_000L, lines[0].words[0].endMs)
        assertEquals(15_000L, lines[0].words[1].endMs)
    }

    @Test
    fun `text before the first word tag is anchored to the line start`() {
        val line = LrcParser.parseLines("[00:10.00]Intro <00:11.00>Hello").single()

        assertEquals("Intro Hello", line.text)
        assertEquals(listOf("Intro", "Hello"), line.words.map { it.text })
        assertEquals(10_000L, line.words[0].startMs)
        assertEquals(11_000L, line.words[0].endMs)
        assertEquals(11_000L, line.words[1].startMs)
    }

    @Test
    fun `repeated line with word tags shifts the words`() {
        val lines = LrcParser.parseLines("[00:10.00][00:20.00]<00:10.00>Hello <00:11.00>world")

        assertEquals(2, lines.size)
        assertEquals(10_000L, lines[0].words[0].startMs)
        assertEquals(20_000L, lines[1].words[0].startMs)
        assertEquals(21_000L, lines[1].words[1].startMs)
    }

    @Test
    fun `parse reports synced and keeps the requested source`() {
        val doc = LrcParser.parse("[00:01.00]Hi", source = "test")

        assertTrue(doc.synced)
        assertEquals("test", doc.source)
        assertEquals(1, doc.lines.size)
    }

    @Test
    fun `parse without timestamps is not synced`() {
        val doc = LrcParser.parse("just some text")

        assertFalse(doc.synced)
        assertTrue(doc.lines.isEmpty())
    }

    @Test
    fun `plain lyrics are split into trimmed unsynced lines`() {
        val raw = "line one\n\n  line two  \n"
        val doc = LrcParser.parsePlain(raw, source = "lrclib")

        assertFalse(doc.synced)
        assertEquals(listOf("line one", "line two"), doc.lines.map { it.text })
        assertEquals(listOf(0L, 0L), doc.lines.map { it.startMs })
        assertEquals(raw, doc.plain)
        assertEquals("lrclib", doc.source)
    }

    @Test
    fun `lineIndexAt finds the active line`() {
        val doc = LrcParser.parse(
            """
            [00:01.00]One
            [00:02.00]Two
            """.trimIndent(),
        )

        assertEquals(-1, doc.lineIndexAt(999L))
        assertEquals(0, doc.lineIndexAt(1_000L))
        assertEquals(0, doc.lineIndexAt(1_999L))
        assertEquals(1, doc.lineIndexAt(2_000L))
        assertEquals(1, doc.lineIndexAt(60_000L))
    }
}
