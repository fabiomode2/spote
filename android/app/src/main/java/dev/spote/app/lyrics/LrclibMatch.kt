package dev.spote.app.lyrics

import java.text.Normalizer
import kotlin.math.abs
import kotlin.math.roundToLong

/**
 * One track returned by LRCLIB. The exact `get` endpoint returns a single object and the
 * `search` endpoint returns an array of these.
 */
internal data class LrclibTrack(
    val id: Long? = null,
    val trackName: String,
    val artistName: String,
    val albumName: String? = null,
    val durationSec: Double? = null,
    val instrumental: Boolean = false,
    val syncedLyrics: String? = null,
    val plainLyrics: String? = null,
) {
    val hasLyrics: Boolean
        get() = !syncedLyrics.isNullOrBlank() || !plainLyrics.isNullOrBlank()

    val hasSyncedLyrics: Boolean
        get() = !syncedLyrics.isNullOrBlank()
}

/**
 * Pure selection logic for LRCLIB search results, kept apart from networking so it can be
 * covered by plain JVM tests.
 *
 * A candidate is only accepted when its title matches (exactly or after removing
 * parentheses/brackets) and its artist matches. When a duration is known on both sides the
 * result must be within [DURATION_TOLERANCE_MS]. Among accepted candidates the highest score
 * wins; synced lyrics are strongly preferred, then exact artist, then duration closeness.
 */
internal object LrclibMatcher {
    const val DURATION_TOLERANCE_MS = 3_000L

    private const val TITLE_EXACT = 50
    private const val TITLE_BASE = 35
    private const val ARTIST_EXACT = 50
    private const val ARTIST_PRIMARY = 30
    private const val ARTIST_CONTAINS = 20
    private const val ALBUM_MATCH = 5
    private const val SYNCED_BONUS = 100
    private const val DURATION_MAX = 20

    private val DECORATION = Regex("\\([^)]*\\)|\\[[^\\]]*]")
    private val ARTIST_SEPARATOR = Regex(
        "\\s+(?:feat\\.?|ft\\.?|featuring|with|and|x)\\s+|\\s*[&,/]\\s*",
        RegexOption.IGNORE_CASE,
    )

    fun selectBest(
        candidates: List<LrclibTrack>,
        title: String,
        artist: String,
        album: String? = null,
        durationMs: Long = 0L,
    ): LrclibTrack? {
        val normalizedTitle = normalize(title)
        val normalizedArtist = normalize(artist)
        val normalizedAlbum = album?.let(::normalize)?.takeIf { it.isNotEmpty() }
        if (normalizedTitle.isEmpty()) return null

        val scored = candidates.mapNotNull { candidate ->
            if (candidate.instrumental || !candidate.hasLyrics) return@mapNotNull null

            val candidateTitle = normalize(candidate.trackName)
            val titleScore = when {
                candidateTitle == normalizedTitle -> TITLE_EXACT
                baseTitle(candidate.trackName) == baseTitle(title) -> TITLE_BASE
                else -> return@mapNotNull null
            }

            val artistScore = when {
                normalizedArtist.isEmpty() -> 0
                normalize(candidate.artistName) == normalizedArtist -> ARTIST_EXACT
                primaryArtist(candidate.artistName) == normalizedArtist -> ARTIST_PRIMARY
                containsArtist(normalize(candidate.artistName), normalizedArtist) -> ARTIST_CONTAINS
                else -> return@mapNotNull null
            }

            val durationDiff = when {
                durationMs <= 0L || candidate.durationSec == null -> null
                else -> abs(candidate.durationSec.toMillis() - durationMs)
            }
            if (durationDiff != null && durationDiff > DURATION_TOLERANCE_MS) return@mapNotNull null

            val durationScore = if (durationDiff == null) {
                0
            } else {
                ((DURATION_TOLERANCE_MS - durationDiff) * DURATION_MAX / DURATION_TOLERANCE_MS).toInt()
            }
            val albumScore = if (
                normalizedAlbum != null &&
                normalize(candidate.albumName.orEmpty()) == normalizedAlbum
            ) {
                ALBUM_MATCH
            } else {
                0
            }
            val syncedScore = if (candidate.hasSyncedLyrics) SYNCED_BONUS else 0

            Scored(
                track = candidate,
                score = titleScore + artistScore + albumScore + syncedScore + durationScore,
                durationDiffMs = durationDiff ?: Long.MAX_VALUE,
            )
        }

        return scored.sortedWith(
            compareByDescending<Scored> { it.score }
                .thenBy { it.durationDiffMs },
        ).firstOrNull()?.track
    }

    /** Lowercases, removes diacritics and keeps only letters and digits. */
    fun normalize(value: String): String =
        Normalizer.normalize(value, Normalizer.Form.NFKD)
            .filter { it.isLetterOrDigit() }
            .lowercase()

    private fun baseTitle(value: String): String = normalize(DECORATION.replace(value, " "))

    private fun primaryArtist(value: String): String =
        normalize(ARTIST_SEPARATOR.split(value).firstOrNull().orEmpty())

    private fun containsArtist(candidate: String, requested: String): Boolean {
        if (requested.length < 3 || candidate.length < 3) return false
        return candidate.contains(requested) || requested.contains(candidate)
    }

    private fun Double.toMillis(): Long = (this * 1000.0).roundToLong()

    private data class Scored(
        val track: LrclibTrack,
        val score: Int,
        val durationDiffMs: Long,
    )
}
