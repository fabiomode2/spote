package dev.spote.app.model

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color

@Immutable
data class Cover(
    val start: Color,
    val end: Color,
    val label: String,
    val localPath: String? = null,
    val remoteUrl: String? = null,
)

@Immutable
data class Track(
    val key: String,
    val title: String,
    val artist: String,
    val album: String?,
    val durationMs: Long,
    val cover: Cover,
    val trackNumber: Int = 0,
    val downloaded: Boolean = false,
    val favorite: Boolean = false,
    val explicit: Boolean = false,
    val localPath: String? = null,
    val streamUrl: String? = null,
    /** MIME del stream resuelto (HLS necesita `application/vnd.apple.mpegurl`). */
    val streamMimeType: String? = null,
    /** Página web de origen (YouTube/SoundCloud) para resolverla al reproducir. */
    val remoteUrl: String? = null,
    /** Consulta de catálogo (artista + título) para resolver el audio bajo demanda. */
    val remoteQuery: String? = null,
    val sourceLabel: String? = null,
) {
    val isRemote: Boolean get() = remoteUrl != null || remoteQuery != null
}

@Immutable
data class Album(
    val id: String,
    val title: String,
    val artist: String,
    val year: Int,
    val cover: Cover,
    val tracks: List<Track>,
) {
    val durationMs: Long get() = tracks.sumOf { it.durationMs }
}

@Immutable
data class Playlist(
    val id: String,
    val name: String,
    val description: String,
    val cover: Cover,
    val trackCount: Int,
    val tracks: List<Track> = emptyList(),
)

@Immutable
data class Room(
    val id: String,
    val name: String,
    val members: List<String>,
    val live: Boolean,
    val nowPlaying: Track? = null,
    val memberAvatars: Map<String, String> = emptyMap(),
)

@Immutable
data class Genre(
    val name: String,
    val cover: Cover,
)

@Immutable
data class LyricLine(
    val startMs: Long,
    val text: String,
)

@Immutable
data class HomeState(
    val greeting: String,
    val chips: List<String>,
    val recent: List<Track>,
    val rooms: List<Room>,
    val playlists: List<Playlist>,
    val albums: List<Album>,
)

@Immutable
data class SearchState(
    val query: String = "",
    val recentQueries: List<String> = emptyList(),
    val genres: List<Genre> = emptyList(),
    val results: List<Track> = emptyList(),
    val albums: List<Album> = emptyList(),
    val artists: List<String> = emptyList(),
    /** Resultados del catálogo, separados de la biblioteca local. */
    val remoteResults: List<Track> = emptyList(),
    val remoteAlbums: List<Album> = emptyList(),
    val remoteArtists: List<String> = emptyList(),
    val remoteLoading: Boolean = false,
    val remoteError: String? = null,
)

@Immutable
data class ChatMessage(
    val author: String,
    val text: String,
)

enum class DownloadStatus { Resolving, Downloading, Importing, Failed }

@Immutable
data class DownloadUi(
    val key: String,
    val title: String,
    val progress: Float = 0f,
    val status: DownloadStatus = DownloadStatus.Resolving,
    val error: String? = null,
)

enum class LibraryFilter(val label: String) {
    All("Todo"),
    Playlists("Playlists"),
    Albums("Álbumes"),
    Artists("Artistas"),
    Downloaded("Descargado"),
    Favorites("Favoritas"),
}

@Immutable
data class LibraryState(
    val filter: LibraryFilter = LibraryFilter.All,
    val isGrid: Boolean = false,
    val playlists: List<Playlist> = emptyList(),
    val albums: List<Album> = emptyList(),
    val tracks: List<Track> = emptyList(),
    val downloadedCount: Int = 0,
    val storageLabel: String = "0 MB",
)

enum class RepeatMode { Off, All, One }

@Immutable
data class PlayerState(
    val track: Track? = null,
    val isPlaying: Boolean = false,
    val positionMs: Long = 0L,
    val durationMs: Long = 0L,
    val shuffle: Boolean = false,
    val repeat: RepeatMode = RepeatMode.Off,
    val queue: List<Track> = emptyList(),
    val lyrics: List<LyricLine> = emptyList(),
    val lyricsSynced: Boolean = false,
    val lyricsSource: String? = null,
    val lyricsLoading: Boolean = false,
)

enum class Tab { Home, Search, Now, Library }

@Immutable
data class AppUiState(
    val tab: Tab = Tab.Home,
    val home: HomeState,
    val search: SearchState,
    val library: LibraryState,
    val player: PlayerState,
    val downloads: Map<String, DownloadUi> = emptyMap(),
)
