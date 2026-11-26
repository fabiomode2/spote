package dev.spote.app.rust

import android.content.Context
import androidx.compose.ui.graphics.Color
import dev.spote.app.model.Album
import dev.spote.app.model.Cover
import dev.spote.app.model.Playlist
import dev.spote.app.model.Track
import dev.spote.rust.AlbumDto
import dev.spote.rust.LibraryStatsDto
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
)

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
