package dev.spote.app.social

import org.json.JSONArray
import org.json.JSONObject

// ---------------------------------------------------------------------------
// DTOs — mirror of crates/protocol/src/lib.rs
//
// Field names on the wire are snake_case exactly as serde emits them. Optional
// Rust fields (`Option<T>` with `#[serde(default)]`) are nullable Kotlin fields
// and may be missing or explicit `null` in inbound JSON.
// ---------------------------------------------------------------------------

/** Unix timestamp in milliseconds (`Ts` in the Rust protocol). */
typealias TimestampMs = Long

/** Source where a track was resolved (YouTube, SoundCloud, local...). */
data class SourceHint(
    val provider: String,
    val id: String,
    val durationMs: Long = 0L,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("provider", provider)
        put("id", id)
        put("duration_ms", durationMs)
    }

    companion object {
        fun fromJson(json: JSONObject): SourceHint = SourceHint(
            provider = json.stringField("provider"),
            id = json.stringField("id"),
            durationMs = json.longField("duration_ms"),
        )
    }
}

/** Canonical song identity, independent of the audio provider. */
data class TrackRef(
    val key: String,
    val title: String,
    val artist: String,
    val album: String? = null,
    val durationMs: Long = 0L,
    val artUrl: String? = null,
    val sourceHint: SourceHint? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("key", key)
        put("title", title)
        put("artist", artist)
        put("album", album ?: JSONObject.NULL)
        put("duration_ms", durationMs)
        put("art_url", artUrl ?: JSONObject.NULL)
        put("source_hint", sourceHint?.toJson() ?: JSONObject.NULL)
    }

    companion object {
        fun fromJson(json: JSONObject): TrackRef = TrackRef(
            key = json.stringField("key"),
            title = json.stringField("title"),
            artist = json.stringField("artist"),
            album = json.nullableStringField("album"),
            durationMs = json.longField("duration_ms"),
            artUrl = json.nullableStringField("art_url"),
            sourceHint = json.objectField("source_hint")?.let(SourceHint::fromJson),
        )
    }
}

data class User(
    val id: String,
    val name: String,
    val avatar: String? = null,
) {
    companion object {
        fun fromJson(json: JSONObject): User = User(
            id = json.stringField("id"),
            name = json.stringField("name"),
            avatar = json.nullableStringField("avatar"),
        )
    }
}

data class Presence(
    val user: User,
    val online: Boolean,
    val lastSeen: Long,
    val nowPlaying: TrackRef? = null,
) {
    companion object {
        fun fromJson(json: JSONObject): Presence = Presence(
            user = json.objectField("user")?.let(User::fromJson) ?: User("", ""),
            online = json.booleanField("online"),
            lastSeen = json.longField("last_seen"),
            nowPlaying = json.objectField("now_playing")?.let(TrackRef::fromJson),
        )
    }
}

/** Role of a member inside a room. Wire values are `guest` / `host`. */
enum class Role(val wire: String) {
    GUEST("guest"),
    HOST("host"),
    ;

    companion object {
        fun fromWire(value: String?): Role =
            entries.firstOrNull { it.wire == value } ?: GUEST
    }
}

data class Member(
    val user: User,
    val role: Role = Role.GUEST,
    val ready: Boolean = false,
) {
    companion object {
        fun fromJson(json: JSONObject): Member = Member(
            user = json.objectField("user")?.let(User::fromJson) ?: User("", ""),
            role = Role.fromWire(json.nullableStringField("role")),
            ready = json.booleanField("ready"),
        )
    }
}

/** Playback state of a room. The server is the clock authority. */
data class RoomState(
    val roomId: String = "",
    val track: TrackRef? = null,
    val playing: Boolean = false,
    val positionMs: Long = 0L,
    /** Server clock when [positionMs] was fixed. */
    val anchorMs: Long = 0L,
    val updatedBy: String? = null,
    /** Scheduled track change; every client starts at the same instant. */
    val startAtMs: Long? = null,
) {
    /**
     * Expected position at [serverNowMs], mirroring `RoomState::target_position`
     * in the Rust protocol. Use the server time from [SpoteSocialClient.serverNowMs].
     */
    fun targetPositionMs(serverNowMs: Long): Long {
        if (!playing) return positionMs
        val elapsed = (serverNowMs - anchorMs).coerceAtLeast(0L)
        return positionMs + elapsed
    }

    companion object {
        fun fromJson(json: JSONObject): RoomState = RoomState(
            roomId = json.stringField("room_id"),
            track = json.objectField("track")?.let(TrackRef::fromJson),
            playing = json.booleanField("playing"),
            positionMs = json.longField("position_ms"),
            anchorMs = json.longField("anchor_ms"),
            updatedBy = json.nullableStringField("updated_by"),
            startAtMs = json.nullableLongField("start_at_ms"),
        )
    }
}

data class QueueItem(
    val id: String,
    val track: TrackRef,
    val addedBy: String,
    val position: Long,
    val votes: Long = 0L,
) {
    companion object {
        fun fromJson(json: JSONObject): QueueItem = QueueItem(
            id = json.stringField("id"),
            track = json.objectField("track")?.let(TrackRef::fromJson) ?: TrackRef("", "", ""),
            addedBy = json.stringField("added_by"),
            position = json.longField("position"),
            votes = json.longField("votes"),
        )
    }
}

data class Room(
    val id: String,
    val name: String,
    val hostId: String,
    val isPublic: Boolean = false,
    val members: List<Member> = emptyList(),
    val state: RoomState = RoomState(),
) {
    companion object {
        fun fromJson(json: JSONObject): Room {
            val id = json.stringField("id")
            return Room(
                id = id,
                name = json.stringField("name"),
                hostId = json.stringField("host_id"),
                isPublic = json.booleanField("is_public"),
                members = json.arrayField("members").mapObjects(Member::fromJson),
                state = json.objectField("state")?.let(RoomState::fromJson)
                    ?: RoomState(roomId = id),
            )
        }
    }
}

data class ChatMsg(
    val id: String,
    val roomId: String,
    val userId: String,
    val userName: String,
    val text: String,
    val ts: Long,
) {
    companion object {
        fun fromJson(json: JSONObject): ChatMsg = ChatMsg(
            id = json.stringField("id"),
            roomId = json.stringField("room_id"),
            userId = json.stringField("user_id"),
            userName = json.stringField("user_name"),
            text = json.stringField("text"),
            ts = json.longField("ts"),
        )
    }
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

data class LoginRequest(
    val name: String,
    val invite: String = "",
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("name", name)
        put("invite", invite)
    }
}

data class LoginResponse(
    val token: String,
    val user: User,
    /** Legacy field: the server always returns an empty string now. */
    val invite: String = "",
) {
    companion object {
        fun fromJson(json: JSONObject): LoginResponse = LoginResponse(
            token = json.stringField("token"),
            user = json.objectField("user")?.let(User::fromJson) ?: User("", ""),
            invite = json.stringField("invite"),
        )
    }
}

/** Parses the `GET /api/v1/rooms` array. */
fun parseRoomList(raw: String): List<Room> =
    JSONArray(raw).mapObjects(Room::fromJson)

// ---------------------------------------------------------------------------
// Client -> server WebSocket messages
// ---------------------------------------------------------------------------

/** `ControlAction` in the Rust protocol; wire values are snake_case. */
enum class ControlAction(val wire: String) {
    PLAY("play"),
    PAUSE("pause"),
    NEXT("next"),
    PREV("prev"),
    SEEK("seek"),
    ;

    companion object {
        fun fromWire(value: String?): ControlAction? =
            entries.firstOrNull { it.wire == value }
    }
}

/**
 * Outbound messages, mirroring `ClientMsg`. Serde uses an internal tag
 * (`#[serde(tag = "type", rename_all = "snake_case")]`), so every object has a
 * `type` field and the variant fields at the top level.
 */
sealed interface ClientMsg {

    fun toJson(): JSONObject

    /** One WebSocket text frame. No trailing newline is needed (or wanted). */
    fun encode(): String = toJson().toString()

    data class Hello(
        val device: String,
        val nowPlaying: TrackRef? = null,
    ) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "hello")
            put("device", device)
            put("now_playing", nowPlaying?.toJson() ?: JSONObject.NULL)
        }
    }

    data class Ping(val t0: Long) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "ping")
            put("t0", t0)
        }
    }

    data class Join(val room: String) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "join")
            put("room", room)
        }
    }

    data class Leave(val room: String) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "leave")
            put("room", room)
        }
    }

    data class Control(
        val action: ControlAction,
        val positionMs: Long? = null,
    ) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "control")
            put("action", action.wire)
            put("position_ms", positionMs ?: JSONObject.NULL)
        }
    }

    data class QueueAdd(val track: TrackRef) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "queue_add")
            put("track", track.toJson())
        }
    }

    data class QueueRemove(val item: String) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "queue_remove")
            put("item", item)
        }
    }

    /** The client already has the first seconds of the track buffered. */
    data class Ready(
        val trackKey: String,
        val bufferedMs: Long,
    ) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "ready")
            put("track_key", trackKey)
            put("buffered_ms", bufferedMs)
        }
    }

    /**
     * Now-playing update for presence. Named `PresenceUpdate` to avoid a clash
     * with the [Presence] DTO; the wire type is `presence`.
     */
    data class PresenceUpdate(
        val nowPlaying: TrackRef? = null,
        val playing: Boolean = false,
    ) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "presence")
            put("now_playing", nowPlaying?.toJson() ?: JSONObject.NULL)
            put("playing", playing)
        }
    }

    data class Chat(val text: String) : ClientMsg {
        override fun toJson(): JSONObject = JSONObject().apply {
            put("type", "chat")
            put("text", text)
        }
    }
}

// ---------------------------------------------------------------------------
// Server -> client WebSocket messages
// ---------------------------------------------------------------------------

/**
 * Inbound messages, mirroring `ServerMsg`. Variant class names get a `Msg`
 * suffix where the bare name would collide with a DTO.
 */
sealed interface ServerMsg {

    data class Welcome(
        val serverTime: Long,
        val user: User,
    ) : ServerMsg

    data class Pong(
        val t0: Long,
        val t1: Long,
    ) : ServerMsg

    data class RoomStateMsg(
        val state: RoomState,
        val members: List<Member>,
    ) : ServerMsg

    data class QueueMsg(
        val items: List<QueueItem>,
    ) : ServerMsg

    data class StartAtMsg(
        val track: TrackRef,
        val atMs: Long,
    ) : ServerMsg

    data class PresenceMsg(
        val presences: List<Presence>,
    ) : ServerMsg

    data class ChatMessage(
        val msg: ChatMsg,
    ) : ServerMsg

    data class ErrorMessage(
        val message: String,
    ) : ServerMsg

    companion object {
        /**
         * Parses one text frame. Returns `null` for an unknown `type` so the
         * caller can keep the connection; malformed JSON throws
         * `org.json.JSONException` (caller turns that into an error event).
         */
        fun parse(raw: String): ServerMsg? {
            val json = JSONObject(raw)
            return when (json.stringField("type")) {
                "welcome" -> Welcome(
                    serverTime = json.longField("server_time"),
                    user = json.objectField("user")?.let(User::fromJson) ?: User("", ""),
                )

                "pong" -> Pong(
                    t0 = json.longField("t0"),
                    t1 = json.longField("t1"),
                )

                "room_state" -> RoomStateMsg(
                    state = json.objectField("state")?.let(RoomState::fromJson) ?: RoomState(),
                    members = json.arrayField("members").mapObjects(Member::fromJson),
                )

                "queue" -> QueueMsg(
                    items = json.arrayField("items").mapObjects(QueueItem::fromJson),
                )

                "start_at" -> StartAtMsg(
                    track = json.objectField("track")?.let(TrackRef::fromJson)
                        ?: TrackRef("", "", ""),
                    atMs = json.longField("at_ms"),
                )

                "presence" -> PresenceMsg(
                    presences = json.arrayField("presences").mapObjects(Presence::fromJson),
                )

                "chat" -> ChatMessage(
                    msg = json.objectField("msg")?.let(ChatMsg::fromJson) ?: ChatMsg(
                        id = "",
                        roomId = "",
                        userId = "",
                        userName = "",
                        text = "",
                        ts = 0L,
                    ),
                )

                "error" -> ErrorMessage(json.stringField("message"))
                else -> null
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UI events
// ---------------------------------------------------------------------------

/**
 * Everything the UI needs from the social session. It is transport-agnostic:
 * [Rooms] comes from REST and the rest come from the WebSocket (or the local
 * connection state machine).
 */
sealed interface SocialEvent {
    /** The `hello` handshake was accepted. [serverTimeMs] is the server clock. */
    data class Connected(val serverTimeMs: Long, val user: User) : SocialEvent

    /** Visible rooms from `GET /api/v1/rooms`. */
    data class Rooms(val rooms: List<Room>) : SocialEvent

    data class RoomState(
        val state: dev.spote.app.social.RoomState,
        val members: List<Member>,
    ) : SocialEvent

    data class Queue(val items: List<QueueItem>) : SocialEvent

    /** Synchronized track change scheduled at [atMs] (server clock). */
    data class StartAt(val track: TrackRef, val atMs: Long) : SocialEvent

    data class Presence(
        val presences: List<dev.spote.app.social.Presence>,
    ) : SocialEvent

    data class Chat(val message: ChatMsg) : SocialEvent

    /**
     * Server clock minus local clock. Add it to `System.currentTimeMillis()` to
     * estimate the server time (`SpoteSocialClient.serverNowMs` does this).
     */
    data class ClockOffset(val offsetMs: Long, val rttMs: Long) : SocialEvent

    data class Error(val message: String) : SocialEvent

    data class Disconnected(
        val reason: String?,
        val willRetry: Boolean,
    ) : SocialEvent
}

/** Converts a parsed frame into a UI event. */
fun ServerMsg.toSocialEvent(receivedAtMs: Long = System.currentTimeMillis()): SocialEvent =
    when (this) {
        is ServerMsg.Welcome -> SocialEvent.Connected(serverTime, user)
        is ServerMsg.Pong -> {
            val rtt = (receivedAtMs - t0).coerceAtLeast(0L)
            SocialEvent.ClockOffset(
                offsetMs = t1 - (t0 + rtt / 2),
                rttMs = rtt,
            )
        }

        is ServerMsg.RoomStateMsg -> SocialEvent.RoomState(state, members)
        is ServerMsg.QueueMsg -> SocialEvent.Queue(items)
        is ServerMsg.StartAtMsg -> SocialEvent.StartAt(track, atMs)
        is ServerMsg.PresenceMsg -> SocialEvent.Presence(presences)
        is ServerMsg.ChatMessage -> SocialEvent.Chat(msg)
        is ServerMsg.ErrorMessage -> SocialEvent.Error(message)
    }

// ---------------------------------------------------------------------------
// org.json helpers
//
// The contract keeps every field present, but `#[serde(default)]` means an
// older server may omit optional fields. These helpers treat missing and
// explicit null the same and never call the `opt*` methods on a JSON null.
// ---------------------------------------------------------------------------

internal fun JSONObject.stringField(key: String, default: String = ""): String =
    if (!has(key) || isNull(key)) default else optString(key, default)

internal fun JSONObject.nullableStringField(key: String): String? =
    if (!has(key) || isNull(key)) null else optString(key, "")

internal fun JSONObject.longField(key: String, default: Long = 0L): Long =
    if (!has(key) || isNull(key)) default else optLong(key, default)

internal fun JSONObject.nullableLongField(key: String): Long? =
    if (!has(key) || isNull(key)) null else optLong(key)

internal fun JSONObject.booleanField(key: String, default: Boolean = false): Boolean =
    if (!has(key) || isNull(key)) default else optBoolean(key, default)

internal fun JSONObject.objectField(key: String): JSONObject? =
    if (!has(key) || isNull(key)) null else optJSONObject(key)

internal fun JSONObject.arrayField(key: String): JSONArray =
    if (!has(key) || isNull(key)) JSONArray() else optJSONArray(key) ?: JSONArray()

internal inline fun <T> JSONArray.mapObjects(transform: (JSONObject) -> T): List<T> {
    val out = ArrayList<T>(length())
    for (index in 0 until length()) {
        val item = optJSONObject(index) ?: continue
        out += transform(item)
    }
    return out
}
