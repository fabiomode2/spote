package dev.spote.app.setup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** Tests JVM del formato `spote://setup` (sin Android framework). */
class SetupLinkTest {

    private val fullLink = SetupLink(
        serverUrl = "https://spote.example.com:8443/base",
        clientKey = "clave/segura+==",
        invite = "invitacion 2026&ok",
        name = "Ana Pérez",
    )

    @Test
    fun `round trip keeps every field`() {
        assertEquals(fullLink, SetupLink.parse(fullLink.encode()))
    }

    @Test
    fun `encode uses the documented format and percent encoding`() {
        assertEquals(
            "spote://setup?server=https%3A%2F%2Fspote.example.com%3A8443%2Fbase" +
                "&key=clave%2Fsegura%2B%3D%3D" +
                "&invite=invitacion%202026%26ok" +
                "&name=Ana%20P%C3%A9rez",
            fullLink.encode(),
        )
    }

    @Test
    fun `round trip preserves unicode and emoji`() {
        val link = fullLink.copy(name = "Naïve 東京 🎧")
        assertEquals(link, SetupLink.parse(link.encode()))
    }

    @Test
    fun `optional name is omitted when blank`() {
        val link = fullLink.copy(name = "   ")
        val raw = link.encode()

        assertFalse(raw.contains("name="))
        assertNull(SetupLink.parse(raw)?.name)
    }

    @Test
    fun `parse accepts plus as space but percent plus as literal plus`() {
        val parsed = SetupLink.parse(
            "spote://setup?server=https%3A%2F%2Fexample.com&key=a+b&invite=c%2Bd",
        )

        assertEquals("a b", parsed?.clientKey)
        assertEquals("c+d", parsed?.invite)
    }

    @Test
    fun `parse ignores unknown parameters and a trailing slash`() {
        val raw = "spote://setup/?server=https%3A%2F%2Fexample.com&key=k&invite=i" +
            "&extra=ignored&name=Ana"

        val parsed = SetupLink.parse(raw)

        assertEquals("https://example.com", parsed?.serverUrl)
        assertEquals("k", parsed?.clientKey)
        assertEquals("i", parsed?.invite)
        assertEquals("Ana", parsed?.name)
    }

    @Test
    fun `parse rejects repeated known parameters`() {
        assertRejected(
            "spote://setup?server=https%3A%2F%2Fexample.com&key=first&key=second&invite=i",
        )
        assertRejected(
            "spote://setup?server=https%3A%2F%2Fexample.com&key=k&invite=i&invite=j",
        )
    }

    @Test
    fun `parse accepts http and a path`() {
        val parsed = SetupLink.parse(
            "spote://setup?server=http%3A%2F%2Flocalhost%3A8080%2Fspote&key=k&invite=i",
        )

        assertEquals("http://localhost:8080/spote", parsed?.serverUrl)
    }

    @Test
    fun `parse rejects links outside the setup scheme`() {
        val validQuery = "?server=https%3A%2F%2Fexample.com&key=k&invite=i"

        assertRejected(null)
        assertRejected("")
        assertRejected("   ")
        assertRejected("https://example.com$validQuery")
        assertRejected("spote://other$validQuery")
        assertRejected("spote://setup/path$validQuery")
        assertRejected("spote://setup")
    }

    @Test
    fun `parse rejects missing required fields`() {
        assertRejected("spote://setup?key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k")
        assertRejected("spote://setup?server=&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k&invite=")
    }

    @Test
    fun `parse rejects a server that is not absolute http or https`() {
        assertRejected("spote://setup?server=example.com&key=k&invite=i")
        assertRejected("spote://setup?server=ftp%3A%2F%2Fexample.com&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2F&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com%3Fq%3D1&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com%23frag&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fuser%3Apass%40example.com&key=k&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexa%20mple.com&key=k&invite=i")
    }

    @Test
    fun `parse rejects malformed percent encoding and malformed pairs`() {
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=%ZZ&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=%FF&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&=x&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k%2&invite=i")
    }

    @Test
    fun `parse rejects control characters in decoded fields`() {
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k%0A&invite=i")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k&invite=i%00")
        assertRejected("spote://setup?server=https%3A%2F%2Fexample.com&key=k&invite=i&name=ana%1B%5D")
    }

    @Test
    fun `parse never throws on arbitrary input`() {
        val samples = listOf(
            "spote://",
            "spote://setup?",
            "spote://setup?&&&",
            "spote://setup?%",
            "%%%",
            "spote://setup?server=%00&key=k&invite=i",
        )
        samples.forEach { assertNull(it, SetupLink.parse(it)) }
    }

    @Test
    fun `encode rejects invalid required fields`() {
        assertThrows(IllegalArgumentException::class.java) {
            SetupLink("", "k", "i").encode()
        }
        assertThrows(IllegalArgumentException::class.java) {
            SetupLink("https://example.com", "", "i").encode()
        }
        assertThrows(IllegalArgumentException::class.java) {
            SetupLink("https://example.com", "k", " ").encode()
        }
        assertThrows(IllegalArgumentException::class.java) {
            SetupLink("ftp://example.com", "k", "i").encode()
        }
        assertThrows(IllegalArgumentException::class.java) {
            SetupLink("https://example.com", "k\n", "i").encode()
        }
    }

    @Test
    fun `encoded link keeps the setup prefix`() {
        assertTrue(fullLink.encode().startsWith("spote://setup?"))
    }

    private fun assertRejected(raw: String?) {
        assertNull("expected rejection for: $raw", SetupLink.parse(raw))
    }
}
