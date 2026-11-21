package io.github.prejas3.windconnect.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.sin

/**
 * WN-169: the one linear progress (M3 ProgressIndicator): round 4 dp track ([thick]: 8 dp, Expressive), the active part
 * primary, the rest primaryContainer after a 4 dp gap, a stop dot at the end. [progress] null = indeterminate: a segment
 * runs along the track (never a frozen bar). [wavy] (transfers) draws the active part as a moving wave. With motion
 * reduced it moves nothing: determinate values jump, indeterminate shows the bare track.
 */
@Composable
fun WindLinearProgress(progress: Float?, modifier: Modifier = Modifier, wavy: Boolean = false, thick: Boolean = false) {
    val colors = MaterialTheme.colorScheme
    val reduce = LocalReduceMotion.current
    val thickness = if (thick) 8.dp else 4.dp
    val amplitude = if (wavy) 3.dp else 0.dp
    val shown by animateFloatAsState((progress ?: 0f).coerceIn(0f, 1f), motion(WindMotion.defaultSpatial()), label = "progress")
    val loop = rememberInfiniteTransition(label = "progressLoop")
    val run by loop.animateFloat(0f, 1f, infiniteRepeatable(tween(1750, easing = LinearEasing)), label = "indeterminate")
    val phase by loop.animateFloat(0f, 1f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "wave")
    Canvas(modifier.fillMaxWidth().height(thickness + amplitude * 2).semantics {
        progressBarRangeInfo = progress?.let { ProgressBarRangeInfo(it.coerceIn(0f, 1f), 0f..1f) } ?: ProgressBarRangeInfo.Indeterminate
    }) {
        val stroke = thickness.toPx(); val gap = 4.dp.toPx(); val y = size.height / 2; val cap = stroke / 2
        val left = cap; val right = size.width - cap
        fun line(from: Float, to: Float, color: androidx.compose.ui.graphics.Color) {
            if (to - from >= 0f) drawLine(color, Offset(from, y), Offset(to, y), stroke, StrokeCap.Round)
        }
        if (progress == null) {
            if (reduce) { line(left, right, colors.primaryContainer); return@Canvas }
            // A segment whose head leads its tail: both eased, so it stretches then shrinks as it crosses.
            val head = FastOutSlowInEasing.transform((run * 1.3f).coerceIn(0f, 1f))
            val tail = FastOutSlowInEasing.transform((run * 1.3f - .3f).coerceIn(0f, 1f))
            val a = left + (right - left) * tail; val b = left + (right - left) * head
            line(left, a - gap - stroke, colors.primaryContainer)
            line(a, b, colors.primary)
            line(b + gap + stroke, right, colors.primaryContainer)
            return@Canvas
        }
        val end = left + (right - left) * shown
        val restStart = end + gap + stroke
        line(restStart, right, colors.primaryContainer)
        // The stop dot: where the track ends, in the active colour.
        if (right - restStart > stroke) drawCircle(colors.primary, minOf(cap, 2.dp.toPx()), Offset(right, y))
        if (shown <= 0f) return@Canvas
        if (wavy && shown < 1f) wave(left, end, y, amplitude.toPx(), 40.dp.toPx(), if (reduce) 0f else phase, stroke, colors.primary)
        else line(left, end, colors.primary)
    }
}

private fun DrawScope.wave(from: Float, to: Float, y: Float, amplitude: Float, length: Float, phase: Float, stroke: Float, color: androidx.compose.ui.graphics.Color) {
    val path = Path()
    var x = from
    path.moveTo(x, y + amplitude * sin(2 * PI * (x / length - phase)).toFloat())
    while (x < to) { x = minOf(to, x + 2f); path.lineTo(x, y + amplitude * sin(2 * PI * (x / length - phase)).toFloat()) }
    drawPath(path, color, style = Stroke(stroke, cap = StrokeCap.Round))
}

/**
 * WN-169: the circular indeterminate progress for waits (pairing: searching, connecting, verifying): a 4 dp arc that turns
 * and breathes over a primaryContainer track, 4 dp apart. With motion reduced it stands still.
 */
@Composable
fun WindCircularProgress(modifier: Modifier = Modifier, diameter: Dp = 40.dp) {
    val colors = MaterialTheme.colorScheme
    val reduce = LocalReduceMotion.current
    val loop = rememberInfiniteTransition(label = "spinner")
    val turn by loop.animateFloat(0f, 360f, infiniteRepeatable(tween(1400, easing = LinearEasing)), label = "turn")
    val sweep by loop.animateFloat(40f, 270f, infiniteRepeatable(tween(900, easing = FastOutSlowInEasing), RepeatMode.Reverse), label = "sweep")
    Canvas(modifier.size(diameter).semantics { progressBarRangeInfo = ProgressBarRangeInfo.Indeterminate }) {
        val stroke = 4.dp.toPx()
        val inset = stroke / 2
        val arcSize = Size(size.width - stroke, size.height - stroke)
        val radius = arcSize.width / 2
        // The gap (4 dp) plus the round caps, as an angle on this circle.
        val gapAngle = ((4.dp.toPx() + stroke) / radius * 180f / PI).toFloat()
        val start = if (reduce) -90f else turn - 90f
        val active = if (reduce) 270f else sweep
        drawArc(colors.primaryContainer, start + active + gapAngle, 360f - active - 2 * gapAngle, false, Offset(inset, inset), arcSize, style = Stroke(stroke, cap = StrokeCap.Round))
        drawArc(colors.primary, start, active, false, Offset(inset, inset), arcSize, style = Stroke(stroke, cap = StrokeCap.Round))
    }
}
