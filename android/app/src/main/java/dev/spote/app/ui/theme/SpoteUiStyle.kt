package dev.spote.app.ui.theme

import androidx.compose.runtime.staticCompositionLocalOf

/** Nivel de estética terminal que se aplica sobre el tema activo. */
enum class SpoteUiStyle(val label: String, val subtitle: String) {
    Off("Apagado", "Solo el tema, sin adornos"),
    Subtle("Sutil", "Monoespaciada en títulos y tiempos"),
    Medium("Medio", "Mono, scanlines y espectro discreto"),
    ;

    companion object {
        fun fromName(value: String?): SpoteUiStyle =
            entries.firstOrNull { it.name == value } ?: Off
    }
}

val LocalSpoteUiStyle = staticCompositionLocalOf { SpoteUiStyle.Off }
