package dev.spote.app.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val MidnightTypography = Typography(
    displayLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 34.sp,
        lineHeight = 40.sp,
        letterSpacing = (-0.9).sp,
    ),
    displayMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 30.sp,
        lineHeight = 36.sp,
        letterSpacing = (-0.7).sp,
    ),
    headlineLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 28.sp,
        lineHeight = 34.sp,
        letterSpacing = (-0.6).sp,
    ),
    headlineMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 24.sp,
        lineHeight = 30.sp,
        letterSpacing = (-0.4).sp,
    ),
    headlineSmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 20.sp,
        lineHeight = 26.sp,
        letterSpacing = (-0.2).sp,
    ),
    titleLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 18.sp,
        lineHeight = 24.sp,
        letterSpacing = (-0.1).sp,
    ),
    titleMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 15.sp,
        lineHeight = 20.sp,
    ),
    titleSmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Medium,
        fontSize = 13.sp,
        lineHeight = 18.sp,
    ),
    bodyLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 15.sp,
        lineHeight = 22.sp,
    ),
    bodyMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 13.sp,
        lineHeight = 19.sp,
    ),
    bodySmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 12.sp,
        lineHeight = 17.sp,
    ),
    labelLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 13.sp,
        lineHeight = 17.sp,
        letterSpacing = 0.1.sp,
    ),
    labelMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Medium,
        fontSize = 11.sp,
        lineHeight = 15.sp,
        letterSpacing = 0.3.sp,
    ),
    labelSmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Medium,
        fontSize = 10.sp,
        lineHeight = 14.sp,
        letterSpacing = 0.4.sp,
    ),
)

/**
 * La serif solo entra en los titulares. El resto sigue en sans para que la
 * lista de canciones se lea rápido; es la diferencia entre editorial y disfraz.
 */
private val NocturneTypography = MidnightTypography.copy(
    displayLarge = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Normal,
        fontSize = 38.sp,
        lineHeight = 44.sp,
        letterSpacing = (-0.4).sp,
    ),
    displayMedium = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Normal,
        fontSize = 34.sp,
        lineHeight = 40.sp,
        letterSpacing = (-0.3).sp,
    ),
    headlineLarge = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Normal,
        fontSize = 30.sp,
        lineHeight = 36.sp,
        letterSpacing = (-0.2).sp,
    ),
    headlineMedium = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Normal,
        fontSize = 26.sp,
        lineHeight = 32.sp,
        letterSpacing = (-0.1).sp,
    ),
    headlineSmall = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Medium,
        fontSize = 21.sp,
        lineHeight = 27.sp,
    ),
)

private val SpoteShapes = Shapes(
    extraSmall = RoundedCornerShape(6.dp),
    small = RoundedCornerShape(10.dp),
    medium = RoundedCornerShape(14.dp),
    large = RoundedCornerShape(20.dp),
    extraLarge = RoundedCornerShape(28.dp),
)

@Composable
fun SpoteTheme(
    mode: SpoteThemeMode = SpoteThemeMode.Midnight,
    uiStyle: SpoteUiStyle = SpoteUiStyle.Medium,
    content: @Composable () -> Unit,
) {
    val colors = mode.colors()
    val baseTypography = when (mode) {
        SpoteThemeMode.Midnight -> MidnightTypography
        SpoteThemeMode.Nocturne -> NocturneTypography
    }
    val typography = if (uiStyle == SpoteUiStyle.Off) {
        baseTypography
    } else {
        val mono = FontFamily.Monospace
        baseTypography.copy(
            displayLarge = baseTypography.displayLarge.copy(fontFamily = mono),
            displayMedium = baseTypography.displayMedium.copy(fontFamily = mono),
            headlineLarge = baseTypography.headlineLarge.copy(fontFamily = mono),
            headlineMedium = baseTypography.headlineMedium.copy(fontFamily = mono),
            headlineSmall = baseTypography.headlineSmall.copy(fontFamily = mono),
            titleLarge = baseTypography.titleLarge.copy(fontFamily = mono),
            titleMedium = baseTypography.titleMedium.copy(fontFamily = mono),
            titleSmall = baseTypography.titleSmall.copy(fontFamily = mono),
            labelLarge = baseTypography.labelLarge.copy(fontFamily = mono),
            labelMedium = baseTypography.labelMedium.copy(fontFamily = mono),
            labelSmall = baseTypography.labelSmall.copy(fontFamily = mono),
        )
    }
    val scheme = darkColorScheme(
        primary = colors.accent,
        onPrimary = colors.accentInk,
        primaryContainer = colors.surfaceHigh,
        onPrimaryContainer = colors.onBackground,
        secondary = colors.accent,
        onSecondary = colors.accentInk,
        background = colors.background,
        onBackground = colors.onBackground,
        surface = colors.surface,
        onSurface = colors.onBackground,
        surfaceVariant = colors.surfaceHigh,
        onSurfaceVariant = colors.onSurfaceVariant,
        surfaceContainer = colors.surface,
        surfaceContainerHigh = colors.surfaceHigh,
        outline = colors.outline,
        outlineVariant = colors.outline,
        error = colors.danger,
    )

    CompositionLocalProvider(
        LocalSpoteColors provides colors,
        LocalSpoteUiStyle provides uiStyle,
    ) {
        MaterialTheme(
            colorScheme = scheme,
            typography = typography,
            shapes = SpoteShapes,
            content = content,
        )
    }
}
