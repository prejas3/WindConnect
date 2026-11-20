package io.github.prejas3.windconnect.ui

import android.os.Build
import android.app.Activity
import android.graphics.drawable.ColorDrawable
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.ExperimentalTextApi
import androidx.compose.ui.unit.sp
import io.github.prejas3.windconnect.R

enum class ThemeMode { System, Light, Dark }

private tailrec fun Context.activity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}

// CCS/1 DEP:accepted WN-20, DM Sans OFL | RULE:Android 10/11 use the same brand palette.
val BrandLight = lightColorScheme(
    primary = Color(0xFF0D7F74), onPrimary = Color.White,
    primaryContainer = Color(0xFFD7EFEB), onPrimaryContainer = Color(0xFF0B5E55),
    background = Color(0xFFF8FBFB), onBackground = Color(0xFF15282F),
    surface = Color(0xFFF8FBFB), onSurface = Color(0xFF15282F),
    surfaceVariant = Color(0xFFE8EFEF), onSurfaceVariant = Color(0xFF52686E),
    outline = Color(0xFFCCDADB), error = Color(0xFFAD2838),
)
val BrandDark = darkColorScheme(
    primary = Color(0xFF43C9B8), onPrimary = Color(0xFF062E28),
    primaryContainer = Color(0xFF1F5C55), onPrimaryContainer = Color(0xFFDFF7F2),
    background = Color(0xFF16242B), onBackground = Color(0xFFE9F1F1),
    surface = Color(0xFF16242B), onSurface = Color(0xFFE9F1F1),
    surfaceVariant = Color(0xFF22343C), onSurfaceVariant = Color(0xFFA1B2B6),
    outline = Color(0xFF34454B), error = Color(0xFFFF9DA9),
)
// CCS/1 RULE:FontWeight metadata alone does not instantiate a variable font's wght axis.
@OptIn(ExperimentalTextApi::class)
private fun dmFont(weight: FontWeight) = Font(
    R.font.dm_sans, weight,
    variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)),
)
private val DmSans = FontFamily(
    dmFont(FontWeight.Normal), dmFont(FontWeight.Medium),
    dmFont(FontWeight.SemiBold), dmFont(FontWeight.Bold),
)
private fun type(size: Int, line: Int, weight: FontWeight = FontWeight.Normal) = TextStyle(
    fontFamily = DmSans, fontWeight = weight, fontSize = size.sp, lineHeight = line.sp,
)
val WindTypography = Typography(
    displayLarge = type(40, 48, FontWeight.Bold), displayMedium = type(36, 44, FontWeight.Bold), displaySmall = type(32, 40, FontWeight.Bold),
    headlineLarge = type(28, 36, FontWeight.Bold), headlineMedium = type(25, 32, FontWeight.Bold), headlineSmall = type(22, 30, FontWeight.Bold),
    titleLarge = type(21, 28, FontWeight.Bold), titleMedium = type(16, 24, FontWeight.SemiBold), titleSmall = type(14, 20, FontWeight.SemiBold),
    bodyLarge = type(16, 24), bodyMedium = type(14, 21), bodySmall = type(12, 18),
    labelLarge = type(14, 20, FontWeight.SemiBold), labelMedium = type(12, 18, FontWeight.SemiBold), labelSmall = type(11, 16, FontWeight.Medium),
)

/** WCAG contrast ratio of two opaque colours (1..21). */
fun contrast(a: Color, b: Color): Double {
    val la = a.luminance().toDouble(); val lb = b.luminance().toDouble()
    return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
}

/** [fg] moved towards [toward] in small steps until it reaches [ratio] on [bg] (unchanged if it already does). */
fun readableOn(fg: Color, bg: Color, toward: Color, ratio: Double): Color {
    var color = fg
    var step = 0
    while (contrast(color, bg) < ratio && step < 20) { step++; color = lerp(fg, toward, step / 20f) }
    return color
}

/**
 * WN-160: one readable scheme for the app and the widget, whatever the palette. Under Material You the cards were the
 * light grey surfaceVariant with grey text on them (Samsung): cards take the calmer surfaceContainerHigh, and secondary
 * text, accents and lines on a card are pushed to a sure contrast (text ≥ 7:1, accents ≥ 4.5:1, lines ≥ 1.8:1).
 */
fun readable(scheme: ColorScheme, dynamic: Boolean): ColorScheme {
    val card = if (dynamic) scheme.surfaceContainerHigh else scheme.surfaceVariant
    return scheme.copy(
        surfaceVariant = card,
        onSurfaceVariant = readableOn(scheme.onSurfaceVariant, card, scheme.onSurface, 7.0),
        primary = readableOn(scheme.primary, card, scheme.onSurface, 4.5),
        outline = readableOn(scheme.outline, card, scheme.onSurface, 1.8),
        error = readableOn(scheme.error, card, scheme.onSurface, 4.5),
    )
}

@Composable
fun WindConnectTheme(mode: ThemeMode = ThemeMode.System, dynamicColor: Boolean = true, content: @Composable () -> Unit) {
    val dark = when (mode) { ThemeMode.System -> isSystemInDarkTheme(); ThemeMode.Light -> false; ThemeMode.Dark -> true }
    val dynamic = dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val colors = readable(if (dynamic) {
        if (dark) dynamicDarkColorScheme(LocalContext.current) else dynamicLightColorScheme(LocalContext.current)
    } else if (dark) BrandDark else BrandLight, dynamic)
    val view = LocalView.current
    SideEffect {
        view.context.activity()?.window?.let { window ->
            // CCS/1 RULE:system-bar backing follows the app's selected theme, including manual overrides.
            window.setBackgroundDrawable(ColorDrawable(colors.background.toArgb()))
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    MaterialTheme(colorScheme = colors, typography = WindTypography) {
        CompositionLocalProvider(LocalContentColor provides colors.onBackground, content = content)
    }
}
