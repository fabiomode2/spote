package dev.spote.app.data

import dev.spote.app.model.Album
import dev.spote.app.model.HomeState
import dev.spote.app.model.LibraryState
import dev.spote.app.model.PlayerState
import dev.spote.app.model.Playlist
import dev.spote.app.model.SearchState
import dev.spote.app.model.Track
import dev.spote.app.rust.RustBridge
import dev.spote.app.rust.toModel

/**
 * Adaptador entre la librería SQLite del core y el estado que pinta Compose.
 * Todo es local y síncrono: SQLite con FTS5 responde en microsegundos. Las
 * llamadas de red (catálogo/letras/salas) viven fuera de este repositorio.
 */
class RustSpoteRepository(private val bridge: RustBridge) : SpoteRepository {

    override fun home(): HomeState {
        val all = bridge.allTracks(200)
        val recent = bridge.recent(12)
        val albums = bridge.albums().map { it.toModel(emptyList()) }
        val playlists = loadPlaylists()
        return HomeState(
            greeting = greeting(),
            chips = listOf("Todo", "Descargas", "Salas", "Nuevo"),
            recent = if (recent.isNotEmpty()) recent else all.take(12),
            rooms = emptyList(),
            playlists = playlists,
            albums = albums.take(6),
        )
    }

    override fun search(query: String): SearchState {
        val base = SearchState(query = query)
        if (query.isBlank()) return base
        val tracks = bridge.search(query, 60)
        return base.copy(
            results = tracks,
            albums = buildAlbums(tracks).take(8),
            artists = tracks.map { it.artist }.distinct(),
        )
    }

    override fun library(): LibraryState {
        val tracks = bridge.allTracks(500)
        val albums = bridge.albums().map { it.toModel(emptyList()) }
        return LibraryState(
            playlists = loadPlaylists(),
            albums = albums,
            tracks = tracks,
            downloadedCount = tracks.count { it.downloaded },
            storageLabel = "Local",
        )
    }

    override fun album(id: String): Album? {
        val dto = bridge.albums().firstOrNull { it.toModel(emptyList<Track>()).id == id } ?: return null
        val tracks = bridge.albumTracks(dto.album, dto.artist)
        return dto.toModel(tracks)
    }

    override fun playerFor(track: Track): PlayerState {
        val all = bridge.allTracks(500)
        val start = all.indexOfFirst { it.key == track.key }
        val queue = if (start >= 0) {
            all.drop(start) + all.take(start)
        } else {
            listOf(track) + all.filterNot { it.key == track.key }.take(20)
        }
        return PlayerState(
            track = track,
            isPlaying = false,
            positionMs = 0L,
            durationMs = track.durationMs,
            queue = queue,
        )
    }

    override fun playlist(id: String): Playlist? {
        val numeric = id.removePrefix("rpl:").toLongOrNull() ?: return null
        return loadPlaylists().firstOrNull { it.id == id }
            ?: bridge.playlists()
                .firstOrNull { it.id == numeric }
                ?.let { it.toModel(bridge.playlistTracks(it.id)) }
    }

    override fun playlists(): List<Playlist> = loadPlaylists()

    override fun createPlaylist(name: String): String? = runCatching {
        "rpl:${bridge.createPlaylist(name.trim())}"
    }.getOrNull()

    override fun deletePlaylist(id: String): Boolean {
        val numeric = id.removePrefix("rpl:").toLongOrNull() ?: return false
        return runCatching { bridge.deletePlaylist(numeric) }.isSuccess
    }

    override fun addToPlaylist(id: String, key: String): Boolean {
        val numeric = id.removePrefix("rpl:").toLongOrNull() ?: return false
        return runCatching { bridge.addToPlaylist(numeric, key) }.isSuccess
    }

    override fun removeFromPlaylist(id: String, key: String): Boolean {
        val numeric = id.removePrefix("rpl:").toLongOrNull() ?: return false
        return runCatching { bridge.removeFromPlaylist(numeric, key) }.isSuccess
    }

    override fun duplicateCount(): Long = runCatching { bridge.duplicateCount() }.getOrDefault(0L)

    override fun mergeDuplicates(): Int = runCatching { bridge.mergeDuplicates() }.getOrDefault(0)

    override fun backupLibrary(path: String): Boolean =
        runCatching { bridge.backupLibrary(path) }.isSuccess

    override fun rememberTrack(track: Track): Boolean = runCatching {
        val provider = track.provider ?: "deezer"
        val providerId = track.providerId ?: return false
        bridge.rememberTrack(
            key = track.key,
            title = track.title,
            artist = track.artist,
            album = track.album,
            durationMs = track.durationMs,
            artUrl = track.cover.remoteUrl,
            provider = provider,
            providerId = providerId,
        )
    }.isSuccess

    override fun removeFromLibrary(key: String): Boolean =
        runCatching { bridge.removeFromLibrary(key) }.isSuccess

    private fun loadPlaylists(): List<Playlist> =
        bridge.playlists().map { dto ->
            dto.toModel(bridge.playlistTracks(dto.id))
        }

    private fun buildAlbums(tracks: List<Track>): List<Album> =
        tracks
            .groupBy { (it.album ?: "Sin álbum") to it.artist }
            .map { (pair, rows) ->
                val (albumName, artist) = pair
                val cover = rows.first().cover
                Album(
                    id = "ralbum:${slug(artist)}:${slug(albumName)}",
                    title = albumName,
                    artist = artist,
                    year = 0,
                    cover = cover,
                    tracks = rows.sortedBy { it.title.lowercase() },
                )
            }
            .sortedBy { it.title.lowercase() }

    private fun slug(value: String): String = value
        .lowercase()
        .replace(Regex("[^a-z0-9]+"), "-")
        .trim('-')
        .take(48)
        .ifBlank { "x" }

    private fun greeting(): String = when (java.time.LocalTime.now().hour) {
        in 6..11 -> "Buenos días"
        in 12..19 -> "Buenas tardes"
        else -> "Buenas noches"
    }
}
