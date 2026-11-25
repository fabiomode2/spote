package dev.spote.app.lyrics

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class LrclibMatcherTest {

    private fun track(
        trackName: String = "Song",
        artistName: String = "Artist",
        albumName: String? = null,
        durationSec: Double? = 200.0,
        synced: String? = "[00:01.00]line",
        plain: String? = null,
        instrumental: Boolean = false,
    ) = LrclibTrack(
        trackName = trackName,
        artistName = artistName,
        albumName = albumName,
        durationSec = durationSec,
        syncedLyrics = synced,
        plainLyrics = plain,
        instrumental = instrumental,
    )

    @Test
    fun `prefers a synced result over a plain one`() {
        val plainTrack = track(durationSec = 200.0, synced = null, plain = "line")
        val syncedTrack = track(durationSec = 202.0)

        val best = LrclibMatcher.selectBest(
            candidates = listOf(plainTrack, syncedTrack),
            title = "Song",
            artist = "Artist",
            durationMs = 200_000L,
        )

        assertSame(syncedTrack, best)
    }

    @Test
    fun `rejects results outside the three second tolerance`() {
        val tooLong = track(durationSec = 203.5)
        val tooShort = track(durationSec = 196.5)

        assertNull(
            LrclibMatcher.selectBest(listOf(tooLong), "Song", "Artist", durationMs = 200_000L),
        )
        assertNull(
            LrclibMatcher.selectBest(listOf(tooShort), "Song", "Artist", durationMs = 200_000L),
        )
    }

    @Test
    fun `accepts the tolerance boundary`() {
        val boundary = track(durationSec = 203.0)

        assertSame(
            boundary,
            LrclibMatcher.selectBest(listOf(boundary), "Song", "Artist", durationMs = 200_000L),
        )
    }

    @Test
    fun `picks the closest duration when everything else is equal`() {
        val offByOneSecond = track(durationSec = 201.0)
        val offByFourHundredMs = track(durationSec = 200.4)
        val offByTwoSeconds = track(durationSec = 202.0)

        val best = LrclibMatcher.selectBest(
            candidates = listOf(offByOneSecond, offByFourHundredMs, offByTwoSeconds),
            title = "Song",
            artist = "Artist",
            durationMs = 200_000L,
        )

        assertSame(offByFourHundredMs, best)
    }

    @Test
    fun `matches title and artist case and punctuation insensitively`() {
        val candidate = track(trackName = "SONG!", artistName = "the artist")

        assertSame(
            candidate,
            LrclibMatcher.selectBest(listOf(candidate), "Song", "The Artist"),
        )
    }

    @Test
    fun `matches accents against plain letters`() {
        val candidate = track(trackName = "Café", artistName = "Björk")

        assertSame(
            candidate,
            LrclibMatcher.selectBest(listOf(candidate), "Cafe", "Bjork"),
        )
    }

    @Test
    fun `matches a decorated title against its base title`() {
        val candidate = track(trackName = "Song (Remastered 2011)")

        assertSame(
            candidate,
            LrclibMatcher.selectBest(listOf(candidate), "Song", "Artist"),
        )
    }

    @Test
    fun `prefers an exact artist over a featured artist`() {
        val featured = track(artistName = "Artist feat. Guest", durationSec = 200.0)
        val exact = track(artistName = "Artist", durationSec = 202.0)

        val best = LrclibMatcher.selectBest(
            candidates = listOf(featured, exact),
            title = "Song",
            artist = "Artist",
            durationMs = 200_000L,
        )

        assertSame(exact, best)
    }

    @Test
    fun `accepts a featured credit when no exact artist exists`() {
        val featured = track(artistName = "Artist feat. Guest")

        assertNotNull(
            LrclibMatcher.selectBest(listOf(featured), "Song", "Artist"),
        )
    }

    @Test
    fun `album match breaks a tie`() {
        val withAlbum = track(albumName = "The Album")
        val otherAlbum = track(albumName = "Other Album")

        val best = LrclibMatcher.selectBest(
            candidates = listOf(otherAlbum, withAlbum),
            title = "Song",
            artist = "Artist",
            album = "The Album",
            durationMs = 200_000L,
        )

        assertSame(withAlbum, best)
    }

    @Test
    fun `ignores instrumental results`() {
        val instrumental = track(instrumental = true)

        assertNull(LrclibMatcher.selectBest(listOf(instrumental), "Song", "Artist"))
    }

    @Test
    fun `ignores results without lyrics`() {
        val noLyrics = track(synced = null, plain = null)

        assertNull(LrclibMatcher.selectBest(listOf(noLyrics), "Song", "Artist"))
    }

    @Test
    fun `ignores results with a different title`() {
        val otherTitle = track(trackName = "Another Song")

        assertNull(LrclibMatcher.selectBest(listOf(otherTitle), "Song", "Artist"))
    }

    @Test
    fun `ignores results with a different artist`() {
        val otherArtist = track(artistName = "Someone Else")

        assertNull(LrclibMatcher.selectBest(listOf(otherArtist), "Song", "Artist"))
    }

    @Test
    fun `returns null for an empty list or a blank title`() {
        assertNull(LrclibMatcher.selectBest(emptyList(), "Song", "Artist"))
        assertNull(LrclibMatcher.selectBest(listOf(track()), "   ", "Artist"))
    }

    @Test
    fun `candidates with unknown duration are still accepted`() {
        val unknownDuration = track(durationSec = null)

        assertSame(
            unknownDuration,
            LrclibMatcher.selectBest(listOf(unknownDuration), "Song", "Artist", durationMs = 200_000L),
        )
    }

    @Test
    fun `normalization removes diacritics and punctuation`() {
        assertEquals("bjork", LrclibMatcher.normalize("Björk"))
        assertEquals("song", LrclibMatcher.normalize("SonG!"))
    }
}
