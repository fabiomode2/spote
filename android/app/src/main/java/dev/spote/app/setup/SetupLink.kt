package dev.spote.app.setup

import java.io.ByteArrayOutputStream
import java.net.URI
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/**
 * Enlace de configuración que viaja dentro de un QR.
 *
 * Formato:
 *
 * ```
 * spote://setup?server=https%3A%2F%2Fspote.example.com&key=...&invite=...&name=...
 * ```
 *
 * `server`, `key` e `invite` son obligatorios; `name` es opcional. Todos los
 * valores van codificados con percent-encoding RFC 3986 sobre UTF-8. Al
 * decodificar se acepta `+` como espacio para enlaces generados con
 * `URLEncoder`, pero [encode] siempre emite `%20`.
 *
 * [parse] nunca lanza: devuelve `null` si el enlace está mal formado. [encode]
 * sí lanza [IllegalArgumentException] si faltan campos obligatorios o el
 * servidor no es una URL `http(s)` absoluta.
 *
 * Contiene la clave de cliente y la invitación: no lo escribas en logs ni lo
 * muestres fuera de la pantalla de configuración.
 */
data class SetupLink(
    val serverUrl: String,
    val clientKey: String,
    val invite: String,
    val name: String? = null,
) {

    /** Serializa el enlace listo para dibujarlo en un QR. */
    fun encode(): String {
        require(serverUrl.isNotBlank()) { "serverUrl is required" }
        require(clientKey.isNotBlank()) { "clientKey is required" }
        require(invite.isNotBlank()) { "invite is required" }
        require(isHttpServer(serverUrl)) { "serverUrl must be an absolute http(s) URL" }
        require(
            !hasControlChars(serverUrl) &&
                !hasControlChars(clientKey) &&
                !hasControlChars(invite) &&
                (name == null || !hasControlChars(name)),
        ) { "control characters are not allowed" }

        return buildString {
            append(SCHEME)
            append("://")
            append(HOST)
            append("?server=")
            append(percentEncode(serverUrl))
            append("&key=")
            append(percentEncode(clientKey))
            append("&invite=")
            append(percentEncode(invite))
            val linkName = name?.takeIf { it.isNotBlank() }
            if (linkName != null) {
                append("&name=")
                append(percentEncode(linkName))
            }
        }
    }

    companion object {
        const val SCHEME = "spote"
        const val HOST = "setup"

        private val KNOWN_PARAMS = setOf("server", "key", "invite", "name")

        /** Interpreta un `spote://setup?...`; devuelve `null` si no es válido. */
        fun parse(raw: String?): SetupLink? {
            if (raw.isNullOrBlank()) return null
            val value = raw.trim()
            val prefix = "$SCHEME://"
            if (!value.startsWith(prefix, ignoreCase = true)) return null

            // El fragmento no forma parte del formato y se descarta.
            val rest = value.substring(prefix.length).substringBefore('#')
            val queryStart = rest.indexOf('?')
            if (queryStart < 0) return null

            val hostPart = rest.substring(0, queryStart).removeSuffix("/")
            if (!hostPart.equals(HOST, ignoreCase = true)) return null

            val params = parseQuery(rest.substring(queryStart + 1)) ?: return null
            val serverUrl = params["server"]?.takeIf { it.isNotBlank() } ?: return null
            val clientKey = params["key"]?.takeIf { it.isNotBlank() } ?: return null
            val invite = params["invite"]?.takeIf { it.isNotBlank() } ?: return null
            val name = params["name"]
            if (!isHttpServer(serverUrl)) return null
            if (
                hasControlChars(serverUrl) ||
                hasControlChars(clientKey) ||
                hasControlChars(invite)
            ) {
                return null
            }
            if (name != null && hasControlChars(name)) return null

            return SetupLink(
                serverUrl = serverUrl,
                clientKey = clientKey,
                invite = invite,
                name = name?.takeIf { it.isNotBlank() },
            )
        }

        private fun parseQuery(query: String): Map<String, String>? {
            val params = LinkedHashMap<String, String>()
            if (query.isEmpty()) return params
            for (part in query.split('&')) {
                if (part.isEmpty()) continue
                val separator = part.indexOf('=')
                if (separator <= 0) return null
                val key = percentDecode(part.substring(0, separator)) ?: return null
                val value = percentDecode(part.substring(separator + 1)) ?: return null
                if (key.isEmpty()) return null
                // Igual que la TUI: los campos conocidos no pueden repetirse.
                val known = key in KNOWN_PARAMS
                if (known && params.containsKey(key)) return null
                params[key] = value
            }
            return params
        }

        /**
         * Solo se acepta una URL absoluta `http(s)` con host y sin credenciales,
         * query ni fragmento: se usa como base para concatenar rutas de la API.
         */
        private fun isHttpServer(value: String): Boolean {
            val uri = runCatching { URI(value) }.getOrNull() ?: return false
            if (uri.isOpaque) return false
            val scheme = uri.scheme?.lowercase() ?: return false
            if (scheme != "http" && scheme != "https") return false
            if (uri.host.isNullOrBlank()) return false
            if (uri.userInfo != null) return false
            if (uri.rawQuery != null || uri.rawFragment != null) return false
            return true
        }

        private fun hasControlChars(value: String): Boolean =
            value.any { char -> Character.isISOControl(char) }
    }
}

// ---------------------------------------------------------------------------
// Percent-encoding RFC 3986 (sin dependencias de Android para poder testearlo
// en la JVM).
// ---------------------------------------------------------------------------

private const val HEX_DIGITS = "0123456789ABCDEF"

private fun percentEncode(value: String): String {
    val bytes = value.toByteArray(Charsets.UTF_8)
    val out = StringBuilder(bytes.size)
    for (byte in bytes) {
        val code = byte.toInt() and 0xFF
        val char = code.toChar()
        if (
            char in 'A'..'Z' ||
            char in 'a'..'z' ||
            char in '0'..'9' ||
            char == '-' ||
            char == '.' ||
            char == '_' ||
            char == '~'
        ) {
            out.append(char)
        } else {
            out.append('%')
            out.append(HEX_DIGITS[code ushr 4])
            out.append(HEX_DIGITS[code and 0x0F])
        }
    }
    return out.toString()
}

/** Devuelve `null` ante escapes truncados, hex inválido o UTF-8 mal formado. */
private fun percentDecode(value: String): String? {
    val bytes = ByteArrayOutputStream(value.length)
    var index = 0
    while (index < value.length) {
        when (value[index]) {
            '+' -> {
                bytes.write(' '.code)
                index++
            }

            '%' -> {
                if (index + 2 >= value.length) return null
                val high = Character.digit(value[index + 1], 16)
                val low = Character.digit(value[index + 2], 16)
                if (high < 0 || low < 0) return null
                bytes.write((high shl 4) or low)
                index += 3
            }

            else -> {
                val codePoint = value.codePointAt(index)
                val text = String(Character.toChars(codePoint))
                bytes.write(text.toByteArray(Charsets.UTF_8))
                index += Character.charCount(codePoint)
            }
        }
    }

    val decoder = Charsets.UTF_8.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT)
    return runCatching {
        decoder.decode(ByteBuffer.wrap(bytes.toByteArray())).toString()
    }.getOrNull()
}
