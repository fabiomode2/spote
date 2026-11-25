package dev.spote.app.logging

import java.util.Locale
import org.json.JSONObject

/** Marca con la que se sustituyen los valores sensibles en el log. */
const val REDACTED_VALUE = "<redacted>"

private val SENSITIVE_MARKERS = listOf("password", "token", "secret", "key", "invite")

/** `true` si la clave debe ocultarse (contiene password/token/secret/key/invite). */
internal fun isSensitiveKey(key: String): Boolean {
    val normalized = key.lowercase(Locale.ROOT)
    return SENSITIVE_MARKERS.any { marker -> normalized.contains(marker) }
}

/**
 * Una línea del log de acciones (JSONL).
 *
 * Estructura:
 *
 * ```json
 * {"timestamp":"2026-11-25T12:34:56.789Z","sessionId":"...","event":"login","data":{"server":"https://..."}}
 * ```
 *
 * [encode] escribe una sola línea física (los saltos de línea internos quedan
 * escapados) y redacta las claves sensibles antes de serializar.
 */
data class ActionLogLine(
    val timestamp: String,
    val sessionId: String,
    val event: String,
    val data: Map<String, Any?> = emptyMap(),
) {

    /** Serializa la línea y aplica la redacción de datos sensibles. */
    fun encode(): String = encodeLine(timestamp, sessionId, event, data)

    companion object {

        /** Parsea una línea JSONL. Devuelve `null` si no tiene el formato esperado. */
        fun parse(line: String?): ActionLogLine? {
            if (line.isNullOrBlank()) return null
            return runCatching {
                val json = JSONObject(line)
                val timestamp = json.getString("timestamp")
                val sessionId = json.getString("sessionId")
                val event = json.getString("event")
                require(timestamp.isNotBlank() && sessionId.isNotBlank() && event.isNotBlank())
                ActionLogLine(
                    timestamp = timestamp,
                    sessionId = sessionId,
                    event = event,
                    data = parseData(json.optJSONObject("data")),
                )
            }.getOrNull()
        }

        private fun parseData(json: JSONObject?): Map<String, Any?> {
            if (json == null) return emptyMap()
            val data = LinkedHashMap<String, Any?>()
            val keys = json.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                val value = json.opt(key)
                data[key] = if (value == null || value == JSONObject.NULL) null else value
            }
            return data
        }
    }
}

internal fun encodeLine(
    timestamp: String,
    sessionId: String,
    event: String,
    data: Map<String, Any?>,
): String {
    val root = JSONObject()
    root.put("timestamp", timestamp)
    root.put("sessionId", sessionId)
    root.put("event", event)

    val payload = JSONObject()
    data.forEach { (key, value) -> payload.put(key, jsonValue(key, value)) }
    root.put("data", payload)

    return root.toString()
}

private fun jsonValue(key: String, value: Any?): Any {
    if (isSensitiveKey(key)) return REDACTED_VALUE
    return when (value) {
        null -> JSONObject.NULL
        is String, is Boolean -> value
        is Int, is Long, is Short, is Byte -> value
        is Float -> if (value.isFinite()) value else value.toString()
        is Double -> if (value.isFinite()) value else value.toString()
        else -> runCatching { value.toString() }.getOrDefault("<unserializable>")
    }
}
