package dev.spote.app.data

import dev.spote.app.model.Album
import dev.spote.app.model.Artist
import dev.spote.app.model.ArtistProfile
import dev.spote.app.model.Track
import dev.spote.app.rust.coverFor
import java.io.IOException
import java.security.MessageDigest
import java.text.Normalizer
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlin.math.max
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

data class CatalogSnapshot(
    val tracks: List<Track> = emptyList(),
    val albums: List<Album> = emptyList(),
    val artists: List<Artist> = emptyList(),
)

/**
 * Catálogo Deezer en Kotlin/OkHttp. La app usa este cliente para buscar y
 * abrir fichas; el core Rust sigue siendo la fuente de la biblioteca local y
 * de la paleta. Así ninguna búsqueda pasa por TLS nativo, que era lo que
 * tumbaba el proceso en Android.
 */
object CatalogClient {

    private const val API = "https://api.deezer.com"
    private const val USER_AGENT = "spote-android/0.5 (catalog)"
    private val FEATURED_IN_TITLE = Regex(
        "(?i)[\\(\\[]\\s*(?:feat\\.?|ft\\.?|with|con)\\s+([^\\)\\]]+)",
    )
    private val FEATURED_SUFFIX = Regex(
        "(?i)\\s+(?:feat\\.?|ft\\.?|with|con)\\s+(.+)$",
    )

    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .callTimeout(20, TimeUnit.SECONDS)
            .build()
    }

    private val albumCache = ConcurrentHashMap<String, Album>()
    private val artistCache = ConcurrentHashMap<String, ArtistProfile>()

    suspend fun search(query: String, limit: Int = 20): CatalogSnapshot = withContext(Dispatchers.IO) {
        val trimmed = query.trim()
        if (trimmed.isEmpty()) return@withContext CatalogSnapshot()
        kotlinx.coroutines.coroutineScope {
            val tracks = async { searchTracks(trimmed, limit.coerceIn(1, 25)) }
            val albums = async {
                runCatching { searchAlbums(trimmed, 10) }.getOrDefault(emptyList())
            }
            val artists = async {
                runCatching { searchArtists(trimmed, 10) }.getOrDefault(emptyList())
            }
            buildSnapshot(tracks.await(), albums.await(), artists.await())
        }
    }

    suspend fun album(provider: String, id: String): Album = withContext(Dispatchers.IO) {
        require(provider == "deezer" || provider.isBlank()) { "Proveedor no soportado: $provider" }
        albumCache["deezer:$id"]?.let { return@withContext it }
        val root = get("/album/$id")
        val album = mapAlbumInfo(root)
        albumCache["deezer:$id"] = album
        album
    }

    suspend fun findAlbum(title: String, artist: String): Album? = withContext(Dispatchers.IO) {
        val wantedTitle = norm(title)
        val wantedArtist = norm(artist)
        val albums = runCatching { searchAlbums("$artist $title", 8) }.getOrDefault(emptyList())
        val candidate = albums
            .filter {
                val got = norm(it.title)
                got == wantedTitle || got.contains(wantedTitle) || wantedTitle.contains(got)
            }
            .maxByOrNull {
                val gotArtist = norm(it.artist)
                val artistMatch = gotArtist == wantedArtist ||
                    gotArtist.contains(wantedArtist) ||
                    wantedArtist.contains(gotArtist)
                (if (artistMatch) 1 else 0) * 1000 + it.trackCount
            }
        if (candidate != null && candidate.providerId != null) {
            return@withContext runCatching { album("deezer", candidate.providerId) }
                .getOrElse { candidate }
        }

        // Si el álbum no sale en /search/album, se busca por sus pistas.
        val tracks = runCatching { searchTracks("$artist $title", 10) }.getOrDefault(emptyList())
        val trackAlbum = tracks.firstOrNull { it.albumProviderId != null && norm(it.album.orEmpty()) == wantedTitle }
            ?: tracks.firstOrNull { it.albumProviderId != null }
        trackAlbum?.albumProviderId?.let { albumId ->
            return@withContext runCatching { album("deezer", albumId) }.getOrNull()
        }
        candidate
    }

    suspend fun artist(name: String): ArtistProfile = withContext(Dispatchers.IO) {
        val wanted = norm(name)
        val results = searchArtists(name, 5)
        val candidate = results.firstOrNull { norm(it.name) == wanted }
            ?: results.firstOrNull { norm(it.name).contains(wanted) || wanted.contains(norm(it.name)) }
            ?: throw IOException("sin artista para «$name»")
        val id = candidate.providerId ?: throw IOException("artista sin id")
        artistById("deezer", id)
    }

    /**
     * Créditos completos de una pista (Deezer los da en `/track/{id}`).
     * La búsqueda solo devuelve el artista principal; esto añade invitados.
     */
    suspend fun trackCredits(provider: String, id: String): List<String> = withContext(Dispatchers.IO) {
        if (provider.isNotBlank() && provider != "deezer") return@withContext emptyList()
        val root = get("/track/$id")
        val primary = root.optJSONObject("artist")?.optString("name").orEmpty().trim()
        if (primary.isEmpty()) return@withContext emptyList()
        creditsOf(root, primary, root.optString("title"))
    }

    suspend fun artistById(provider: String, id: String): ArtistProfile = withContext(Dispatchers.IO) {        require(provider == "deezer" || provider.isBlank()) { "Proveedor no soportado: $provider" }
        artistCache["deezer:$id"]?.let { return@withContext it }
        val root = get("/artist/$id")
        val artist = mapArtist(root) ?: throw IOException("artista vacío")
        val top = runCatching {
            get("/artist/$id/top", mapOf("limit" to "10"))
                .optJSONArray("data")
                .toTracks()
        }.getOrDefault(emptyList())
        val albums = runCatching {
            get("/artist/$id/albums", mapOf("limit" to "20"))
                .optJSONArray("data")
                .toAlbums(artist.name)
        }.getOrDefault(emptyList())
        val profile = ArtistProfile(
            artist = artist,
            genres = emptyList(),
            popularity = 0,
            bio = null,
            topTracks = top,
            albums = albums,
        )
        artistCache["deezer:$id"] = profile
        profile
    }

    private fun searchTracks(query: String, limit: Int): List<Track> {
        val data = get("/search", mapOf("q" to query, "limit" to limit.toString()))
            .optJSONArray("data")
            ?: return emptyList()
        return buildList {
            for (index in 0 until data.length()) {
                data.optJSONObject(index)?.let { item -> mapTrack(item)?.let(::add) }
            }
        }.distinctBy { it.key }
    }

    private fun searchAlbums(query: String, limit: Int): List<Album> {
        val data = get("/search/album", mapOf("q" to query, "limit" to limit.toString()))
            .optJSONArray("data")
            ?: return emptyList()
        return buildList {
            for (index in 0 until data.length()) {
                data.optJSONObject(index)?.let { item -> mapAlbumSummary(item)?.let(::add) }
            }
        }.distinctBy { it.id }
    }

    private fun searchArtists(query: String, limit: Int): List<Artist> {
        val data = get("/search/artist", mapOf("q" to query, "limit" to limit.toString()))
            .optJSONArray("data")
            ?: return emptyList()
        return buildList {
            for (index in 0 until data.length()) {
                data.optJSONObject(index)?.let { item -> mapArtist(item)?.let(::add) }
            }
        }.distinctBy { it.id }
    }

    /**
     * Completa la búsqueda con los álbumes y artistas que aparecen en las
     * pistas aunque los endpoints de búsqueda no los devuelvan.
     */
    private fun buildSnapshot(
        tracks: List<Track>,
        searchedAlbums: List<Album>,
        searchedArtists: List<Artist>,
    ): CatalogSnapshot {
        val extraAlbums = mutableListOf<Album>()
        val extraArtists = mutableListOf<Artist>()
        tracks.forEach { track ->
            val primaryArtist = track.artists.firstOrNull() ?: track.artist
            val albumId = track.albumProviderId
            if (!albumId.isNullOrBlank() && track.album?.isNotBlank() == true) {
                if (extraAlbums.none { it.providerId == albumId } &&
                    searchedAlbums.none { it.providerId == albumId }
                ) {
                    extraAlbums += Album(
                        id = "catalog:deezer:$albumId",
                        title = track.album.orEmpty(),
                        artist = primaryArtist,
                        year = 0,
                        cover = track.cover,
                        tracks = emptyList(),
                        provider = "deezer",
                        providerId = albumId,
                        trackCount = 0,
                    )
                }
            }
            val names = track.artists.ifEmpty { listOf(track.artist) }
            names.forEachIndexed { index, name ->
                val artistId = if (index == 0) track.artistProviderId else null
                val already = if (!artistId.isNullOrBlank()) {
                    extraArtists.any { it.providerId == artistId } ||
                        searchedArtists.any { it.providerId == artistId }
                } else {
                    extraArtists.any { norm(it.name) == norm(name) } ||
                        searchedArtists.any { norm(it.name) == norm(name) }
                }
                if (name.isNotBlank() && !already) {
                    extraArtists += Artist(
                        id = if (!artistId.isNullOrBlank()) {
                            "catalog:deezer:$artistId"
                        } else {
                            "name:${norm(name)}"
                        },
                        name = name,
                        imageUrl = null,
                        provider = if (!artistId.isNullOrBlank()) "deezer" else null,
                        providerId = artistId,
                    )
                }
            }
        }
        return CatalogSnapshot(
            tracks = tracks,
            albums = (extraAlbums + searchedAlbums).distinctBy { it.id },
            artists = (extraArtists + searchedArtists).distinctBy { it.id },
        )
    }

    private fun mapTrack(item: JSONObject): Track? {
        val id = item.optLong("id", 0L)
        val title = item.optString("title").trim()
        if (id <= 0L || title.isEmpty()) return null
        val artistObj = item.optJSONObject("artist")
        val primaryArtist = artistObj?.optString("name").orEmpty().trim()
        if (primaryArtist.isEmpty()) return null
        val artistId = artistObj?.optLong("id", 0L)?.takeIf { it > 0L }
        val albumObj = item.optJSONObject("album")
        val album = albumObj?.optString("title")?.trim()?.takeIf { it.isNotEmpty() }
        val albumId = albumObj?.optLong("id", 0L)?.takeIf { it > 0L }
        val cover = albumObj?.let(::coverOf)
        val durationMs = item.optLong("duration", 0L) * 1000L
        val isrc = item.optString("isrc").trim().takeIf { it.isNotEmpty() }
        val credits = creditsOf(item, primaryArtist, title)
        return Track(
            key = isrc?.let { "isrc:${it.lowercase(Locale.ROOT)}" }
                ?: fallbackKey(primaryArtist, title, durationMs),
            title = title,
            artist = credits.joinToString(", "),
            artists = credits,
            album = album,
            durationMs = durationMs,
            cover = coverFor(title, null, cover),
            explicit = item.optBoolean("explicit_lyrics", false),
            provider = "deezer",
            providerId = id.toString(),
            albumProvider = "deezer",
            albumProviderId = albumId?.toString(),
            artistProvider = "deezer",
            artistProviderId = artistId?.toString(),
            isrc = isrc,
            sourceLabel = "Deezer",
            remoteQuery = "$primaryArtist $title",
        )
    }

    /** Artistas principales, invitados y colaboraciones del título. */
    private fun creditsOf(item: JSONObject, primary: String, title: String): List<String> {
        val credits = mutableListOf(primary)
        item.optJSONArray("contributors")?.let { array ->
            for (index in 0 until array.length()) {
                array.optJSONObject(index)?.optString("name")?.trim()
                    ?.takeIf { it.isNotEmpty() }
                    ?.let(credits::add)
            }
        }
        FEATURED_IN_TITLE.findAll(title).forEach { match ->
            match.groupValues.getOrNull(1)?.trim()?.takeIf { it.isNotEmpty() }
                ?.let(credits::add)
        }
        FEATURED_SUFFIX.find(title)?.groupValues?.getOrNull(1)?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?.let(credits::add)
        return credits.distinctBy(::norm)
    }

    private fun mapAlbumSummary(item: JSONObject, fallbackArtist: String? = null): Album? {
        val id = item.optLong("id", 0L)
        val title = item.optString("title").trim()
        if (id <= 0L || title.isEmpty()) return null
        val artist = item.optJSONObject("artist")?.optString("name").orEmpty().trim()
            .ifBlank { fallbackArtist.orEmpty().trim() }
        if (artist.isEmpty()) return null
        return Album(
            id = "catalog:deezer:$id",
            title = title,
            artist = artist,
            year = yearOf(item.optString("release_date")),
            cover = coverFor(title, null, coverOf(item)),
            tracks = emptyList(),
            provider = "deezer",
            providerId = id.toString(),
            trackCount = item.optInt("nb_tracks", 0),
            releaseDate = item.optString("release_date").takeIf { it.isNotBlank() },
        )
    }

    private fun mapAlbumInfo(root: JSONObject): Album {
        val id = root.optLong("id", 0L)
        val title = root.optString("title").trim()
        val artist = root.optJSONObject("artist")?.optString("name").orEmpty().trim()
        val tracks = root.optJSONObject("tracks")
            ?.optJSONArray("data")
            ?.let { data ->
                buildList {
                    for (index in 0 until data.length()) {
                        data.optJSONObject(index)?.let { item ->
                            val track = mapTrack(item) ?: return@let
                            track.copy(
                                album = title,
                                albumProvider = "deezer",
                                albumProviderId = id.toString(),
                                cover = coverFor(track.title, null, coverOf(root)),
                            ).let(::add)
                        }
                    }
                }
            }
            .orEmpty()
        return Album(
            id = "catalog:deezer:$id",
            title = title,
            artist = artist,
            year = yearOf(root.optString("release_date")),
            cover = coverFor(title, null, coverOf(root)),
            tracks = tracks,
            provider = "deezer",
            providerId = id.toString(),
            trackCount = tracks.size,
            releaseDate = root.optString("release_date").takeIf { it.isNotBlank() },
        )
    }

    private fun mapArtist(item: JSONObject): Artist? {
        val id = item.optLong("id", 0L)
        val name = item.optString("name").trim()
        if (id <= 0L || name.isEmpty()) return null
        return Artist(
            id = "catalog:deezer:$id",
            name = name,
            imageUrl = pictureOf(item),
            followers = item.optLong("nb_fan", 0L),
            provider = "deezer",
            providerId = id.toString(),
        )
    }

    private fun coverOf(item: JSONObject): String? =
        listOf("cover_xl", "cover_big", "cover_medium", "cover")
            .firstNotNullOfOrNull { key ->
                item.optString(key).takeIf { it.isNotBlank() }
            }

    private fun pictureOf(item: JSONObject): String? =
        listOf("picture_xl", "picture_big", "picture_medium", "picture")
            .firstNotNullOfOrNull { key ->
                item.optString(key).takeIf { it.isNotBlank() }
            }

    private fun yearOf(date: String): Int =
        date.trim().take(4).toIntOrNull() ?: 0

    private fun get(path: String, params: Map<String, String> = emptyMap()): JSONObject {
        val url = (API + path).toHttpUrl().newBuilder().apply {
            params.forEach { (key, value) -> addQueryParameter(key, value) }
        }.build()
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .get()
            .build()
        val body = client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw IOException("Deezer HTTP ${response.code}")
            response.body?.string().orEmpty()
        }
        val root = JSONObject(body)
        if (root.has("error")) {
            throw IOException(root.optJSONObject("error")?.optString("message") ?: "Deezer error")
        }
        return root
    }

    private fun org.json.JSONArray?.toTracks(): List<Track> {
        if (this == null) return emptyList()
        return buildList {
            for (index in 0 until length()) {
                optJSONObject(index)?.let { mapTrack(it)?.let(::add) }
            }
        }.distinctBy { it.key }
    }

    private fun org.json.JSONArray?.toAlbums(fallbackArtist: String? = null): List<Album> {
        if (this == null) return emptyList()
        return buildList {
            for (index in 0 until length()) {
                optJSONObject(index)?.let { mapAlbumSummary(it, fallbackArtist)?.let(::add) }
            }
        }.distinctBy { it.id }
    }

    private fun norm(value: String): String = Normalizer.normalize(value, Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(Locale.ROOT)
        .replace(Regex("[^a-z0-9]+"), " ")
        .trim()

    private fun fallbackKey(artist: String, title: String, durationMs: Long): String {
        val raw = "${norm(artist)}|${norm(title)}|${max(0L, durationMs / 1000L)}"
        val digest = MessageDigest.getInstance("SHA-1").digest(raw.toByteArray())
        return "sha1:" + digest.joinToString("") { "%02x".format(it) }
    }
}
