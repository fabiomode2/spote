package dev.spote.app.social

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.coroutineContext
import kotlin.random.Random
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject

/**
 * Client for the `spoted` social backend.
 *
 * Two layers, matching `crates/server/src/auth.rs`:
 *  - every `/api/v1` request needs `X-Spote-Key`;
 *  - everything except login/logout/time also needs `Authorization: Bearer`.
 *
 * Login is `POST /api/v1/auth/login` with `{name, invite}`; the invite is in
 * the body, the client key is in the header. On success the credentials and
 * token are written to [SocialConfig].
 *
 * The WebSocket is `GET /api/v1/ws` (HTTP or HTTPS URL; OkHttp performs the
 * upgrade), with both headers. The server only sends `welcome` after the
 * client sends `hello`.
 *
 * Threading: all public send methods are fire-and-forget and safe from the main
 * thread. [connect] should be collected by exactly one collector at a time.
 */
class SpoteSocialClient(
    private val config: SocialConfig,
    private val httpClient: OkHttpClient = defaultHttpClient(),
    private val deviceName: String = DEFAULT_DEVICE,
    private val now: () -> Long = System::currentTimeMillis,
) {

    /** Same client with no read timeout: the server pings/pongs at app level. */
    private val wsClient: OkHttpClient = httpClient.newBuilder()
        .readTimeout(0, TimeUnit.SECONDS)
        .build()

    @Volatile
    private var socket: WebSocket? = null

    @Volatile
    private var connectionScope: CoroutineScope? = null

    @Volatile
    private var eventSink: ((SocialEvent) -> Unit)? = null

    @Volatile
    private var stopping = false

    @Volatile
    private var connecting = false

    @Volatile
    private var clockOffsetMs = 0L

    /** Room the user asked to join; re-joined automatically after a reconnect. */
    @Volatile
    private var activeRoomId: String? = null

    /** Estimated `server clock - local clock`, updated from every `pong`. */
    val clockOffset: Long get() = clockOffsetMs

    /** Estimated server time in milliseconds. Use it for [RoomState.anchorMs]. */
    fun serverNowMs(): Long = now() + clockOffsetMs

    // -----------------------------------------------------------------------
    // REST
    // -----------------------------------------------------------------------

    /**
     * Logs in and persists server URL, key, name, invite and token.
     *
     * The server rejects a wrong invite or an unknown client key with `401` and
     * a generic `{"error":"login rejected"}`; a name already bound to another
     * key is also rejected.
     */
    suspend fun login(
        serverUrl: String,
        clientKey: String,
        name: String,
        invite: String,
    ): Result<LoginResponse> = withContext(Dispatchers.IO) {
        runCatching {
            val base = normalizeServerUrl(serverUrl)
            require(base.isNotEmpty()) { "server URL is required" }
            require(base.toHttpUrlOrNull() != null) { "invalid server URL: $base" }
            require(clientKey.isNotBlank()) { "client key is required" }
            require(name.isNotBlank()) { "name is required" }
            require(invite.isNotBlank()) { "invite code is required" }

            val request = Request.Builder()
                .url("$base$API_PREFIX/auth/login")
                .header(HEADER_CLIENT_KEY, clientKey.trim())
                .header(HEADER_ACCEPT, ACCEPT_JSON)
                .post(
                    LoginRequest(name = name.trim(), invite = invite.trim())
                        .toJson()
                        .toString()
                        .toRequestBody(JSON_MEDIA_TYPE),
                )
                .build()

            httpClient.newCall(request).execute().use { response ->
                val body = response.body?.string().orEmpty()
                if (!response.isSuccessful) throw httpError(response.code, body)
                LoginResponse.fromJson(JSONObject(body)).also { login ->
                    config.serverUrl = base
                    config.clientKey = clientKey.trim()
                    config.displayName = name.trim()
                    config.invite = invite.trim()
                    config.token = login.token
                }
            }
        }
    }

    /** Revokes the session server-side and always clears the local token. */
    suspend fun logout(): Result<Unit> = withContext(Dispatchers.IO) {
        val result = runCatching {
            val base = normalizeServerUrl(config.serverUrl)
            val token = config.token
            if (base.isNotEmpty() && token.isNotEmpty()) {
                val request = Request.Builder()
                    .url("$base$API_PREFIX/auth/logout")
                    .header(HEADER_CLIENT_KEY, config.clientKey)
                    .header(HEADER_AUTHORIZATION, "Bearer $token")
                    .post("".toRequestBody(null))
                    .build()
                httpClient.newCall(request).execute().use { response ->
                    if (!response.isSuccessful && response.code != 401) {
                        throw httpError(response.code, response.body?.string().orEmpty())
                    }
                }
            }
        }
        config.clearSession()
        result
    }

    /** `GET /api/v1/rooms` — public rooms plus private ones the user belongs to. */
    suspend fun listRooms(): Result<List<Room>> = withContext(Dispatchers.IO) {
        runCatching {
            val base = normalizeServerUrl(config.serverUrl)
            require(base.isNotEmpty()) { "server URL is not configured" }
            require(config.token.isNotEmpty()) { "not logged in" }

            val request = Request.Builder()
                .url("$base$API_PREFIX/rooms")
                .header(HEADER_CLIENT_KEY, config.clientKey)
                .header(HEADER_AUTHORIZATION, "Bearer ${config.token}")
                .header(HEADER_ACCEPT, ACCEPT_JSON)
                .get()
                .build()

            httpClient.newCall(request).execute().use { response ->
                val body = response.body?.string().orEmpty()
                if (!response.isSuccessful) throw httpError(response.code, body)
                parseRoomList(body)
            }
        }
    }

    // -----------------------------------------------------------------------
    // WebSocket
    // -----------------------------------------------------------------------

    /**
     * Opens the social WebSocket and emits [SocialEvent]s until the flow is
     * cancelled. Reconnects with exponential backoff; the token/key are read
     * again on every attempt (so a re-login while the flow is alive works).
     *
     * A `401` during the handshake stops reconnecting: the UI must log in
     * again. Only one collector should exist per client instance.
     */
    fun connect(scope: CoroutineScope): Flow<SocialEvent> = callbackFlow {
        if (connecting) {
            trySend(SocialEvent.Error("social client is already connected"))
            trySend(SocialEvent.Disconnected("already connected", willRetry = false))
            close()
            return@callbackFlow
        }

        val hasCredentials = normalizeServerUrl(config.serverUrl).isNotEmpty() &&
            config.clientKey.isNotEmpty() &&
            config.token.isNotEmpty()
        if (!hasCredentials) {
            trySend(SocialEvent.Error("social client is not configured: log in first"))
            trySend(SocialEvent.Disconnected("missing credentials", willRetry = false))
            close()
            return@callbackFlow
        }

        connecting = true
        stopping = false
        connectionScope = scope
        eventSink = { event -> trySend(event) }

        val sender: (SocialEvent) -> Unit = { event -> trySend(event) }
        val worker = scope.launch {
            runConnection(sender) { close() }
        }

        awaitClose {
            eventSink = null
            connectionScope = null
            stopping = true
            worker.cancel()
            socket?.close(CLOSE_NORMAL, "client closed")
            socket = null
            connecting = false
        }
    }

    /** Joins a room over the socket. The server replies with state + queue + chat. */
    fun join(roomId: String) {
        activeRoomId = roomId
        send(ClientMsg.Join(roomId))
    }

    /** Leaves [roomId], or the active room when `null`. Safe while disconnected. */
    fun leave(roomId: String? = null) {
        val target = roomId ?: activeRoomId ?: return
        if (activeRoomId == target) activeRoomId = null
        send(ClientMsg.Leave(target))
    }

    /** Sends `play`/`pause`/`next`/`prev`/`seek` to the current room. */
    fun control(action: ControlAction, positionMs: Long? = null): Boolean {
        if (action == ControlAction.SEEK && positionMs == null) return false
        return send(ClientMsg.Control(action, positionMs))
    }

    fun addToQueue(track: TrackRef): Boolean = send(ClientMsg.QueueAdd(track))

    fun removeFromQueue(itemId: String): Boolean = send(ClientMsg.QueueRemove(itemId))

    /** Announces that the first [bufferedMs] of [trackKey] are ready. */
    fun markReady(trackKey: String, bufferedMs: Long): Boolean =
        send(ClientMsg.Ready(trackKey, bufferedMs))

    /** Sends a presence update (now playing + playing). Throttled server-side. */
    fun updatePresence(nowPlaying: TrackRef?, playing: Boolean): Boolean =
        send(ClientMsg.PresenceUpdate(nowPlaying, playing))

    fun sendChat(text: String): Boolean = send(ClientMsg.Chat(text))

    /** Sends a ping now; the reply produces a [SocialEvent.ClockOffset]. */
    fun ping(): Boolean = send(ClientMsg.Ping(now()))

    /** Fetches rooms over REST and emits [SocialEvent.Rooms] on the active flow. */
    fun refreshRooms() {
        val scope = connectionScope ?: return
        scope.launch {
            listRooms()
                .onSuccess { rooms -> emit(SocialEvent.Rooms(rooms)) }
                .onFailure { error -> emit(SocialEvent.Error(errorMessage(error))) }
        }
    }

    /**
     * Closes the socket without emitting a retry. The flow ends once the
     * connection loop notices the close.
     */
    fun disconnect() {
        stopping = true
        val current = socket
        socket = null
        current?.close(CLOSE_NORMAL, "client closed")
    }

    private fun send(message: ClientMsg): Boolean =
        socket?.send(message.encode()) ?: false

    private fun emit(event: SocialEvent) {
        eventSink?.invoke(event)
    }

    // -----------------------------------------------------------------------
    // Connection loop
    // -----------------------------------------------------------------------

    private suspend fun runConnection(
        sender: (SocialEvent) -> Unit,
        closeFlow: () -> Unit,
    ) {
        var attempt = 0
        try {
            while (coroutineContext[Job]?.isActive == true && !stopping) {
                val serverUrl = normalizeServerUrl(config.serverUrl)
                val clientKey = config.clientKey
                val token = config.token
                val url = webSocketUrl(serverUrl)

                if (serverUrl.isEmpty() || clientKey.isEmpty() || token.isEmpty() || url == null) {
                    sender(SocialEvent.Error("social client is not configured: log in first"))
                    sender(SocialEvent.Disconnected("missing credentials", willRetry = false))
                    return
                }

                val request = Request.Builder()
                    .url(url)
                    .header(HEADER_CLIENT_KEY, clientKey)
                    .header(HEADER_AUTHORIZATION, "Bearer $token")
                    .header(HEADER_ACCEPT, ACCEPT_JSON)
                    .build()

                val opened = CompletableDeferred<Unit>()
                val welcomed = CompletableDeferred<Unit>()
                val closed = CompletableDeferred<CloseInfo>()
                val activity = ActivityProbe(now())
                val listener = SocialSocketListener(sender, opened, welcomed, closed, activity)
                val webSocket = wsClient.newWebSocket(request, listener)
                socket = webSocket

                // disconnect() may have run between newWebSocket and this point.
                if (stopping) webSocket.close(CLOSE_NORMAL, "client closed")

                val didOpen = withTimeoutOrNull(OPEN_TIMEOUT_MS) { opened.await() } != null &&
                    !closed.isCompleted
                val didWelcome = didOpen &&
                    withTimeoutOrNull(WELCOME_TIMEOUT_MS) { welcomed.await() } != null &&
                    !closed.isCompleted

                if (!didWelcome) {
                    if (didOpen) {
                        sender(SocialEvent.Error("server did not send welcome"))
                    }
                    webSocket.cancel()
                    if (!closed.isCompleted) {
                        closed.complete(CloseInfo(code = 0, reason = "connection handshake failed"))
                    }
                } else {
                    attempt = 0
                    // Get a clock sample immediately instead of waiting 30 s.
                    webSocket.send(ClientMsg.Ping(now()).encode())
                    emitRooms(sender)
                    activeRoomId?.takeIf { it.isNotEmpty() }?.let { room ->
                        webSocket.send(ClientMsg.Join(room).encode())
                    }
                }

                val info = if (didWelcome) {
                    coroutineScope {
                        val pingJob = launch { pingLoop(webSocket, activity) }
                        try {
                            closed.await()
                        } finally {
                            pingJob.cancel()
                        }
                    }
                } else {
                    closed.await()
                }

                socket = null

                if (stopping) {
                    sender(SocialEvent.Disconnected("closed by client", willRetry = false))
                    return
                }

                val status = info.httpStatus
                if (status == 401 || status == 403) {
                    sender(
                        SocialEvent.Error(
                            "session expired or invalid (HTTP $status); log in again",
                        ),
                    )
                    sender(SocialEvent.Disconnected(info.reason, willRetry = false))
                    return
                }

                if (status != null) {
                    sender(SocialEvent.Error("WebSocket handshake rejected with HTTP $status"))
                }

                val reason = info.error?.message?.takeIf { it.isNotBlank() }
                    ?: info.reason.takeIf { it.isNotBlank() }
                sender(SocialEvent.Disconnected(reason, willRetry = true))
                delay(backoffMs(attempt))
                attempt += 1
            }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            sender(SocialEvent.Error(errorMessage(error)))
            sender(SocialEvent.Disconnected(errorMessage(error), willRetry = false))
        } finally {
            socket = null
            closeFlow()
        }
    }

    private suspend fun emitRooms(sender: (SocialEvent) -> Unit) {
        listRooms()
            .onSuccess { rooms -> sender(SocialEvent.Rooms(rooms)) }
            .onFailure { error -> sender(SocialEvent.Error(errorMessage(error))) }
    }

    private suspend fun pingLoop(webSocket: WebSocket, activity: ActivityProbe) {
        while (coroutineContext[Job]?.isActive == true) {
            delay(PING_INTERVAL_MS)
            if (!webSocket.send(ClientMsg.Ping(now()).encode())) return
            if (now() - activity.lastInboundAt > DEAD_CONNECTION_MS) {
                webSocket.cancel()
                return
            }
        }
    }

    private inner class SocialSocketListener(
        private val sender: (SocialEvent) -> Unit,
        private val opened: CompletableDeferred<Unit>,
        private val welcomed: CompletableDeferred<Unit>,
        private val closed: CompletableDeferred<CloseInfo>,
        private val activity: ActivityProbe,
    ) : WebSocketListener() {

        override fun onOpen(webSocket: WebSocket, response: Response) {
            activity.touch(now())
            opened.complete(Unit)
            val device = deviceName.trim()
                .filterNot { it.isISOControl() }
                .take(MAX_DEVICE_CHARS)
                .ifEmpty { DEFAULT_DEVICE }
            if (!webSocket.send(ClientMsg.Hello(device).encode())) {
                webSocket.cancel()
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            activity.touch(now())
            val message = try {
                ServerMsg.parse(text)
            } catch (error: Exception) {
                sender(SocialEvent.Error("invalid server message: ${error.message ?: "parse error"}"))
                return
            }
            if (message == null) {
                sender(SocialEvent.Error("unknown server message type"))
                return
            }

            if (message is ServerMsg.Welcome) welcomed.complete(Unit)
            val event = message.toSocialEvent(now())
            if (event is SocialEvent.ClockOffset) clockOffsetMs = event.offsetMs
            sender(event)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(code, reason)
            closed.complete(CloseInfo(code = code, reason = reason))
            finishHandshake()
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            closed.complete(CloseInfo(code = code, reason = reason))
            finishHandshake()
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            // Complete `closed` first: `opened`/`welcomed` are completed below and
            // may wake the connection loop immediately, which reads httpStatus.
            closed.complete(
                CloseInfo(
                    code = 0,
                    reason = t.message.orEmpty(),
                    error = t,
                    httpStatus = response?.code,
                ),
            )
            finishHandshake()
        }

        /** Releases the open/welcome waits when the socket dies early. */
        private fun finishHandshake() {
            opened.complete(Unit)
            welcomed.complete(Unit)
        }
    }

    private class ActivityProbe(initial: Long) {
        @Volatile
        var lastInboundAt: Long = initial

        fun touch(value: Long) {
            lastInboundAt = value
        }
    }

    private data class CloseInfo(
        val code: Int,
        val reason: String,
        val error: Throwable? = null,
        val httpStatus: Int? = null,
    )
}

// ---------------------------------------------------------------------------
// Errors and URL helpers
// ---------------------------------------------------------------------------

/** Non-2xx REST/WebSocket response. [code] is the HTTP status. */
class SocialHttpException(
    val code: Int,
    override val message: String,
    cause: Throwable? = null,
) : IOException(message, cause)

/**
 * Normalizes a user-entered server URL: trims whitespace/trailing slashes and
 * tolerates a URL that already ends in `/api/v1`. The result never has a
 * trailing slash, e.g. `https://spote.example.com` or `https://host/spote`.
 */
fun normalizeServerUrl(raw: String): String {
    var value = raw.trim().trimEnd('/')
    if (value.endsWith("/api/v1/ws")) {
        value = value.removeSuffix("/api/v1/ws").trimEnd('/')
    } else if (value.endsWith(API_PREFIX)) {
        value = value.removeSuffix(API_PREFIX).trimEnd('/')
    }
    return value
}

/**
 * Builds the WebSocket endpoint URL. OkHttp upgrades an `http`/`https` URL to a
 * WebSocket, so no `ws`/`wss` scheme rewriting is needed and path prefixes
 * behind a reverse proxy keep working (`https://host/spote` →
 * `https://host/spote/api/v1/ws`).
 */
fun webSocketUrl(serverUrl: String): String? {
    val base = normalizeServerUrl(serverUrl)
    if (base.isEmpty()) return null
    val parsed = base.toHttpUrlOrNull() ?: return null
    return parsed.newBuilder()
        .query(null)
        .fragment(null)
        .addPathSegments("api/v1/ws")
        .build()
        .toString()
}

private fun httpError(code: Int, body: String): SocialHttpException {
    val remote = try {
        JSONObject(body).optString("error", "")
    } catch (_: Exception) {
        ""
    }
    val message = when {
        remote.isNotBlank() -> remote
        code == 401 -> "unauthorized (check client key and invite)"
        else -> "HTTP $code"
    }
    return SocialHttpException(code, message)
}

private fun errorMessage(error: Throwable): String =
    error.message?.takeIf { it.isNotBlank() }
        ?: error::class.java.simpleName

private fun backoffMs(attempt: Int): Long {
    val exponent = attempt.coerceIn(0, MAX_BACKOFF_SHIFT)
    val base = (1_000L shl exponent).coerceAtMost(MAX_BACKOFF_MS)
    return base + Random.nextLong(0L, 500L)
}

private fun defaultHttpClient(): OkHttpClient = OkHttpClient.Builder()
    .connectTimeout(15, TimeUnit.SECONDS)
    .readTimeout(30, TimeUnit.SECONDS)
    .writeTimeout(15, TimeUnit.SECONDS)
    .build()

private val JSON_MEDIA_TYPE = "application/json; charset=utf-8".toMediaType()

private const val API_PREFIX = "/api/v1"
private const val HEADER_CLIENT_KEY = "X-Spote-Key"
private const val HEADER_AUTHORIZATION = "Authorization"
private const val HEADER_ACCEPT = "Accept"
private const val ACCEPT_JSON = "application/json"

private const val DEFAULT_DEVICE = "android"
private const val MAX_DEVICE_CHARS = 64
private const val CLOSE_NORMAL = 1000

private const val OPEN_TIMEOUT_MS = 15_000L
private const val WELCOME_TIMEOUT_MS = 10_000L
private const val PING_INTERVAL_MS = 30_000L

/** Two missed pong windows; keeps the client ahead of the server's 90 s idle cut. */
private const val DEAD_CONNECTION_MS = 55_000L
private const val MAX_BACKOFF_MS = 30_000L
private const val MAX_BACKOFF_SHIFT = 5
