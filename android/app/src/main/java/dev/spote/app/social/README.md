# Social client (Android)

Kotlin client for `spoted`, the social backend in `crates/server`. It talks to
the existing API only; no Rust or Gradle changes are required.

| file | purpose |
| --- | --- |
| `SocialModels.kt` | DTOs mirroring `crates/protocol/src/lib.rs`, `ClientMsg`/`ServerMsg` codecs, `SocialEvent` UI stream, org.json helpers |
| `SocialConfig.kt` | `EncryptedSharedPreferences` storage for URL, client key, token, name and invite |
| `SpoteSocialClient.kt` | login/logout/room list over OkHttp, WebSocket session with reconnect, send/receive |
| `SocialClientTest.kt` | JVM parsing/serialization tests against representative serde JSON |

## Auth flow

Every `/api/v1` call needs the personal client key:

```
X-Spote-Key: <client_key>
```

The session is created with the shared invite (there is no registration step):

```http
POST /api/v1/auth/login
X-Spote-Key: <client_key>
Content-Type: application/json

{"name":"ana","invite":"<group invite>"}
```

The response is `{"token":"…","user":{…},"invite":""}`. The token is tied to
the client key, expires after `SPOTE_SESSION_DAYS` (30 by default), and is sent
on everything else as:

```
Authorization: Bearer <token>
```

The WebSocket handshake needs **both** headers. The client saves server URL,
key, name, invite and token after a successful login. A `401` on login means a
wrong key/invite or a name already bound to another key; the server deliberately
does not say which.

`SocialConfig` uses an AES-256-GCM master key in the Android Keystore. If the
crypto provider cannot be initialized it falls back to an app-private
`MODE_PRIVATE` file and sets `storageIsEncrypted = false` (the downgrade is
logged). Callers that refuse plaintext tokens can check that flag.

## Wiring checklist

1. Create one `SocialConfig(context.applicationContext)` and one
   `SpoteSocialClient(config, deviceName = Build.MODEL)` in the app container
   (or a ViewModel factory). Both are meant to be process-wide singletons.
2. Settings screen: bind the four inputs to `config.serverUrl`, `clientKey`,
   `displayName`, `invite` and call
   `viewModelScope.launch { client.login(url, key, name, invite) }`.
   Persistence on success is automatic.
3. When `config.isLoggedIn`, collect **one** flow:

   ```kotlin
   viewModelScope.launch {
       client.connect(viewModelScope).collect { event -> /* update state */ }
   }
   ```

   Cancel the collecting job to close the socket; `client.disconnect()` also
   works and ends the flow without a retry. Keep the collection in one long-lived
   place (app/room ViewModel), not in each screen.
4. Map events:
   - `Connected` → session identity/server time.
   - `Rooms` → room picker (also re-emitted after each reconnect).
   - `RoomState` → members, current track, playing, anchor.
   - `Queue` → queue list.
   - `StartAt` → schedule playback at `atMs` (server clock).
   - `Presence` → online users / now playing.
   - `Chat` → chat list (the server replays the last 50 messages on join).
   - `ClockOffset` → store for `client.serverNowMs()`.
   - `Error` → non-fatal message; `Disconnected(willRetry = true)` → show
     "reconnecting"; `willRetry = false` → back to the login screen.
5. Room actions: `join(roomId)`, `leave()`, `control(ControlAction.SEEK, ms)`,
   `addToQueue(track)`, `removeFromQueue(itemId)`, `sendChat(text)`,
   `markReady(trackKey, bufferedMs)`, `updatePresence(track, playing)`.
   All return `false` when the socket is not open; `join` remembers the room and
   rejoins automatically after a reconnect.
6. Playback sync: `state.targetPositionMs(client.serverNowMs())` gives the
   position to seek to. `RoomState.anchorMs` and `StartAt.atMs` are server-clock
   values, never `System.currentTimeMillis()` directly.
7. `client.refreshRooms()` fetches the REST list and emits `Rooms` on the active
   flow. There is no server push for the room list itself.
8. `client.logout()` revokes server-side and clears the local token. Call
   `client.disconnect()` first (or immediately after) so the live socket does
   not stay open with a revoked session.
9. `build.gradle.kts` already has OkHttp, security-crypto and coroutines; the
   manifest sets `usesCleartextTraffic="false"`, so use HTTPS/WSS. A debug build
   against a LAN `http://` server needs a debug network-security config (outside
   this package's scope).

## Choices where the server leaves room

- **WebSocket URL**: `https://host[/prefix]/api/v1/ws` (or `http://…`). OkHttp
  performs the upgrade; using `ws`/`wss` is unnecessary. A user-entered URL that
  already ends in `/api/v1` is accepted and normalized.
- **Ping**: the protocol carries `ping`/`pong`. This client sends one every 30 s
  (server idle timeout is 90 s), measures RTT and emits `ClockOffset`. If no
  inbound frame arrives for ~60 s the socket is cancelled and reconnected.
- **Reconnect**: exponential backoff 1 s → 30 s (jittered). A handshake
  `401`/`403` is not retried; it emits `Error` + `Disconnected(willRetry=false)`
  so the UI can ask for a new login.
- **`hello`**: sent on every open with `device` (max 64 chars, default
  `"android"`). The server answers `welcome`; rooms/auto-join happen after it.
- **Login validation**: the client rejects a blank name/invite/key before the
  request so the error is actionable; the server is still the authority.
- **Refresh**: `ClientMsg` has no refresh variant. "Refresh" is
  `GET /api/v1/rooms`, exposed as `refreshRooms()`.
- **Message frames**: one JSON object per WebSocket text frame. No newline is
  added; NDJSON framing only applies to other transports in the protocol crate.
- **Limits**: presence 20/10 s, chat 10/10 s, queue 30/60 s, controls 60/10 s,
  chat text max 2000 chars. The server returns an `error` frame when exceeded.

## Tests

`SocialClientTest` runs on the JVM with Robolectric (already a test dependency)
because local unit tests otherwise use the stubbed `org.json` from the mockable
`android.jar`. It performs no network I/O.
