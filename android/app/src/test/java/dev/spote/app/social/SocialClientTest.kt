package dev.spote.app.social

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pure parsing/serialization tests for the contract in
 * `crates/protocol/src/lib.rs`. No network and no Android permissions.
 *
 * Robolectric is used only because local JVM unit tests otherwise get the
 * stubbed `org.json` from the mockable android.jar.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SocialClientTest {

    // -----------------------------------------------------------------------
    // REST DTOs
    // -----------------------------------------------------------------------

    @Test
    fun `login response matches serde shape`() {
        val raw = """
            {"token":"tok-123","user":{"id":"u1","name":"ana","avatar":null},"invite":""}
        """.trimIndent()

        val login = LoginResponse.fromJson(JSONObject(raw))

        assertEquals("tok-123", login.token)
        assertEquals("u1", login.user.id)
        assertEquals("ana", login.user.name)
        assertNull(login.user.avatar)
        assertEquals("", login.invite)
    }

    @Test
    fun `login request serializes name and invite in the body`() {
        val json = LoginRequest(name = "ana", invite = "grupo-0123456789").toJson()

        assertEquals("ana", json.getString("name"))
        assertEquals("grupo-0123456789", json.getString("invite"))
        assertEquals(2, json.length())
    }

    @Test
    fun `room list tolerates defaulted fields`() {
        val raw = """
            [
              {
                "id":"r1",
                "name":"Sala",
                "host_id":"u1",
                "is_public":true,
                "members":[],
                "state":{
                  "room_id":"r1",
                  "track":null,
                  "playing":false,
                  "position_ms":0,
                  "anchor_ms":0,
                  "updated_by":null,
                  "start_at_ms":null
                }
              },
              {"id":"r2","name":"Privada","host_id":"u2"}
            ]
        """.trimIndent()

        val rooms = parseRoomList(raw)

        assertEquals(2, rooms.size)
        assertEquals("r1", rooms[0].id)
        assertTrue(rooms[0].isPublic)
        assertNull(rooms[0].state.track)
        assertEquals("r2", rooms[1].id)
        assertFalse(rooms[1].isPublic)
        assertEquals("r2", rooms[1].state.roomId)
        assertTrue(rooms[1].members.isEmpty())
    }

    // -----------------------------------------------------------------------
    // Server -> client frames
    // -----------------------------------------------------------------------

    @Test
    fun `welcome parses and becomes Connected`() {
        val raw = """
            {"type":"welcome","server_time":1700000000123,
             "user":{"id":"u1","name":"ana","avatar":"https://cdn.example/a.png"}}
        """.trimIndent()

        val welcome = ServerMsg.parse(raw) as ServerMsg.Welcome

        assertEquals(1_700_000_000_123L, welcome.serverTime)
        assertEquals("https://cdn.example/a.png", welcome.user.avatar)

        val event = welcome.toSocialEvent() as SocialEvent.Connected
        assertEquals(1_700_000_000_123L, event.serverTimeMs)
        assertEquals("ana", event.user.name)
    }

    @Test
    fun `pong computes rtt and offset`() {
        val pong = ServerMsg.parse("""{"type":"pong","t0":1000,"t1":2500}""") as ServerMsg.Pong

        val event = pong.toSocialEvent(receivedAtMs = 1600) as SocialEvent.ClockOffset

        assertEquals(600L, event.rttMs)
        // t1 - (t0 + rtt / 2) = 2500 - 1300
        assertEquals(1200L, event.offsetMs)
    }

    @Test
    fun `room state parses track, source hint and members`() {
        val raw = """
            {
              "type":"room_state",
              "state":{
                "room_id":"r1",
                "track":{
                  "key":"isrc:ABC123",
                  "title":"Cancion",
                  "artist":"Artista",
                  "album":null,
                  "duration_ms":215000,
                  "art_url":"https://cdn.example/cover.jpg",
                  "source_hint":{"provider":"youtube","id":"abc","duration_ms":215000}
                },
                "playing":true,
                "position_ms":42000,
                "anchor_ms":1700000000000,
                "updated_by":"u1",
                "start_at_ms":null
              },
              "members":[
                {"user":{"id":"u1","name":"ana"},"role":"host","ready":true},
                {"user":{"id":"u2","name":"bea","avatar":null},"role":"guest","ready":false}
              ]
            }
        """.trimIndent()

        val message = ServerMsg.parse(raw) as ServerMsg.RoomStateMsg
        val state = message.state

        assertEquals("r1", state.roomId)
        assertTrue(state.playing)
        assertEquals(42_000L, state.positionMs)
        assertEquals(1_700_000_000_000L, state.anchorMs)
        assertEquals("u1", state.updatedBy)
        assertNull(state.startAtMs)
        assertEquals("isrc:ABC123", state.track?.key)
        assertNull(state.track?.album)
        assertEquals("youtube", state.track?.sourceHint?.provider)
        assertEquals(215_000L, state.track?.sourceHint?.durationMs)

        assertEquals(2, message.members.size)
        assertEquals(Role.HOST, message.members[0].role)
        assertTrue(message.members[0].ready)
        assertNull(message.members[0].user.avatar)
        assertEquals(Role.GUEST, message.members[1].role)
        assertFalse(message.members[1].ready)

        assertEquals(43_000L, state.targetPositionMs(1_700_000_001_000L))
        assertEquals(42_000L, state.copy(playing = false).targetPositionMs(1_700_000_001_000L))
    }

    @Test
    fun `queue items default missing votes to zero`() {
        val raw = """
            {
              "type":"queue",
              "items":[
                {"id":"q1","track":{"key":"k","title":"T","artist":"A"},
                 "added_by":"u1","position":0}
              ]
            }
        """.trimIndent()

        val message = ServerMsg.parse(raw) as ServerMsg.QueueMsg

        assertEquals(1, message.items.size)
        assertEquals("q1", message.items[0].id)
        assertEquals(0L, message.items[0].votes)
        assertNull(message.items[0].track.album)
    }

    @Test
    fun `start at carries the scheduled server time`() {
        val raw = """
            {"type":"start_at",
             "track":{"key":"k1","title":"T","artist":"A","duration_ms":1000},
             "at_ms":1700000004000}
        """.trimIndent()

        val message = ServerMsg.parse(raw) as ServerMsg.StartAtMsg

        assertEquals("k1", message.track.key)
        assertEquals(1_700_000_004_000L, message.atMs)
    }

    @Test
    fun `presence parses online flags and now playing`() {
        val raw = """
            {
              "type":"presence",
              "presences":[
                {"user":{"id":"u1","name":"ana"},"online":true,"last_seen":1700000000000,
                 "now_playing":{"key":"k","title":"T","artist":"A"}},
                {"user":{"id":"u2","name":"bea"},"online":false,"last_seen":1700000000001,
                 "now_playing":null}
              ]
            }
        """.trimIndent()

        val message = ServerMsg.parse(raw) as ServerMsg.PresenceMsg

        assertEquals(2, message.presences.size)
        assertTrue(message.presences[0].online)
        assertEquals("k", message.presences[0].nowPlaying?.key)
        assertFalse(message.presences[1].online)
        assertNull(message.presences[1].nowPlaying)
    }

    @Test
    fun `chat and error frames parse`() {
        val chatRaw = """
            {"type":"chat","msg":{"id":"c1","room_id":"r1","user_id":"u1",
             "user_name":"ana","text":"hola","ts":1700000000000}}
        """.trimIndent()
        val chat = ServerMsg.parse(chatRaw) as ServerMsg.ChatMessage

        assertEquals("c1", chat.msg.id)
        assertEquals("r1", chat.msg.roomId)
        assertEquals("hola", chat.msg.text)
        assertEquals(1_700_000_000_000L, chat.msg.ts)

        val error = ServerMsg.parse("""{"type":"error","message":"join a room first"}""")
            as ServerMsg.ErrorMessage
        assertEquals("join a room first", error.message)
    }

    @Test
    fun `unknown server type returns null`() {
        assertNull(ServerMsg.parse("""{"type":"future_thing","value":1}"""))
    }

    // -----------------------------------------------------------------------
    // Client -> server encoding
    // -----------------------------------------------------------------------

    @Test
    fun `hello encodes device and null now playing`() {
        val json = ClientMsg.Hello(device = "Pixel 9").toJson()

        assertEquals("hello", json.getString("type"))
        assertEquals("Pixel 9", json.getString("device"))
        assertTrue(json.isNull("now_playing"))
    }

    @Test
    fun `ping join leave and chat use serde snake case tags`() {
        assertEquals(
            "ping",
            ClientMsg.Ping(t0 = 1234).toJson().getString("type"),
        )
        val join = ClientMsg.Join(room = "r1").toJson()
        assertEquals("join", join.getString("type"))
        assertEquals("r1", join.getString("room"))

        val leave = ClientMsg.Leave(room = "r1").toJson()
        assertEquals("leave", leave.getString("type"))
        assertEquals("r1", leave.getString("room"))

        val chat = ClientMsg.Chat(text = "hola").toJson()
        assertEquals("chat", chat.getString("type"))
        assertEquals("hola", chat.getString("text"))
    }

    @Test
    fun `control serializes action wire value and optional position`() {
        val seek = ClientMsg.Control(ControlAction.SEEK, positionMs = 42_000).toJson()
        assertEquals("control", seek.getString("type"))
        assertEquals("seek", seek.getString("action"))
        assertEquals(42_000L, seek.getLong("position_ms"))

        val pause = ClientMsg.Control(ControlAction.PAUSE).toJson()
        assertEquals("pause", pause.getString("action"))
        assertTrue(pause.isNull("position_ms"))
    }

    @Test
    fun `queue messages serialize the exact track shape`() {
        val track = TrackRef(
            key = "isrc:ABC",
            title = "Cancion",
            artist = "Artista",
            album = null,
            durationMs = 215_000,
            artUrl = "https://cdn.example/cover.jpg",
            sourceHint = SourceHint("youtube", "abc", 215_000),
        )

        val add = ClientMsg.QueueAdd(track).toJson()
        assertEquals("queue_add", add.getString("type"))
        val wireTrack = add.getJSONObject("track")
        assertEquals("isrc:ABC", wireTrack.getString("key"))
        assertEquals(215_000L, wireTrack.getLong("duration_ms"))
        assertTrue(wireTrack.isNull("album"))
        assertEquals(
            "https://cdn.example/cover.jpg",
            wireTrack.getString("art_url"),
        )
        assertEquals("youtube", wireTrack.getJSONObject("source_hint").getString("provider"))

        val remove = ClientMsg.QueueRemove(item = "q1").toJson()
        assertEquals("queue_remove", remove.getString("type"))
        assertEquals("q1", remove.getString("item"))
    }

    @Test
    fun `presence ready and control action enums encode correctly`() {
        val presence = ClientMsg.PresenceUpdate(nowPlaying = null, playing = true).toJson()
        assertEquals("presence", presence.getString("type"))
        assertTrue(presence.isNull("now_playing"))
        assertTrue(presence.getBoolean("playing"))

        val ready = ClientMsg.Ready(trackKey = "isrc:ABC", bufferedMs = 2_500).toJson()
        assertEquals("ready", ready.getString("type"))
        assertEquals("isrc:ABC", ready.getString("track_key"))
        assertEquals(2_500L, ready.getLong("buffered_ms"))

        assertEquals(ControlAction.PLAY, ControlAction.fromWire("play"))
        assertEquals(ControlAction.SEEK, ControlAction.fromWire("seek"))
        assertNull(ControlAction.fromWire("stop"))
    }

    // -----------------------------------------------------------------------
    // URL helpers (no network)
    // -----------------------------------------------------------------------

    @Test
    fun `server url normalization accepts a trailing api prefix`() {
        assertEquals("https://spote.example.com", normalizeServerUrl(" https://spote.example.com/ "))
        assertEquals("https://host/spote", normalizeServerUrl("https://host/spote/api/v1/"))
        assertEquals("https://host/spote", normalizeServerUrl("https://host/spote/api/v1/ws"))
    }

    @Test
    fun `websocket url keeps reverse proxy prefixes`() {
        assertEquals(
            "https://spote.example.com/api/v1/ws",
            webSocketUrl("https://spote.example.com"),
        )
        assertEquals(
            "https://host/spote/api/v1/ws",
            webSocketUrl("https://host/spote/"),
        )
        assertEquals(
            "http://10.0.2.2:7070/api/v1/ws",
            webSocketUrl("http://10.0.2.2:7070"),
        )
        assertNull(webSocketUrl("not a url"))
    }
}
