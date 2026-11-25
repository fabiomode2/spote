package dev.spote.app.ui

import android.content.Context
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import dev.spote.app.ui.components.ScanlineOverlay
import dev.spote.app.ui.theme.SpoteTheme
import dev.spote.app.ui.theme.SpoteThemeMode
import dev.spote.app.ui.theme.SpoteUiStyle

@Composable
fun SpoteRoot() {
    val context = LocalContext.current
    val prefs = remember {
        context.getSharedPreferences("spote.ui", Context.MODE_PRIVATE)
    }
    var modeName by rememberSaveable {
        mutableStateOf(prefs.getString("theme", SpoteThemeMode.Midnight.name) ?: SpoteThemeMode.Midnight.name)
    }
    var styleName by rememberSaveable {
        mutableStateOf(prefs.getString("style", SpoteUiStyle.Medium.name) ?: SpoteUiStyle.Medium.name)
    }
    val mode = SpoteThemeMode.fromName(modeName)
    val uiStyle = SpoteUiStyle.fromName(styleName)

    SpoteTheme(mode = mode, uiStyle = uiStyle) {
        Box(modifier = Modifier.fillMaxSize()) {
            SpoteApp(
                themeMode = mode,
                onThemeModeChange = { next ->
                    modeName = next.name
                    prefs.edit().putString("theme", next.name).apply()
                    dev.spote.app.logging.ActionLog.log("settings.theme", "mode" to next.name)
                },
                uiStyle = uiStyle,
                onUiStyleChange = { next ->
                    styleName = next.name
                    prefs.edit().putString("style", next.name).apply()
                    dev.spote.app.logging.ActionLog.log("settings.style", "style" to next.name)
                },
            )
            ScanlineOverlay(Modifier.matchParentSize())
        }
    }
}
