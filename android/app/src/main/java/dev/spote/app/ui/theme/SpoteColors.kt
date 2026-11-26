package dev.spote.app.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance

@Immutable
data class SpoteColors(
    val accent: Color,
    val accentInk: Color,
    val background: Color,
    val surface: Color,
    val surfaceHigh: Color,
    val onBackground: Color,
    val onSurfaceVariant: Color,
    val outline: Color,
    val coverStart: Color,
    val coverEnd: Color,
    val success: Color,
    val danger: Color,
) {
    companion object {
        val Night = SpoteColors(
            accent = Color(0xFFC6F24E),
            accentInk = Color(0xFF0B0F02),
            background = Color(0xFF070708),
            surface = Color(0xFF121214),
            surfaceHigh = Color(0xFF1C1C21),
            onBackground = Color(0xFFF4F4F6),
            onSurfaceVariant = Color(0xFF9B9BA5),
            outline = Color(0xFF2A2A31),
            coverStart = Color(0xFF3A3A42),
            coverEnd = Color(0xFF15151A),
            success = Color(0xFF63D68B),
            danger = Color(0xFFFF6B6B),
        )

        /**
         * Cálido y editorial, con la portada mandando y un coral de acento.
         * Menos "app de streaming genérica", más fanzine/vinilo de noche.
         */
        val Nocturne = SpoteColors(
            accent = Color(0xFFF18477),
            accentInk = Color(0xFF2A120C),
            background = Color(0xFF140C06),
            surface = Color(0xFF24170D),
            surfaceHigh = Color(0xFF322114),
            onBackground = Color(0xFFF8EEE5),
            onSurfaceVariant = Color(0xFFC0AD9C),
            outline = Color(0xFF3A2A1C),
            coverStart = Color(0xFF4A3020),
            coverEnd = Color(0xFF140C06),
            success = Color(0xFF7FD6B0),
            danger = Color(0xFFFF7A6B),
        )
    }
}

val LocalSpoteColors = staticCompositionLocalOf { SpoteColors.Night }

fun SpoteColors.tintedWith(start: Color, end: Color): SpoteColors = copy(
    accent = start,
    accentInk = if (start.luminance() > 0.55f) Color(0xFF0B0F02) else Color.White,
    coverStart = start,
    coverEnd = end,
)
