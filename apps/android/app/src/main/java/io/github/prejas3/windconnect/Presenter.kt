package io.github.prejas3.windconnect

import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.SystemClock
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import io.github.prejas3.windconnect.ui.*
import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInteropFilter
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.bindings.TouchpadCommand
import kotlinx.coroutines.delay

/** CCS/1 M:presenter (WN-53) | RULE:slides are plain input/3 arrow keys (PowerPoint, browsers: no setup); laser needs input/4. */
internal object PresenterKeys {
    const val NEXT = 39L
    const val PREVIOUS = 37L
    /** Volume up: next slide, volume down: previous; other keys are not ours. */
    fun slide(keyCode: Int): Long? = when (keyCode) {
        KeyEvent.KEYCODE_VOLUME_UP -> NEXT
        KeyEvent.KEYCODE_VOLUME_DOWN -> PREVIOUS
        else -> null
    }
    /** CCS/1 RULE:volume keys are taken only in shown, live presenter mode with input/3; otherwise they stay the system's. */
    fun gate(keyCode: Int, presenting: Boolean, live: Boolean, keyboard: Boolean): Long? =
        slide(keyCode)?.takeIf { presenting && live && keyboard }
    fun command(code: Long) = TouchpadCommand.Key(code.toULong(), 0u)
}

/** Turns gyroscope rates into pointer state units (1/100 dp, §8.3); phone held like a remote, top towards the screen. */
internal object LaserMotion {
    /** Pointer units per radian: about 1100 px for a full radian, so a small wrist turn crosses a slide. */
    const val UNITS_PER_RAD = 110_000.0
    /** Ignores sensor noise of a hand held still. */
    const val DEAD_ZONE = 0.02
    fun delta(pitchRate: Float, yawRate: Float, dtNanos: Long): Pair<Long, Long> {
        if (dtNanos <= 0 || dtNanos > 100_000_000) return 0L to 0L
        val dt = dtNanos / 1e9
        fun axis(rate: Float) = if (kotlin.math.abs(rate) < DEAD_ZONE) 0.0 else -rate * dt * UNITS_PER_RAD
        return Math.round(axis(yawRate)) to Math.round(axis(pitchRate))
    }
}

/** Gyroscope reader active only while the laser button is held. */
internal class GyroLaser(private val sensors: SensorManager, private val move: (Long, Long) -> Unit) : SensorEventListener {
    private var last = 0L
    private var running = false
    val available get() = sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE) != null
    fun start() {
        if (running) return
        val gyro = sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE) ?: return
        last = 0L; running = true
        sensors.registerListener(this, gyro, SensorManager.SENSOR_DELAY_GAME)
    }
    fun stop() { if (running) { running = false; sensors.unregisterListener(this) } }
    override fun onSensorChanged(event: SensorEvent) {
        if (!running) return
        val dt = if (last == 0L) 0L else event.timestamp - last
        last = event.timestamp
        // Device X: pitch (top up/down), device Z: yaw (turning left/right).
        val (dx, dy) = LaserMotion.delta(event.values[0], event.values[2], dt)
        if (dx != 0L || dy != 0L) move(dx, dy)
    }
    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
}

/** Talk timer with an injectable clock; it starts by itself on the first slide change. */
internal class PresentationTimer(private val now: () -> Long = SystemClock::elapsedRealtime) {
    private var startedAt = 0L
    private var banked = 0L
    var running = false; private set
    var used = false; private set
    fun elapsed(): Long = banked + if (running) now() - startedAt else 0L
    fun start() { if (!running) { startedAt = now(); running = true; used = true } }
    fun pause() { if (running) { banked += now() - startedAt; running = false } }
    fun reset() { banked = 0L; running = false; used = false }
    fun slideChanged() { if (!used) start() }
    companion object {
        fun format(ms: Long): String {
            val s = ms / 1000
            return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60) else "%02d:%02d".format(s / 60, s % 60)
        }
    }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun PresenterPanel(active: Boolean, laserSupported: Boolean, gyro: Boolean, timer: PresentationTimer, tick: Int,
                            slide: (Long) -> Unit, laser: (Boolean) -> Unit) {
    var now by remember { mutableLongStateOf(timer.elapsed()) }
    var running by remember { mutableStateOf(timer.running) }
    LaunchedEffect(tick) { running = timer.running; now = timer.elapsed() }
    LaunchedEffect(running) { while (running) { now = timer.elapsed(); delay(250) } ; now = timer.elapsed() }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(PresentationTimer.format(now), Modifier.testTag("presenterTimer"), style = MaterialTheme.typography.displayMedium.copy(fontFamily = FontFamily.Monospace))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            WindButton(if (running) str(R.string.presenter_pause) else str(R.string.presenter_start), { if (timer.running) timer.pause() else timer.start(); running = timer.running }, Modifier.testTag("presenterTimerToggle"), kind = ButtonKind.Secondary)
            WindButton(str(R.string.presenter_reset), { timer.reset(); running = false; now = 0L }, kind = ButtonKind.Quiet)
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            WindButton(str(R.string.presenter_previous), { slide(PresenterKeys.PREVIOUS) }, Modifier.weight(1f).heightIn(min = 72.dp).testTag("presenterPrevious"), enabled = active, kind = ButtonKind.Tonal)
            WindButton(str(R.string.presenter_next), { slide(PresenterKeys.NEXT) }, Modifier.weight(1f).heightIn(min = 72.dp).testTag("presenterNext"), enabled = active)
        }
        val laserReady = active && laserSupported && gyro
        var held by remember { mutableStateOf(false) }
        WindButton(if (held) str(R.string.presenter_laser_on) else str(R.string.presenter_laser_hold), {}, Modifier.fillMaxWidth().heightIn(min = 96.dp).testTag("presenterLaser").pointerInteropFilter { e ->
            if (!laserReady) return@pointerInteropFilter false
            when (e.actionMasked) {
                MotionEvent.ACTION_DOWN -> { held = true; laser(true) }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> { held = false; laser(false) }
            }
            true
        }, enabled = laserReady, kind = ButtonKind.Secondary)
        Text(str(R.string.presenter_help), style = MaterialTheme.typography.bodySmall)
        when {
            !gyro -> Text(str(R.string.presenter_no_gyro), style = MaterialTheme.typography.bodySmall)
            active && !laserSupported -> Text(str(R.string.presenter_laser_old), style = MaterialTheme.typography.bodySmall)
        }
    }
}
