package io.github.prejas3.windconnect.core

import android.Manifest
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.provider.ContactsContract
import android.provider.Telephony
import io.github.prejas3.windconnect.bindings.SmsMessage
import io.github.prejas3.windconnect.bindings.SmsMessages
import io.github.prejas3.windconnect.bindings.SmsPerson
import io.github.prejas3.windconnect.bindings.SmsPhoto
import io.github.prejas3.windconnect.bindings.SmsSource
import io.github.prejas3.windconnect.bindings.SmsThread
import io.github.prejas3.windconnect.bindings.SmsThreads
import java.io.ByteArrayOutputStream

/**
 * CCS/1 M:WN-55 the phone's SMS for the PC (PROTOCOL.md §9.11): threads, SMS and MMS text, contact names and photos,
 * read from Android's providers on the core's blocking threads. RULE:READ_SMS checked on every call (revoking it
 * answers "denied" at once); READ_CONTACTS optional (numbers only without it). Nothing is cached on disk and no
 * text or number is logged. Decision D2: the GitHub/F-Droid build only.
 */
class PhoneSms(context: Context) : SmsSource {
    private val app = context.applicationContext
    private val resolver: ContentResolver = app.contentResolver

    fun canRead(): Boolean = granted(Manifest.permission.READ_SMS)
    fun canReadContacts(): Boolean = granted(Manifest.permission.READ_CONTACTS)
    private fun granted(permission: String) = app.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    override fun threads(limit: UInt): SmsThreads = guarded({ SmsThreads(it, emptyList()) }) {
        val addresses = canonicalAddresses()
        val unread = unreadByThread()
        val threads = ArrayList<Pair<SmsThread, List<String>>>()
        resolver.query(
            CONVERSATIONS, arrayOf(Telephony.Threads._ID, Telephony.Threads.DATE, Telephony.Threads.SNIPPET,
                Telephony.Threads.RECIPIENT_IDS, Telephony.Threads.MESSAGE_COUNT),
            null, null, "${Telephony.Threads.DATE} DESC",
        )?.use { c ->
            while (c.moveToNext() && threads.size < limit.toInt()) {
                val count = c.getInt(4)
                if (count <= 0) continue
                val recipients = (c.getString(3) ?: "").split(' ').mapNotNull { it.toLongOrNull()?.let(addresses::get) }
                val id = c.getLong(0)
                threads += SmsThread(id.toULong(), c.getLong(1).coerceAtLeast(0).toULong(), SmsRules.clean(c.getString(2) ?: ""),
                    emptyList(), (unread[id] ?: 0).toUInt(), count.toUInt()) to recipients
            }
        } ?: return@guarded SmsThreads(FAILED, emptyList())
        val contacts = Contacts()
        SmsThreads(OK, threads.map { (t, recipients) ->
            t.copy(people = recipients.distinct().take(20).map { address ->
                val contact = contacts.lookup(address)
                SmsPerson(address, contact?.first, contact?.second != null)
            })
        })
    }

    override fun messages(thread: ULong, before: ULong?, since: ULong?, limit: UInt): SmsMessages =
        guarded({ SmsMessages(it, emptyList()) }) {
            val n = limit.toInt()
            val all = ArrayList<SmsMessage>(n * 2)
            all += sms(thread.toLong(), before?.toLong(), since?.toLong(), n)
            all += mms(thread.toLong(), before?.toLong(), since?.toLong(), n)
            if (all.isEmpty() && !threadExists(thread.toLong())) return@guarded SmsMessages(NOT_FOUND, emptyList())
            all.sortWith(compareByDescending<SmsMessage> { it.dateMs }.thenByDescending { it.id })
            val newest = if (all.size > n) all.subList(0, n) else all
            SmsMessages(OK, newest.map { if (it.mms) withParts(it) else it })
        }

    override fun photo(address: String): SmsPhoto = guarded({ SmsPhoto(it, byteArrayOf()) }) {
        if (!canReadContacts()) return@guarded SmsPhoto(NOT_FOUND, byteArrayOf())
        val uri = Contacts().lookup(address)?.second ?: return@guarded SmsPhoto(NOT_FOUND, byteArrayOf())
        val bitmap = resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) }
            ?: return@guarded SmsPhoto(NOT_FOUND, byteArrayOf())
        val scaled = Bitmap.createScaledBitmap(bitmap, PHOTO_PX, PHOTO_PX, true)
        val png = ByteArrayOutputStream().also { scaled.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        if (png.size > MAX_PHOTO) SmsPhoto(NOT_FOUND, byteArrayOf()) else SmsPhoto(OK, png)
    }

    private inline fun <T> guarded(failure: (UByte) -> T, block: () -> T): T {
        if (!canRead()) return failure(DENIED)
        return try { block() } catch (_: SecurityException) { failure(DENIED) } catch (_: Exception) { failure(FAILED) }
    }

    private fun sms(thread: Long, before: Long?, since: Long?, limit: Int): List<SmsMessage> {
        val (where, args) = range(Telephony.Sms.THREAD_ID, thread, Telephony.Sms.DATE, before, since)
        val out = ArrayList<SmsMessage>(minOf(limit, 1024))
        resolver.query(Telephony.Sms.CONTENT_URI,
            arrayOf(Telephony.Sms._ID, Telephony.Sms.DATE, Telephony.Sms.TYPE, Telephony.Sms.BODY, Telephony.Sms.ADDRESS, Telephony.Sms.READ),
            "$where AND ${Telephony.Sms.TYPE}<>${Telephony.Sms.MESSAGE_TYPE_DRAFT}", args,
            "${Telephony.Sms.DATE} DESC, ${Telephony.Sms._ID} DESC")?.use { c ->
            while (out.size < limit && c.moveToNext()) {
                val folder = SmsRules.smsFolder(c.getInt(2))
                out += SmsMessage(c.getLong(0).toULong(), false, c.getLong(1).coerceAtLeast(0).toULong(), folder,
                    SmsRules.clean(c.getString(3) ?: ""), if (folder == INBOX) c.getString(4)?.takeIf { it.isNotEmpty() && it.length <= 64 } else null,
                    c.getInt(5) != 0, 0u)
            }
        }
        return out
    }

    /** MMS dates are in seconds: the inclusive ms range widened to whole seconds, then filtered exactly. */
    private fun mms(thread: Long, before: Long?, since: Long?, limit: Int): List<SmsMessage> {
        val (where, args) = range(Telephony.Mms.THREAD_ID, thread, Telephony.Mms.DATE, before?.let { it / 1000 }, since?.let { it / 1000 })
        val out = ArrayList<SmsMessage>()
        resolver.query(Telephony.Mms.CONTENT_URI,
            arrayOf(Telephony.Mms._ID, Telephony.Mms.DATE, Telephony.Mms.MESSAGE_BOX, Telephony.Mms.READ),
            "$where AND ${Telephony.Mms.MESSAGE_BOX}<>${Telephony.Mms.MESSAGE_BOX_DRAFTS}", args,
            "${Telephony.Mms.DATE} DESC")?.use { c ->
            while (out.size < limit && c.moveToNext()) {
                val ms = c.getLong(1).coerceAtLeast(0) * 1000
                if ((before != null && ms > before) || (since != null && ms < since)) continue
                out += SmsMessage(c.getLong(0).toULong(), true, ms.toULong(), SmsRules.mmsFolder(c.getInt(2)), "", null, c.getInt(3) != 0, 0u)
            }
        }
        return out
    }

    /** An MMS's text parts joined, the rest counted as attachments (WN-56 shows them). */
    private fun withParts(m: SmsMessage): SmsMessage {
        val text = StringBuilder()
        var attachments = 0u
        resolver.query(Uri.parse("content://mms/part"), arrayOf("ct", "text"), "mid=?", arrayOf(m.id.toString()), null)?.use { c ->
            while (c.moveToNext()) {
                when (c.getString(0)) {
                    "text/plain" -> c.getString(1)?.let { if (text.isNotEmpty()) text.append('\n'); text.append(it) }
                    "application/smil" -> {}
                    else -> attachments++
                }
            }
        }
        var address: String? = null
        if (m.folder == INBOX) {
            resolver.query(Uri.parse("content://mms/${m.id}/addr"), arrayOf("address"), "type=137", null, null)?.use { c ->
                if (c.moveToFirst()) address = c.getString(0)?.takeIf { it.isNotEmpty() && it.length <= 64 && it != "insert-address-token" }
            }
        }
        return m.copy(body = SmsRules.clean(text.toString()), attachments = attachments, address = address)
    }

    private fun range(threadColumn: String, thread: Long, dateColumn: String, before: Long?, since: Long?): Pair<String, Array<String>> {
        val where = StringBuilder("$threadColumn=?")
        val args = arrayListOf(thread.toString())
        before?.let { where.append(" AND $dateColumn<=?"); args += it.toString() }
        since?.let { where.append(" AND $dateColumn>=?"); args += it.toString() }
        return where.toString() to args.toTypedArray()
    }

    private fun threadExists(thread: Long): Boolean =
        resolver.query(CONVERSATIONS, arrayOf(Telephony.Threads._ID), "${Telephony.Threads._ID}=?", arrayOf(thread.toString()), null)
            ?.use { it.moveToFirst() } ?: false

    private fun canonicalAddresses(): Map<Long, String> {
        val out = HashMap<Long, String>()
        resolver.query(Uri.parse("content://mms-sms/canonical-addresses"), arrayOf("_id", "address"), null, null, null)?.use { c ->
            while (c.moveToNext()) c.getString(1)?.takeIf { it.isNotEmpty() && it.length <= 64 }?.let { out[c.getLong(0)] = it }
        }
        return out
    }

    private fun unreadByThread(): Map<Long, Int> {
        val out = HashMap<Long, Int>()
        resolver.query(Telephony.Sms.CONTENT_URI, arrayOf(Telephony.Sms.THREAD_ID),
            "${Telephony.Sms.READ}=0 AND ${Telephony.Sms.TYPE}=${Telephony.Sms.MESSAGE_TYPE_INBOX}", null, null)?.use { c ->
            while (c.moveToNext()) out.merge(c.getLong(0), 1, Int::plus)
        }
        resolver.query(Telephony.Mms.CONTENT_URI, arrayOf(Telephony.Mms.THREAD_ID),
            "${Telephony.Mms.READ}=0 AND ${Telephony.Mms.MESSAGE_BOX}=${Telephony.Mms.MESSAGE_BOX_INBOX}", null, null)?.use { c ->
            while (c.moveToNext()) out.merge(c.getLong(0), 1, Int::plus)
        }
        return out
    }

    /** Names and photo URIs by address, for one answer (READ_CONTACTS only). */
    private inner class Contacts {
        private val allowed = canReadContacts()
        private val seen = HashMap<String, Pair<String?, Uri?>?>()
        fun lookup(address: String): Pair<String?, Uri?>? {
            if (!allowed) return null
            return seen.getOrPut(address) {
                try {
                    resolver.query(Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(address)),
                        arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME, ContactsContract.PhoneLookup.PHOTO_THUMBNAIL_URI),
                        null, null, null)?.use { c ->
                        if (c.moveToFirst()) SmsRules.clean(c.getString(0) ?: "").takeIf { it.isNotEmpty() } to c.getString(1)?.let(Uri::parse) else null
                    }
                } catch (_: IllegalArgumentException) { null }
            }
        }
    }

    // CCS/1 FLOW:provider change -> debounce in the core (CHANGE_DEBOUNCE) -> Changed to every PC watch.
    private var observer: ContentObserver? = null
    private var thread: HandlerThread? = null

    /** Watches SMS and MMS while [on]; [changed] runs on a background thread. */
    @Synchronized fun observe(on: Boolean, changed: () -> Unit) {
        if (on == (observer != null)) return
        if (!on) {
            observer?.let(resolver::unregisterContentObserver)
            observer = null
            thread?.quitSafely(); thread = null
            return
        }
        val looper = HandlerThread("wind-sms-observer").also { it.start(); thread = it }.looper
        val o = object : ContentObserver(Handler(looper)) {
            override fun onChange(selfChange: Boolean) = changed()
        }
        try {
            resolver.registerContentObserver(Uri.parse("content://mms-sms/"), true, o)
            observer = o
        } catch (_: SecurityException) {
            thread?.quitSafely(); thread = null
        }
    }

    companion object {
        private val CONVERSATIONS: Uri = Uri.parse("content://mms-sms/conversations?simple=true")
        private const val OK: UByte = 0u
        private const val DENIED: UByte = 1u
        private const val NOT_FOUND: UByte = 2u
        private const val FAILED: UByte = 4u
        private const val INBOX: UByte = 0u
        private const val PHOTO_PX = 96
        private const val MAX_PHOTO = 64 * 1024
    }
}
