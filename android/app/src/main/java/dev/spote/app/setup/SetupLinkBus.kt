package dev.spote.app.setup

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Buzón de proceso para el último enlace de configuración recibido.
 *
 * [MainActivity] publica aquí lo que llega por `spote://setup` y la UI observa
 * [current]. Se usa `StateFlow` (y no un evento efímero) para no perder el
 * enlace si la pantalla que lo consume aún no está suscrita cuando se abre la
 * app desde el QR. La pantalla debe llamar a [consume] después de aplicarlo
 * para que no se vuelva a ofrecer.
 */
object SetupLinkBus {

    private val _current = MutableStateFlow<SetupLink?>(null)

    /** Último enlace pendiente, o `null` si no hay ninguno. */
    val current: StateFlow<SetupLink?> = _current.asStateFlow()

    /** Publica un enlace ya validado. */
    fun publish(link: SetupLink) {
        _current.value = link
    }

    /**
     * Valida y publica un enlace en crudo (por ejemplo `intent.dataString`).
     * Devuelve `false` sin publicar nada si no es un `spote://setup` válido.
     */
    fun publish(raw: String?): Boolean {
        val link = SetupLink.parse(raw) ?: return false
        publish(link)
        return true
    }

    /** Devuelve el enlace pendiente y lo marca como consumido. */
    fun consume(): SetupLink? {
        val link = _current.value
        _current.value = null
        return link
    }

    /** Descarta el enlace pendiente sin aplicarlo. */
    fun clear() {
        _current.value = null
    }
}
