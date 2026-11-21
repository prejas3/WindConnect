package io.github.prejas3.windconnect.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.SpringSpec
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.composed

/**
 * WN-169: the M3 motion springs (material-components-android docs/theming/Motion.md). Spatial (position, size): damping
 * 0.9, stiffness fast 1400 / default 700 / slow 300. Effects (colour, opacity): damping 1.0, stiffness 3800 / 1600 / 800.
 */
object WindMotion {
    fun <T> fastSpatial(): SpringSpec<T> = spring(dampingRatio = 0.9f, stiffness = 1400f)
    fun <T> defaultSpatial(): SpringSpec<T> = spring(dampingRatio = 0.9f, stiffness = 700f)
    fun <T> slowSpatial(): SpringSpec<T> = spring(dampingRatio = 0.9f, stiffness = 300f)
    fun <T> fastEffects(): SpringSpec<T> = spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = 3800f)
    fun <T> defaultEffects(): SpringSpec<T> = spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = 1600f)
    fun <T> slowEffects(): SpringSpec<T> = spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = 800f)
}

/**
 * WN-169: true when the person turned animations off (Settings › Accessibility › Remove animations, or the animator
 * duration scale at 0). [WindConnectTheme] provides it; every Wind animation then shows its end state without motion.
 */
val LocalReduceMotion = staticCompositionLocalOf { false }

fun reduceMotion(context: Context): Boolean =
    runCatching { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }.getOrDefault(false)

/** A [spec], or an immediate jump when motion is reduced. */
@Composable
@ReadOnlyComposable
fun <T> motion(spec: FiniteAnimationSpec<T>): FiniteAnimationSpec<T> = if (LocalReduceMotion.current) snap() else spec

/** WN-169: M3 shared axis X: forward slides in from the end, back from the start; both cross-fade. */
fun <S> AnimatedContentTransitionScope<S>.sharedAxisX(forward: Boolean, reduce: Boolean, density: Density): ContentTransform {
    if (reduce) return EnterTransition.None togetherWith ExitTransition.None
    val shift = with(density) { 30.dp.roundToPx() }
    val sign = if (forward) 1 else -1
    return (slideInHorizontally(WindMotion.defaultSpatial<IntOffset>()) { sign * shift } + fadeIn(WindMotion.defaultEffects())) togetherWith
        (slideOutHorizontally(WindMotion.defaultSpatial<IntOffset>()) { -sign * shift } + fadeOut(WindMotion.fastEffects()))
}

/** WN-169: details opening under a row (the (i), the clipboard history): expand + fade, or no motion when reduced. */
fun expandIn(reduce: Boolean): EnterTransition =
    if (reduce) EnterTransition.None else expandVertically(WindMotion.defaultSpatial<IntSize>()) + fadeIn(WindMotion.defaultEffects())

fun shrinkOut(reduce: Boolean): ExitTransition =
    if (reduce) ExitTransition.None else shrinkVertically(WindMotion.defaultSpatial<IntSize>()) + fadeOut(WindMotion.fastEffects())

/**
 * WN-170: a pane that slides in by 24 dp and fades in when it first appears (switching the touchpad's mode). It is
 * only a graphics layer: what is inside is not recreated or put in a transition. No motion when animations are off.
 */
fun androidx.compose.ui.Modifier.enterFrom(): androidx.compose.ui.Modifier = composed {
    val reduce = LocalReduceMotion.current
    val shown = androidx.compose.runtime.remember { androidx.compose.animation.core.Animatable(if (reduce) 1f else 0f) }
    androidx.compose.runtime.LaunchedEffect(Unit) { shown.animateTo(1f, WindMotion.defaultSpatial()) }
    graphicsLayer {
        alpha = shown.value.coerceIn(0f, 1f)
        translationX = (1f - shown.value) * 24.dp.toPx()
    }
}
