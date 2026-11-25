package dev.spote.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.DeleteOutline
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.DownloadDone
import androidx.compose.material.icons.rounded.Favorite
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.PlaylistAdd
import androidx.compose.material.icons.rounded.QueueMusic
import androidx.compose.material3.AlertDialog
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
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import dev.spote.app.model.Album
import dev.spote.app.model.Playlist
import dev.spote.app.model.Tab
import dev.spote.app.logging.ActionLog
import dev.spote.app.model.Track
import dev.spote.app.profile.ProfileStore
import dev.spote.app.ui.components.LocalArtistNavigation
import dev.spote.app.ui.components.MiniPlayer
import dev.spote.app.ui.components.SpoteBottomBar
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
import dev.spote.app.ui.update.UpdateCard

@Composable
fun SpoteApp(
    themeMode: SpoteThemeMode = SpoteThemeMode.Midnight,
    onThemeModeChange: (SpoteThemeMode) -> Unit = {},
    uiStyle: dev.spote.app.ui.theme.SpoteUiStyle = dev.spote.app.ui.theme.SpoteUiStyle.Medium,
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
    val colors = LocalSpoteColors.current
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

    val topLevelRoutes = setOf("home", "search", "now", "library")
    val fullscreenRoutes = setOf("now", "queue", "lyrics")
    val noMiniRoutes = fullscreenRoutes + setOf("settings")
    val showMini = route !in noMiniRoutes
    val showBottom = route in topLevelRoutes

    CompositionLocalProvider(
        LocalArtistNavigation provides { artist ->
            appViewModel.cacheArtist(artist)
            navController.navigate("artist/${Uri.encode(artist)}")
        },
    ) {
    Scaffold(
        containerColor = colors.background,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        bottomBar = {
            if (showMini || showBottom) {
                Column {
                    if (showMini) {
                        MiniPlayer(
                            player = state.player,
                            onExpand = { navController.navigate("now") { launchSingleTop = true } },
                            onTogglePlay = appViewModel::togglePlay,
                            onNext = appViewModel::next,
                        )
                    }
                    if (showBottom) {
                        SpoteBottomBar(
                            selected = when (route) {
                                "search" -> Tab.Search
                                "now" -> Tab.Now
                                "library" -> Tab.Library
                                else -> Tab.Home
                            },
                            onSelect = { tab ->
                                val target = when (tab) {
                                    Tab.Home -> "home"
                                    Tab.Search -> "search"
                                    Tab.Now -> "now"
                                    Tab.Library -> "library"
                                }
                                ActionLog.log("nav.tab", "from" to route, "to" to target)
                                navController.navigate(target) {
                                    popUpTo(navController.graph.findStartDestination().id) {
                                        saveState = false
                                    }
                                    launchSingleTop = true
                                }
                            },
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
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate("album/${Uri.encode(it.id)}") },
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
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate("album/${Uri.encode(it.id)}") },
                    onOpenArtist = { appViewModel.cacheArtist(it); navController.navigate("artist/${Uri.encode(it)}") },
                    onMore = { openTrackMenu(it) },
                    onClearSearch = appViewModel::clearSearch,
                    onClearHistory = appViewModel::clearSearchHistory,
                    onSearchCommit = appViewModel::commitSearch,
                )
            }
            composable("library") {
                LibraryScreen(
                    state = state.library,
                    onFilter = appViewModel::setFilter,
                    onToggleGrid = appViewModel::toggleGrid,
                    onPlayTrack = { appViewModel.play(it) },
                    onMore = { openTrackMenu(it) },
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate("album/${Uri.encode(it.id)}") },
                    onOpenPlaylist = { navController.navigate("playlist/${Uri.encode(it.id)}") },
                    onOpenDownloads = { navController.navigate("downloads") },
                    onOpenSearch = { navController.navigate("search") },
                    onOpenArtist = { artist ->
                        appViewModel.cacheArtist(artist)
                        navController.navigate("artist/${Uri.encode(artist)}")
                    },
                    onCreatePlaylist = { showCreatePlaylist = true },
                    onImport = { importLauncher.launch(arrayOf("audio/*")) },
                )
            }
            composable(
                route = "album/{id}",
                enterTransition = { slideInHorizontally(initialOffsetX = { it }) },
                exitTransition = { slideOutHorizontally(targetOffsetX = { -it / 4 }) },
                popEnterTransition = { slideInHorizontally(initialOffsetX = { -it / 4 }) },
                popExitTransition = { slideOutHorizontally(targetOffsetX = { it }) },
            ) { entry ->
                val albumId = Uri.decode(entry.arguments?.getString("id").orEmpty())
                val album = appViewModel.album(albumId)
                LaunchedEffect(albumId) {
                    ActionLog.log(
                        "album.open",
                        "id" to albumId,
                        "found" to (album != null),
                        "catalogue" to state.library.albums.size,
                    )
                }
                if (album != null) {
                    AlbumScreen(
                        album = album,
                        player = state.player,
                        onBack = { navController.popBackStack() },
                        onPlay = {
                            album.tracks.firstOrNull()?.let {
                                appViewModel.play(it)
                            }
                        },
                        onShuffle = {
                            album.tracks.shuffled().firstOrNull()?.let {
                                appViewModel.play(it)
                            }
                        },
                        onPlayTrack = { appViewModel.play(it) },
                        onMore = { openTrackMenu(it) },
                        onDownload = {
                            album.tracks.firstOrNull()?.let { appViewModel.toggleDownload(it.key) }
                        },
                        onLike = {
                            album.tracks.firstOrNull()?.let { appViewModel.toggleFavorite(it.key) }
                        },
                    )
                } else {
                    androidx.compose.foundation.layout.Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(32.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "No se encontró el álbum",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            composable(
                route = "artist/{name}",
                enterTransition = { slideInHorizontally(initialOffsetX = { it }) },
                exitTransition = { slideOutHorizontally(targetOffsetX = { -it / 4 }) },
                popEnterTransition = { slideInHorizontally(initialOffsetX = { -it / 4 }) },
                popExitTransition = { slideOutHorizontally(targetOffsetX = { it }) },
            ) { entry ->
                val rawName = entry.arguments?.getString("name").orEmpty()
                val artistName = Uri.decode(rawName)
                LaunchedEffect(artistName) {
                    ActionLog.log(
                        "artist.open",
                        "name" to artistName,
                        "tracks" to state.library.tracks.count {
                            it.artist.equals(artistName, ignoreCase = true)
                        },
                        "albums" to state.library.albums.count {
                            it.artist.equals(artistName, ignoreCase = true)
                        },
                    )
                }
                val tracks = appViewModel.cachedArtistTracks(artistName).ifEmpty {
                    (
                        state.library.tracks +
                            state.home.recent +
                            state.search.results +
                            state.search.remoteResults
                        )
                        .filter { it.artist.equals(artistName, ignoreCase = true) }
                        .distinctBy { it.key }
                }
                val albums = appViewModel.cachedArtistAlbums(artistName).ifEmpty {
                    (state.library.albums + state.home.albums + state.search.albums)
                        .filter { it.artist.equals(artistName, ignoreCase = true) }
                        .distinctBy { it.id }
                }
                ArtistScreen(
                    name = artistName,
                    tracks = tracks,
                    albums = albums,
                    player = state.player,
                    onBack = { navController.popBackStack() },
                    onPlayTrack = { appViewModel.play(it) },
                    onOpenAlbum = { appViewModel.cacheAlbum(it); navController.navigate("album/${Uri.encode(it.id)}") },
                    onMore = { openTrackMenu(it) },
                )
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
                    onClose = { navController.popBackStack() },
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TrackActionsSheet(
    track: Track,
    onDismiss: () -> Unit,
    onPlay: () -> Unit,
    onDownload: () -> Unit,
    onFavorite: () -> Unit,
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
                    track.remoteUrl != null -> "Descargar"
                    else -> "Ya está en el dispositivo"
                },
                enabled = !track.downloaded && track.remoteUrl != null,
                onClick = onDownload,
            )
            SheetAction(
                icon = if (track.favorite) Icons.Rounded.Favorite else Icons.Rounded.FavoriteBorder,
                text = if (track.favorite) "Quitar de favoritas" else "Añadir a favoritas",
                onClick = onFavorite,
            )
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

