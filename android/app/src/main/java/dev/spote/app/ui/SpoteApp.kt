package dev.spote.app.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Album
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.DeleteOutline
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.Favorite
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.PlaylistAdd
import androidx.compose.material.icons.rounded.QueueMusic
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.launch
import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import dev.spote.app.artwork.rememberCoverPalette
import dev.spote.app.model.Album
import dev.spote.app.model.ArtistProfile
import dev.spote.app.model.Playlist
import dev.spote.app.model.Tab
import dev.spote.app.logging.ActionLog
import dev.spote.app.model.Track
import dev.spote.app.profile.ProfileStore
import dev.spote.app.rust.coverFor
import dev.spote.app.ui.components.LocalAlbumNavigation
import dev.spote.app.ui.components.LocalArtistNavigation
import dev.spote.app.ui.components.MiniPlayer
import dev.spote.app.ui.components.SpoteBottomBar
import dev.spote.app.ui.components.TopBar
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
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.ui.theme.SpoteThemeMode
import dev.spote.app.ui.theme.withCoverPalette
import dev.spote.app.ui.update.UpdateCard

@Composable
fun SpoteApp(
    themeMode: SpoteThemeMode = SpoteThemeMode.Midnight,
    onThemeModeChange: (SpoteThemeMode) -> Unit = {},
    uiStyle: dev.spote.app.ui.theme.SpoteUiStyle = dev.spote.app.ui.theme.SpoteUiStyle.Off,
    onUiStyleChange: (dev.spote.app.ui.theme.SpoteUiStyle) -> Unit = {},
) {
    val navController = rememberNavController()
    val appViewModel: AppViewModel = viewModel()
    val socialViewModel: SocialViewModel = viewModel()
    val state by appViewModel.state.collectAsStateWithLifecycle()
    val socialState by socialViewModel.state.collectAsStateWithLifecycle()
    val socialNotice by socialViewModel.notices.collectAsStateWithLifecycle()
    val presenceAvatars = socialState.presences.mapNotNull { presence ->
        presence.user.avatar?.takeIf { it.isNotBlank() }?.let { presence.user.name to it }
    }.toMap()
    val homeState = remember(state.home, socialState.rooms, presenceAvatars) {
        state.home.copy(
            rooms = socialState.rooms.map { room ->
                val ui = room.toUiRoom()
                ui.copy(memberAvatars = ui.memberAvatars + presenceAvatars)
            },
        )
    }
    var menuTrack by remember { mutableStateOf<Track?>(null) }
    var menuPlaylistId by remember { mutableStateOf<String?>(null) }
    var menuQueueIndex by remember { mutableStateOf<Int?>(null) }
    var showCreatePlaylist by remember { mutableStateOf(false) }
    var pickerTrack by remember { mutableStateOf<Track?>(null) }
    var pendingPlaylistTrack by remember { mutableStateOf<Track?>(null) }
    val backStackEntry by navController.currentBackStackEntryAsState()
    val route = backStackEntry?.destination?.route ?: "home"
    val baseColors = LocalSpoteColors.current
    val activeCover = remember(
        route,
        state.player.track,
        state.home.recent,
        state.search.results,
        state.search.remoteResults,
        state.library.albums,
        state.library.tracks,
        state.albumDetails,
        state.artistDetails,
        backStackEntry,
    ) {
        when {
            route == "now" -> state.player.track?.cover
            route.startsWith("album/") -> backStackEntry?.arguments?.getString("id")
                ?.let { appViewModel.album(it)?.cover }
            route.startsWith("artist/") -> {
                val name = backStackEntry?.arguments?.getString("name").orEmpty()
                val provider = backStackEntry?.arguments?.getString("provider")?.takeIf { it.isNotBlank() }
                val id = backStackEntry?.arguments?.getString("id")?.takeIf { it.isNotBlank() }
                val profile = appViewModel.artistProfile(name, provider, id)
                profile?.artist?.imageUrl?.let { coverFor(name, null, it) }
                    ?: profile?.albums?.firstOrNull()?.cover
            }
            route == "search" -> state.search.remoteResults.firstOrNull()?.cover
                ?: state.search.results.firstOrNull()?.cover
            route == "library" -> state.library.albums.firstOrNull()?.cover
                ?: state.library.tracks.firstOrNull()?.cover
            route == "home" -> state.home.recent.firstOrNull()?.cover
            else -> state.player.track?.cover
        }
    }
    val palette = rememberCoverPalette(activeCover)
    val colors = palette?.let { baseColors.withCoverPalette(it) } ?: baseColors
    val context = LocalContext.current
    val appScope = rememberCoroutineScope()
    ProfileStore.init(context)
    val profileName by ProfileStore.name.collectAsStateWithLifecycle()
    val avatarPath by ProfileStore.avatar.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    val importLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenMultipleDocuments(),
    ) { uris -> appViewModel.importUris(uris) }
    val notificationPermission = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { }

    fun openTrackMenu(track: Track, playlistId: String? = null, queueIndex: Int? = null) {
        menuTrack = track
        menuPlaylistId = playlistId
        menuQueueIndex = queueIndex
    }

    LaunchedEffect(Unit) {
        appViewModel.messages.collect { message -> snackbarHostState.showSnackbar(message) }
    }

    LaunchedEffect(route) {
        ActionLog.log("nav", "route" to route)
    }

    LaunchedEffect(socialNotice) {
        val notice = socialNotice ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(notice)
        socialViewModel.clearNotice()
    }

    LaunchedEffect(state.player.isPlaying) {
        if (
            state.player.isPlaying &&
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    val immersiveRoutes = setOf("now", "queue", "lyrics", "settings")
    val fullScreen = route in immersiveRoutes || route.startsWith("room/")
    val showBottom = !fullScreen
    val showMini = state.player.track != null && !fullScreen

    var lastTab by rememberSaveable { mutableStateOf(Tab.Home) }
    LaunchedEffect(route) {
        when (route) {
            "home" -> lastTab = Tab.Home
            "search" -> lastTab = Tab.Search
            "library" -> lastTab = Tab.Library
        }
    }
    val currentTab = when (route) {
        "home" -> Tab.Home
        "search" -> Tab.Search
        "library" -> Tab.Library
        else -> lastTab
    }

    fun albumRoute(album: Album): String = "album/${Uri.encode(album.id)}"

    fun artistRoute(name: String, provider: String? = null, providerId: String? = null): String {
        val base = "artist/${Uri.encode(name)}"
        return "$base?provider=${Uri.encode(provider.orEmpty())}&id=${Uri.encode(providerId.orEmpty())}"
    }

    fun selectTab(tab: Tab) {
        val target = when (tab) {
            Tab.Home -> "home"
            Tab.Search -> "search"
            Tab.Library -> "library"
        }
        if (tab == currentTab) {
            // Reseleccionar vuelve a la raíz del tab sin recrear la pantalla.
            if (!navController.popBackStack(target, inclusive = false)) {
                navController.navigate(target) { launchSingleTop = true }
            }
            return
        }
        ActionLog.log("nav.tab", "from" to route, "to" to target)
        navController.navigate(target) {
            popUpTo("home") { saveState = false }
            launchSingleTop = true
        }
    }

    BackHandler(enabled = route == "search" || route == "library") {
        navController.popBackStack("home", inclusive = false)
    }

    CompositionLocalProvider(
        LocalSpoteColors provides colors,
        LocalArtistNavigation provides { artist ->
            val ref = appViewModel.artistRef(artist)
            appViewModel.cacheArtist(artist)
            navController.navigate(artistRoute(artist, ref?.provider, ref?.providerId))
        },
        LocalAlbumNavigation provides { track ->
            appViewModel.albumFor(track)?.let { album ->
                appViewModel.cacheAlbum(album)
                navController.navigate(albumRoute(album))
            }
        },
    ) {
    Scaffold(
        containerColor = colors.background,
        contentWindowInsets = WindowInsets.navigationBars,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        bottomBar = {
            if (showMini || showBottom) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(colors.background)
                        .navigationBarsPadding(),
                ) {
                    if (showMini) {
                        MiniPlayer(
                            player = state.player,
                            onExpand = {
                                if (state.player.track != null) {
                                    navController.navigate("now") { launchSingleTop = true }
                                }
                            },
                            onTogglePlay = appViewModel::togglePlay,
                            onNext = appViewModel::next,
                        )
                    }
                    if (showBottom) {
                        SpoteBottomBar(
                            selected = currentTab,
                            onSelect = ::selectTab,
                        )
                    }
                }
            }
        },
    ) { innerPadding ->
        NavHost(
            navController = navController,
            startDestination = "home",
            modifier = Modifier.padding(innerPadding),
        ) {
            composable("home") {
                HomeScreen(
                    state = homeState,
                    onPlayTrack = { appViewModel.play(it) },
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate(albumRoute(it)) },
                    onOpenPlaylist = { navController.navigate("playlist/${Uri.encode(it.id)}") },
                    onOpenRoom = { room ->
                        socialViewModel.join(room.id)
                        navController.navigate("room/${room.id}")
                    },
                    onOpenSettings = { navController.navigate("settings") },
                    onImport = { importLauncher.launch(arrayOf("audio/*")) },
                    userName = socialState.userName.ifBlank { profileName },
                    avatarUri = avatarPath,
                )
            }
            composable("search") {
                SearchScreen(
                    state = state.search,
                    onQueryChange = appViewModel::setQuery,
                    onPlayTrack = {
                        appViewModel.commitSearch(state.search.query)
                        appViewModel.play(it)
                    },
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate(albumRoute(it)) },
                    onOpenArtist = { name, provider, providerId ->
                        appViewModel.cacheArtist(name)
                        navController.navigate(artistRoute(name, provider, providerId))
                    },
                    onMore = { openTrackMenu(it) },
                    onClearSearch = appViewModel::clearSearch,
                    onClearHistory = appViewModel::clearSearchHistory,
                    onSearchCommit = appViewModel::commitSearch,
                    userName = socialState.userName.ifBlank { profileName },
                    avatarUri = avatarPath,
                    onOpenSettings = { navController.navigate("settings") },
                )
            }
            composable("library") {
                LibraryScreen(
                    state = state.library,
                    onFilter = appViewModel::setFilter,
                    onToggleGrid = appViewModel::toggleGrid,
                    onPlayTrack = { appViewModel.play(it) },
                    onMore = { openTrackMenu(it) },
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate(albumRoute(it)) },
                    onOpenPlaylist = { navController.navigate("playlist/${Uri.encode(it.id)}") },
                    onOpenDownloads = { navController.navigate("downloads") },
                    onOpenSearch = { selectTab(Tab.Search) },
                    onOpenArtist = { artist ->
                        val ref = appViewModel.artistRef(artist)
                        appViewModel.cacheArtist(artist)
                        navController.navigate(artistRoute(artist, ref?.provider, ref?.providerId))
                    },
                    onCreatePlaylist = { showCreatePlaylist = true },
                    onImport = { importLauncher.launch(arrayOf("audio/*")) },
                    userName = socialState.userName.ifBlank { profileName },
                    avatarUri = avatarPath,
                    onOpenSettings = { navController.navigate("settings") },
                )
            }
            composable(
                route = "album/{id}",
                enterTransition = { slideInHorizontally(initialOffsetX = { it }) },
                exitTransition = { slideOutHorizontally(targetOffsetX = { -it / 4 }) },
                popEnterTransition = { slideInHorizontally(initialOffsetX = { -it / 4 }) },
                popExitTransition = { slideOutHorizontally(targetOffsetX = { it }) },
            ) { entry ->
                val albumId = entry.arguments?.getString("id").orEmpty()
                val album = appViewModel.album(albumId)
                val loading = appViewModel.albumLoading(albumId)
                val error = appViewModel.albumError(albumId)
                LaunchedEffect(albumId) {
                    appViewModel.loadAlbum(albumId)
                    ActionLog.log(
                        "album.open",
                        "id" to albumId,
                        "found" to (album != null),
                        "loading" to loading,
                        "catalogue" to state.library.albums.size,
                    )
                }
                when {
                    album != null && (
                        album.tracks.isNotEmpty() ||
                            state.albumDetails.containsKey(albumId)
                        ) -> {
                        AlbumScreen(
                            album = album,
                            player = state.player,
                            onBack = { navController.popBackStack() },
                            onPlay = {
                                appViewModel.playAlbum(album, shuffle = false)
                            },
                            onShuffle = {
                                appViewModel.playAlbum(album, shuffle = true)
                            },
                            onPlayTrack = { appViewModel.play(it) },
                            onMore = { openTrackMenu(it) },
                            onDownload = {
                                album.tracks.firstOrNull()?.let { appViewModel.toggleDownload(it.key) }
                            },
                            onLike = {
                                appViewModel.toggleAlbumFavorite(album)
                            },
                            isFavorite = album.tracks.isNotEmpty() &&
                                album.tracks.all { it.favorite },
                        )
                    }
                    loading || error == null -> {
                        LoadingScreen(
                            message = "Cargando álbum…",
                            onBack = { navController.popBackStack() },
                        )
                    }
                    else -> {
                        MessageScreen(
                            title = "No se encontró el álbum",
                            text = error ?: "Puede que el catálogo no lo tenga disponible.",
                            onBack = { navController.popBackStack() },
                            onRetry = { appViewModel.loadAlbum(albumId) },
                        )
                    }
                }
            }
            composable(
                route = "artist/{name}?provider={provider}&id={id}",
                arguments = listOf(
                    navArgument("provider") {
                        type = NavType.StringType
                        nullable = true
                        defaultValue = null
                    },
                    navArgument("id") {
                        type = NavType.StringType
                        nullable = true
                        defaultValue = null
                    },
                ),
                enterTransition = { slideInHorizontally(initialOffsetX = { it }) },
                exitTransition = { slideOutHorizontally(targetOffsetX = { -it / 4 }) },
                popEnterTransition = { slideInHorizontally(initialOffsetX = { -it / 4 }) },
                popExitTransition = { slideOutHorizontally(targetOffsetX = { it }) },
            ) { entry ->
                val artistName = entry.arguments?.getString("name").orEmpty()
                val provider = entry.arguments?.getString("provider")?.takeIf { it.isNotBlank() }
                val providerId = entry.arguments?.getString("id")?.takeIf { it.isNotBlank() }
                val profile = appViewModel.artistProfile(artistName, provider, providerId)
                val loading = appViewModel.artistLoading(artistName, provider, providerId)
                val error = appViewModel.artistError(artistName, provider, providerId)
                LaunchedEffect(artistName, provider, providerId) {
                    appViewModel.cacheArtist(artistName)
                    appViewModel.loadArtist(artistName, provider, providerId)
                    ActionLog.log(
                        "artist.open",
                        "name" to artistName,
                        "provider" to provider,
                        "id" to providerId,
                        "tracks" to (profile?.topTracks?.size ?: 0),
                        "albums" to (profile?.albums?.size ?: 0),
                    )
                }
                val tracks = profile?.topTracks?.takeIf { it.isNotEmpty() }
                    ?: appViewModel.cachedArtistTracks(artistName).ifEmpty {
                        (
                            state.library.tracks +
                                state.home.recent +
                                state.search.results +
                                state.search.remoteResults
                            )
                            .filter { normalizeForMatch(it.artist).contains(normalizeForMatch(artistName)) }
                            .distinctBy { it.key }
                    }
                val albums = profile?.albums?.takeIf { it.isNotEmpty() }
                    ?: (appViewModel.cachedArtistAlbums(artistName) + state.search.remoteAlbums)
                        .filter { normalizeForMatch(it.artist).contains(normalizeForMatch(artistName)) }
                        .distinctBy { it.id }
                when {
                    profile != null || tracks.isNotEmpty() || albums.isNotEmpty() -> {
                        ArtistScreen(
                            name = artistName,
                            tracks = tracks,
                            albums = albums,
                            profile = profile,
                            player = state.player,
                            loading = loading,
                            error = error,
                            onBack = { navController.popBackStack() },
                            onPlayTrack = { appViewModel.play(it) },
                            onOpenAlbum = {
                                appViewModel.cacheAlbum(it)
                                navController.navigate(albumRoute(it))
                            },
                            onMore = { openTrackMenu(it) },
                            onRetry = { appViewModel.loadArtist(artistName, provider, providerId) },
                        )
                    }
                    loading || error == null -> {
                        LoadingScreen(
                            message = "Cargando artista…",
                            onBack = { navController.popBackStack() },
                        )
                    }
                    else -> {
                        MessageScreen(
                            title = "No se encontró el artista",
                            text = error ?: "El catálogo no devolvió resultados.",
                            onBack = { navController.popBackStack() },
                            onRetry = { appViewModel.loadArtist(artistName, provider, providerId) },
                        )
                    }
                }
            }
            composable(
                route = "playlist/{id}",
                enterTransition = { slideInHorizontally(initialOffsetX = { it }) },
                exitTransition = { slideOutHorizontally(targetOffsetX = { -it / 4 }) },
                popEnterTransition = { slideInHorizontally(initialOffsetX = { -it / 4 }) },
                popExitTransition = { slideOutHorizontally(targetOffsetX = { it }) },
            ) { entry ->
                val playlistId = Uri.decode(entry.arguments?.getString("id").orEmpty())
                val playlist = (state.home.playlists + state.library.playlists)
                    .firstOrNull { it.id == playlistId }
                    ?: appViewModel.playlist(playlistId)
                if (playlist != null) {
                    PlaylistScreen(
                        playlist = playlist,
                        tracks = playlist.tracks,
                        onBack = { navController.popBackStack() },
                        onPlayTrack = { appViewModel.play(it) },
                        onMore = { openTrackMenu(it, playlistId = playlist.id) },
                        onDelete = {
                            appViewModel.deletePlaylist(playlist.id)
                            navController.popBackStack()
                        },
                    )
                }
            }
            composable(
                route = "room/{id}",
                enterTransition = { slideInVertically(initialOffsetY = { it }) },
                popExitTransition = { slideOutVertically(targetOffsetY = { it }) },
            ) { entry ->
                val roomId = entry.arguments?.getString("id")
                val baseRoom = socialState.rooms.firstOrNull { it.id == roomId }?.toUiRoom()
                    ?: homeState.rooms.firstOrNull { it.id == roomId }
                if (baseRoom != null) {
                    val selfAvatar = socialState.userName
                        .takeIf { it.isNotBlank() }
                        ?.let { name -> avatarPath?.let { path -> name to path } }
                    val room = baseRoom.copy(
                        members = socialState.effectiveMembers.ifEmpty { baseRoom.members },
                        nowPlaying = socialState.nowPlaying ?: baseRoom.nowPlaying,
                        live = baseRoom.live || socialState.roomState?.playing == true,
                        memberAvatars = baseRoom.memberAvatars +
                            presenceAvatars +
                            (selfAvatar?.let { mapOf(it) } ?: emptyMap()),
                    )
                    val roomState = socialState.roomState
                    LaunchedEffect(
                        roomState?.track?.key,
                        roomState?.anchorMs,
                        roomState?.startAtMs,
                        roomState?.playing,
                    ) {
                        val roomStateValue = roomState ?: return@LaunchedEffect
                        val track = socialState.nowPlaying ?: return@LaunchedEffect
                        val waitMs = (roomStateValue.startAtMs ?: 0L) - socialViewModel.serverNowMs()
                        if (waitMs > 0L) kotlinx.coroutines.delay(waitMs)
                        appViewModel.syncWithRoom(
                            track = track,
                            positionMs = roomStateValue.targetPositionMs(socialViewModel.serverNowMs()),
                            playing = roomStateValue.playing,
                        )
                    }
                    RoomScreen(
                        room = room,
                        queue = socialState.queue.map { it.track.toUiTrack() }
                            .ifEmpty { state.player.queue },
                        messages = socialState.messages,
                        connected = socialState.loggedIn && !socialState.connecting,
                        playing = socialState.roomState?.playing ?: true,
                        onTogglePlay = {
                            socialViewModel.control(
                                if (socialState.roomState?.playing == true) {
                                    dev.spote.app.social.ControlAction.PAUSE
                                } else {
                                    dev.spote.app.social.ControlAction.PLAY
                                },
                            )
                        },
                        onSendChat = socialViewModel::sendChat,
                        onBack = {
                            socialViewModel.leave()
                            navController.popBackStack()
                        },
                        onPlayTrack = { appViewModel.play(it) },
                        onMore = { openTrackMenu(it) },
                    )
                }
            }
            composable("downloads") {
                DownloadsScreen(
                    downloaded = state.library.tracks.filter { it.downloaded },
                    onBack = { navController.popBackStack() },
                    onPlayTrack = { appViewModel.play(it) },
                    onMore = { openTrackMenu(it) },
                    downloads = state.downloads.values.toList(),
                    onCancelDownload = appViewModel::cancelDownload,
                    onDismissDownload = appViewModel::dismissDownload,
                )
            }
            composable("settings") {
                SettingsScreen(
                    onBack = { navController.popBackStack() },
                    onOpenDownloads = { navController.navigate("downloads") },
                    themeMode = themeMode,
                    onThemeModeChange = onThemeModeChange,
                    uiStyle = uiStyle,
                    onUiStyleChange = onUiStyleChange,
                    downloadedCount = state.library.downloadedCount,
                    storageLabel = state.library.storageLabel,
                    profileName = profileName,
                    avatarPath = avatarPath,
                    onProfileNameChange = {
                        ActionLog.log("profile.name")
                        ProfileStore.setName(it)
                    },
                    onDedupe = appViewModel::dedupeLibrary,
                    onRefreshArtwork = appViewModel::refreshArtwork,
                    onAvatarSelected = { uri ->
                        ActionLog.log("profile.avatar")
                        appScope.launch {
                            val path = ProfileStore.importAvatar(context, uri)
                            if (path != null && socialState.loggedIn) {
                                socialViewModel.updateAvatar(ProfileStore.avatarDataUrl())
                            }
                        }
                    },
                    socialLoggedIn = socialState.loggedIn,
                    socialBusy = socialState.busy,
                    socialConnecting = socialState.connecting,
                    socialUser = socialState.userName,
                    socialServer = socialState.serverUrl,
                    socialClientKey = socialState.clientKey,
                    socialInvite = socialState.invite,
                    socialError = socialState.error,
                    socialStorageEncrypted = socialState.storageEncrypted,
                    onSocialLogin = socialViewModel::login,
                    onSocialLogout = socialViewModel::logout,
                    updateContent = {
                        UpdateCard(
                            viewModel = viewModel(),
                            modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                        )
                    },
                )
            }
            composable(
                route = "now",
                enterTransition = { slideInVertically(initialOffsetY = { it }) },
                exitTransition = { slideOutVertically(targetOffsetY = { it }) },
                popEnterTransition = { slideInVertically(initialOffsetY = { it }) },
                popExitTransition = { slideOutVertically(targetOffsetY = { it }) },
            ) {
                NowPlayingScreen(
                    player = state.player,
                    onClose = {
                        if (!navController.popBackStack()) {
                            navController.navigate("home") { launchSingleTop = true }
                        }
                    },
                    onTogglePlay = appViewModel::togglePlay,
                    onNext = appViewModel::next,
                    onPrevious = appViewModel::previous,
                    onSeek = appViewModel::seekTo,
                    onShuffle = appViewModel::toggleShuffle,
                    onRepeat = appViewModel::cycleRepeat,
                    onLike = { state.player.track?.let { appViewModel.toggleFavorite(it.key) } },
                    onDownload = { state.player.track?.let { appViewModel.toggleDownload(it.key) } },
                    onQueue = { navController.navigate("queue") },
                    onRetryLyrics = appViewModel::retryLyrics,
                )
            }
            // Compatibilidad con back stacks antiguos que abrían la letra aparte.
            composable("lyrics") {
                androidx.compose.runtime.LaunchedEffect(Unit) {
                    navController.navigate("now") {
                        popUpTo("lyrics") { inclusive = true }
                        launchSingleTop = true
                    }
                }
                androidx.compose.foundation.layout.Box(Modifier)
            }
            composable(
                route = "queue",
                enterTransition = { slideInVertically(initialOffsetY = { it }) },
                popExitTransition = { slideOutVertically(targetOffsetY = { it }) },
            ) {
                QueueScreen(
                    player = state.player,
                    onBack = { navController.popBackStack() },
                    onPlayTrack = { index ->
                        appViewModel.playQueueItem(index)
                        navController.popBackStack("now", inclusive = false)
                    },
                    onMore = { track, index -> openTrackMenu(track, queueIndex = index) },
                )
            }
        }
    }
    }

    menuTrack?.let { track ->
        val currentPlaylistId = menuPlaylistId
        val currentQueueIndex = menuQueueIndex
        TrackActionsSheet(
            track = track,
            playlistId = currentPlaylistId,
            queueIndex = currentQueueIndex,
            queueSize = state.player.queue.size,
            inLibrary = state.library.tracks.any { it.key == track.key },
            onDismiss = {
                menuTrack = null
                menuPlaylistId = null
                menuQueueIndex = null
            },
            onPlay = {
                menuTrack = null
                if (currentQueueIndex != null) {
                    appViewModel.playQueueItem(currentQueueIndex)
                    navController.navigate("now") { launchSingleTop = true }
                } else {
                    appViewModel.play(track)
                }
            },
            onDownload = {
                menuTrack = null
                appViewModel.toggleDownload(track.key)
            },
            onFavorite = {
                menuTrack = null
                appViewModel.toggleFavorite(track.key)
            },
            onRemoveFromLibrary = {
                menuTrack = null
                appViewModel.removeFromLibrary(track.key)
            },
            onOpenArtist = {
                menuTrack = null
                val name = track.artist.substringBefore(",").substringBefore(" feat.").trim()
                val ref = appViewModel.artistRef(name)
                appViewModel.cacheArtist(name)
                navController.navigate(artistRoute(name, ref?.provider, ref?.providerId))
            },
            onOpenAlbum = {
                menuTrack = null
                appViewModel.albumFor(track)?.let { album ->
                    appViewModel.cacheAlbum(album)
                    navController.navigate(albumRoute(album))
                }
            },
            onAddToPlaylist = {
                pickerTrack = track
                menuTrack = null
            },
            onRemoveFromPlaylist = {
                currentPlaylistId?.let { appViewModel.removeFromPlaylist(it, track.key) }
                menuTrack = null
            },
            onEnqueue = {
                menuTrack = null
                appViewModel.enqueue(track)
            },
            onMoveUp = {
                if (currentQueueIndex != null) {
                    appViewModel.moveQueueItem(currentQueueIndex, currentQueueIndex - 1)
                }
                menuTrack = null
            },
            onMoveDown = {
                if (currentQueueIndex != null) {
                    appViewModel.moveQueueItem(currentQueueIndex, currentQueueIndex + 1)
                }
                menuTrack = null
            },
            onRemoveFromQueue = {
                if (currentQueueIndex != null) {
                    appViewModel.removeQueueItem(currentQueueIndex)
                }
                menuTrack = null
            },
        )
    }

    pickerTrack?.let { track ->
        PlaylistPickerDialog(
            playlists = appViewModel.playlists(),
            onDismiss = { pickerTrack = null },
            onSelect = { playlist ->
                appViewModel.addToPlaylist(playlist.id, track.key)
                pickerTrack = null
            },
            onCreate = {
                pendingPlaylistTrack = track
                pickerTrack = null
                showCreatePlaylist = true
            },
        )
    }

    if (showCreatePlaylist) {
        CreatePlaylistDialog(
            onDismiss = {
                showCreatePlaylist = false
                pendingPlaylistTrack = null
            },
            onCreate = { name ->
                val pending = pendingPlaylistTrack
                appViewModel.createPlaylist(name) { newId ->
                    if (pending != null && newId != null) {
                        appViewModel.addToPlaylist(newId, pending.key)
                    }
                    pendingPlaylistTrack = null
                }
                showCreatePlaylist = false
            },
        )
    }
}

private fun normalizeForMatch(value: String): String =
    java.text.Normalizer.normalize(value, java.text.Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(java.util.Locale.ROOT)
        .replace(Regex("[^a-z0-9]+"), " ")
        .trim()

@Composable
private fun LoadingScreen(
    message: String,
    onBack: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize()) {
        TopBar(onBack = onBack)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                CircularProgressIndicator(color = LocalSpoteColors.current.accent)
                Spacer(Modifier.height(14.dp))
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun MessageScreen(
    title: String,
    text: String,
    onBack: () -> Unit,
    onRetry: (() -> Unit)? = null,
) {
    Column(modifier = Modifier.fillMaxSize()) {
        TopBar(onBack = onBack)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .padding(horizontal = 32.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onBackground,
                )
                Text(
                    text = text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (onRetry != null) {
                    TextButton(onClick = onRetry) { Text("Reintentar") }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TrackActionsSheet(
    track: Track,
    onDismiss: () -> Unit,
    onPlay: () -> Unit,
    onDownload: () -> Unit,
    onFavorite: () -> Unit,
    onOpenArtist: () -> Unit = {},
    onOpenAlbum: () -> Unit = {},
    onRemoveFromLibrary: () -> Unit = {},
    inLibrary: Boolean = false,
    playlistId: String? = null,
    queueIndex: Int? = null,
    queueSize: Int = 0,
    onAddToPlaylist: () -> Unit = {},
    onRemoveFromPlaylist: () -> Unit = {},
    onEnqueue: () -> Unit = {},
    onMoveUp: () -> Unit = {},
    onMoveDown: () -> Unit = {},
    onRemoveFromQueue: () -> Unit = {},
) {
    val localTrack = track.localPath != null
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(bottom = 12.dp),
        ) {
            Column(modifier = Modifier.padding(horizontal = 24.dp, vertical = 4.dp)) {
                Text(
                    text = track.title,
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                )
                Text(
                    text = track.artist,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
            Spacer(Modifier.height(6.dp))
            SheetAction(
                icon = Icons.Rounded.PlayArrow,
                text = "Reproducir",
                onClick = onPlay,
            )
            SheetAction(
                icon = Icons.Rounded.Person,
                text = "Ir al artista",
                onClick = onOpenArtist,
            )
            SheetAction(
                icon = Icons.Rounded.Album,
                text = "Ir al álbum",
                enabled = !track.album.isNullOrBlank(),
                onClick = onOpenAlbum,
            )
            SheetAction(
                icon = Icons.Rounded.QueueMusic,
                text = "Añadir a la cola",
                onClick = onEnqueue,
            )
            SheetAction(
                icon = Icons.Rounded.PlaylistAdd,
                text = if (localTrack) "Añadir a playlist" else "Descárgala para añadirla a una playlist",
                enabled = localTrack,
                onClick = onAddToPlaylist,
            )
            if (playlistId != null && localTrack) {
                SheetAction(
                    icon = Icons.Rounded.DeleteOutline,
                    text = "Quitar de esta playlist",
                    onClick = onRemoveFromPlaylist,
                )
            }
            if (queueIndex != null) {
                SheetAction(
                    icon = Icons.Rounded.ArrowUpward,
                    text = "Mover arriba",
                    enabled = queueIndex > 0,
                    onClick = onMoveUp,
                )
                SheetAction(
                    icon = Icons.Rounded.ArrowDownward,
                    text = "Mover abajo",
                    enabled = queueIndex < queueSize - 1,
                    onClick = onMoveDown,
                )
                SheetAction(
                    icon = Icons.Rounded.DeleteOutline,
                    text = "Quitar de la cola",
                    onClick = onRemoveFromQueue,
                )
            }
            SheetAction(
                icon = if (track.downloaded) Icons.Rounded.DownloadDone else Icons.Rounded.Download,
                text = when {
                    track.downloaded -> "Ya está descargada"
                    track.isRemote -> "Descargar"
                    else -> "Ya está en el dispositivo"
                },
                enabled = !track.downloaded && track.isRemote,
                onClick = onDownload,
            )
            SheetAction(
                icon = if (track.favorite) Icons.Rounded.Favorite else Icons.Rounded.FavoriteBorder,
                text = if (track.favorite) "Quitar de favoritas" else "Añadir a favoritas",
                onClick = onFavorite,
            )
            if (inLibrary) {
                SheetAction(
                    icon = Icons.Rounded.DeleteOutline,
                    text = "Quitar de la biblioteca",
                    onClick = onRemoveFromLibrary,
                )
            }
        }
    }
}

@Composable
private fun CreatePlaylistDialog(
    onDismiss: () -> Unit,
    onCreate: (String) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Nueva playlist") },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text("Nombre") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(
                enabled = name.isNotBlank(),
                onClick = { onCreate(name.trim()) },
            ) { Text("Crear") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancelar") }
        },
        containerColor = MaterialTheme.colorScheme.surface,
    )
}

@Composable
private fun PlaylistPickerDialog(
    playlists: List<Playlist>,
    onDismiss: () -> Unit,
    onSelect: (Playlist) -> Unit,
    onCreate: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Añadir a playlist") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 320.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                if (playlists.isEmpty()) {
                    Text(
                        text = "Todavía no tienes playlists.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                playlists.forEach { playlist ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(playlist) }
                            .padding(vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = playlist.name,
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            text = "${playlist.trackCount}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onCreate) {
                Icon(Icons.Rounded.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Nueva")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancelar") }
        },
        containerColor = MaterialTheme.colorScheme.surface,
    )
}

@Composable
private fun SheetAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    text: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
) {
    val color = if (enabled) {
        MaterialTheme.colorScheme.onSurface
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 24.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = color,
            modifier = Modifier.size(22.dp),
        )
        Spacer(Modifier.width(16.dp))
        Text(text = text, style = MaterialTheme.typography.bodyLarge, color = color)
    }
}

