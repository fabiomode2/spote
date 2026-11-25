package dev.spote.app.data

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Búsquedas recientes persistidas, sin duplicados y con límite. */
object SearchHistory {

    private const val PREFS = "spote.search"
    private const val KEY = "recent"
    private const val LIMIT = 8

    private var prefs: android.content.SharedPreferences? = null
    private val _queries = MutableStateFlow<List<String>>(emptyList())
    val queries: StateFlow<List<String>> = _queries.asStateFlow()

    @Synchronized
    fun init(context: Context) {
        if (prefs != null) return
        val store = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        prefs = store
        _queries.value = store.getString(KEY, "")
            .orEmpty()
            .split('\n')
            .filter { it.isNotBlank() }
            .take(LIMIT)
    }

    fun record(query: String) {
        val trimmed = query.trim()
        if (trimmed.length < 2) return
        val next = (listOf(trimmed) + _queries.value.filterNot { it.equals(trimmed, true) })
            .take(LIMIT)
        _queries.value = next
        prefs?.edit { putString(KEY, next.joinToString("\n")) }
    }

    fun clear() {
        _queries.value = emptyList()
        prefs?.edit { remove(KEY) }
    }
}
