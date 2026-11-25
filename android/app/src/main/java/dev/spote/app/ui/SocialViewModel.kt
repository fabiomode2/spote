package dev.spote.app.ui

import android.app.Application
import android.os.Build
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.spote.app.model.ChatMessage
import dev.spote.app.rust.coverFor
import dev.spote.app.social.ChatMsg
import dev.spote.app.social.Member
import dev.spote.app.social.Presence
import dev.spote.app.social.QueueItem
import dev.spote.app.social.RoomState
import dev.spote.app.social.SocialConfig
import dev.spote.app.social.SocialEvent
import dev.spote.app.social.SpoteSocialClient
import dev.spote.app.social.TrackRef
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class SocialUiState(
    val loggedIn: Boolean = false,
    val busy: Boolean = false,
    val connecting: Boolean = false,
    val userName: String = "",
    val serverUrl: String = "",
    val rooms: List<dev.spote.app.social.Room> = emptyList(),
    val activeRoomId: String? = null,
    val roomState: RoomState? = null,
    val members: List<Member> = emptyList(),
    val queue: List<QueueItem> = emptyList(),
    val chat: List<ChatMsg> = emptyList(),
    val presences: List<Presence> = emptyList(),
    val error: String? = null,
    val storageEncrypted: Boolean = true,
) {
    val activeRoom: dev.spote.app.social.Room?
        get() = rooms.firstOrNull { it.id == activeRoomId }

    val messages: List<ChatMessage>
        get() = chat.map { ChatMessage(it.userName.ifBlank { "Alguien" }, it.text) }

    val effectiveMembers: List<String>
        get() = (members.map { it.user.name.ifBlank { "Alguien" } } +
            activeRoom?.members.orEmpty().map { it.user.name })
            .filter { it.isNotBlank() }
            .distinct()

    val nowPlaying: dev.spote.app.model.Track?
        get() = roomState?.track?.toUiTrack()
}

class SocialViewModel(application: Application) : AndroidViewModel(application) {

    private val config = SocialConfig(application)
    private val client = SpoteSocialClient(config, deviceName = Build.MODEL)
    private var eventsJob: Job? = null

    private val _state = MutableStateFlow(
        SocialUiState(
            loggedIn = config.isLoggedIn,
            userName = config.displayName,
            serverUrl = config.serverUrl,
            storageEncrypted = config.storageIsEncrypted,
            connecting = config.isLoggedIn,
        ),
    )
    val state: StateFlow<SocialUiState> = _state.asStateFlow()

    private val _notices = MutableStateFlow<String?>(null)
    val notices: StateFlow<String?> = _notices.asStateFlow()

    init {
        if (config.isLoggedIn) {
            connect()
            refreshRoomsFromRest()
        }
    }

    override fun onCleared() {
        eventsJob?.cancel()
        client.disconnect()
        super.onCleared()
    }

    fun login(serverUrl: String, clientKey: String, name: String, invite: String) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            val result = client.login(serverUrl, clientKey, name, invite)
            result.onSuccess { response ->
                _state.update {
                    it.copy(
                        busy = false,
                        loggedIn = true,
                        userName = response.user.name,
                        serverUrl = config.serverUrl,
                        connecting = true,
                        error = null,
                    )
                }
                connect()
                refreshRoomsFromRest()
                _notices.value = "Sesión iniciada como ${response.user.name}"
            }.onFailure { error ->
                _state.update {
                    it.copy(busy = false, error = error.message ?: "No se pudo iniciar sesión")
                }
            }
        }
    }

    fun logout() {
        viewModelScope.launch {
            eventsJob?.cancel()
            eventsJob = null
            client.disconnect()
            client.logout()
            _state.value = SocialUiState(
                storageEncrypted = config.storageIsEncrypted,
                serverUrl = config.serverUrl,
            )
            _notices.value = "Sesión cerrada"
        }
    }

    fun refreshRooms() {
        refreshRoomsFromRest()
    }

    private fun refreshRoomsFromRest() {
        if (!config.isLoggedIn) return
        viewModelScope.launch {
            client.listRooms().onSuccess { rooms ->
                _state.update { it.copy(rooms = rooms, error = null) }
            }.onFailure { error ->
                _state.update { it.copy(error = error.message ?: "No se pudo leer la lista de salas") }
            }
        }
    }

    fun join(roomId: String) {
        if (!config.isLoggedIn) return
        _state.update {
            it.copy(
                activeRoomId = roomId,
                chat = emptyList(),
                queue = emptyList(),
                roomState = null,
                members = emptyList(),
            )
        }
        if (eventsJob == null) connect()
        client.join(roomId)
    }

    fun leave() {
        client.leave()
        _state.update {
            it.copy(
                activeRoomId = null,
                chat = emptyList(),
                queue = emptyList(),
                roomState = null,
                members = emptyList(),
            )
        }
    }

    fun sendChat(text: String) {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        if (!client.sendChat(trimmed)) {
            _notices.value = "Sin conexión con la sala"
        }
    }

    fun control(action: dev.spote.app.social.ControlAction, positionMs: Long? = null) {
        if (!client.control(action, positionMs)) {
            _notices.value = "Sin conexión con la sala"
        }
    }

    fun clearNotice() {
        _notices.value = null
    }

    fun serverNowMs(): Long = client.serverNowMs()

    private fun connect() {
        if (eventsJob != null) return
        _state.update { it.copy(connecting = true, error = null) }
        eventsJob = viewModelScope.launch {
            try {
                client.connect(viewModelScope).collect { event -> onEvent(event) }
            } finally {
                eventsJob = null
            }
        }
    }

    private fun onEvent(event: SocialEvent) {
        when (event) {
            is SocialEvent.Connected -> {
                _state.update {
                    it.copy(
                        connecting = false,
                        loggedIn = true,
                        userName = event.user.name.ifBlank { it.userName },
                        error = null,
                    )
                }
                client.refreshRooms()
                _state.value.activeRoomId?.let(client::join)
            }

            is SocialEvent.Rooms -> _state.update { it.copy(rooms = event.rooms) }

            is SocialEvent.RoomState -> _state.update {
                it.copy(roomState = event.state, members = event.members)
            }

            is SocialEvent.Queue -> _state.update { it.copy(queue = event.items) }

            is SocialEvent.Chat -> _state.update { current ->
                if (current.chat.any { it.id == event.message.id }) {
                    current
                } else {
                    current.copy(chat = (current.chat + event.message).takeLast(200))
                }
            }

            is SocialEvent.Presence -> _state.update { it.copy(presences = event.presences) }

            is SocialEvent.StartAt -> _state.update { current ->
                val state = current.roomState
                if (state == null) {
                    current
                } else {
                    current.copy(
                        roomState = state.copy(
                            track = event.track,
                            playing = true,
                            positionMs = 0L,
                            anchorMs = event.atMs,
                            startAtMs = event.atMs,
                        ),
                    )
                }
            }

            is SocialEvent.ClockOffset -> Unit

            is SocialEvent.Error -> _state.update {
                it.copy(error = event.message)
            }

            is SocialEvent.Disconnected -> {
                if (!event.willRetry) eventsJob = null
                _state.update {
                    it.copy(
                        connecting = event.willRetry,
                        loggedIn = if (event.willRetry) it.loggedIn else false,
                        error = if (event.willRetry) {
                            it.error
                        } else {
                            event.reason ?: "Desconectado de la sala"
                        },
                    )
                }
            }
        }
    }
}

fun TrackRef.toUiTrack(): dev.spote.app.model.Track {
    val provider = sourceHint?.provider
    val remoteUrl = when (provider) {
        "youtube" -> sourceHint.id
            .takeIf { it.isNotBlank() }
            ?.let { "https://www.youtube.com/watch?v=$it" }

        else -> null
    }
    return dev.spote.app.model.Track(
        key = key,
        title = title,
        artist = artist,
        album = album,
        durationMs = durationMs,
        cover = coverFor(title, null, artUrl),
        remoteUrl = remoteUrl,
        sourceLabel = provider?.replaceFirstChar { it.uppercase() },
    )
}

fun dev.spote.app.social.Room.toUiRoom(): dev.spote.app.model.Room {
    val playing = state.track?.toUiTrack()
    val names = members.map { it.user.name }.filter { it.isNotBlank() }
    return dev.spote.app.model.Room(
        id = id,
        name = name,
        members = names,
        live = state.playing || members.any { it.ready },
        nowPlaying = playing,
    )
}
