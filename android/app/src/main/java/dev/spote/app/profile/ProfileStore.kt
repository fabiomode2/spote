package dev.spote.app.profile

import android.content.Context
import android.content.SharedPreferences
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Base64
import java.io.File
import java.io.FileOutputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext

/**
 * Perfil local del dispositivo: nombre visible y avatar.
 *
 * El avatar se recorta a un cuadrado de 256 px y se guarda en el almacenamiento
 * privado. El mismo JPEG se puede enviar al servidor social como data URL.
 */
object ProfileStore {

    private const val PREFS = "spote.profile"
    private const val KEY_NAME = "name"
    private const val KEY_AVATAR = "avatar"
    private const val AVATAR_SIDE = 256

    private var prefs: SharedPreferences? = null

    private val _name = MutableStateFlow("")
    val name: StateFlow<String> = _name.asStateFlow()

    private val _avatar = MutableStateFlow<String?>(null)
    val avatar: StateFlow<String?> = _avatar.asStateFlow()

    @Synchronized
    fun init(context: Context) {
        if (prefs != null) return
        val store = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        prefs = store
        _name.value = store.getString(KEY_NAME, "").orEmpty()
        _avatar.value = store.getString(KEY_AVATAR, null)?.takeIf { File(it).isFile }
    }

    fun setName(value: String) {
        val trimmed = value.trim().take(32)
        _name.value = trimmed
        prefs?.edit()?.putString(KEY_NAME, trimmed)?.apply()
    }

    fun clearAvatar() {
        _avatar.value?.let { runCatching { File(it).delete() } }
        _avatar.value = null
        prefs?.edit()?.remove(KEY_AVATAR)?.apply()
    }

    suspend fun importAvatar(context: Context, uri: Uri): String? = withContext(Dispatchers.IO) {
        val dir = File(context.filesDir, "profile").apply { mkdirs() }
        val target = File(dir, "avatar.jpg")
        val bitmap = context.contentResolver.openInputStream(uri)?.use(BitmapFactory::decodeStream)
            ?: return@withContext null
        val side = minOf(bitmap.width, bitmap.height)
        if (side <= 0) return@withContext null
        val square = Bitmap.createBitmap(
            bitmap,
            (bitmap.width - side) / 2,
            (bitmap.height - side) / 2,
            side,
            side,
        )
        val scaled = Bitmap.createScaledBitmap(square, AVATAR_SIDE, AVATAR_SIDE, true)
        FileOutputStream(target).use { output ->
            scaled.compress(Bitmap.CompressFormat.JPEG, 82, output)
        }
        if (square !== bitmap) square.recycle()
        if (scaled !== square) scaled.recycle()
        bitmap.recycle()
        _avatar.value = target.absolutePath
        prefs?.edit()?.putString(KEY_AVATAR, target.absolutePath)?.apply()
        target.absolutePath
    }

    /** Data URL lista para `PUT /api/v1/me/avatar`. */
    fun avatarDataUrl(): String? {
        val path = _avatar.value ?: return null
        val file = File(path)
        if (!file.isFile) return null
        val bytes = runCatching { file.readBytes() }.getOrNull() ?: return null
        if (bytes.size > 320_000) return null
        return "data:image/jpeg;base64," + Base64.encodeToString(bytes, Base64.NO_WRAP)
    }
}
