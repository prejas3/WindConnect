package io.github.prejas3.windconnect.ui

import android.os.Build
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
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
private val DmSans = FontFamily(
    Font(R.font.dm_sans, FontWeight.Normal),
    Font(R.font.dm_sans, FontWeight.Medium),
    Font(R.font.dm_sans, FontWeight.SemiBold),
    Font(R.font.dm_sans, FontWeight.Bold),
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

@Composable
fun WindConnectTheme(mode: ThemeMode = ThemeMode.System, dynamicColor: Boolean = true, content: @Composable () -> Unit) {
    val dark = when (mode) { ThemeMode.System -> isSystemInDarkTheme(); ThemeMode.Light -> false; ThemeMode.Dark -> true }
    val colors = if (dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        if (dark) dynamicDarkColorScheme(LocalContext.current) else dynamicLightColorScheme(LocalContext.current)
    } else if (dark) BrandDark else BrandLight
    val view = LocalView.current
    SideEffect {
        view.context.activity()?.window?.let { window ->
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
