package dev.spote.app.screenshot

import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
import dev.spote.app.data.MockData
import dev.spote.app.model.ChatMessage
import dev.spote.app.ui.nowplaying.NowPlayingScreen
import dev.spote.app.ui.nowplaying.QueueScreen
import dev.spote.app.ui.screens.AlbumScreen
import dev.spote.app.ui.screens.ArtistScreen
import dev.spote.app.ui.screens.DownloadsScreen
import dev.spote.app.ui.screens.HomeScreen
import dev.spote.app.ui.screens.LibraryScreen
import dev.spote.app.ui.screens.PlaylistScreen
import dev.spote.app.ui.screens.RoomScreen
import dev.spote.app.ui.screens.SearchScreen
import dev.spote.app.ui.screens.SettingsScreen
import dev.spote.app.ui.screens.ThemePickerDialog
import dev.spote.app.ui.theme.SpoteTheme
import dev.spote.app.ui.theme.SpoteThemeMode
import dev.spote.app.ui.update.UpdateCardContent
import dev.spote.app.update.UpdateUiState
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w412dp-h915dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ScreenShotsTest {

    @get:Rule
    val compose = createComposeRule()

    private fun shot(
        name: String,
        mode: SpoteThemeMode = SpoteThemeMode.Midnight,
        content: @Composable () -> Unit,
    ) {
        compose.setContent {
            SpoteTheme(mode = mode) { content() }
        }
        val output = File("build/screenshots").apply { mkdirs() }
        compose.onRoot().captureRoboImage(File(output, "$name.png").absolutePath)
    }

    @Test
    fun home() = shot("01-home") {
        HomeScreen(
            state = MockData.home,
            onPlayTrack = {},
            onOpenAlbum = {},
            onOpenPlaylist = {},
            onOpenRoom = {},
            onOpenSettings = {},
        )
    }

    @Test
    fun search() = shot("02-search") {
        SearchScreen(
            state = MockData.search("jamie"),
            onQueryChange = {},
            onPlayTrack = {},
            onOpenAlbum = {},
            onMore = {},
        )
    }

    @Test
    fun searchEmpty() = shot("03-search-empty") {
        SearchScreen(
            state = MockData.search,
            onQueryChange = {},
            onPlayTrack = {},
            onOpenAlbum = {},
            onMore = {},
        )
    }

    @Test
    fun library() = shot("04-library") {
        LibraryScreen(
            state = MockData.library,
            onFilter = {},
            onToggleGrid = {},
            onPlayTrack = {},
            onMore = {},
            onOpenAlbum = {},
            onOpenPlaylist = {},
            onOpenDownloads = {},
            onOpenSearch = {},
            onOpenArtist = {},
        )
    }

    @Test
    fun album() = shot("05-album") {
        AlbumScreen(
            album = MockData.albums.first(),
            player = MockData.player,
            onBack = {},
            onPlay = {},
            onShuffle = {},
            onPlayTrack = {},
            onMore = {},
            onDownload = {},
            onLike = {},
        )
    }

    @Test
    fun nowPlaying() = shot("06-now-playing") {
        NowPlayingScreen(
            player = MockData.player.copy(isPlaying = true),
            onClose = {},
            onTogglePlay = {},
            onNext = {},
            onPrevious = {},
            onSeek = {},
            onShuffle = {},
            onRepeat = {},
            onLike = {},
            onDownload = {},
            onQueue = {},
        )
    }

    @Test
    fun queue() = shot("08-queue") {
        QueueScreen(
            player = MockData.player,
            onBack = {},
            onPlayTrack = {},
            onMore = { _, _ -> },
        )
    }

    @Test
    fun libraryGrid() = shot("09-library-grid") {
        LibraryScreen(
            state = MockData.library.copy(
                filter = dev.spote.app.model.LibraryFilter.Albums,
                isGrid = true,
            ),
            onFilter = {},
            onToggleGrid = {},
            onPlayTrack = {},
            onMore = {},
            onOpenAlbum = {},
            onOpenPlaylist = {},
            onOpenDownloads = {},
            onOpenSearch = {},
            onOpenArtist = {},
        )
    }

    @Test
    fun playlist() = shot("10-playlist") {
        PlaylistScreen(
            playlist = MockData.playlists.first(),
            tracks = MockData.tracks.take(8),
            onBack = {},
            onPlayTrack = {},
            onMore = {},
        )
    }

    @Test
    fun downloads() = shot("11-downloads") {
        DownloadsScreen(
            downloaded = MockData.tracks.filter { it.downloaded },
            onBack = {},
            onPlayTrack = {},
            onMore = {},
        )
    }

    @Test
    fun settings() = shot("12-settings") {
        SettingsScreen(
            onBack = {},
            onOpenDownloads = {},
            updateContent = {
                UpdateCardContent(
                    state = UpdateUiState.Available(
                        current = "0.1.0",
                        latest = "0.2.0",
                        notes = "Cola compartida, letras a pantalla completa y descargas con una sola conexión.",
                        apkUrl = "",
                        sha256 = null,
                    ),
                    onCheck = {},
                    onDownload = {},
                    onInstall = {},
                    onDismiss = {},
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                )
            },
        )
    }

    @Test
    fun updateCard() = shot("14-update-card") {
        UpdateCardContent(
            state = UpdateUiState.Available(
                current = "0.1.0",
                latest = "0.2.0",
                notes = "Cola compartida, letras a pantalla completa y descargas con una sola conexión.",
                apkUrl = "",
                sha256 = null,
            ),
            onCheck = {},
            onDownload = {},
            onInstall = {},
            onDismiss = {},
            modifier = Modifier.padding(20.dp),
        )
    }

    @Test
    fun nocturneHome() = shot("15-nocturne-home", SpoteThemeMode.Nocturne) {
        HomeScreen(
            state = MockData.home,
            onPlayTrack = {},
            onOpenAlbum = {},
            onOpenPlaylist = {},
            onOpenRoom = {},
            onOpenSettings = {},
        )
    }

    @Test
    fun nocturneNowPlaying() = shot("16-nocturne-now-playing", SpoteThemeMode.Nocturne) {
        NowPlayingScreen(
            player = MockData.player.copy(isPlaying = true),
            onClose = {},
            onTogglePlay = {},
            onNext = {},
            onPrevious = {},
            onSeek = {},
            onShuffle = {},
            onRepeat = {},
            onLike = {},
            onDownload = {},
            onQueue = {},
        )
    }

    @Test
    fun nocturneAlbum() = shot("17-nocturne-album", SpoteThemeMode.Nocturne) {
        AlbumScreen(
            album = MockData.albums.first(),
            player = MockData.player,
            onBack = {},
            onPlay = {},
            onShuffle = {},
            onPlayTrack = {},
            onMore = {},
            onDownload = {},
            onLike = {},
        )
    }

    @Test
    fun themePicker() = shot("18-theme-picker", SpoteThemeMode.Nocturne) {
        ThemePickerDialog(
            current = SpoteThemeMode.Nocturne,
            onSelect = {},
            onDismiss = {},
        )
    }

    @Test
    fun artist() = shot("19-artist") {
        ArtistScreen(
            name = "Tame Impala",
            tracks = MockData.tracks.filter { it.artist == "Tame Impala" },
            albums = MockData.home.albums.filter { it.artist == "Tame Impala" },
            player = MockData.player,
            onBack = {},
            onPlayTrack = {},
            onOpenAlbum = {},
            onMore = {},
        )
    }

    @Test
    fun room() = shot("13-room") {
        RoomScreen(
            room = MockData.rooms.first(),
            queue = MockData.queue,
            messages = listOf(
                ChatMessage("Marta", "esto entra muy bien"),
                ChatMessage("Leo", "espera al drop"),
                ChatMessage("Tú", "la dejo sonar entera"),
            ),
            connected = true,
            onSendChat = {},
            onBack = {},
            onPlayTrack = {},
            onMore = {},
        )
    }
}
