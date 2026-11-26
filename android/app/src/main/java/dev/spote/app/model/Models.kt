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

/** Paleta completa extraída por el core Rust a partir de una portada. */
@Immutable
data class CoverPalette(
    val background: Color,
    val surface: Color,
    val surfaceHigh: Color,
    val deep: Color,
    val text: Color,
    val muted: Color,
    val outline: Color,
    val primary: Color,
    val secondary: Color,
    val accent: Color,
    val success: Color,
    val danger: Color,
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
    /** `spotify` / `deezer` cuando la pista viene del catálogo. */
    val provider: String? = null,
    val providerId: String? = null,
    val albumProvider: String? = null,
    val albumProviderId: String? = null,
    val artistProvider: String? = null,
    val artistProviderId: String? = null,
    val isrc: String? = null,
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
    /** `local` o el proveedor del catálogo (`spotify`/`deezer`). */
    val provider: String? = null,
    val providerId: String? = null,
    val artistProvider: String? = null,
    val artistProviderId: String? = null,
    val trackCount: Int = tracks.size,
    val releaseDate: String? = null,
) {
    val durationMs: Long get() = tracks.sumOf { it.durationMs }
    val isCatalog: Boolean get() = provider != null && provider != "local"
}

@Immutable
data class Artist(
    val id: String,
    val name: String,
    val imageUrl: String? = null,
    val followers: Long = 0,
    val provider: String? = null,
    val providerId: String? = null,
)

@Immutable
data class ArtistProfile(
    val artist: Artist,
    val genres: List<String> = emptyList(),
    val popularity: Int = 0,
    val bio: String? = null,
    val topTracks: List<Track> = emptyList(),
    val albums: List<Album> = emptyList(),
)

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
    val remoteArtists: List<Artist> = emptyList(),
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

enum class Tab { Home, Search, Library }

@Immutable
data class AppUiState(
    val home: HomeState,
    val search: SearchState,
    val library: LibraryState,
    val player: PlayerState,
    val downloads: Map<String, DownloadUi> = emptyMap(),
    /** Álbumes de catálogo cargados bajo demanda, por id de ruta. */
    val albumDetails: Map<String, Album> = emptyMap(),
    val albumLoading: Set<String> = emptySet(),
    val albumErrors: Map<String, String> = emptyMap(),
    /** Perfiles de artista cargados bajo demanda. */
    val artistDetails: Map<String, ArtistProfile> = emptyMap(),
    val artistLoading: Set<String> = emptySet(),
    val artistErrors: Map<String, String> = emptyMap(),
)
