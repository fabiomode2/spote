package dev.spote.app.rust

import android.content.Context
import androidx.compose.ui.graphics.Color
import dev.spote.app.model.Album
import dev.spote.app.model.Artist
import dev.spote.app.model.ArtistProfile
import dev.spote.app.model.Cover
import dev.spote.app.model.CoverPalette
import dev.spote.app.model.Playlist
import dev.spote.app.model.Track
import dev.spote.rust.AlbumDto
import dev.spote.rust.CatalogAlbumDto
import dev.spote.rust.CatalogAlbumInfoDto
import dev.spote.rust.CatalogArtistDto
import dev.spote.rust.CatalogArtistProfileDto
import dev.spote.rust.CatalogSearchDto
import dev.spote.rust.CatalogTrackDto
import dev.spote.rust.LibraryStatsDto
import dev.spote.rust.PaletteDto
import dev.spote.rust.PlaylistDto
import dev.spote.rust.SpoteEngine
import dev.spote.rust.TrackDto
import java.io.File

/**
 * Envoltorio pequeño sobre las bindings de UniFFI. Traduce DTOs de Rust a los
 * modelos de Compose; la UI nunca importa el paquete generado.
 */
class RustBridge private constructor(private val engine: SpoteEngine) {

    fun version(): String = engine.version()

    fun home(): String = engine.home()

    fun stats(): LibraryStatsDto = engine.stats()

    fun recent(limit: Int = 50): List<Track> =
        engine.recent(limit.toUInt()).map { it.toModel() }

    fun allTracks(limit: Int = 200): List<Track> =
        engine.allTracks(limit.toUInt()).map { it.toModel() }

    fun search(query: String, limit: Int = 50): List<Track> =
        engine.search(query, limit.toUInt()).map { it.toModel() }

    fun searchCatalog(query: String, limit: Int = 40): CatalogSearchDto =
        engine.searchCatalog(query, limit.toUInt())

    fun albumInfo(provider: String, id: String): CatalogAlbumInfoDto =
        engine.albumInfo(provider, id)

    fun artistProfile(name: String): CatalogArtistProfileDto =
        engine.artistProfile(name)

    fun artistProfileById(provider: String, id: String): CatalogArtistProfileDto =
        engine.artistProfileById(provider, id)

    fun findAlbum(album: String, artist: String): CatalogAlbumInfoDto? =
        engine.findAlbum(album, artist)

    fun albumTracks(album: String, artist: String): List<Track> =
        engine.albumTracks(album, artist).map { it.toModel() }

    fun findTrackByMeta(artist: String, title: String, durationMs: Long): Track? =
        engine.findTrackByMeta(artist, title, durationMs.toULong())?.toModel()

    fun albums(): List<AlbumDto> = engine.albums()

    fun playlists(): List<PlaylistDto> = engine.playlists()

    fun playlistTracks(id: Long): List<Track> =
        engine.playlistTracks(id).map { it.toModel() }

    fun toggleFavorite(key: String): Boolean = engine.toggleFavorite(key)

    fun tracksWithoutArt(limit: Int): List<Track> =
        engine.tracksWithoutArt(limit.toUInt()).map { it.toModel() }

    fun setTrackArt(key: String, artPath: String?, artUrl: String?) =
        engine.setTrackArt(key, artPath, artUrl)

    fun updateTrackMetadata(
        key: String,
        artist: String,
        title: String,
        album: String?,
        durationMs: Long,
        artUrl: String?,
    ) = engine.updateTrackMetadata(
        key,
        artist,
        title,
        album,
        durationMs.toULong(),
        artUrl,
    )

    fun duplicateCount(): Long = engine.duplicateCount()

    fun mergeDuplicates(): Int = engine.mergeDuplicates().toInt()

    fun backupLibrary(path: String) = engine.backupLibrary(path)

    fun createPlaylist(name: String): Long = engine.createPlaylist(name)

    fun deletePlaylist(id: Long) = engine.deletePlaylist(id)

    fun addToPlaylist(id: Long, key: String) = engine.addToPlaylist(id, key)

    fun removeFromPlaylist(id: Long, key: String) = engine.removeFromPlaylist(id, key)

    fun importLocal(
        path: String,
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
        artPath: String?,
    ): String = engine.importLocal(path, title, artist, album, durationMs.toULong(), artPath)

    fun importLocalWithKey(
        key: String,
        path: String,
        title: String,
        artist: String,
        album: String?,
        durationMs: Long,
        artPath: String?,
    ): String = engine.importLocalWithKey(
        key,
        path,
        title,
        artist,
        album,
        durationMs.toULong(),
        artPath,
    )

    fun seedDemo(title: String, artist: String, album: String? = null, durationMs: Long = 0L) {
        engine.upsertDemo(title, artist, album, durationMs.toULong())
    }

    fun palette(imagePath: String): Cover? = runCatching {
        val palette = engine.palette(imagePath)
        Cover(
            start = Color(android.graphics.Color.parseColor(palette.primary)),
            end = Color(android.graphics.Color.parseColor(palette.surface)),
            label = "",
            localPath = imagePath,
        )
    }.getOrNull()

    fun paletteTheme(imagePath: String): CoverPalette? = runCatching {
        engine.palette(imagePath).toModel()
    }.getOrNull()

    companion object {
        fun create(context: Context): Result<RustBridge> = runCatching {
            val home = File(context.filesDir, "spote")
            RustBridge(SpoteEngine(home.absolutePath))
        }
    }
}

fun TrackDto.toModel(): Track = Track(
    key = key,
    title = title,
    artist = artist,
    album = album,
    durationMs = durationMs.toLong(),
    cover = coverFor(title, artPath, artUrl),
    downloaded = downloaded,
    favorite = favorite,
    localPath = path,
    sourceLabel = provider,
)

fun PlaylistDto.toModel(tracks: List<Track>): Playlist = Playlist(
    id = "rpl:$id",
    name = name,
    description = description,
    cover = coverFor(name, null, coverUrl),
    trackCount = trackCount.toInt(),
    tracks = tracks,
)

fun AlbumDto.toModel(tracks: List<Track>): Album = Album(
    id = "ralbum:${artist.lowercase()}:${album.lowercase()}",
    title = album,
    artist = artist,
    year = 0,
    cover = coverFor(album, artPath, artUrl),
    tracks = tracks,
    provider = "local",
    trackCount = trackCount.toInt().coerceAtLeast(tracks.size),
)

fun CatalogTrackDto.toModel(
    albumProvider: String? = null,
    albumProviderId: String? = null,
    albumFallback: String? = this.album,
    coverFallback: String? = null,
): Track = Track(
    key = key,
    title = title,
    artist = artists,
    album = albumFallback,
    durationMs = durationMs.toLong(),
    cover = coverFor(title, null, artUrl ?: coverFallback),
    provider = provider,
    providerId = providerId,
    albumProvider = albumProvider ?: provider,
    albumProviderId = albumProviderId,
    isrc = isrc,
    sourceLabel = provider.replaceFirstChar { it.uppercase() },
    remoteQuery = listOf(artists, title).filter { it.isNotBlank() }.joinToString(" "),
)

fun CatalogAlbumDto.toModel(): Album = Album(
    id = "catalog:$provider:$id",
    title = title,
    artist = artist,
    year = releaseYear?.toInt() ?: 0,
    cover = coverFor(title, null, coverUrl),
    tracks = emptyList(),
    provider = provider,
    providerId = id,
    trackCount = trackCount.toInt(),
)

fun CatalogAlbumInfoDto.toModel(): Album = Album(
    id = "catalog:$provider:$id",
    title = title,
    artist = artist,
    year = year?.toInt()
        ?: releaseDate?.trim()?.take(4)?.toIntOrNull()
        ?: 0,
    cover = coverFor(title, null, coverUrl),
    tracks = tracks.map { track ->
        track.toModel(
            albumProvider = provider,
            albumProviderId = id,
            albumFallback = title,
            coverFallback = coverUrl,
        )
    },
    provider = provider,
    providerId = id,
    trackCount = tracks.size,
    releaseDate = releaseDate,
)

fun CatalogArtistDto.toModel(): Artist = Artist(
    id = "catalog:$provider:$id",
    name = name,
    imageUrl = imageUrl,
    followers = followers.toLong(),
    provider = provider,
    providerId = id,
)

fun CatalogArtistProfileDto.toModel(): ArtistProfile = ArtistProfile(
    artist = Artist(
        id = "catalog:$provider:$id",
        name = name,
        imageUrl = imageUrl,
        followers = followers.toLong(),
        provider = provider,
        providerId = id,
    ),
    genres = genres,
    popularity = popularity.toInt(),
    bio = bio,
    topTracks = topTracks.map { track ->
        track.toModel(albumProvider = provider, albumFallback = track.album)
    },
    albums = albums.map { it.toModel() },
)

fun PaletteDto.toModel(): CoverPalette = CoverPalette(
    background = color(background, Color(0xFF070708)),
    surface = color(surface, Color(0xFF121214)),
    surfaceHigh = color(surfaceAlt, Color(0xFF1C1C21)),
    deep = color(bgDeep, Color(0xFF050506)),
    text = color(text, Color(0xFFF4F4F6)),
    muted = color(muted, Color(0xFF9B9BA5)),
    outline = color(border, Color(0xFF2A2A31)),
    primary = color(primary, Color(0xFFC6F24E)),
    secondary = color(secondary, Color(0xFF63D68B)),
    accent = color(accent, Color(0xFFC6F24E)),
    success = color(ok, Color(0xFF63D68B)),
    danger = color(error, Color(0xFFFF6B6B)),
)

private fun color(hex: String, fallback: Color): Color =
    runCatching { Color(android.graphics.Color.parseColor(hex)) }.getOrDefault(fallback)

/**
 * Portada de reserva determinista: mismo título → mismo degradado. Si hay
 * arte local, `CoverArt` lo carga y el degradado solo hace de placeholder.
 */
fun coverFor(label: String, localPath: String?, remoteUrl: String? = null): Cover {
    val seed = (label.hashCode() * 31) xor 0x5f3759df
    val hue = (seed and 0x7fffffff) % 360
    val start = hsl(hue.toFloat(), 0.55f, 0.52f)
    val end = hsl(((hue + 38) % 360).toFloat(), 0.48f, 0.16f)
    val initials = label
        .split(' ', '-', '_')
        .filter { it.isNotBlank() }
        .take(2)
        .joinToString("") { it.take(1).uppercase() }
        .ifBlank { label.take(2).uppercase() }
    return Cover(start = start, end = end, label = initials, localPath = localPath, remoteUrl = remoteUrl)
}

private fun hsl(h: Float, s: Float, l: Float): Color {
    val c = (1f - kotlin.math.abs(2f * l - 1f)) * s
    val x = c * (1f - kotlin.math.abs((h / 60f) % 2f - 1f))
    val m = l - c / 2f
    val (r, g, b) = when {
        h < 60f -> Triple(c, x, 0f)
        h < 120f -> Triple(x, c, 0f)
        h < 180f -> Triple(0f, c, x)
        h < 240f -> Triple(0f, x, c)
        h < 300f -> Triple(x, 0f, c)
        else -> Triple(c, 0f, x)
    }
    return Color(r + m, g + m, b + m)
}
