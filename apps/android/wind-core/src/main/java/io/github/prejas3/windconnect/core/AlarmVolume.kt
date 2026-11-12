package io.github.prejas3.windconnect.core

/**
 * CCS/1 RULE:the ring raises the alarm volume to max and puts back what the person had (WN-50). The previous value is
 * written down before raising, so a process killed mid-ring is repaired on the next start ([restore]); a volume the
 * person changed since (no longer our max) is theirs and stays.
 */
internal class AlarmVolume(private val store: Store, private val volume: Volume) {
    interface Store {
        fun get(key: String): Int?
        fun put(key: String, value: Int)
        fun remove(key: String)
    }

    interface Volume {
        val current: Int
        val max: Int
        fun set(value: Int)
    }

    /** Raises to max; the oldest unrestored value is kept as the one to restore. */
    fun raise() {
        if (store.get(PREVIOUS) == null) store.put(PREVIOUS, volume.current)
        store.put(SET, volume.max)
        volume.set(volume.max)
    }

    /** Puts back the saved value unless the person changed the volume since; forgets it either way. */
    fun restore() {
        val previous = store.get(PREVIOUS) ?: return
        val set = store.get(SET)
        if (set == null || volume.current == set) volume.set(previous)
        store.remove(PREVIOUS)
        store.remove(SET)
    }

    companion object {
        const val PREVIOUS = "find_alarm_previous"
        const val SET = "find_alarm_set"
    }
}
