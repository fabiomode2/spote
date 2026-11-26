package dev.spote.app.ui.components

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp
import dev.spote.app.ui.theme.LocalSpoteColors
import dev.spote.app.ui.theme.LocalSpoteUiStyle
import dev.spote.app.ui.theme.SpoteUiStyle
import kotlin.math.PI
import kotlin.math.sin

/** Líneas horizontales muy suaves; solo en el nivel Medio. */
@Composable
fun ScanlineOverlay(modifier: Modifier = Modifier) {
    if (LocalSpoteUiStyle.current != SpoteUiStyle.Medium) return
    val color = LocalSpoteColors.current.onBackground.copy(alpha = 0.035f)
    Canvas(modifier = modifier) {
        val step = 3.dp.toPx().coerceAtLeast(2f)
        var y = 0f
        while (y < size.height) {
            drawLine(
                color = color,
                start = Offset(0f, y),
                end = Offset(size.width, y),
                strokeWidth = 1f,
            )
            y += step
        }
    }
}

/**
 * Espectro de barras con fase continua. Es decorativo: no analiza audio, solo
 * mantiene la sensación de terminal viva sin animar toda la pantalla.
 */
@Composable
fun SpectrumStrip(
    playing: Boolean,
    modifier: Modifier = Modifier,
    bars: Int = 28,
) {
    if (LocalSpoteUiStyle.current != SpoteUiStyle.Medium) return
    val colors = LocalSpoteColors.current
    val transition = rememberInfiniteTransition(label = "spectrum")
    val phase by transition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 2200, easing = LinearEasing),
            repeatMode = RepeatMode.Restart,
        ),
        label = "spectrum-phase",
    )
    val activeColor = colors.accent
    val idleColor = colors.accent.copy(alpha = 0.25f)
    Canvas(modifier = modifier) {
        if (bars <= 0 || size.width <= 0f) return@Canvas
        val gap = 2.dp.toPx()
        val barWidth = ((size.width - gap * (bars - 1)) / bars).coerceAtLeast(1f)
        val radius = CornerRadius(barWidth / 2f, barWidth / 2f)
        repeat(bars) { index ->
            val angle = phase * 2f * PI.toFloat() + index * 0.58f
            val wave = sin(angle) * 0.5f + 0.5f
            val energy = if (playing) 1f else 0.22f
            val height = size.height * (0.12f + 0.88f * wave * energy)
            drawRoundRect(
                color = if (playing) activeColor else idleColor,
                topLeft = Offset(index * (barWidth + gap), size.height - height),
                size = Size(barWidth, height),
                cornerRadius = radius,
            )
        }
    }
}

/** Marco de terminal opcional para tarjetas destacadas. */
@Composable
fun TuiFrame(color: Color, modifier: Modifier = Modifier) {
    if (LocalSpoteUiStyle.current == SpoteUiStyle.Off) return
    Canvas(modifier = modifier) {
        drawRect(
            color = color.copy(alpha = 0.16f),
            style = Stroke(width = 1.dp.toPx()),
        )
    }
}
