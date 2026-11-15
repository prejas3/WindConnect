package io.github.prejas3.windconnect

import android.app.KeyguardManager
import android.os.Bundle
import android.os.SystemClock
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
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

/** CCS/1 M:TouchpadActivity | RULE:foreground focus + unlocked screen only; every pause closes the pinned native session. */
@OptIn(ExperimentalComposeUiApi::class)
class TouchpadActivity : ComponentActivity() {
    private val owner get() = (application as WindApplication).connections
    private var core: WindCore? = null
    private var opening: Job? = null
    private var capture: PadCapture? = null
    private var resumed = false
    private var target: String? = null
    private var status by mutableStateOf("Przygotowywanie touchpada…")
    private var active by mutableStateOf(false)
    private var gestures by mutableStateOf(false)
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
            catch (_:Exception) { status="Nie udało się otworzyć połączenia" }
            finally {if(core!==acquired)withContext(NonCancellable){acquired?.let{owner.release(it)}}}
        }
        lifecycleScope.launch {
            owner.state.collect { state ->
                if (state !is ConnectionState.Connected || state.fingerprint != target) { stop(); status="Połączenie zmienione. Otwórz touchpad ponownie dla wybranego komputera." }
            }
        }
        lifecycleScope.launch {
            owner.touchpadAccepted.collect { accepted ->
                if (!accepted) { stop();status="Komputer nie przyjmuje sterowania. Włącz touchpad na PC." }
                else maybeStart()
            }
        }
        setContent {
            WindApp(forceContent=true) {
                Column(Modifier.fillMaxSize().padding(20.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween) {
                        Text("Touchpad",style=MaterialTheme.typography.headlineSmall)
                        WindButton("Zamknij",{finish()},kind=ButtonKind.Quiet)
                    }
                    Text(status,Modifier.testTag("touchpadStatus"),style=MaterialTheme.typography.bodyMedium)
                    AndroidView(factory={PadView(it,{capture}, {active})},modifier=Modifier.fillMaxWidth().weight(1f).background(MaterialTheme.colorScheme.surfaceContainer).testTag("touchpadSurface"))
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        for(left in listOf(true,false)) Button(onClick={capture?.button(left,true);capture?.button(left,false)},enabled=active,
                            modifier=Modifier.weight(1f).heightIn(min=56.dp).pointerInteropFilter { event ->
                                if(active)when(event.actionMasked){MotionEvent.ACTION_DOWN->capture?.button(left,true);MotionEvent.ACTION_UP,MotionEvent.ACTION_CANCEL->capture?.button(left,false)}
                                active
                            }){Text(if(left)"Lewy · przytrzymaj" else "Prawy")}
                    }
                    Column(Modifier.heightIn(max=270.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    Text("Czułość",style=MaterialTheme.typography.labelLarge)
                    Slider(value=sensitivity,onValueChange={sensitivity=it;capture?.engine?.sensitivity=it},onValueChangeFinished={settings.edit().putFloat("sensitivity",sensitivity).apply()},valueRange=0.5f..3f)
                    WindSwitch("Naturalne przewijanie",natural,{natural=it;capture?.engine?.natural=it;settings.edit().putBoolean("natural",it).apply()})
                    Text("1 palec: kursor, stuknięcie: klik. Przytrzymaj: przeciąganie. 2 palce: przewijanie i prawy klik.",style=MaterialTheme.typography.bodySmall)
                    Text(if(gestures) "3 palce: ↑ widok zadań, ↓ pulpit, ← poprzednia aplikacja, → następna aplikacja." else "Gesty 3 palców wymagają nowszej wersji WindConnect na PC.",style=MaterialTheme.typography.bodySmall)
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
                capture=PadCapture(handle,{message -> stop();status=message}).also { it.engine.sensitivity=sensitivity;it.engine.natural=natural }
                handle=null;active=true;status="Sterujesz: ${computerLabel(state.peerName)}"
            } catch(e:CancellationException) {throw e}
            catch(_:Exception) {status="Komputer nie przyjmuje sterowania"}
            finally {
                withContext(NonCancellable+Dispatchers.IO){handle?.finishInput();handle?.destroy()}
                val interrupted=!currentCoroutineContext().isActive
                opening=null
                if(interrupted && resumed && hasWindowFocus())maybeStart()
            }
        }
    }
    private fun stop() {opening?.cancel();capture?.close();capture=null;active=false}
    override fun onResume(){super.onResume();resumed=true;maybeStart()}
    override fun onPause(){resumed=false;stop();super.onPause()}
    override fun onWindowFocusChanged(focus:Boolean){super.onWindowFocusChanged(focus);if(focus)maybeStart() else stop()}
    override fun onDestroy(){stop();core?.let{opened->CoroutineScope(Dispatchers.IO).launch{owner.release(opened)}};core=null;super.onDestroy()}
}

private class PadView(context:android.content.Context,private val capture:()->PadCapture?,private val active:()->Boolean):View(context){
    init {isFocusable=true;contentDescription="Obszar touchpada. Użyj przycisków poniżej do kliknięć."}
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
        catch(_:Exception){failed("Sterowanie przerwane. Wróć i otwórz touchpad ponownie.")}
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
    private fun flush(){if(pending.isEmpty()||closed)return;val batch=pending.toList();pending.clear();if(batches.trySend(batch).isFailure){close();failed("Połączenie zbyt wolne. Touchpad zatrzymany.")}}
    fun close(){if(closed)return;closed=true;choreographer.removeFrameCallback(this);pending.clear();engine.cancel();batches.close();scope.cancel()}
}
