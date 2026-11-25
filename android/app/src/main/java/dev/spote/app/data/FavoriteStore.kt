package dev.spote.app.data

import android.content.Context
import androidx.core.content.edit

/** Favoritas de pistas de catálogo que todavía no están en SQLite. */
object FavoriteStore {

    private const val PREFS = "spote.favorites"
    private const val KEY = "keys"

    private var prefs: android.content.SharedPreferences? = null
    private val keys = linkedSetOf<String>()

    @Synchronized
    fun init(context: Context) {
        if (prefs != null) return
        val store = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        prefs = store
        keys.clear()
        keys.addAll(store.getStringSet(KEY, emptySet()).orEmpty())
    }

    fun isFavorite(key: String): Boolean = key in keys

    fun toggle(key: String): Boolean {
        val next = if (key in keys) {
            keys.remove(key)
            false
        } else {
            keys.add(key)
            true
        }
        prefs?.edit { putStringSet(KEY, keys.toSet()) }
        return next
    }

    fun remove(key: String) {
        if (keys.remove(key)) {
            prefs?.edit { putStringSet(KEY, keys.toSet()) }
        }
    }
}
