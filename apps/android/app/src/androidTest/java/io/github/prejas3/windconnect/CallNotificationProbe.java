package io.github.prejas3.windconnect;

import io.github.prejas3.windconnect.core.CallNotifications;

/** CCS/1 PURPOSE:test-only JVM access to the core's internal key set; no production debug API. */
public final class CallNotificationProbe {
    private CallNotificationProbe() {}
    public static int count() { return CallNotifications.INSTANCE.getActive().getValue().size(); }
}
