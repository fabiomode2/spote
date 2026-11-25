package dev.spote.app.ui

import android.app.Application
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.spote.app.BuildConfig
import dev.spote.app.artwork.ArtworkResolver
import dev.spote.app.data.CatalogResult
import dev.spote.app.data.DeezerCatalog
import dev.spote.app.data.FavoriteStore
import dev.spote.app.data.MockData
import dev.spote.app.data.EmptySpoteRepository
import dev.spote.app.data.MockSpoteRepository
import dev.spote.app.data.RustSpoteRepository
import dev.spote.app.data.SearchHistory
import dev.spote.app.data.SpoteRepository
import dev.spote.app.logging.ActionLog
import dev.spote.app.lyrics.LyricsRepository
import dev.spote.app.model.Album
import dev.spote.app.model.AppUiState
import dev.spote.app.model.DownloadStatus
import dev.spote.app.model.DownloadUi
import dev.spote.app.model.HomeState
import dev.spote.app.model.LibraryFilter
import dev.spote.app.model.LibraryState
import dev.spote.app.model.LyricLine
import dev.spote.app.model.Playlist
import dev.spote.app.model.PlayerState
import dev.spote.app.model.RepeatMode
import dev.spote.app.model.SearchState
import dev.spote.app.model.Tab
import dev.spote.app.model.Track
import dev.spote.app.playback.PlaybackConnection
import dev.spote.app.playback.PlaybackTrack
import dev.spote.app.playback.PlaybackUiState
import dev.spote.app.remote.RemoteDownloader
import dev.spote.app.remote.RemoteResolver
import dev.spote.app.remote.RemoteTrack
import dev.spote.app.remote.ResolvedAudio
import dev.spote.app.rust.RustBridge
import dev.spote.app.rust.coverFor
import java.io.File
import java.net.URI
import java.util.Locale
import java.util.UUID
import kotlin.math.abs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class AppViewModel(application: Application) : AndroidViewModel(application) {

    private val bridge: RustBridge? =
        if (BuildConfig.USE_MOCK_DATA) null else RustBridge.create(application).getOrNull()
    private val repo: SpoteRepository = when {
        BuildConfig.USE_MOCK_DATA -> MockSpoteRepository()
        bridge != null -> RustSpoteRepository(bridge)
        else -> EmptySpoteRepository()
    }
    private val playback = PlaybackConnection(application)
    private val lyrics = LyricsRepository(application)
    private var lyricsJob: Job? = null
    private var lyricsKey: String? = null
    private var searchJob: Job? = null
    private var remoteSearchJob: Job? = null
    private var artworkJob: Job? = null
    private val artworkAttempts = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private val downloadJobs = java.util.concurrent.ConcurrentHashMap<String, Job>()

    private data class Loaded(
        val home: HomeState,
        val search: SearchState,
        val library: LibraryState,
        val player: PlayerState,
    )

    private val remoteCache = object : LinkedHashMap<String, CatalogResult>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, CatalogResult>): Boolean =
            size > 40
    }

    private var knownLibraryRef: List<Track>? = null
    private var knownRecentRef: List<Track>? = null
    private var knownQueueRef: List<Track>? = null
    private var knownTracks: Map<String, Track> = emptyMap()

    private val _state = MutableStateFlow(
        AppUiState(
            home = HomeState(
                greeting = "Hola",
                chips = listOf("Todo", "Descargas", "Salas"),
                recent = emptyList(),
                rooms = emptyList(),
                playlists = emptyList(),
                albums = emptyList(),
            ),
            search = SearchState(),
            library = LibraryState(storageLabel = "Local"),
            player = PlayerState(),
        ),
    )
    val state: StateFlow<AppUiState> = _state.asStateFlow()

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    init {
        SearchHistory.init(application)
        FavoriteStore.init(application)
        viewModelScope.launch {
            SearchHistory.queries.collect { recent ->
                _state.update { it.copy(search = it.search.copy(recentQueries = recent)) }
            }
        }
        playback.connect(viewModelScope)
        viewModelScope.launch {
            val startedAt = System.currentTimeMillis()
            val loaded = withContext(Dispatchers.IO) {
                val home = repo.home()
                Loaded(
                    home = home,
                    search = repo.search(""),
                    library = repo.library(),
                    player = home.recent.firstOrNull()?.let(repo::playerFor) ?: PlayerState(),
                )
            }
            ActionLog.log(
                "library.load",
                "ms" to (System.currentTimeMillis() - startedAt),
                "tracks" to loaded.library.tracks.size,
                "albums" to loaded.library.albums.size,
                "playlists" to loaded.library.playlists.size,
                "mock" to BuildConfig.USE_MOCK_DATA,
            )
            _state.update {
                it.copy(
                    home = loaded.home,
                    search = loaded.search.copy(recentQueries = it.search.recentQueries),
                    library = loaded.library,
                    player = loaded.player,
                )
            }
            backfillArtwork()
        }
        viewModelScope.launch {
            playback.state.collect { playbackState ->
                if (playbackState.current != null) {
                    _state.update { ui -> ui.copy(player = playbackState.toPlayerState(ui.player)) }
                    _state.value.player.track?.let(::requestLyrics)
                }
            }
        }
    }

    override fun onCleared() {
        playback.release()
        super.onCleared()
    }

    fun setTab(tab: Tab) {
        _state.update { it.copy(tab = tab) }
    }

    fun setQuery(query: String) {
        searchJob?.cancel()
        remoteSearchJob?.cancel()
        val trimmed = query.trim()
        _state.update {
            it.copy(
                search = it.search.copy(
                    query = query,
                    remoteLoading = trimmed.length >= 2,
                    remoteError = null,
                ),
            )
        }
        searchJob = viewModelScope.launch {
            delay(60)
            val local = withContext(Dispatchers.IO) { repo.search(query) }
            _state.update { current ->
                if (current.search.query != query) return@update current
                current.copy(
                    search = local.copy(
                        remoteResults = current.search.remoteResults,
                        remoteAlbums = current.search.remoteAlbums,
                        remoteArtists = current.search.remoteArtists,
                        remoteLoading = current.search.remoteLoading,
                        remoteError = null,
                        recentQueries = current.search.recentQueries,
                    ),
                )
            }
        }
        if (trimmed.length < 2) return

        val normalized = trimmed.lowercase(Locale.ROOT)
        remoteCache[normalized]?.let { cached ->
            val refreshed = withFavorites(cached)
            _state.update {
                it.copy(
                    search = it.search.copy(
                        remoteResults = refreshed.tracks,
                        remoteAlbums = refreshed.albums,
                        remoteArtists = refreshed.artists,
                        remoteLoading = false,
                    ),
                )
            }
            return
        }

        remoteSearchJob = viewModelScope.launch {
            delay(350)
            if (_state.value.search.query.trim() != trimmed) return@launch
            _state.update { it.copy(search = it.search.copy(remoteLoading = true)) }
            val startedAt = System.currentTimeMillis()
            val result = runCatching { DeezerCatalog.search(trimmed, 25) }
            val catalog = withFavorites(result.getOrDefault(CatalogResult()))
            if (result.isSuccess) remoteCache[normalized] = catalog
            ActionLog.log(
                "search.catalog",
                "query" to trimmed,
                "tracks" to catalog.tracks.size,
                "albums" to catalog.albums.size,
                "artists" to catalog.artists.size,
                "ms" to (System.currentTimeMillis() - startedAt),
                "error" to result.exceptionOrNull()?.message,
            )
            _state.update { current ->
                if (current.search.query.trim() != trimmed) return@update current
                current.copy(
                    search = current.search.copy(
                        remoteResults = catalog.tracks,
                        remoteAlbums = catalog.albums,
                        remoteArtists = catalog.artists,
                        remoteLoading = false,
                        remoteError = result.exceptionOrNull()?.message,
                    ),
                )
            }
        }
    }

    private fun withFavorites(catalog: CatalogResult): CatalogResult {
        if (catalog.tracks.isEmpty()) return catalog
        val tracks = catalog.tracks.map { track ->
            track.copy(favorite = FavoriteStore.isFavorite(track.key))
        }
        val byKey = tracks.associateBy { it.key }
        val albums = catalog.albums.map { album ->
            album.copy(tracks = album.tracks.map { byKey[it.key] ?: it })
        }
        return catalog.copy(tracks = tracks, albums = albums)
    }

    fun commitSearch(query: String) {
        SearchHistory.record(query)
    }

    fun clearSearch() {
        searchJob?.cancel()
        remoteSearchJob?.cancel()
        remoteCache.clear()
        _state.update {
            it.copy(search = SearchState(query = "", recentQueries = it.search.recentQueries))
        }
    }

    fun clearSearchHistory() {
        SearchHistory.clear()
    }

    fun setFilter(filter: LibraryFilter) {
        _state.update { it.copy(library = it.library.copy(filter = filter)) }
    }

    fun toggleGrid() {
        _state.update { it.copy(library = it.library.copy(isGrid = !it.library.isGrid)) }
    }

    private val albumCache = mutableMapOf<String, Album>()
    private val artistTracksCache = mutableMapOf<String, List<Track>>()
    private val artistAlbumsCache = mutableMapOf<String, List<Album>>()

    fun cacheAlbum(album: Album) {
        albumCache[album.id] = album
    }

    fun album(id: String): Album? {
        albumCache[id]?.let { return it }
        val current = _state.value
        return current.library.albums.firstOrNull { it.id == id }
            ?: current.home.albums.firstOrNull { it.id == id }
    }

    fun cacheArtist(name: String) {
        val current = _state.value
        val key = normalizeName(name)
        artistTracksCache[key] = (
            current.library.tracks +
                current.home.recent +
                current.search.results +
                current.search.remoteResults +
                current.player.queue
            )
            .filter { normalizeName(it.artist) == key }
            .distinctBy { it.key }
        artistAlbumsCache[key] = (
            current.library.albums +
                current.home.albums +
                current.search.albums +
                current.search.remoteAlbums
            )
            .filter { normalizeName(it.artist) == key }
            .distinctBy { it.id }
    }

    fun cachedArtistTracks(name: String): List<Track> =
        artistTracksCache[normalizeName(name)].orEmpty()

    fun cachedArtistAlbums(name: String): List<Album> =
        artistAlbumsCache[normalizeName(name)].orEmpty()

    private fun normalizeName(value: String): String =
        java.text.Normalizer.normalize(value, java.text.Normalizer.Form.NFD)
            .replace(Regex("\\p{M}+"), "")
            .lowercase(Locale.ROOT)
            .trim()

    fun playlist(id: String): Playlist? {
        val current = _state.value
        return (current.library.playlists + current.home.playlists).firstOrNull { it.id == id }
    }

    fun play(track: Track) {
        ensureArtwork(track)
        ActionLog.log(
            "play.track",
            "key" to track.key,
            "title" to track.title,
            "artist" to track.artist,
            "remote" to (track.remoteUrl != null),
        )
        if (track.isRemote && track.localPath == null) {
            playRemote(track)
            return
        }
        val next = repo.playerFor(track).copy(isPlaying = false)
        _state.update { it.copy(player = next) }
        val playable = next.queue.mapNotNull { it.toPlaybackTrack() }
        if (playable.isEmpty()) {
            viewModelScope.launch {
                _messages.emit("Esta pista no tiene archivo local todavía")
            }
            return
        }
        val index = playable.indexOfFirst { it.key == track.key }.coerceAtLeast(0)
        requestLyrics(track)
        playback.playQueue(playable, index)
    }

    /**
     * Resuelve una pista de catálogo a audio real. Si viene de Deezer no hay
     * URL de audio: se busca su equivalente en YouTube y se resuelve al momento.
     */
    private suspend fun resolveRemoteAudio(track: Track): ResolvedAudio {
        track.remoteUrl?.let { return RemoteResolver.resolveUrl(it) }
        val query = track.remoteQuery ?: "${track.artist} ${track.title}"
        val candidates = RemoteResolver.search(query, 10)
        if (candidates.isEmpty()) {
            throw IllegalStateException("sin resultados para «$query»")
        }
        val best = candidates.maxByOrNull { candidate ->
            var score = 0
            val wantedTitle = track.title.lowercase(Locale.ROOT)
            val wantedArtist = track.artist.lowercase(Locale.ROOT)
            val offeredTitle = candidate.title.lowercase(Locale.ROOT)
            val offeredArtist = candidate.artist.lowercase(Locale.ROOT)
            if (offeredTitle == wantedTitle) score += 120
            else if (offeredTitle.contains(wantedTitle) || wantedTitle.contains(offeredTitle)) score += 60
            if (offeredArtist.contains(wantedArtist) || wantedArtist.contains(offeredArtist)) score += 40
            if (track.durationMs > 0L && candidate.durationMs > 0L) {
                score -= (abs(track.durationMs - candidate.durationMs) / 1000L).toInt()
            }
            score
        } ?: candidates.first()
        return RemoteResolver.resolve(best)
    }

    private fun playRemote(track: Track) {
        viewModelScope.launch {
            _messages.emit("Resolviendo «${track.title}»…")
            val result = runCatching { resolveRemoteAudio(track) }
            val audio = result.getOrNull()
            if (audio == null) {
                _messages.emit(
                    "No se pudo reproducir: ${result.exceptionOrNull()?.message ?: "error de red"}",
                )
                return@launch
            }
            val resolved = track.copy(
                streamUrl = audio.url,
                streamMimeType = audio.mimeType,
                album = track.album ?: audio.album,
                durationMs = if (track.durationMs > 0L) track.durationMs else audio.durationMs,
                cover = track.cover.remoteUrl?.let { track.cover }
                    ?: coverFor(audio.title, null, audio.thumbnailUrl),
                sourceLabel = track.sourceLabel ?: "YouTube",
            )
            _state.update { ui ->
                ui.copy(
                    player = PlayerState(
                        track = resolved,
                        isPlaying = true,
                        positionMs = 0L,
                        durationMs = resolved.durationMs,
                        shuffle = ui.player.shuffle,
                        repeat = ui.player.repeat,
                        queue = listOf(resolved),
                    ),
                )
            }
            requestLyrics(resolved)
            val playbackTrack = resolved.toPlaybackTrack()
            if (playbackTrack == null) {
                _messages.emit("El stream resuelto no es reproducible")
                return@launch
            }
            playback.playQueue(listOf(playbackTrack), 0)
        }
    }

    private fun requestLyrics(track: Track) {
        if (track.key == lyricsKey) return
        lyricsKey = track.key
        lyricsJob?.cancel()
        _state.update { ui ->
            if (ui.player.track?.key == track.key) {
                ui.copy(
                    player = ui.player.copy(
                        lyrics = emptyList(),
                        lyricsSynced = false,
                        lyricsSource = null,
                        lyricsLoading = true,
                    ),
                )
            } else {
                ui
            }
        }
        lyricsJob = viewModelScope.launch {
            val doc = lyrics.forTrack(
                key = track.key,
                title = track.title,
                artist = track.artist,
                album = track.album,
                durationMs = track.durationMs,
            )
            val lines = doc?.lines.orEmpty().map { LyricLine(it.startMs, it.text) }
            _state.update { ui ->
                if (ui.player.track?.key == track.key) {
                    ui.copy(
                        player = ui.player.copy(
                            lyrics = lines,
                            lyricsSynced = doc?.synced == true,
                            lyricsSource = doc?.source,
                            lyricsLoading = false,
                        ),
                    )
                } else {
                    ui
                }
            }
        }
    }

    fun retryLyrics() {
        val track = _state.value.player.track ?: return
        lyricsKey = null
        requestLyrics(track)
    }

    fun togglePlay() {
        if (playback.connected.value) {
            playback.togglePlayPause()
        } else {
            _state.update { it.copy(player = it.player.copy(isPlaying = !it.player.isPlaying)) }
        }
    }

    fun next() {
        if (playback.connected.value) {
            playback.next()
            return
        }
        _state.update { state ->
            val player = state.player
            val queue = player.queue
            if (queue.isEmpty()) return@update state
            val index = queue.indexOfFirst { it.key == player.track?.key }
            val nextIndex = when {
                index == -1 -> 0
                index + 1 < queue.size -> index + 1
                player.repeat == RepeatMode.All -> 0
                else -> queue.lastIndex
            }
            val nextTrack = queue[nextIndex]
            state.copy(
                player = player.copy(
                    track = nextTrack,
                    positionMs = 0L,
                    durationMs = nextTrack.durationMs,
                    isPlaying = true,
                ),
            )
        }
    }

    fun previous() {
        if (playback.connected.value) {
            playback.previous()
            return
        }
        _state.update { state ->
            val player = state.player
            if (player.positionMs > 3_000L) {
                return@update state.copy(player = player.copy(positionMs = 0L))
            }
            val queue = player.queue
            if (queue.isEmpty()) return@update state
            val index = queue.indexOfFirst { it.key == player.track?.key }
            val prevIndex = if (index <= 0) queue.lastIndex else index - 1
            val prev = queue[prevIndex]
            state.copy(
                player = player.copy(
                    track = prev,
                    positionMs = 0L,
                    durationMs = prev.durationMs,
                    isPlaying = true,
                ),
            )
        }
    }

    fun seekTo(positionMs: Long) {
        _state.update { state ->
            state.copy(player = state.player.copy(positionMs = positionMs.coerceIn(0L, state.player.durationMs)))
        }
        if (playback.connected.value) playback.seekTo(positionMs)
    }

    fun toggleShuffle() {
        val next = !_state.value.player.shuffle
        _state.update { it.copy(player = it.player.copy(shuffle = next)) }
        if (playback.connected.value) playback.setShuffle(next)
    }

    fun cycleRepeat() {
        val next = when (_state.value.player.repeat) {
            RepeatMode.Off -> RepeatMode.All
            RepeatMode.All -> RepeatMode.One
            RepeatMode.One -> RepeatMode.Off
        }
        _state.update { it.copy(player = it.player.copy(repeat = next)) }
        if (playback.connected.value) playback.cycleRepeat()
    }

    fun toggleFavorite(key: String) {
        val current = findTrack(key) ?: return
        val next = !current.favorite
        ActionLog.log("favorite.toggle", "track" to key, "next" to next)
        if (current.isRemote) {
            FavoriteStore.toggle(key)
            _state.update { state -> state.mapTrack(key) { it.copy(favorite = next) } }
            knownTracks = knownTracks + (key to current.copy(favorite = next))
            return
        }
        _state.update { state -> state.mapTrack(key) { it.copy(favorite = next) } }
        knownTracks = knownTracks + (key to current.copy(favorite = next))
        if (bridge != null) {
            viewModelScope.launch(Dispatchers.IO) {
                val result = runCatching { bridge.toggleFavorite(key) }
                ActionLog.log(
                    "favorite.result",
                    "track" to key,
                    "ok" to result.isSuccess,
                    "stored" to result.getOrNull(),
                    "error" to result.exceptionOrNull()?.message,
                )
                refreshFromRepo()
            }
        }
    }

    fun toggleDownload(key: String) {
        val track = findTrack(key) ?: return
        if (track.isRemote) {
            if (track.downloaded) {
                viewModelScope.launch { _messages.emit("«${track.title}» ya está descargada") }
            } else {
                downloadRemote(track)
            }
            return
        }
        viewModelScope.launch {
            _messages.emit("«${track.title}» ya está en el dispositivo")
        }
    }

    /** Descarga una pista remota, la indexa en el SQLite del core y refresca la UI. */
    private fun downloadRemote(track: Track) {
        if (downloadJobs.containsKey(track.key)) return

        fun publish(progress: Float, status: DownloadStatus, error: String? = null) {
            _state.update { current ->
                current.copy(
                    downloads = current.downloads + (
                        track.key to DownloadUi(
                            key = track.key,
                            title = track.title,
                            progress = progress.coerceIn(0f, 1f),
                            status = status,
                            error = error,
                        )
                        ),
                )
            }
        }

        val job = viewModelScope.launch {
            ActionLog.log("download.start", "key" to track.key, "title" to track.title)
            _messages.emit("Descargando «${track.title}»…")
            publish(0f, DownloadStatus.Resolving)
            val audio = runCatching { resolveRemoteAudio(track) }
                .getOrElse { error ->
                    publish(0f, DownloadStatus.Failed, error.message ?: "error de red")
                    return@launch
                }
            if (audio.isHls) {
                publish(0f, DownloadStatus.Failed, "Solo está en HLS: se puede reproducir, no descargar")
                return@launch
            }
            val artPath = ArtworkResolver.downloadThumbnail(
                context = getApplication(),
                key = track.key,
                url = track.cover.remoteUrl ?: audio.thumbnailUrl,
            )
            publish(0f, DownloadStatus.Downloading)
            val dir = File(getApplication<Application>().filesDir, "media").apply { mkdirs() }
            var lastProgress = -1f
            val file = RemoteDownloader.download(audio, dir) { progress ->
                if (progress - lastProgress >= 0.01f || progress >= 1f) {
                    lastProgress = progress
                    publish(progress, DownloadStatus.Downloading)
                }
            }.getOrElse { error ->
                publish(lastProgress.coerceAtLeast(0f), DownloadStatus.Failed, error.message ?: "error de red")
                return@launch
            }
            val engine = bridge
            if (engine == null) {
                publish(1f, DownloadStatus.Failed, "La biblioteca local no está disponible")
                return@launch
            }
            publish(1f, DownloadStatus.Importing)
            val newKey = withContext(Dispatchers.IO) {
                engine.importLocal(
                    path = file.absolutePath,
                    title = track.title.ifBlank { audio.title },
                    artist = track.artist.ifBlank { audio.artist },
                    album = track.album ?: audio.album,
                    durationMs = if (track.durationMs > 0L) track.durationMs else audio.durationMs,
                    artPath = artPath,
                )
            }
            if (FavoriteStore.isFavorite(track.key)) {
                withContext(Dispatchers.IO) {
                    runCatching { engine.toggleFavorite(newKey) }
                }
                FavoriteStore.remove(track.key)
            }
            refreshFromRepo()
            _state.update { current ->
                current.copy(
                    downloads = current.downloads - track.key,
                    search = current.search.copy(
                        remoteResults = current.search.remoteResults.map { item ->
                            if (item.key == track.key) item.copy(downloaded = true) else item
                        },
                    ),
                )
            }
            val label = audio.title.ifBlank { track.title }
            ActionLog.log(
                "download.done",
                "key" to track.key,
                "title" to label,
                "bytes" to file.length(),
                "hls" to audio.isHls,
                "art" to (artPath != null),
            )
            _messages.emit("Descargada «$label»")
        }
        downloadJobs[track.key] = job
        job.invokeOnCompletion { downloadJobs.remove(track.key) }
    }

    /** Fusiona duplicados del core con copia de seguridad previa. */
    fun dedupeLibrary() {
        viewModelScope.launch {
            val count = withContext(Dispatchers.IO) { repo.duplicateCount() }
            if (count <= 0L) {
                _messages.emit("No hay canciones duplicadas")
                return@launch
            }
            val backupDir = File(getApplication<Application>().filesDir, "backups").apply { mkdirs() }
            val backup = File(backupDir, "spote-before-dedupe-${System.currentTimeMillis()}.db")
            val backedUp = withContext(Dispatchers.IO) { repo.backupLibrary(backup.absolutePath) }
            if (!backedUp) {
                _messages.emit("No se pudo crear la copia de seguridad")
                return@launch
            }
            backupDir.listFiles()
                ?.filter { it.name.startsWith("spote-before-dedupe-") }
                ?.sortedByDescending { it.lastModified() }
                ?.drop(3)
                ?.forEach { runCatching { it.delete() } }
            val merged = withContext(Dispatchers.IO) { repo.mergeDuplicates() }
            ActionLog.log(
                "library.dedupe",
                "candidates" to count,
                "merged" to merged,
                "backup" to backup.absolutePath,
            )
            refreshFromRepo()
            _messages.emit(
                if (merged > 0) "Fusionadas $merged canciones duplicadas" else "No se pudo fusionar",
            )
        }
    }

    fun cancelDownload(key: String) {
        downloadJobs.remove(key)?.cancel()
        _state.update { it.copy(downloads = it.downloads - key) }
        viewModelScope.launch { _messages.emit("Descarga cancelada") }
    }

    fun dismissDownload(key: String) {
        _state.update { it.copy(downloads = it.downloads - key) }
    }

    /** Rellena portadas que falten desde iTunes, en background y de una en una. */
    private fun backfillArtwork(priorityKey: String? = null) {
        val engine = bridge ?: return
        if (artworkJob?.isActive == true) return
        artworkJob = viewModelScope.launch {
            val missing = withContext(Dispatchers.IO) {
                runCatching { engine.tracksWithoutArt(80) }.getOrDefault(emptyList())
            }
            val ordered = missing.sortedByDescending { it.key == priorityKey }
            var changed = 0
            for (item in ordered.take(40)) {
                if (!artworkAttempts.add(item.key)) continue
                val art = runCatching {
                    ArtworkResolver.resolve(
                        context = getApplication(),
                        key = item.key,
                        title = item.title,
                        artist = item.artist,
                        album = item.album,
                        durationMs = item.durationMs,
                    )
                }.getOrNull()
                if (art != null && (art.localPath != null || art.remoteUrl != null)) {
                    val stored = withContext(Dispatchers.IO) {
                        runCatching {
                            engine.setTrackArt(item.key, art.localPath, art.remoteUrl)
                            engine.updateTrackMetadata(
                                item.key,
                                item.artist,
                                item.title,
                                art.album,
                                item.durationMs,
                                art.remoteUrl,
                            )
                        }.isSuccess
                    }
                    if (stored) {
                        changed++
                        _state.update { current ->
                            current.mapTrack(item.key) { track ->
                                track.copy(
                                    cover = coverFor(track.title, art.localPath, art.remoteUrl),
                                )
                            }
                        }
                        ActionLog.log(
                            "artwork.done",
                            "title" to item.title,
                            "artist" to item.artist,
                            "local" to (art.localPath != null),
                        )
                    }
                } else {
                    ActionLog.log("artwork.miss", "title" to item.title, "artist" to item.artist)
                }
                delay(180)
            }
            ActionLog.log(
                "artwork.batch",
                "missing" to missing.size,
                "changed" to changed,
                "attempted" to artworkAttempts.size,
            )
            if (changed > 0) refreshFromRepo()
        }
    }

    fun refreshArtwork() {
        artworkAttempts.clear()
        backfillArtwork(priorityKey = _state.value.player.track?.key)
    }

    private fun ensureArtwork(track: Track) {
        if (track.cover.localPath != null || track.cover.remoteUrl != null) return
        if (bridge == null) return
        if (!artworkAttempts.add(track.key)) return
        viewModelScope.launch {
            val art = runCatching {
                ArtworkResolver.resolve(
                    context = getApplication(),
                    key = track.key,
                    title = track.title,
                    artist = track.artist,
                    album = track.album,
                    durationMs = track.durationMs,
                )
            }.getOrNull() ?: return@launch
            val stored = withContext(Dispatchers.IO) {
                runCatching {
                    bridge.setTrackArt(track.key, art.localPath, art.remoteUrl)
                    bridge.updateTrackMetadata(
                        track.key,
                        track.artist,
                        track.title,
                        art.album,
                        track.durationMs,
                        art.remoteUrl,
                    )
                }.isSuccess
            }
            if (!stored) return@launch
            ActionLog.log("artwork.done", "title" to track.title, "artist" to track.artist)
            _state.update { current ->
                current.mapTrack(track.key) { item ->
                    item.copy(cover = coverFor(item.title, art.localPath, art.remoteUrl))
                }
            }
            refreshFromRepo()
        }
    }

    private fun findTrack(key: String): Track? {
        val current = _state.value
        return (current.library.tracks +
            current.home.recent +
            current.search.results +
            current.search.remoteResults +
            current.player.queue +
            listOfNotNull(current.player.track))
            .firstOrNull { it.key == key }
    }

    // ------------------------------------------------------------------ listas

    fun playlists(): List<Playlist> =
        (_state.value.library.playlists + _state.value.home.playlists).distinctBy { it.id }

    fun createPlaylist(name: String, onCreated: (String?) -> Unit = {}) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        viewModelScope.launch {
            val id = withContext(Dispatchers.IO) { repo.createPlaylist(trimmed) }
            ActionLog.log("playlist.create", "name" to trimmed, "ok" to (id != null))
            if (id == null) {
                _messages.emit("Las playlists necesitan la biblioteca local")
            } else {
                refreshFromRepo()
                _messages.emit("Playlist «$trimmed» creada")
            }
            onCreated(id)
        }
    }

    fun deletePlaylist(id: String) {
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { repo.deletePlaylist(id) }
            if (ok) {
                refreshFromRepo()
                _messages.emit("Playlist eliminada")
            } else {
                _messages.emit("No se pudo eliminar la playlist")
            }
        }
    }

    fun addToPlaylist(id: String, key: String) {
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { repo.addToPlaylist(id, key) }
            refreshFromRepo()
            _messages.emit(if (ok) "Añadida a la playlist" else "No se pudo añadir")
        }
    }

    fun removeFromPlaylist(id: String, key: String) {
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { repo.removeFromPlaylist(id, key) }
            refreshFromRepo()
            _messages.emit(if (ok) "Quitada de la playlist" else "No se pudo quitar")
        }
    }

    // -------------------------------------------------------------------- cola

    fun playQueueItem(index: Int) {
        val queue = _state.value.player.queue
        val track = queue.getOrNull(index) ?: return
        if (playback.connected.value) {
            playback.playAt(index)
        }
        _state.update {
            it.copy(player = it.player.copy(track = track, positionMs = 0L, isPlaying = true))
        }
    }

    fun moveQueueItem(from: Int, to: Int) {
        val queue = _state.value.player.queue
        if (from !in queue.indices) return
        ActionLog.log("queue.move", "from" to from, "to" to to)
        val target = to.coerceIn(0, queue.lastIndex)
        if (from == target) return
        val moved = queue.toMutableList().apply { add(target, removeAt(from)) }
        _state.update { it.copy(player = it.player.copy(queue = moved)) }
        if (playback.connected.value) playback.moveInQueue(from, target)
    }

    fun removeQueueItem(index: Int) {
        val queue = _state.value.player.queue
        if (index !in queue.indices) return
        ActionLog.log("queue.remove", "index" to index)
        val removed = queue[index]
        val remaining = queue.toMutableList().apply { removeAt(index) }
        _state.update { current ->
            current.copy(
                player = current.player.copy(
                    queue = remaining,
                    track = if (current.player.track?.key == removed.key) {
                        remaining.getOrNull(index)
                    } else {
                        current.player.track
                    },
                ),
            )
        }
        if (playback.connected.value) playback.removeAt(index)
    }

    fun enqueue(track: Track) {
        ActionLog.log("queue.add", "key" to track.key, "title" to track.title)
        if (track.isRemote && track.localPath == null) {
            viewModelScope.launch {
                _messages.emit("Resolviendo «${track.title}»…")
                val audio = runCatching { resolveRemoteAudio(track) }.getOrNull()
                if (audio == null) {
                    _messages.emit("No se pudo resolver la pista")
                    return@launch
                }
                val resolved = track.copy(
                    streamUrl = audio.url,
                    streamMimeType = audio.mimeType,
                    durationMs = if (track.durationMs > 0L) track.durationMs else audio.durationMs,
                )
                val playbackTrack = resolved.toPlaybackTrack()
                if (playbackTrack == null || !playback.addToQueue(playbackTrack)) {
                    _messages.emit("La cola no está lista todavía")
                } else {
                    _messages.emit("Añadida a la cola")
                }
            }
            return
        }
        val playbackTrack = track.toPlaybackTrack()
        if (playbackTrack == null) {
            viewModelScope.launch { _messages.emit("Esta pista no tiene archivo reproducible") }
            return
        }
        if (playback.addToQueue(playbackTrack)) {
            viewModelScope.launch { _messages.emit("Añadida a la cola") }
        } else {
            viewModelScope.launch { _messages.emit("La cola no está lista todavía") }
        }
    }

    // ------------------------------------------------------------- sala social

    /** Aplica el estado de la sala: pista, posición de servidor y play/pause. */
    fun syncWithRoom(track: Track, positionMs: Long, playing: Boolean) {
        val sameTrack = _state.value.player.track?.key == track.key
        if (sameTrack && playback.connected.value) {
            playback.seekTo(positionMs)
            playback.setPlaying(playing)
            _state.update {
                it.copy(player = it.player.copy(positionMs = positionMs, isPlaying = playing))
            }
            return
        }

        if (track.isRemote && track.localPath == null) {
            viewModelScope.launch {
                val audio = runCatching { resolveRemoteAudio(track) }.getOrNull() ?: return@launch
                val resolved = track.copy(
                    streamUrl = audio.url,
                    streamMimeType = audio.mimeType,
                    album = track.album ?: audio.album,
                    durationMs = if (track.durationMs > 0L) track.durationMs else audio.durationMs,
                    cover = track.cover.remoteUrl?.let { track.cover }
                        ?: coverFor(audio.title, null, audio.thumbnailUrl),
                )
                val playbackTrack = resolved.toPlaybackTrack() ?: return@launch
                _state.update {
                    it.copy(
                        player = PlayerState(
                            track = resolved,
                            isPlaying = playing,
                            positionMs = positionMs,
                            durationMs = resolved.durationMs,
                            queue = listOf(resolved),
                        ),
                    )
                }
                requestLyrics(resolved)
                playback.playQueue(
                    listOf(playbackTrack),
                    0,
                    positionMs,
                    autoPlay = playing,
                )
            }
            return
        }

        val next = repo.playerFor(track)
        val playable = next.queue.mapNotNull { it.toPlaybackTrack() }
        if (playable.isEmpty()) return
        val index = playable.indexOfFirst { it.key == track.key }.coerceAtLeast(0)
        _state.update {
            it.copy(
                player = next.copy(
                    isPlaying = playing,
                    positionMs = positionMs,
                ),
            )
        }
        requestLyrics(track)
        playback.playQueue(playable, index, positionMs, autoPlay = playing)
    }

    /**
     * Importa audio del selector del sistema. Copia al almacenamiento privado,
     * lee los tags con MediaMetadataRetriever y lo indexa en el SQLite del core.
     */
    fun importUris(uris: List<Uri>) {
        if (uris.isEmpty() || bridge == null) return
        ActionLog.log("import.start", "files" to uris.size)
        viewModelScope.launch {
            _messages.emit("Importando ${uris.size} archivo(s)…")
            val result = withContext(Dispatchers.IO) { importBlocking(uris) }
            refreshFromRepo()
            backfillArtwork()
            ActionLog.log("import.done", "ok" to result.first, "failed" to result.second)
            val text = buildString {
                append("Importadas ${result.first} canciones")
                if (result.second > 0) append(" · ${result.second} con error")
            }
            _messages.emit(text)
        }
    }

    private fun importBlocking(uris: List<Uri>): Pair<Int, Int> {
        val engine = bridge ?: return 0 to uris.size
        val context = getApplication<Application>()
        val resolver = context.contentResolver
        val mediaDir = File(context.filesDir, "media").apply { mkdirs() }
        var imported = 0
        var failed = 0

        uris.forEach { uri ->
            var target: File? = null
            try {
                val displayName = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                    ?.use { cursor ->
                        if (cursor.moveToFirst()) cursor.getString(0) else null
                    }
                    ?: "audio"

                val mime = resolver.getType(uri)
                val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)
                    ?: displayName.substringAfterLast('.', "opus")

                val id = UUID.randomUUID().toString()
                target = File(mediaDir, "$id.$extension")
                resolver.openInputStream(uri)?.use { input ->
                    target.outputStream().use { output -> input.copyTo(output) }
                } ?: error("no se pudo abrir el archivo")

                val retriever = MediaMetadataRetriever()
                var title = displayName.substringBeforeLast('.')
                var artist = "Desconocido"
                var album: String? = null
                var durationMs = 0L
                var artPath: String? = null
                try {
                    retriever.setDataSource(context, uri)
                    retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_TITLE)
                        ?.takeIf { it.isNotBlank() }
                        ?.let { title = it }
                    retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_ARTIST)
                        ?.takeIf { it.isNotBlank() }
                        ?.let { artist = it }
                    retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_ALBUM)
                        ?.takeIf { it.isNotBlank() }
                        ?.let { album = it }
                    retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                        ?.toLongOrNull()
                        ?.let { durationMs = it }
                    retriever.embeddedPicture?.let { picture ->
                        val art = File(mediaDir, "$id.jpg")
                        art.writeBytes(picture)
                        artPath = art.absolutePath
                    }
                } finally {
                    retriever.release()
                }

                engine.importLocal(
                    path = target.absolutePath,
                    title = title,
                    artist = artist,
                    album = album,
                    durationMs = durationMs,
                    artPath = artPath,
                )
                imported++
            } catch (error: Exception) {
                failed++
                target?.delete()
            }
        }
        return imported to failed
    }

    private fun PlaybackUiState.toPlayerState(existing: PlayerState): PlayerState {
        val ui = state.value
        if (
            knownLibraryRef !== ui.library.tracks ||
            knownRecentRef !== ui.home.recent ||
            knownQueueRef !== ui.player.queue
        ) {
            knownLibraryRef = ui.library.tracks
            knownRecentRef = ui.home.recent
            knownQueueRef = ui.player.queue
            knownTracks = (ui.library.tracks + ui.home.recent + ui.player.queue)
                .associateBy { it.key }
        }
        val known = knownTracks
        fun map(pb: PlaybackTrack): Track = known[pb.key] ?: Track(
            key = pb.key,
            title = pb.title,
            artist = pb.artist,
            album = pb.album,
            durationMs = pb.durationMs,
            cover = coverFor(pb.title, pb.artUri?.toFilePath()),
            downloaded = true,
            localPath = pb.uri.toFilePath(),
        )
        val mapped = current?.let(::map)
        // La cola cambia muy poco; reutilizarla evita remapear cientos de pistas
        // en cada tick de posición (500 ms).
        val sameQueue = existing.queue.size == queue.size &&
            existing.queue.indices.all { existing.queue[it].key == queue[it].key }
        if (existing.track?.key != mapped?.key) {
            ActionLog.log(
                "playback.track",
                "key" to mapped?.key,
                "title" to mapped?.title,
                "artist" to mapped?.artist,
                "playing" to isPlaying,
            )
        }
        return PlayerState(
            track = mapped,
            isPlaying = isPlaying,
            positionMs = positionMs,
            durationMs = if (durationMs > 0L) durationMs else mapped?.durationMs ?: 0L,
            shuffle = shuffle,
            repeat = when (repeatMode) {
                1 -> RepeatMode.All
                2 -> RepeatMode.One
                else -> RepeatMode.Off
            },
            queue = if (sameQueue) existing.queue else queue.map(::map),
            lyrics = if (existing.track?.key == mapped?.key) existing.lyrics else emptyList(),
            lyricsSynced = if (existing.track?.key == mapped?.key) existing.lyricsSynced else false,
            lyricsSource = if (existing.track?.key == mapped?.key) existing.lyricsSource else null,
            lyricsLoading = if (existing.track?.key == mapped?.key) existing.lyricsLoading else false,
        )
    }

    private fun refreshFromRepo() {
        viewModelScope.launch {
            val previous = _state.value.search
            val loaded = withContext(Dispatchers.IO) {
                Loaded(
                    home = repo.home(),
                    search = repo.search(previous.query),
                    library = repo.library(),
                    player = PlayerState(),
                )
            }
            _state.update { state ->
                state.copy(
                    home = loaded.home,
                    search = loaded.search.copy(
                        remoteResults = state.search.remoteResults,
                        remoteAlbums = state.search.remoteAlbums,
                        remoteArtists = state.search.remoteArtists,
                        remoteLoading = state.search.remoteLoading,
                        remoteError = state.search.remoteError,
                        recentQueries = state.search.recentQueries,
                    ),
                    library = loaded.library,
                )
            }
        }
    }
}

private fun AppUiState.mapTrack(key: String, transform: (Track) -> Track): AppUiState {    fun List<Track>.mapTracks() = map { if (it.key == key) transform(it) else it }
    fun List<Album>.mapAlbums() = map { album -> album.copy(tracks = album.tracks.mapTracks()) }

    return copy(
        home = home.copy(
            recent = home.recent.mapTracks(),
            albums = home.albums.mapAlbums(),
            playlists = home.playlists.map { playlist -> playlist.copy(tracks = playlist.tracks.mapTracks()) },
        ),
        search = search.copy(
            results = search.results.mapTracks(),
            albums = search.albums.mapAlbums(),
        ),
        library = library.copy(
            tracks = library.tracks.mapTracks(),
            albums = library.albums.mapAlbums(),
            playlists = library.playlists.map { playlist -> playlist.copy(tracks = playlist.tracks.mapTracks()) },
        ),
        player = player.copy(
            track = player.track?.let { if (it.key == key) transform(it) else it },
            queue = player.queue.mapTracks(),
        ),
    )
}

private fun Track.toPlaybackTrack(): PlaybackTrack? {
    val resolved = localPath?.let { path ->
        if (path.contains("://")) path else File(path).toURI().toString()
    } ?: streamUrl ?: return null
    return PlaybackTrack(
        key = key,
        uri = resolved,
        title = title,
        artist = artist,
        album = album,
        artUri = cover.localPath?.let { File(it).toURI().toString() } ?: cover.remoteUrl,
        durationMs = durationMs,
        mimeType = streamMimeType,
    )
}

private fun String.toFilePath(): String? = when {
    startsWith("file://") -> runCatching { URI(this).path }.getOrNull()
    startsWith("/") -> this
    else -> null
}
