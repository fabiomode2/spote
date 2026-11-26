package dev.spote.app.data

import dev.spote.app.model.Album
import dev.spote.app.model.HomeState
import dev.spote.app.model.LibraryState
import dev.spote.app.model.Playlist
import dev.spote.app.model.PlayerState
import dev.spote.app.model.RepeatMode
import dev.spote.app.model.SearchState
import dev.spote.app.model.Track

interface SpoteRepository {
    fun home(): HomeState
    fun search(query: String): SearchState
    fun library(): LibraryState
    fun album(id: String): Album?
    fun playlist(id: String): Playlist? = null
    fun playerFor(track: Track): PlayerState
    fun playlists(): List<Playlist> = emptyList()
    fun createPlaylist(name: String): String? = null
    fun deletePlaylist(id: String): Boolean = false
    fun addToPlaylist(id: String, key: String): Boolean = false
    fun removeFromPlaylist(id: String, key: String): Boolean = false
    fun duplicateCount(): Long = 0L
    fun mergeDuplicates(): Int = 0
    fun backupLibrary(path: String): Boolean = false
    fun rememberTrack(track: Track): Boolean = false
    fun removeFromLibrary(key: String): Boolean = false
}

/** Fallback sin datos: si el core no carga, no inventamos una biblioteca. */
class EmptySpoteRepository : SpoteRepository {
    override fun home(): HomeState = HomeState(
        greeting = "Hola",
        chips = listOf("Todo", "Descargas", "Salas"),
        recent = emptyList(),
        rooms = emptyList(),
        playlists = emptyList(),
        albums = emptyList(),
    )

    override fun search(query: String): SearchState = SearchState(query = query)

    override fun library(): LibraryState = LibraryState(storageLabel = "Local")

    override fun album(id: String): Album? = null

    override fun playerFor(track: Track): PlayerState = PlayerState(
        track = track,
        durationMs = track.durationMs,
        queue = listOf(track),
    )
}

class MockSpoteRepository : SpoteRepository {
    override fun home(): HomeState = MockData.home
    override fun search(query: String): SearchState = MockData.search(query)
    override fun library(): LibraryState = MockData.library
    override fun album(id: String): Album? = MockData.album(id)
    override fun playlist(id: String): Playlist? = MockData.playlists.firstOrNull { it.id == id }

    override fun playerFor(track: Track): PlayerState {
        val rest = MockData.queue.filterNot { it.key == track.key }
        return PlayerState(
            track = track,
            isPlaying = false,
            positionMs = 0L,
            durationMs = track.durationMs,
            queue = listOf(track) + rest,
            lyrics = if (track.key == "t-borderline") MockData.lyrics else emptyList(),
        )
    }
}
