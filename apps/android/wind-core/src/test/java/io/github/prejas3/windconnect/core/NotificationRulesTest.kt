package io.github.prejas3.windconnect.core

import android.app.Notification
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NotificationRulesTest {
    private val own = "io.github.prejas3.windconnect"

    private fun parts(
        title: CharSequence? = "Ola",
        text: CharSequence? = "Coming at 7?",
        flags: Int = 0,
        category: String? = null,
        visibility: Int = Notification.VISIBILITY_PRIVATE,
        template: String? = null,
        conversationTitle: CharSequence? = null,
        bigTitle: CharSequence? = null,
        bigText: CharSequence? = null,
        messages: List<Pair<CharSequence?, CharSequence?>> = emptyList(),
        packageName: String = "org.thoughtcrime.securesms",
        progressMax: Int = 0,
        indeterminate: Boolean = false,
    ) = NotificationParts(packageName, flags, category, visibility, template, conversationTitle, bigTitle, title, messages,
        bigText, text, progressMax, indeterminate)

    @Test fun anOrdinaryNotificationGoes() {
        assertEquals(NotificationContent("Ola", "Coming at 7?", false), NotificationRules.content(parts(), own))
    }

    @Test fun persistentAndOtherKindsAreSkipped() {
        for (flag in listOf(Notification.FLAG_ONGOING_EVENT, Notification.FLAG_FOREGROUND_SERVICE, Notification.FLAG_NO_CLEAR,
            Notification.FLAG_GROUP_SUMMARY)) {
            assertNull("flag $flag", NotificationRules.content(parts(flags = flag), own))
        }
        assertNull("our own", NotificationRules.content(parts(packageName = own), own))
        assertNull("secret", NotificationRules.content(parts(visibility = Notification.VISIBILITY_SECRET), own))
        assertNull("a player", NotificationRules.content(parts(category = Notification.CATEGORY_TRANSPORT), own))
        assertNull("a player", NotificationRules.content(parts(template = "android.app.Notification\$MediaStyle"), own))
        // Flags that don't make it persistent.
        assertTrue(NotificationRules.content(parts(flags = Notification.FLAG_AUTO_CANCEL or Notification.FLAG_ONLY_ALERT_ONCE), own) != null)
    }

    @Test fun emptyOrRedactedToNothingIsSkipped() {
        assertNull(NotificationRules.content(parts(title = null, text = null), own))
        assertNull("whitespace and controls only", NotificationRules.content(parts(title = " \n", text = "\u0007‮ "), own))
        // Android 15 redacts a sensitive text for listeners: what is left is sent as is; with nothing left, nothing goes.
        assertNull(NotificationRules.content(parts(title = null, text = ""), own))
        assertEquals(NotificationContent("Bank", null, false), NotificationRules.content(parts(title = "Bank", text = ""), own))
    }

    @Test fun titlesAndTextsComeFromTheRightExtras() {
        assertEquals("Family", NotificationRules.content(parts(conversationTitle = "Family", bigTitle = "x"), own)!!.title)
        assertEquals("Long title", NotificationRules.content(parts(bigTitle = "Long title"), own)!!.title)
        assertEquals("whole text", NotificationRules.content(parts(bigText = "whole text"), own)!!.text)
        // Messaging: the last message; who wrote it when that isn't already the title.
        val chat = parts(title = "Ola", text = "2 new messages", messages = listOf("Ola" to "first", "Ola" to "second"))
        assertEquals("second", NotificationRules.content(chat, own)!!.text)
        val group = parts(conversationTitle = "Family", messages = listOf("Ola" to "hi", "Piotr" to "Picking up the keys"))
        assertEquals(NotificationContent("Family", "Piotr: Picking up the keys", false), NotificationRules.content(group, own))
        val lastEmpty = parts(title = "Ola", messages = listOf("Ola" to "text", "Ola" to ""))
        assertEquals("text", NotificationRules.content(lastEmpty, own)!!.text)
    }

    @Test fun progressIsMarked() {
        assertTrue(NotificationRules.content(parts(progressMax = 100), own)!!.progress)
        assertTrue(NotificationRules.content(parts(indeterminate = true), own)!!.progress)
        assertTrue(NotificationRules.content(parts(category = Notification.CATEGORY_PROGRESS), own)!!.progress)
        assertFalse(NotificationRules.content(parts(), own)!!.progress)
    }

    @Test fun textsAreCleaned() {
        assertEquals("a b c", NotificationRules.clean("  a\n\tb  c \r\n", 100))
        assertEquals("evil.exe", NotificationRules.clean("‮evil⁦.exe‏", 100))
        assertEquals("bell", NotificationRules.clean("be\u0007ll\u0000", 100))
        assertEquals("👨‍👩‍👧", NotificationRules.clean("👨‍👩‍👧", 100))
        val long = NotificationRules.clean("ż".repeat(2000), NotificationRules.MAX_TEXT)!! // i18n: data
        assertTrue(long.toByteArray().size <= NotificationRules.MAX_TEXT)
        assertNull(NotificationRules.clean(null, 10))
        assertNull(NotificationRules.clean("\n \t", 10))
    }

    @Test fun theReplyActionIsChosenCarefully() {
        val reply = Notification.Action.SEMANTIC_ACTION_REPLY
        val none = Notification.Action.SEMANTIC_ACTION_NONE
        fun a(label: String, semantic: Int = none, contextual: Boolean = false, vararg freeForm: Boolean) =
            ActionParts(label, semantic, contextual, freeForm.toList())
        // Mark as read (no input), then Reply (free-form): the reply.
        assertEquals(1, NotificationRules.replyAction(listOf(a("Przeczytane"), a("Odpowiedz", none, false, true))))
        // Two with free-form input: SEMANTIC_ACTION_REPLY wins over the first.
        assertEquals(2, NotificationRules.replyAction(listOf(a("Note", none, false, true), a("x"), a("Reply", reply, false, true))))
        // Contextual (system smart replies) and data-only / choice-only inputs don't count.
        assertEquals(null, NotificationRules.replyAction(listOf(a("Smart", reply, true, true), a("Choice", reply, false, false))))
        assertEquals(null, NotificationRules.replyAction(emptyList()))
        // The label goes to the PC (cleaned, bounded); a reply action without one gets a default.
        val withReply = NotificationRules.content(parts().copy(actions = listOf(a("Przeczytane"), a("  Odpowiedz\n", none, false, true))), own)!!
        assertEquals("Odpowiedz", withReply.reply)
        assertEquals(1, withReply.replyIndex)
        val blank = NotificationRules.content(parts().copy(actions = listOf(ActionParts(null, reply, false, listOf(true)))), own)!!
        assertEquals("Reply", blank.reply)
        // WN-165: the listener passes the label in the phone's language.
        val blankPl = NotificationRules.content(parts().copy(actions = listOf(ActionParts(null, reply, false, listOf(true)))), own, "Odpowiedz")!!
        assertEquals("Odpowiedz", blankPl.reply)
        assertNull(NotificationRules.content(parts(), own)!!.reply)
    }

    @Test fun idsAreStableSaltedAndShort() {
        val salt = ByteArray(16) { it.toByte() }
        val key = "0|org.thoughtcrime.securesms|42|null|10123"
        val id = NotificationRules.id(key, salt)
        assertEquals(16, id.size)
        assertArrayEquals(id, NotificationRules.id(key, salt))
        assertNotEquals(id.toList(), NotificationRules.id(key, ByteArray(16)).toList())
        assertNotEquals(id.toList(), NotificationRules.id("0|org.thoughtcrime.securesms|43|null|10123", salt).toList())
    }
}
