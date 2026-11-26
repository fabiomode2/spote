package dev.spote.app.logging

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Tests de redacción y del contrato JSONL. Robolectric solo aporta la
 * implementación real de `org.json` (en la JVM pura es un stub).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ActionLogLineTest {

    @Test
    fun `encode redacts sensitive keys`() {
        val line = ActionLogLine(
            timestamp = "2026-11-25T10:00:00Z",
            sessionId = "session-1",
            event = "login",
            data = linkedMapOf(
                "server" to "https://spote.example.com",
                "password" to "hunter2",
                "accessToken" to "tok-123",
                "ClientKey" to "key-123",
                "invite" to "invite-123",
                "api_secret" to "shh",
                "attempts" to 3,
                "ok" to true,
                "missing" to null,
            ),
        ).encode()

        val parsed = ActionLogLine.parse(line)
        requireNotNull(parsed)

        assertEquals("https://spote.example.com", parsed.data["server"])
        assertEquals(REDACTED_VALUE, parsed.data["password"])
        assertEquals(REDACTED_VALUE, parsed.data["accessToken"])
        assertEquals(REDACTED_VALUE, parsed.data["ClientKey"])
        assertEquals(REDACTED_VALUE, parsed.data["invite"])
        assertEquals(REDACTED_VALUE, parsed.data["api_secret"])
        assertEquals(3L, (parsed.data["attempts"] as Number).toLong())
        assertEquals(true, parsed.data["ok"])
        assertNull(parsed.data["missing"])

        assertFalse(line.contains("hunter2"))
        assertFalse(line.contains("tok-123"))
        assertFalse(line.contains("invite-123"))
    }

    @Test
    fun `sensitive key detection is case insensitive and by substring`() {
        assertTrue(isSensitiveKey("PASSWORD"))
        assertTrue(isSensitiveKey("refresh_token"))
        assertTrue(isSensitiveKey("clientKey"))
        assertTrue(isSensitiveKey("joinInvite"))
        assertTrue(isSensitiveKey("api_secret"))
        assertFalse(isSensitiveKey("server"))
        assertFalse(isSensitiveKey("username"))
        assertFalse(isSensitiveKey("count"))
    }

    @Test
    fun `parse reads a hand written jsonl line`() {
        val parsed = ActionLogLine.parse(
            """{"timestamp":"2026-11-25T10:00:00Z","sessionId":"s-1","event":"scan","data":{"length":12,"ok":true,"name":"Ana"}}""",
        )

        requireNotNull(parsed)
        assertEquals("2026-11-25T10:00:00Z", parsed.timestamp)
        assertEquals("s-1", parsed.sessionId)
        assertEquals("scan", parsed.event)
        assertEquals(12L, (parsed.data["length"] as Number).toLong())
        assertEquals(true, parsed.data["ok"])
        assertEquals("Ana", parsed.data["name"])
    }

    @Test
    fun `encode keeps one physical line and round trips escapes`() {
        val line = ActionLogLine(
            timestamp = "2026-11-25T10:00:00Z",
            sessionId = "s-1",
            event = "line\nbreak",
            data = mapOf("stacktrace" to "a\nb\t\"c\""),
        ).encode()

        assertFalse(line.contains('\n'))

        val parsed = ActionLogLine.parse(line)
        requireNotNull(parsed)
        assertEquals("line\nbreak", parsed.event)
        assertEquals("a\nb\t\"c\"", parsed.data["stacktrace"])
    }

    @Test
    fun `parse rejects malformed or incomplete lines`() {
        val samples = listOf(
            null,
            "",
            "   ",
            "{",
            "[]",
            "not json",
            """{"timestamp":"t","sessionId":"s"}""",
            """{"timestamp":"","sessionId":"s","event":"e"}""",
            """{"timestamp":"t","sessionId":"","event":"e"}""",
            """{"timestamp":"t","sessionId":"s","event":""}""",
        )

        samples.forEach { assertNull("expected rejection for: $it", ActionLogLine.parse(it)) }
    }

    @Test
    fun `encode without data writes an empty object`() {
        val line = ActionLogLine(
            timestamp = "2026-11-25T10:00:00Z",
            sessionId = "s-1",
            event = "ping",
        ).encode()

        val parsed = ActionLogLine.parse(line)
        requireNotNull(parsed)
        assertTrue(parsed.data.isEmpty())
    }
}
