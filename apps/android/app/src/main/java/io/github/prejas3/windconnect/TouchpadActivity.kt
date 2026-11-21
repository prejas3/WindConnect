package io.github.prejas3.windconnect

import android.app.KeyguardManager
import android.os.Bundle
import android.hardware.SensorManager
import android.os.SystemClock
import android.view.KeyEvent
import android.view.WindowManager
import android.view.Choreographer
import android.view.MotionEvent
import android.view.View
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.border
import androidx.compose.ui.semantics.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.input.pointer.pointerInteropFilter
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.lifecycle.lifecycleScope
import io.github.prejas3.windconnect.bindings.TouchpadCommand
import io.github.prejas3.windconnect.bindings.TouchpadSession
import io.github.prejas3.windconnect.core.*
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel

/** CCS/1 M:TouchpadActivity | RULE:foreground focus + unlocked screen only; every pause closes the pinned native session. Presenter mode (WN-53) takes volume keys only while it is shown and live. */
@OptIn(ExperimentalComposeUiApi::class)
class TouchpadActivity : ComponentActivity() {
    private val owner get() = (application as WindApplication).connections
    private var core: WindCore? = null
    private var opening: Job? = null
    private var capture: PadCapture? = null
    private var resumed = false
    private var target: String? = null
    private var status by mutableStateOf(str(R.string.touchpad_preparing))
    private var active by mutableStateOf(false)
    private var gestures by mutableStateOf(false)
    private var keyboardSupported by mutableStateOf(false)
    private var typing by mutableStateOf(false)
    private var presenting by mutableStateOf(false)
    private var laserSupported by mutableStateOf(false)
    private var timerTick by mutableIntStateOf(0)
    private val timer = PresentationTimer()
    private val gyro by lazy { GyroLaser(getSystemService(SENSOR_SERVICE) as SensorManager) { dx, dy -> capture?.pointer(dx, dy) } }
    private var draft by mutableStateOf(TextFieldValue())
    private var sensitivity by mutableFloatStateOf(1.5f)
    private var natural by mutableStateOf(true)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState); enableEdgeToEdge()
        target = (owner.state.value as? ConnectionState.Connected)?.fingerprint
        val settings=getSharedPreferences("touchpad",MODE_PRIVATE)
        sensitivity=settings.getFloat("sensitivity",1.5f).coerceIn(0.5f,3f); natural=settings.getBoolean("natural",true)
        lifecycleScope.launch {
            var acquired:WindCore?=null
            try { withContext(NonCancellable){acquired=owner.acquire()};ensureActive();core=acquired;maybeStart() }
            catch(e:CancellationException){throw e}
            catch (_:Exception) { status=str(R.string.touchpad_open_failed) }
            finally {if(core!==acquired)withContext(NonCancellable){acquired?.let{owner.release(it)}}}
        }
        lifecycleScope.launch {
            owner.state.collect { state ->
                if (state !is ConnectionState.Connected || state.fingerprint != target) { stop(); status=str(R.string.touchpad_changed) }
            }
        }
        lifecycleScope.launch {
            owner.touchpadAccepted.collect { accepted ->
                if (!accepted) { stop();status=str(R.string.touchpad_refused_enable) }
                else maybeStart()
            }
        }
        setContent {
            WindApp(forceContent=true) {
                Column(Modifier.fillMaxSize().imePadding().padding(20.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween) {
                        Text(str(R.string.action_touchpad),style=MaterialTheme.typography.headlineSmall)
                        WindButton(str(R.string.close),{finish()},kind=ButtonKind.Quiet)
                    }
                    Row(horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                        WindButton(if(typing)str(R.string.action_touchpad) else str(R.string.touchpad_keyboard),{showPresenter(false);typing=!typing},Modifier.testTag("remoteKeyboardToggle"),enabled=active && keyboardSupported,kind=ButtonKind.Quiet)
                        WindButton(if(presenting)str(R.string.action_touchpad) else str(R.string.touchpad_presentation),{typing=false;showPresenter(!presenting)},Modifier.testTag("presenterToggle"),enabled=active && keyboardSupported,kind=ButtonKind.Quiet)
                    }
                    Text(status,Modifier.testTag("touchpadStatus"),style=MaterialTheme.typography.bodyMedium)
                    if(active && !keyboardSupported)Text(str(R.string.touchpad_keyboard_old),style=MaterialTheme.typography.bodySmall)
                    if(presenting) {
                        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                            PresenterPanel(active && keyboardSupported,laserSupported,gyro.available,timer,timerTick,{slide(it)}){on->
                                if(on){ if(capture?.keyboard(TouchpadCommand.Laser(true))==true)gyro.start() }
                                else { gyro.stop();capture?.keyboard(TouchpadCommand.Laser(false)) }
                            }
                        }
                    } else if(typing) {
                        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                            RemoteKeyboard(draft,{draft=it},active && keyboardSupported){capture?.keyboard(it)==true}
                        }
                    } else {
                    AndroidView(factory={PadView(it,{capture}, {active})},modifier=Modifier.fillMaxWidth().weight(1f).background(MaterialTheme.colorScheme.surfaceContainer).testTag("touchpadSurface"))
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        // WN-130: own control (outlined, like WindButton Secondary); press and release stay separate for drag-and-hold.
                        for(left in listOf(true,false)) Box(Modifier.weight(1f).heightIn(min=56.dp).clip(WindShapes.full)
                            .border(1.dp,if(active)MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.outline,WindShapes.full)
                            .semantics(mergeDescendants=true){role=Role.Button;if(!active)disabled();onClick{capture?.button(left,true);capture?.button(left,false);active}}
                            .pointerInteropFilter { event ->
                                if(active)when(event.actionMasked){MotionEvent.ACTION_DOWN->capture?.button(left,true);MotionEvent.ACTION_UP,MotionEvent.ACTION_CANCEL->capture?.button(left,false)}
                                active
                            },contentAlignment=Alignment.Center){Text(if(left)str(R.string.touchpad_left_hold) else str(R.string.touchpad_right),style=MaterialTheme.typography.labelLarge,color=if(active)MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)}
                    }
                    Column(Modifier.heightIn(max=270.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    Text(str(R.string.touchpad_sensitivity),style=MaterialTheme.typography.labelLarge)
                    WindSlider(value=sensitivity,step=0.125f,onValueChange={sensitivity=it;capture?.engine?.sensitivity=it},onValueChangeFinished={settings.edit().putFloat("sensitivity",sensitivity).apply()},valueRange=0.5f..3f)
                    WindSwitch(str(R.string.touchpad_natural),natural,{natural=it;capture?.engine?.natural=it;settings.edit().putBoolean("natural",it).apply()})
                    Text(str(R.string.touchpad_help),style=MaterialTheme.typography.bodySmall)
                    Text(if(gestures) str(R.string.touchpad_gestures) else str(R.string.touchpad_gestures_old),style=MaterialTheme.typography.bodySmall)
                    }
                    }
                }
            }
        }
    }
    private fun maybeStart() {
        val opened=core ?: return
        if (!resumed || !hasWindowFocus() || !owner.touchpadAccepted.value || capture!=null || opening!=null || (getSystemService(KEYGUARD_SERVICE) as KeyguardManager).isKeyguardLocked) return
        val state=owner.state.value as? ConnectionState.Connected ?: return
        if(target==null || state.fingerprint!=target)return
        opening=lifecycleScope.launch {
            var handle:TouchpadSession?=null
            try {
                handle=withContext(NonCancellable+Dispatchers.IO){opened.openTouchpad(state.fingerprint)}
                ensureActive()
                if(!resumed || !hasWindowFocus())return@launch
                gestures=handle.gestures()
                keyboardSupported=handle.keyboard()
                laserSupported=handle.laser()
                capture=PadCapture(handle,{message -> stop();status=message}).also { it.engine.sensitivity=sensitivity;it.engine.natural=natural }
                handle=null;active=true;status=str(R.string.touchpad_controlling, computerLabel(state.peerName))
            } catch(e:CancellationException) {throw e}
            catch(_:Exception) {status=str(R.string.touchpad_refused)}
            finally {
                withContext(NonCancellable+Dispatchers.IO){handle?.finishInput();handle?.destroy()}
                val interrupted=!currentCoroutineContext().isActive
                opening=null
                if(interrupted && resumed && hasWindowFocus())maybeStart()
            }
        }
    }
    private fun showPresenter(on:Boolean){
        presenting=on
        if(on)window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else {gyro.stop();window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)}
    }
    private fun slide(code:Long){if(capture?.keyboard(PresenterKeys.command(code))==true){timer.slideChanged();timerTick++}}
    /** Volume keys become slide keys only in live presenter mode; repeats and key-ups are swallowed so the volume never changes. */
    private fun presenterKey(keyCode:Int)=PresenterKeys.gate(keyCode,presenting,active,keyboardSupported)
    override fun onKeyDown(keyCode:Int,event:KeyEvent):Boolean {
        val code=presenterKey(keyCode) ?: return super.onKeyDown(keyCode,event)
        if(event.repeatCount==0)slide(code)
        return true
    }
    override fun onKeyUp(keyCode:Int,event:KeyEvent):Boolean = presenterKey(keyCode)!=null || super.onKeyUp(keyCode,event)
    private fun stop() {gyro.stop();laserSupported=false;opening?.cancel();capture?.close();capture=null;active=false;keyboardSupported=false;draft=TextFieldValue()}
    override fun onResume(){super.onResume();resumed=true;maybeStart()}
    override fun onPause(){resumed=false;stop();super.onPause()}
    override fun onWindowFocusChanged(focus:Boolean){super.onWindowFocusChanged(focus);if(focus)maybeStart() else stop()}
    override fun onDestroy(){stop();core?.let{opened->CoroutineScope(Dispatchers.IO).launch{owner.release(opened)}};core=null;super.onDestroy()}
}

private class PadView(context:android.content.Context,private val capture:()->PadCapture?,private val active:()->Boolean):View(context){
    init {isFocusable=true;contentDescription=str(R.string.touchpad_area)}
    override fun onTouchEvent(event:MotionEvent):Boolean {
        if(!active())return false
        val pad=capture()?:return false
        parent?.requestDisallowInterceptTouchEvent(true)
        val density=resources.displayMetrics.density
        if(event.actionMasked==MotionEvent.ACTION_CANCEL){pad.cancel();return true}
        fun fingers(history:Int?):List<TouchpadGestures.Finger> = (0 until event.pointerCount).filterNot{event.actionMasked==MotionEvent.ACTION_POINTER_UP && it==event.actionIndex}.map { i->
            TouchpadGestures.Finger(event.getPointerId(i),(if(history==null)event.getX(i)else event.getHistoricalX(i,history))/density,(if(history==null)event.getY(i)else event.getHistoricalY(i,history))/density)
        }
        if(event.actionMasked==MotionEvent.ACTION_MOVE) for(h in 0 until event.historySize)pad.sample(fingers(h),event.getHistoricalEventTime(h))
        pad.sample(if(event.actionMasked==MotionEvent.ACTION_UP)emptyList()else fingers(null),event.eventTime)
        if(event.actionMasked==MotionEvent.ACTION_UP)performClick()
        return true
    }
    override fun performClick():Boolean {super.performClick();return true}
}
private class PadCapture(private val handle:TouchpadSession,private val failed:(String)->Unit):Choreographer.FrameCallback {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    private val batches=Channel<List<TouchpadCommand>>(8)
    private val pending=mutableListOf<TouchpadCommand>()
    private val choreographer=Choreographer.getInstance()
    private var scheduled=false;private var closed=false;private var lastFrame=0L
    private val buttons=PadButtons()
    val engine=TouchpadGestures{emit(it)}
    init {scope.launch {
        try {for(batch in batches){if(closed)break;withContext(Dispatchers.IO){handle.send(batch)}}}
        catch(e:CancellationException){throw e}
        catch(_:Exception){failed(str(R.string.touchpad_interrupted))}
        finally {withContext(NonCancellable+Dispatchers.IO){handle.finishInput();handle.destroy()};scope.cancel()}
    }}
    fun sample(fingers:List<TouchpadGestures.Finger>,time:Long){if(!closed){engine.sample(fingers,time);schedule()}}
    fun cancel(){engine.cancel();flush()}
    fun button(left:Boolean,down:Boolean){
        if(closed)return
        buttons.hold(left,down)?.let(::queue)
        engine.cancel()
        flush()
    }
    fun keyboard(command:TouchpadCommand):Boolean {
        if(closed)return false
        if(!(if(command is TouchpadCommand.Laser)handle.laser() else handle.keyboard())){close();failed(str(R.string.touchpad_no_keyboard));return false}
        flush()
        if(closed)return false
        if(batches.trySend(listOf(command)).isFailure){close();failed(str(R.string.touchpad_slow_control));return false}
        return true
    }
    fun pointer(dx:Long,dy:Long){if(!closed)queue(PadCommand.Pointer(dx,dy))}
    private fun emit(command:PadCommand){
        buttons.gesture(command)?.let(::queue)
    }
    private fun queue(command:PadCommand){
        if(closed)return
        if(command !is PadCommand.Pointer && command !is PadCommand.Scroll && command !is PadCommand.Left && command !is PadCommand.Right && !handle.gestures())return
        val mapped=when(command){
            is PadCommand.Pointer->TouchpadCommand.Pointer(command.dx,command.dy)
            is PadCommand.Scroll->TouchpadCommand.Scroll(command.dx,command.dy)
            is PadCommand.Left->TouchpadCommand.Left(command.down)
            is PadCommand.Right->TouchpadCommand.Right(command.down)
            PadCommand.TaskView->TouchpadCommand.TaskView;PadCommand.ShowDesktop->TouchpadCommand.ShowDesktop
            PadCommand.PreviousApp->TouchpadCommand.PreviousApp;PadCommand.NextApp->TouchpadCommand.NextApp
        }
        val last=pending.lastOrNull()
        if(last is TouchpadCommand.Pointer && mapped is TouchpadCommand.Pointer){pending[pending.lastIndex]=TouchpadCommand.Pointer(last.dx+mapped.dx,last.dy+mapped.dy)}
        else if(last is TouchpadCommand.Scroll && mapped is TouchpadCommand.Scroll){pending[pending.lastIndex]=TouchpadCommand.Scroll(last.dx+mapped.dx,last.dy+mapped.dy)}
        else pending.add(mapped)
        if(pending.size>=32 || mapped !is TouchpadCommand.Pointer && mapped !is TouchpadCommand.Scroll)flush() else schedule()
    }
    private fun schedule(){if(!scheduled&&!closed){scheduled=true;choreographer.postFrameCallback(this)}}
    override fun doFrame(frameTimeNanos:Long){scheduled=false;if(closed)return
        if(frameTimeNanos-lastFrame<8_000_000){schedule();return};lastFrame=frameTimeNanos
        engine.frame(SystemClock.uptimeMillis());flush();if(engine.needsFrame)schedule()
    }
    private fun flush(){if(pending.isEmpty()||closed)return;val batch=pending.toList();pending.clear();if(batches.trySend(batch).isFailure){close();failed(str(R.string.touchpad_slow))}}
    fun close(){if(closed)return;closed=true;choreographer.removeFrameCallback(this);pending.clear();engine.cancel();batches.close();scope.cancel()}
}
