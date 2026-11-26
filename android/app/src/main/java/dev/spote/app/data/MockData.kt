package dev.spote.app.data

import androidx.compose.ui.graphics.Color
import dev.spote.app.model.Album
import dev.spote.app.model.Cover
import dev.spote.app.model.Genre
import dev.spote.app.model.HomeState
import dev.spote.app.model.LibraryState
import dev.spote.app.model.LyricLine
import dev.spote.app.model.Playlist
import dev.spote.app.model.PlayerState
import dev.spote.app.model.RepeatMode
import dev.spote.app.model.Room
import dev.spote.app.model.SearchState
import dev.spote.app.model.Track

/**
 * Datos de mentira para diseñar la UI sin emulador, sin Rust y sin red.
 * Cualquier cambio aquí se ve en las capturas de Roborazzi al instante.
 */
object MockData {

    private fun cov(start: Long, end: Long, label: String) =
        Cover(Color(start), Color(end), label)

    private fun t(
        key: String,
        title: String,
        artist: String,
        album: String,
        durationMs: Long,
        cover: Cover,
        number: Int = 0,
        downloaded: Boolean = false,
        favorite: Boolean = false,
    ) = Track(
        key = key,
        title = title,
        artist = artist,
        album = album,
        durationMs = durationMs,
        cover = cover,
        trackNumber = number,
        downloaded = downloaded,
        favorite = favorite,
    )

    val slowRushCover = cov(0xFFF2A33A, 0xFF6E2A1A, "TSR")
    val currentsCover = cov(0xFF3A8FA8, 0xFF102E4A, "CUR")
    val inColourCover = cov(0xFF3E8E9E, 0xFF123A4A, "IC")
    val swimCover = cov(0xFFD86A9A, 0xFF4A1E4F, "SWIM")
    val ourLoveCover = cov(0xFF8A5AC8, 0xFF24124A, "OL")
    val crushCover = cov(0xFF7A6BD8, 0xFF1E1A4F, "CR")
    val untrueCover = cov(0xFF8A9AA8, 0xFF1A2028, "UN")
    val mezzanineCover = cov(0xFF3A6EA8, 0xFF0E1A2E, "MZ")
    val outRunCover = cov(0xFFD84A5A, 0xFF2A0E1E, "OR")
    val hurryCover = cov(0xFFC84AD8, 0xFF1A1E4A, "HU")
    val rainbowsCover = cov(0xFF4A8AD8, 0xFF0E2A2A, "IR")
    val sawCover = cov(0xFFC6F24E, 0xFF2A3A0E, "SAW")
    val windowCover = cov(0xFFE8D84A, 0xFF3A2A0E, "WL")

    val borderline = t(
        "t-borderline", "Borderline", "Tame Impala", "The Slow Rush", 237_000,
        slowRushCover, 4, downloaded = true, favorite = true,
    )
    val lessIKnow = t(
        "t-less", "The Less I Know the Better", "Tame Impala", "Currents", 216_000,
        currentsCover, 3,
    )
    val lostInYesterday = t(
        "t-lost", "Lost in Yesterday", "Tame Impala", "The Slow Rush", 297_000,
        slowRushCover, 6,
    )
    val posthumous = t(
        "t-posthumous", "Posthumous Forgiveness", "Tame Impala", "The Slow Rush", 366_000,
        slowRushCover, 5,
    )
    val loudPlaces = t(
        "t-loud", "Loud Places", "Jamie xx", "In Colour", 285_000,
        inColourCover, 6, downloaded = true, favorite = true,
    )
    val gosh = t(
        "t-gosh", "Gosh", "Jamie xx", "In Colour", 275_000,
        inColourCover, 1,
    )
    val seaOfVoices = t(
        "t-sea", "Sea of Voices", "Jamie xx", "In Colour", 300_000,
        inColourCover, 9,
    )
    val odessa = t(
        "t-odessa", "Odessa", "Caribou", "Swim", 315_000,
        swimCover, 1, favorite = true,
    )
    val sun = t(
        "t-sun", "Sun", "Caribou", "Swim", 260_000,
        swimCover, 2,
    )
    val cantDo = t(
        "t-cant", "Can't Do Without You", "Caribou", "Our Love", 235_000,
        ourLoveCover, 1,
    )
    val silhouettes = t(
        "t-sil", "Silhouettes", "Floating Points", "Crush", 245_000,
        crushCover, 3, downloaded = true,
    )
    val lastBloom = t(
        "t-bloom", "Last Bloom", "Floating Points", "Crush", 331_000,
        crushCover, 4,
    )
    val archangel = t(
        "t-archangel", "Archangel", "Burial", "Untrue", 240_000,
        untrueCover, 2,
    )
    val etched = t(
        "t-etched", "Etched Headplate", "Burial", "Untrue", 360_000,
        untrueCover, 8,
    )
    val teardrop = t(
        "t-teardrop", "Teardrop", "Massive Attack", "Mezzanine", 330_000,
        mezzanineCover, 2, favorite = true,
    )
    val angel = t(
        "t-angel", "Angel", "Massive Attack", "Mezzanine", 378_000,
        mezzanineCover, 1,
    )
    val nightcall = t(
        "t-nightcall", "Nightcall", "Kavinsky", "OutRun", 258_000,
        outRunCover, 1,
    )
    val oddLook = t(
        "t-odd", "Odd Look", "Kavinsky", "OutRun", 250_000,
        outRunCover, 2,
    )
    val midnightCity = t(
        "t-midnight", "Midnight City", "M83", "Hurry Up, We're Dreaming", 244_000,
        hurryCover, 2,
    )
    val wait = t(
        "t-wait", "Wait", "M83", "Hurry Up, We're Dreaming", 300_000,
        hurryCover, 6,
    )
    val weirdFishes = t(
        "t-weird", "Weird Fishes / Arpeggi", "Radiohead", "In Rainbows", 320_000,
        rainbowsCover, 3,
    )
    val nude = t(
        "t-nude", "Nude", "Radiohead", "In Rainbows", 255_000,
        rainbowsCover, 4,
    )
    val xtal = t(
        "t-xtal", "Xtal", "Aphex Twin", "Selected Ambient Works 85-92", 292_000,
        sawCover, 1,
    )
    val tha = t(
        "t-tha", "Tha", "Aphex Twin", "Selected Ambient Works 85-92", 546_000,
        sawCover, 2,
    )
    val windowlicker = t(
        "t-window", "Windowlicker", "Aphex Twin", "Windowlicker", 366_000,
        windowCover, 1,
    )

    val tracks = listOf(
        borderline, lessIKnow, loudPlaces, gosh, odessa, cantDo, silhouettes,
        archangel, teardrop, nightcall, midnightCity, weirdFishes, xtal,
        windowlicker, lastBloom, etched, angel, nude,
    )

    val albums = listOf(
        Album("a-slow-rush", "The Slow Rush", "Tame Impala", 2020, slowRushCover, listOf(borderline, posthumous, lostInYesterday)),
        Album("a-in-colour", "In Colour", "Jamie xx", 2015, inColourCover, listOf(gosh, loudPlaces, seaOfVoices)),
        Album("a-swim", "Swim", "Caribou", 2010, swimCover, listOf(odessa, sun)),
        Album("a-crush", "Crush", "Floating Points", 2019, crushCover, listOf(silhouettes, lastBloom)),
        Album("a-untrue", "Untrue", "Burial", 2007, untrueCover, listOf(archangel, etched)),
        Album("a-mezzanine", "Mezzanine", "Massive Attack", 1998, mezzanineCover, listOf(angel, teardrop)),
        Album("a-outrun", "OutRun", "Kavinsky", 2013, outRunCover, listOf(nightcall, oddLook)),
        Album("a-hurry", "Hurry Up, We're Dreaming", "M83", 2011, hurryCover, listOf(midnightCity, wait)),
        Album("a-rainbows", "In Rainbows", "Radiohead", 2007, rainbowsCover, listOf(weirdFishes, nude)),
        Album("a-saw", "Selected Ambient Works 85-92", "Aphex Twin", 1992, sawCover, listOf(xtal, tha)),
    )

    val playlists = listOf(
        Playlist("p-night", "Noche larga", "Para cuando la ciudad duerme", cov(0xFF4A6BD8, 0xFF16123A, "NL"), 38),
        Playlist("p-drive", "Para conducir", "Ritmo y carretera", cov(0xFFD8844A, 0xFF3A1A0E, "PC"), 24),
        Playlist("p-discover", "Descubrimientos", "Lo que ha entrado esta semana", cov(0xFF4AC8A8, 0xFF0E2A28, "D"), 17),
        Playlist("p-fav", "Favoritas", "Las de siempre", cov(0xFFC84A8A, 0xFF3A0E24, "F"), 52),
    )

    val rooms = listOf(
        Room("r-sala", "Sala de los colegas", listOf("Tú", "Marta", "Leo"), true, loudPlaces),
        Room("r-after", "After en casa", listOf("Tú", "Nico"), true, nightcall),
    )

    val genres = listOf(
        Genre("Noche", cov(0xFF3E8E9E, 0xFF0E2A3A, "N")),
        Genre("Concentración", cov(0xFF7A6BD8, 0xFF18123A, "C")),
        Genre("Conducir", cov(0xFFD8844A, 0xFF3A1A0E, "D")),
        Genre("Descubrir", cov(0xFFC6F24E, 0xFF2A3A0E, "X")),
    )

    val home = HomeState(
        greeting = "Buenas noches",
        chips = listOf("Todo", "Descargas", "Salas", "Nuevo"),
        recent = listOf(borderline, loudPlaces, odessa, silhouettes, nightcall, teardrop),
        rooms = rooms,
        playlists = playlists,
        albums = albums.take(6),
    )

    val search = SearchState(
        recentQueries = listOf("borderline", "jamie xx", "noche", "floating points"),
        genres = genres,
    )

    val library = LibraryState(
        playlists = playlists,
        albums = albums.take(5),
        tracks = tracks,
        downloadedCount = tracks.count { it.downloaded },
        storageLabel = "1,2 GB",
    )

    val lyrics = listOf(
        LyricLine(0, "La ciudad se apaga"),
        LyricLine(4_200, "y el bajo no"),
        LyricLine(7_800, "Queda la luz de la portada"),
        LyricLine(12_500, "y un rumor de habitación"),
        LyricLine(17_000, "No sé si es tarde"),
        LyricLine(21_400, "o si esto acaba de empezar"),
        LyricLine(26_000, "Déjalo sonar"),
        LyricLine(30_200, "que la noche sabe a sal"),
        LyricLine(35_000, "Y si mañana no recuerdo"),
        LyricLine(39_500, "que me despierten lentamente"),
        LyricLine(44_000, "otra vez"),
        LyricLine(48_000, "otra vez"),
    )

    val queue = listOf(lessIKnow, gosh, loudPlaces, odessa, silhouettes, archangel, teardrop)

    val player = PlayerState(
        track = borderline,
        isPlaying = false,
        positionMs = 84_000,
        durationMs = borderline.durationMs,
        shuffle = false,
        repeat = RepeatMode.Off,
        queue = queue,
        lyrics = lyrics,
    )

    fun album(id: String): Album? = albums.firstOrNull { it.id == id }

    fun search(query: String): SearchState {
        val q = query.trim()
        if (q.isEmpty()) return search
        val matches = tracks.filter {
            it.title.contains(q, ignoreCase = true) ||
                it.artist.contains(q, ignoreCase = true) ||
                it.album?.contains(q, ignoreCase = true) == true
        }
        return search.copy(
            query = query,
            results = matches,
            albums = albums.filter {
                it.title.contains(q, ignoreCase = true) || it.artist.contains(q, ignoreCase = true)
            },
            artists = tracks.filter { it.artist.contains(q, ignoreCase = true) }
                .map { it.artist }
                .distinct(),
        )
    }
}
