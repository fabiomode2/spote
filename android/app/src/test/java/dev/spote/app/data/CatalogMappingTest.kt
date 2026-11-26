package dev.spote.app.data

import dev.spote.rust.CatalogAlbumDto
import dev.spote.rust.CatalogAlbumInfoDto
import dev.spote.rust.CatalogArtistProfileDto
import dev.spote.rust.CatalogTrackDto
import dev.spote.app.rust.toModel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CatalogMappingTest {

    private val track = CatalogTrackDto(
        key = "isrc:USUM72212345",
        provider = "deezer",
        providerId = "4419915281",
        title = "Regular",
        artists = "Bíberon",
        artistList = listOf("Bíberon"),
        album = "!Se cayó la bocina¡",
        durationMs = 139_000u,
        artUrl = "https://cdn-images.dzcdn.net/cover/1000x1000.jpg",
        isrc = "USUM72212345",
    )

    private val album = CatalogAlbumDto(
        provider = "deezer",
        id = "1149045271",
        title = "!Se cayó la bocina¡",
        artist = "Bíberon",
        coverUrl = "https://cdn-images.dzcdn.net/cover/1000x1000.jpg",
        releaseYear = 2024u,
        trackCount = 6u,
    )

    @Test
    fun `catalog track keeps canonical key and provider identity`() {
        val model = track.toModel()
        assertEquals("isrc:USUM72212345", model.key)
        assertEquals("deezer", model.provider)
        assertEquals("4419915281", model.providerId)
        assertEquals("Bíberon Regular", model.remoteQuery)
        assertEquals(139_000L, model.durationMs)
    }

    @Test
    fun `catalog album uses a stable route id`() {
        val model = album.toModel()
        assertEquals("catalog:deezer:1149045271", model.id)
        assertEquals(6, model.trackCount)
        assertEquals(2024, model.year)
        assertTrue(model.tracks.isEmpty())
    }

    @Test
    fun `album info keeps provider order and fallback cover`() {
        val info = CatalogAlbumInfoDto(
            provider = "deezer",
            id = "1149045271",
            title = "!Se cayó la bocina¡",
            artist = "Bíberon",
            coverUrl = null,
            releaseDate = "2024-03-01",
            year = 2024u,
            tracks = listOf(track.copy(artUrl = null)),
        )
        val model = info.toModel()
        assertEquals("catalog:deezer:1149045271", model.id)
        assertEquals(2024, model.year)
        assertEquals(1, model.tracks.size)
        assertEquals("Regular", model.tracks.first().title)
        assertEquals("deezer", model.tracks.first().albumProvider)
        assertEquals("1149045271", model.tracks.first().albumProviderId)
    }

    @Test
    fun `artist profile maps followers and discography`() {
        val profile = CatalogArtistProfileDto(
            provider = "deezer",
            id = "14033485",
            name = "Bíberon",
            imageUrl = "https://cdn-images.dzcdn.net/artist/1000x1000.jpg",
            genres = listOf("Trap"),
            followers = 12_345uL,
            popularity = 0u,
            topTracks = listOf(track),
            albums = listOf(album),
            bio = null,
        )
        val model = profile.toModel()
        assertEquals("catalog:deezer:14033485", model.artist.id)
        assertEquals(12_345L, model.artist.followers)
        assertEquals(1, model.topTracks.size)
        assertEquals("catalog:deezer:1149045271", model.albums.first().id)
    }
}
