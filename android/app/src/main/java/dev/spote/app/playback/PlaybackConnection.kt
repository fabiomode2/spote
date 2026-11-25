package dev.spote.app.playback

import android.content.ComponentName
import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Single point of contact between the UI and [SpotePlaybackService].
 *
 * A [MediaController] is bound asynchronously; until it is ready, [playQueue] requests are kept
 * and applied on connection. All public methods, [connect] and [release] must be called from the
 * main thread, as required by Media3's `MediaController` API.
 */
class PlaybackConnection(context: Context) {

    private val appContext: Context = context.applicationContext
    private val sessionToken = SessionToken(
        appContext,
        ComponentName(appContext, SpotePlaybackService::class.java),
    )

    private val _state = MutableStateFlow(PlaybackUiState())
    val state: StateFlow<PlaybackUiState> = _state.asStateFlow()

    private val _connected = MutableStateFlow(false)
    val connected: StateFlow<Boolean> = _connected.asStateFlow()

    private var controller: MediaController? = null
    private var controllerFuture: ListenableFuture<MediaController>? = null
    private var scope: CoroutineScope? = null
    private var pollJob: Job? = null
    private var pendingQueue: PendingQueue? = null

    /** Incremented on release/reset so stale connection callbacks cannot publish state. */
    private var generation = 0

    private data class PendingQueue(
        val tracks: List<PlaybackTrack>,
        val startIndex: Int,
        val startPositionMs: Long = 0L,
        val autoPlay: Boolean = true,
    )

    private val controllerListener = object : MediaController.Listener {
        override fun onDisconnected(controller: MediaController) {
            if (this@PlaybackConnection.controller === controller) {
                this@PlaybackConnection.controller = null
            }
            pollJob?.cancel()
            pollJob = null
            _connected.value = false
            _state.value = _state.value.copy(isPlaying = false)
        }
    }

    private val playerListener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) {
            refresh()
            syncPolling()
        }

        override fun onPlayerError(error: PlaybackException) {
            Log.w(TAG, "Playback error", error)
        }
    }

    /**
     * Binds the controller. Safe to call multiple times; subsequent calls are ignored while a
     * controller exists. A failed connection is logged and leaves [connected] false.
     */
    fun connect(scope: CoroutineScope) {
        if (controller != null || controllerFuture != null) return

        this.scope = scope
        val requestGeneration = ++generation
        val future = MediaController.Builder(appContext, sessionToken)
            .setListener(controllerListener)
            .buildAsync()
        controllerFuture = future

        future.addListener({
            if (controllerFuture === future) {
                controllerFuture = null
            }
            if (requestGeneration == generation) {
                try {
                    val connectedController = future.get()
                    controller = connectedController
                    connectedController.addListener(playerListener)
                    _connected.value = connectedController.isConnected

                    pendingQueue?.let { pending ->
                        pendingQueue = null
                        playQueue(
                            pending.tracks,
                            pending.startIndex,
                            pending.startPositionMs,
                            pending.autoPlay,
                        )
                    }
                    refresh()
                    syncPolling()
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to connect to the playback service", e)
                    _connected.value = false
                    _state.value = _state.value.copy(isPlaying = false)
                }
            }
        }, ContextCompat.getMainExecutor(appContext))
    }

    /** Unbinds the controller. Service playback itself is not stopped (use [stop] for that). */
    fun release() {
        generation++
        pollJob?.cancel()
        pollJob = null
        scope = null
        pendingQueue = null

        val current = controller
        controller = null
        val future = controllerFuture
        controllerFuture = null

        if (current != null) {
            current.removeListener(playerListener)
            current.release()
        } else if (future != null) {
            MediaController.releaseFuture(future)
        }

        _connected.value = false
        _state.value = PlaybackUiState()
    }

    /**
     * Replaces the queue and starts playback at [startIndex]. If the controller is still
     * connecting, the request is applied as soon as it is ready.
     */
    fun playQueue(
        tracks: List<PlaybackTrack>,
        startIndex: Int,
        startPositionMs: Long = 0L,
        autoPlay: Boolean = true,
    ) {
        if (tracks.isEmpty()) {
            stop()
            return
        }

        val player = controller
        if (player == null || !player.isConnected) {
            pendingQueue = PendingQueue(tracks, startIndex, startPositionMs, autoPlay)
            return
        }

        val mediaItems = tracks.map(::toMediaItem)
        val index = startIndex.coerceIn(0, tracks.lastIndex)
        player.setMediaItems(mediaItems, index, startPositionMs.coerceAtLeast(0L))
        player.prepare()
        player.playWhenReady = autoPlay
        refresh()
        syncPolling()
    }

    /** Salta a la pista [index] de la cola actual. */
    fun playAt(index: Int) {
        val player = controller ?: return
        if (index !in 0 until player.mediaItemCount) return
        player.seekToDefaultPosition(index)
        player.play()
        refresh()
        syncPolling()
    }

    fun setPlaying(playing: Boolean) {
        val player = controller ?: return
        if (playing) {
            if (player.playbackState == Player.STATE_IDLE) player.prepare()
            player.play()
        } else {
            player.pause()
        }
        refresh()
        syncPolling()
    }

    fun moveInQueue(from: Int, to: Int): Boolean {
        val player = controller ?: return false
        if (from !in 0 until player.mediaItemCount) return false
        val target = to.coerceIn(0, player.mediaItemCount - 1)
        if (from == target) return false
        player.moveMediaItem(from, target)
        refresh()
        return true
    }

    fun removeAt(index: Int): Boolean {
        val player = controller ?: return false
        if (index !in 0 until player.mediaItemCount) return false
        player.removeMediaItem(index)
        if (player.mediaItemCount == 0) player.stop()
        refresh()
        syncPolling()
        return true
    }

    /** Añade al final de la cola; false si el player aún no está listo. */
    fun addToQueue(track: PlaybackTrack): Boolean {
        val player = controller ?: return false
        if (!player.isConnected) return false
        player.addMediaItem(toMediaItem(track))
        refresh()
        return true
    }

    fun togglePlayPause() {
        val player = controller ?: return
        if (player.isPlaying) {
            player.pause()
        } else {
            when (player.playbackState) {
                Player.STATE_IDLE -> player.prepare()
                Player.STATE_ENDED -> player.seekToDefaultPosition()
            }
            player.play()
        }
        refresh()
        syncPolling()
    }

    fun next() {
        val player = controller ?: return
        if (player.hasNextMediaItem()) {
            player.seekToNextMediaItem()
        }
        refresh()
    }

    fun previous() {
        val player = controller ?: return
        // Standard music-player behavior: restart the current track after 3 seconds.
        if (player.currentPosition > PREVIOUS_RESTART_MS) {
            player.seekTo(0L)
        } else if (player.hasPreviousMediaItem()) {
            player.seekToPreviousMediaItem()
        } else {
            player.seekTo(0L)
        }
        refresh()
    }

    fun seekTo(ms: Long) {
        val player = controller ?: return
        val duration = player.duration
        val target = if (duration > 0L) ms.coerceIn(0L, duration) else ms.coerceAtLeast(0L)
        player.seekTo(target)
        refresh()
    }

    fun setShuffle(enabled: Boolean) {
        val player = controller ?: return
        player.shuffleModeEnabled = enabled
        refresh()
    }

    /** Cycles off -> all -> one -> off. */
    fun cycleRepeat() {
        val player = controller ?: return
        val next = when (player.repeatMode) {
            Player.REPEAT_MODE_OFF -> Player.REPEAT_MODE_ALL
            Player.REPEAT_MODE_ALL -> Player.REPEAT_MODE_ONE
            else -> Player.REPEAT_MODE_OFF
        }
        player.repeatMode = next
        refresh()
    }

    fun stop() {
        pendingQueue = null
        val player = controller ?: return
        player.stop()
        refresh()
        syncPolling()
    }

    /** Starts/stops the 500 ms position poll based on the live controller state. */
    private fun syncPolling() {
        val player = controller
        val shouldPoll = player != null && player.isConnected && player.isPlaying
        if (!shouldPoll) {
            pollJob?.cancel()
            pollJob = null
            return
        }
        if (pollJob?.isActive == true) return

        val pollScope = scope ?: return
        pollJob = pollScope.launch(Dispatchers.Main.immediate) {
            while (isActive) {
                val activePlayer = controller
                if (activePlayer == null || !activePlayer.isConnected || !activePlayer.isPlaying) break
                refresh()
                delay(POSITION_POLL_MS)
            }
        }
    }

    /** Maps Media3 state into [PlaybackUiState]. Never throws on a disconnected controller. */
    private fun refresh() {
        val player = controller
        if (player == null) {
            _connected.value = false
            return
        }

        try {
            if (!player.isConnected) {
                _connected.value = false
                _state.value = _state.value.copy(isPlaying = false)
                return
            }

            _connected.value = true
            val queue = (0 until player.mediaItemCount).map { toTrack(player.getMediaItemAt(it)) }
            val current = player.currentMediaItem?.let(::toTrack)
            val playerDuration = player.duration
            val duration = if (playerDuration > 0L) {
                playerDuration
            } else {
                current?.durationMs ?: 0L
            }

            _state.value = PlaybackUiState(
                current = current,
                isPlaying = player.isPlaying,
                positionMs = player.currentPosition.coerceAtLeast(0L),
                durationMs = duration,
                shuffle = player.shuffleModeEnabled,
                repeatMode = repeatModeToApp(player.repeatMode),
                queue = queue,
            )
        } catch (e: IllegalStateException) {
            // The controller was released or disconnected between calls.
            Log.w(TAG, "Player state is no longer available", e)
            _connected.value = false
            _state.value = _state.value.copy(isPlaying = false)
        }
    }

    private fun toMediaItem(track: PlaybackTrack): MediaItem {
        val uri = Uri.parse(track.uri)
        val metadata = MediaMetadata.Builder()
            .setTitle(track.title)
            .setArtist(track.artist)
            .setAlbumTitle(track.album)
            .setDurationMs(track.durationMs.coerceAtLeast(0L))
            .setArtworkUri(track.artUri?.takeIf { it.isNotBlank() }?.let(Uri::parse))
            .build()

        return MediaItem.Builder()
            .setMediaId(track.key)
            .setUri(uri)
            .setMimeType(track.mimeType)
            // RequestMetadata survives process/binder boundaries where localConfiguration may not.
            .setRequestMetadata(MediaItem.RequestMetadata.Builder().setMediaUri(uri).build())
            .setMediaMetadata(metadata)
            .build()
    }

    private fun toTrack(item: MediaItem): PlaybackTrack {
        val metadata = item.mediaMetadata
        val uri = item.localConfiguration?.uri ?: item.requestMetadata.mediaUri
        return PlaybackTrack(
            key = item.mediaId,
            uri = uri?.toString().orEmpty(),
            title = metadata.title?.toString().orEmpty(),
            artist = metadata.artist?.toString().orEmpty(),
            album = metadata.albumTitle?.toString(),
            artUri = metadata.artworkUri?.toString(),
            durationMs = metadata.durationMs ?: 0L,
        )
    }

    private fun repeatModeToApp(repeatMode: Int): Int = when (repeatMode) {
        Player.REPEAT_MODE_ALL -> REPEAT_ALL
        Player.REPEAT_MODE_ONE -> REPEAT_ONE
        else -> REPEAT_OFF
    }

    private companion object {
        private const val TAG = "PlaybackConnection"
        private const val POSITION_POLL_MS = 500L
        private const val PREVIOUS_RESTART_MS = 3_000L

        // App-level repeat mapping used by PlaybackUiState.
        private const val REPEAT_OFF = 0
        private const val REPEAT_ALL = 1
        private const val REPEAT_ONE = 2
    }
}
