package io.github.prejas3.windconnect.core

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test

class CallNotificationsTest {
    @Before fun reset() = CallNotifications.clear()
    @After fun clear() = CallNotifications.clear()

    @Test fun reconnectReplacesMissedRemovalsWithTheActiveSnapshot() {
        CallNotifications.posted("a")
        CallNotifications.posted("b")
        CallNotifications.reconcile(setOf("b"))
        assertEquals(setOf("b"), CallNotifications.active.value)
        CallNotifications.reconcile(emptySet())
        assertEquals(CallState.Idle, CallStates.combine(CallState.Idle, CallNotifications.active.value.isNotEmpty()))
    }

    @Test fun disconnectOrConsentOffForgetsCallsBeforeTheNextSession() {
        CallNotifications.posted("missed-removal")
        assertEquals(CallState.Ringing, CallStates.combine(CallState.Idle, CallNotifications.active.value.isNotEmpty()))
        CallNotifications.clear()
        CallNotifications.removed("missed-removal")
        assertEquals(emptySet<String>(), CallNotifications.active.value)
        assertEquals(CallState.Idle, CallStates.combine(CallState.Idle, CallNotifications.active.value.isNotEmpty()))
        CallNotifications.reconcile(setOf("new-call"))
        assertEquals(setOf("new-call"), CallNotifications.active.value)
    }
}
