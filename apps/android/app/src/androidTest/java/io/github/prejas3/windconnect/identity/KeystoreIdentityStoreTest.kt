package io.github.prejas3.windconnect.identity

import android.os.Bundle
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.spec.InvalidKeySpecException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.SecretKeySpec
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * WC-09: the identity key survives a restart and its Keystore key can't be
 * exported.
 *
 * Instrumentation arguments:
 * - `requireHardware=true`: also require the wrapping key to be in secure
 *   hardware (TEE or StrongBox). For real devices; an emulator's Keystore is
 *   software.
 * - `phase=write`, then after a device reboot `phase=verify`: the restart
 *   test (README). Without `phase` both phases are skipped.
 */
@RunWith(AndroidJUnit4::class)
class KeystoreIdentityStoreTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val args: Bundle = InstrumentationRegistry.getArguments()
    private val alias = "windconnect.test.${System.nanoTime()}"
    private val fileName = "test-$alias.key"

    private fun store() = KeystoreIdentityStore(context, alias, fileName)

    private fun keystore() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun randomPem(): ByteArray {
        val raw = ByteArray(48).also { SecureRandom().nextBytes(it) }
        val b64 = android.util.Base64.encodeToString(raw, android.util.Base64.NO_WRAP)
        return "-----BEGIN PRIVATE KEY-----\n$b64\n-----END PRIVATE KEY-----\n"
            .toByteArray(Charsets.US_ASCII)
    }

    private fun report(key: String, value: String) {
        Log.i(TAG, "$key=$value")
        InstrumentationRegistry.getInstrumentation()
            .sendStatus(0, Bundle().apply { putString(key, value) })
    }

    @Before
    fun clean() = cleanup(alias, fileName)

    @After
    fun cleanAfter() = cleanup(alias, fileName)

    private fun cleanup(alias: String, fileName: String) {
        keystore().deleteEntry(alias)
        File(context.noBackupFilesDir, fileName).delete()
        File(context.noBackupFilesDir, "$fileName.lock").delete()
        File(context.noBackupFilesDir, "$fileName.tmp").delete()
    }

    @Test
    fun storesLoadsAndHidesThePlaintext() {
        val pem = randomPem()
        assertNull(store().load())
        assertTrue(store().storeNew(pem))
        // A new instance reads it back.
        assertArrayEquals(pem, store().load())

        val bytes = store().file.readBytes()
        assertArrayEquals(KeystoreIdentityStore.MAGIC, bytes.copyOfRange(0, 4))
        assertEquals(KeystoreIdentityStore.FORMAT_KEYSTORE_AES_GCM, bytes[4])
        val text = String(bytes, Charsets.ISO_8859_1)
        assertFalse(text.contains("PRIVATE KEY"))
        assertFalse(text.contains(String(pem, Charsets.US_ASCII).lines()[1]))
    }

    @Test
    fun storedKeyIsNeverReplaced() {
        val first = randomPem()
        assertTrue(store().storeNew(first))
        assertFalse(store().storeNew(randomPem()))
        assertArrayEquals(first, store().load())
    }

    @Test
    fun wrappingKeyCannotBeExported() {
        assertTrue(store().storeNew(randomPem()))
        val key = keystore().getKey(alias, null) as SecretKey
        // The key material never leaves the Keystore.
        assertNull(key.encoded)
        val factory = SecretKeyFactory.getInstance(key.algorithm, "AndroidKeyStore")
        assertThrows(InvalidKeySpecException::class.java) {
            factory.getKeySpec(key, SecretKeySpec::class.java)
        }

        val level = store().securityLevel()
        assertNotNull(level)
        report("wrappingKeySecurityLevel", level.toString())
        if (args.getString("requireHardware") == "true") {
            assertTrue(
                "wrapping key is $level, not in secure hardware",
                level in setOf(
                    KeystoreIdentityStore.SecurityLevel.STRONGBOX,
                    KeystoreIdentityStore.SecurityLevel.TRUSTED_ENVIRONMENT,
                    KeystoreIdentityStore.SecurityLevel.SECURE_HARDWARE,
                ),
            )
        }
    }

    @Test
    fun ciphertextDependsOnTheKeystoreKey() {
        assertTrue(store().storeNew(randomPem()))
        val bytes = store().file.readBytes()
        keystore().deleteEntry(alias)

        val e = assertThrows(IdentityStoreException::class.java) { store().load() }
        assertTrue(e.message!!, e.message!!.contains("Keystore key"))
        // Nothing replaced: the file stays and no new key is made.
        assertFalse(store().storeNew(randomPem()))
        assertArrayEquals(bytes, store().file.readBytes())
        assertFalse(keystore().containsAlias(alias))
    }

    @Test
    fun damagedOrUnknownFilesAreErrorsAndKept() {
        assertTrue(store().storeNew(randomPem()))
        val good = store().file.readBytes()
        val damaged = listOf(
            "tag" to good.copyOf().also { it[it.size - 1] = (it[it.size - 1].toInt() xor 1).toByte() },
            "ciphertext" to good.copyOf().also { it[20] = (it[20].toInt() xor 1).toByte() },
            "header" to good.copyOf().also { it[0] = 'X'.code.toByte() },
            "newer format" to good.copyOf().also { it[4] = 3 },
            "format 1 header" to good.copyOf().also { it[4] = 1 },
            "truncated" to good.copyOfRange(0, 20),
        )
        for ((what, content) in damaged) {
            store().file.writeBytes(content)
            assertThrows(what, IdentityStoreException::class.java) { store().load() }
            assertFalse(what, store().storeNew(randomPem()))
            assertArrayEquals(what, content, store().file.readBytes())
        }
    }

    @Test
    fun concurrentFirstStoreConvergesOnOneKey() {
        repeat(4) { round ->
            cleanup(alias, fileName)
            val threads = 8
            val pems = List(threads) { randomPem() }
            val start = CountDownLatch(1)
            val pool = Executors.newFixedThreadPool(threads)
            val results = pems.map { pem ->
                pool.submit<Boolean> {
                    start.await()
                    KeystoreIdentityStore(context, alias, fileName).storeNew(pem)
                }
            }
            start.countDown()
            val stored = results.map { it.get(60, TimeUnit.SECONDS) }
            pool.shutdown()
            assertEquals("round $round: exactly one store wins", 1, stored.count { it })
            val winner = pems[stored.indexOf(true)]
            assertArrayEquals("round $round", winner, store().load())
            assertFalse(File(context.noBackupFilesDir, "$fileName.tmp").exists())
        }
    }

    // --- Restart: phase=write, reboot, phase=verify --------------------------

    @Test
    fun restartPhaseWrite() {
        assumeTrue(args.getString("phase") == "write")
        cleanup(RESTART_ALIAS, RESTART_FILE)
        File(context.noBackupFilesDir, RESTART_EXPECTED).delete()
        val pem = randomPem()
        val restart = KeystoreIdentityStore(context, RESTART_ALIAS, RESTART_FILE)
        assertTrue(restart.storeNew(pem))
        File(context.noBackupFilesDir, RESTART_EXPECTED).writeText(sha256(pem))
        report("restartWritten", sha256(pem))
        report("bootId", bootId())
    }

    @Test
    fun restartPhaseVerify() {
        assumeTrue(args.getString("phase") == "verify")
        val expected = File(context.noBackupFilesDir, RESTART_EXPECTED).readText()
        val restart = KeystoreIdentityStore(context, RESTART_ALIAS, RESTART_FILE)
        val loaded = restart.load()
        assertNotNull("no key after restart", loaded)
        assertEquals(expected, sha256(loaded!!))
        val key = keystore().getKey(RESTART_ALIAS, null) as SecretKey
        assertNull(key.encoded)
        report("restartVerified", expected)
        report("bootId", bootId())
        report("wrappingKeySecurityLevel", restart.securityLevel().toString())
        cleanup(RESTART_ALIAS, RESTART_FILE)
        File(context.noBackupFilesDir, RESTART_EXPECTED).delete()
    }

    private fun bootId(): String =
        File("/proc/sys/kernel/random/boot_id").readText().trim()

    private fun sha256(data: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }

    companion object {
        private const val TAG = "KeystoreIdentityTest"
        private const val RESTART_ALIAS = "windconnect.test.restart"
        private const val RESTART_FILE = "test-restart.key"
        private const val RESTART_EXPECTED = "test-restart.sha256"
    }
}
