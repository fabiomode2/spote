package dev.spote.app.artwork

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import dev.spote.app.model.Cover
import dev.spote.app.model.CoverPalette
import dev.spote.app.rust.RustBridge
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Paletas de portada cacheadas. Usa el mismo extractor del core que la TUI:
 * la identidad de spote (superficies oscuras) se tiñe con los colores reales
 * del álbum. Si la portada no está disponible, la UI conserva el tema base.
 */
object CoverPalettes {

    private val cache = ConcurrentHashMap<String, CoverPalette>()
    private val failed = ConcurrentHashMap.newKeySet<String>()
    private var bridge: RustBridge? = null
    private var context: Context? = null

    fun init(context: Context, bridge: RustBridge?) {
        this.context = context.applicationContext
        this.bridge = bridge
    }

    fun cached(cover: Cover?): CoverPalette? {
        val key = coverKey(cover) ?: return null
        return cache[key]
    }

    suspend fun load(cover: Cover?): CoverPalette? {
        val key = coverKey(cover) ?: return null
        cache[key]?.let { return it }
        if (key in failed) return null
        val bridge = bridge ?: return null
        val context = context ?: return null
        val path = cover?.localPath?.takeIf { it.isNotBlank() }
            ?: cover?.remoteUrl?.let { url ->
                ArtworkResolver.cacheRemote(context, url)
            }
        if (path.isNullOrBlank()) {
            failed += key
            return null
        }
        val palette = withContext(Dispatchers.IO) {
            runCatching { bridge.paletteTheme(path) }.getOrNull()
        }
        if (palette == null) {
            failed += key
        } else {
            cache[key] = palette
        }
        return palette
    }

    private fun coverKey(cover: Cover?): String? =
        cover?.localPath?.takeIf { it.isNotBlank() }
            ?: cover?.remoteUrl?.takeIf { it.isNotBlank() }
}

@Composable
fun rememberCoverPalette(cover: Cover?): CoverPalette? {
    val key = cover?.localPath?.takeIf { it.isNotBlank() }
        ?: cover?.remoteUrl?.takeIf { it.isNotBlank() }
    var palette by remember(key) {
        mutableStateOf(key?.let { CoverPalettes.cached(cover) })
    }
    LaunchedEffect(key) {
        if (key != null && palette == null) {
            palette = CoverPalettes.load(cover)
        }
    }
    return palette
}
