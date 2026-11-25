package dev.spote.app.ui

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import dev.spote.app.ui.theme.SpoteTheme
import dev.spote.app.ui.theme.SpoteThemeMode

@Composable
fun SpoteRoot() {
    val context = LocalContext.current
    val prefs = remember {
        context.getSharedPreferences("spote.ui", Context.MODE_PRIVATE)
    }
    var modeName by rememberSaveable {
        mutableStateOf(prefs.getString("theme", SpoteThemeMode.Midnight.name) ?: SpoteThemeMode.Midnight.name)
    }
    val mode = SpoteThemeMode.fromName(modeName)

    SpoteTheme(mode = mode) {
        SpoteApp(
            themeMode = mode,
            onThemeModeChange = { next ->
                modeName = next.name
                prefs.edit().putString("theme", next.name).apply()
            },
        )
    }
}
