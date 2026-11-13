package io.github.prejas3.windconnect.core

import org.junit.Assert.*
import org.junit.Test

class TouchpadGesturesTest {
    @Test fun liftingThePadDoesNotReleaseAHeldDedicatedButton(){
        for(left in listOf(true,false)) {
            val buttons=PadButtons()
            val down=if(left)PadCommand.Left(true)else PadCommand.Right(true)
            val up=if(left)PadCommand.Left(false)else PadCommand.Right(false)
            assertEquals(down,buttons.hold(left,true));assertNull(buttons.gesture(down));assertNull(buttons.gesture(up));assertEquals(up,buttons.hold(left,false))
            assertEquals(down,buttons.gesture(down));assertNull(buttons.hold(left,true));assertNull(buttons.gesture(up));assertEquals(up,buttons.hold(left,false))
        }
    }
    private fun fingers(n:Int,x:Float=0f,y:Float=0f)=List(n){TouchpadGestures.Finger(it,x+it*10,y)}
    @Test fun tapsDragCancelAndNoJumpWhenCountChanges(){
        val out=mutableListOf<PadCommand>();val g=TouchpadGestures(out::add)
        g.sample(fingers(1),0);g.sample(emptyList(),100)
        assertEquals(listOf(PadCommand.Left(true),PadCommand.Left(false)),out);out.clear()
        g.sample(fingers(2),200);g.sample(fingers(1),210);g.sample(emptyList(),220)
        assertEquals(listOf(PadCommand.Right(true),PadCommand.Right(false)),out);out.clear()
        g.sample(fingers(1),300);g.frame(800);g.sample(fingers(1,10f),810);g.cancel()
        assertEquals(PadCommand.Left(true),out.first());assertEquals(PadCommand.Left(false),out.last());out.clear()
        g.sample(fingers(1),900);g.sample(fingers(2,100f,100f),910)
        assertTrue(out.isEmpty());g.sample(fingers(1,500f),920);g.sample(fingers(1,510f),930)
        assertTrue(out.isEmpty())
    }
    @Test fun eachThreeFingerStrokeFiresExactlyOnceAndFourCancels(){
        for((x,y,expected) in listOf(Triple(0f,-70f,PadCommand.TaskView),Triple(0f,70f,PadCommand.ShowDesktop),Triple(-70f,0f,PadCommand.PreviousApp),Triple(70f,0f,PadCommand.NextApp))){
            val out=mutableListOf<PadCommand>();val g=TouchpadGestures(out::add)
            g.sample(fingers(3),0);g.sample(fingers(3,x,y),100);g.sample(fingers(3,x*2,y*2),200);g.sample(emptyList(),210)
            assertEquals(listOf(expected),out)
            out.clear();g.sample(fingers(4),300);g.sample(fingers(3,x,y),310);g.sample(emptyList(),320);assertTrue(out.isEmpty())
        }
    }
    @Test fun inertiaIsFrameIndependentAndNewTouchStopsIt(){
        fun run(step:Long):Long{
            val out=mutableListOf<PadCommand>();val g=TouchpadGestures(out::add)
            g.sample(fingers(2),0);g.sample(fingers(2,0f,-20f),10);g.sample(fingers(1,0f,-20f),11);g.sample(emptyList(),12)
            assertTrue(g.needsFrame)
            var now=12L+step;while(now<=1012){g.frame(now);now+=step}
            g.frame(1012)
            val sum=out.filterIsInstance<PadCommand.Scroll>().sumOf{it.dy}
            g.sample(fingers(1),1020);out.clear();g.frame(1030);assertTrue(out.isEmpty());g.cancel();assertFalse(g.needsFrame)
            return sum
        }
        assertTrue(kotlin.math.abs(run(8)-run(16))<150)
    }
    @Test fun oldOneFingerTravelDoesNotTriggerAThreeFingerGestureAndOldVelocityDoesNotRestart(){
        val out=mutableListOf<PadCommand>();val g=TouchpadGestures(out::add)
        g.sample(fingers(1),0);g.sample(fingers(1,100f),50);out.clear()
        g.sample(fingers(3,100f),60);g.sample(fingers(3,101f),70);assertTrue(out.isEmpty());g.cancel()
        g.sample(fingers(2),100);g.sample(fingers(2,0f,-20f),110);g.sample(fingers(1,0f,-20f),120)
        g.sample(fingers(1,0f,-20f),300);g.sample(emptyList(),310);assertFalse(g.needsFrame)
    }
}
