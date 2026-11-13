package io.github.prejas3.windconnect.core

import kotlin.math.*

/** CCS/1 M:touchpad recognizer | IO:dp, monotonic ms -> commands; pointer-count change resets centroid and latches mode. */
class TouchpadGestures(private val emit: (PadCommand) -> Unit) {
    data class Finger(val id: Int, val x: Float, val y: Float)
    var sensitivity = 1.5f
    var natural = true
    private var ids = emptySet<Int>()
    private var mode = 0
    private var start = 0L
    private var lastTime = 0L
    private var lastX = 0f; private var lastY = 0f
    private var travel = 0f
    private var strokeX = 0f; private var strokeY = 0f
    private var dragging = false
    private var fired = false
    private var vx = 0f; private var vy = 0f
    private var velocityAt = 0L
    private var inertiaAt = 0L; private var inertiaStart = 0L
    private var inertia = false
    private var pointerX = 0f; private var pointerY = 0f
    private var scrollX = 0f; private var scrollY = 0f
    val needsFrame get() = inertia || (ids.isNotEmpty() && mode == 1 && !dragging && travel < 8)

    fun sample(fingers: List<Finger>, now: Long, cancelled: Boolean = false) {
        if (mode == 4) { if (fingers.isEmpty()) cancel(); return }
        if (cancelled) {cancel();return}
        if (fingers.size > 3) { cancel(); mode = 4; return }
        if (fingers.isNotEmpty()) inertia = false
        val next = fingers.map { it.id }.toSet()
        if (next != ids) {
            if (ids.isEmpty() && next.isNotEmpty()) {
                mode = next.size; start = now; travel = 0f; strokeX = 0f; strokeY = 0f; fired = false
                vx = 0f; vy = 0f
            } else if (next.isNotEmpty()) {
                if (dragging) { emit(PadCommand.Left(false)); dragging = false }
                mode = max(mode, next.size)
                if (next.size > ids.size) {strokeX=0f;strokeY=0f}
                // CCS/1 RULE:centroid resets; preserve scroll velocity while fingers lift in sequence.
                if (next.size >= ids.size) { vx = 0f; vy = 0f }
            } else {
                val duration = now - start
                if (dragging) { emit(PadCommand.Left(false)); dragging = false }
                else if (!fired && travel < 8 && duration < 300 && mode in 1..2) {
                    if (mode == 1) { emit(PadCommand.Left(true)); emit(PadCommand.Left(false)) }
                    else { emit(PadCommand.Right(true)); emit(PadCommand.Right(false)) }
                } else if (mode == 2 && travel >= 8 && now-velocityAt <= 80 && hypot(vx,vy) > 80) {
                    inertia = true; inertiaAt = now; inertiaStart = now
                }
                mode = 0
            }
            ids = next
            if (next.isNotEmpty()) { lastX = fingers.map { it.x }.average().toFloat(); lastY = fingers.map { it.y }.average().toFloat(); lastTime = now }
            return
        }
        if (next.isEmpty()) return
        val x = fingers.map { it.x }.average().toFloat(); val y = fingers.map { it.y }.average().toFloat()
        val dx = x-lastX; val dy = y-lastY
        travel += hypot(dx,dy); strokeX += dx; strokeY += dy
        val dt = (now-lastTime).coerceAtLeast(1)/1000f
        when {
            mode == 1 && next.size == 1 -> pointer(dx*sensitivity,dy*sensitivity)
            mode == 2 && next.size == 2 -> {
                val sign = if (natural) -1f else 1f
                scroll(dx*sign,dy*sign)
                vx = (vx*0.35f + dx*sign/dt*0.65f).coerceIn(-2400f,2400f)
                vy = (vy*0.35f + dy*sign/dt*0.65f).coerceIn(-2400f,2400f)
                velocityAt=now
            }
            mode == 3 && next.size == 3 && !fired && max(abs(strokeX),abs(strokeY)) >= 60 -> {
                emit(if (abs(strokeY)>abs(strokeX)) { if (strokeY<0) PadCommand.TaskView else PadCommand.ShowDesktop }
                    else {if (strokeX<0) PadCommand.PreviousApp else PadCommand.NextApp}); fired = true
            }
        }
        lastX=x; lastY=y; lastTime=now
        frame(now)
    }
    fun frame(now: Long) {
        if (ids.size==1 && mode==1 && !dragging && travel<8 && now-start>=500) { dragging=true;emit(PadCommand.Left(true)) }
        if (!inertia) return
        val dt=(now-inertiaAt).coerceAtLeast(0)/1000f
        val decay=exp(-6f*dt)
        scroll(vx*(1-decay)/6,vy*(1-decay)/6)
        vx*=decay;vy*=decay;inertiaAt=now
        if (hypot(vx,vy)<15 || now-inertiaStart>=1200) inertia=false
    }
    private fun pointer(dx:Float,dy:Float) { pointerX+=dx*100;pointerY+=dy*100;val x=pointerX.toLong();val y=pointerY.toLong();pointerX-=x;pointerY-=y;if(x!=0L||y!=0L)emit(PadCommand.Pointer(x,y)) }
    private fun scroll(dx:Float,dy:Float) { scrollX+=dx*100;scrollY+=dy*100;val x=scrollX.toLong();val y=scrollY.toLong();scrollX-=x;scrollY-=y;if(x!=0L||y!=0L)emit(PadCommand.Scroll(x,y)) }
    fun cancel() {
        if (dragging) emit(PadCommand.Left(false))
        ids=emptySet(); mode=0;dragging=false;inertia=false;vx=0f;vy=0f
        pointerX=0f;pointerY=0f;scrollX=0f;scrollY=0f
    }
}
sealed interface PadCommand {
    data class Pointer(val dx:Long,val dy:Long):PadCommand
    data class Scroll(val dx:Long,val dy:Long):PadCommand
    data class Left(val down:Boolean):PadCommand
    data class Right(val down:Boolean):PadCommand
    data object TaskView:PadCommand;data object ShowDesktop:PadCommand
    data object PreviousApp:PadCommand;data object NextApp:PadCommand
}

/** CCS/1 RULE:pad gestures and a held on-screen button own separate presses; lifting one never releases the other. */
class PadButtons {
    private val held=booleanArrayOf(false,false)
    private val gesture=booleanArrayOf(false,false)
    private val sent=booleanArrayOf(false,false)
    fun hold(left:Boolean,down:Boolean):PadCommand? {val index=if(left)0 else 1;held[index]=down;return changed(index)}
    fun gesture(command:PadCommand):PadCommand? {
        val index=when(command){is PadCommand.Left->0;is PadCommand.Right->1;else->return command}
        gesture[index]=when(command){is PadCommand.Left->command.down;is PadCommand.Right->command.down;else->false}
        return changed(index)
    }
    private fun changed(index:Int):PadCommand? {
        val down=held[index]||gesture[index];if(down==sent[index])return null
        sent[index]=down;return if(index==0)PadCommand.Left(down)else PadCommand.Right(down)
    }
}
