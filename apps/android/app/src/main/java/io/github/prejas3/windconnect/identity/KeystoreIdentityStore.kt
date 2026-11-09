package io.github.prejas3.windconnect.identity

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.RandomAccessFile
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec

/**
 * The device identity key (PKCS#8 PEM made by wind-core, WC-07) kept on
 * Android (WC-09): encrypted with an AES-256-GCM key that lives in the
 * Android Keystore (StrongBox when the device has it) and can't be exported.
 *
 * The Ed25519 identity key itself can't live in the Keystore: StrongBox has
 * no Ed25519, the Keystore has it only from API 33, and TLS in wind-core
 * signs in process. So the key is protected at rest by the hardware-held
 * wrapping key, and is in the app's memory while it is used. Which hardware
 * actually holds the wrapping key is reported by [securityLevel], never
 * assumed.
 *
 * File `noBackupFilesDir/identity.key` (no backup: a copy is undecryptable on
 * any other device or install anyway):
 *
 * | bytes | content |
 * |---|---|
 * | 4 | magic `WCID` |
 * | 1 | format `2`: Android Keystore AES-256-GCM |
 * | 12 | IV |
 * | rest | ciphertext with 16-byte tag; AAD = the 5 header bytes + alias |
 *
 * A stored key is never replaced: [storeNew] doesn't write over a file, and a
 * file whose wrapping key is gone, whose format is unknown or whose tag fails
 * is an error, never a reason to make a new key.
 *
 * Mirrors wind-core's `KeyStore` trait; the bridge to it is WC-17.
 */
class KeystoreIdentityStore(
    private val context: Context,
    val alias: String = DEFAULT_ALIAS,
    fileName: String = DEFAULT_FILE,
) {
    val file: File = File(context.noBackupFilesDir, fileName)
    private val lockFile = File(context.noBackupFilesDir, "$fileName.lock")

    /** The stored key, or null when none has been stored. */
    @Throws(IOException::class)
    fun load(): ByteArray? = locked {
        if (!file.exists()) return@locked null
        val bytes = file.readBytes()
        if (bytes.size < HEADER_SIZE + IV_SIZE + TAG_SIZE ||
            !bytes.copyOfRange(0, MAGIC.size).contentEquals(MAGIC)
        ) {
            throw IdentityStoreException("identity key file has an unknown format")
        }
        val format = bytes[MAGIC.size]
        if (format != FORMAT_KEYSTORE_AES_GCM) {
            throw IdentityStoreException(
                "identity key file has format $format, newer than this version knows",
            )
        }
        val key = existingKey()
            ?: throw IdentityStoreException(
                "identity key file exists but its Keystore key \"$alias\" is gone",
            )
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                key,
                GCMParameterSpec(TAG_SIZE * 8, bytes, HEADER_SIZE, IV_SIZE),
            )
            cipher.updateAAD(aad())
            cipher.doFinal(bytes, HEADER_SIZE + IV_SIZE, bytes.size - HEADER_SIZE - IV_SIZE)
        } catch (e: GeneralSecurityException) {
            throw IdentityStoreException("identity key can't be decrypted", e)
        }
    }

    /**
     * Stores [pem] unless a key is already stored: false, storing nothing,
     * when one is. The caller owns [pem] and should zero it when done.
     */
    @Throws(IOException::class)
    fun storeNew(pem: ByteArray): Boolean = locked {
        require(pem.isNotEmpty()) { "empty key" }
        if (file.exists()) return@locked false
        val key = existingKey() ?: createKey()
        val sealed = try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            cipher.updateAAD(aad())
            val iv = cipher.iv
            check(iv.size == IV_SIZE) { "unexpected IV size ${iv.size}" }
            HEADER + iv + cipher.doFinal(pem)
        } catch (e: GeneralSecurityException) {
            throw IdentityStoreException("identity key can't be encrypted", e)
        }
        val tmp = File(file.parentFile, "${file.name}.tmp")
        FileOutputStream(tmp).use { out ->
            out.write(sealed)
            out.fd.sync()
        }
        if (!tmp.renameTo(file)) {
            tmp.delete()
            throw IOException("can't move the identity key into place")
        }
        true
    }

    /** Where the wrapping key lives, as the Keystore reports it; null if none yet. */
    fun securityLevel(): SecurityLevel? {
        val key = existingKey() ?: return null
        val factory = SecretKeyFactory.getInstance(key.algorithm, ANDROID_KEYSTORE)
        val info = factory.getKeySpec(key, KeyInfo::class.java) as KeyInfo
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            when (info.securityLevel) {
                KeyProperties.SECURITY_LEVEL_STRONGBOX -> SecurityLevel.STRONGBOX
                KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> SecurityLevel.TRUSTED_ENVIRONMENT
                KeyProperties.SECURITY_LEVEL_UNKNOWN_SECURE -> SecurityLevel.SECURE_HARDWARE
                KeyProperties.SECURITY_LEVEL_SOFTWARE -> SecurityLevel.SOFTWARE
                else -> SecurityLevel.UNKNOWN
            }
        } else {
            @Suppress("DEPRECATION")
            if (info.isInsideSecureHardware) SecurityLevel.SECURE_HARDWARE else SecurityLevel.SOFTWARE
        }
    }

    private fun aad(): ByteArray = HEADER + alias.toByteArray(Charsets.UTF_8)

    private fun keystore(): KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

    private fun existingKey(): SecretKey? = keystore().getKey(alias, null) as SecretKey?

    private fun createKey(): SecretKey {
        val strongBox = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
        if (strongBox) {
            try {
                return generate(strongBox = true)
            } catch (_: StrongBoxUnavailableException) {
                // Falls back to the default Keystore; securityLevel() says where it went.
            }
        }
        return generate(strongBox = false)
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            // No user authentication: the background service reconnects with
            // the screen locked.
            .apply { if (strongBox) setIsStrongBoxBacked(true) }
            .build()
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(spec)
        return generator.generateKey()
    }

    /**
     * Runs [block] holding the store's lock: one monitor for the process
     * (every instance) and a file lock for other processes. Key creation and
     * file creation happen under the same lock.
     */
    private inline fun <T> locked(block: () -> T): T = synchronized(PROCESS_LOCK) {
        file.parentFile?.mkdirs()
        RandomAccessFile(lockFile, "rw").use { raf ->
            raf.channel.lock().use { block() }
        }
    }

    enum class SecurityLevel { STRONGBOX, TRUSTED_ENVIRONMENT, SECURE_HARDWARE, SOFTWARE, UNKNOWN }

    companion object {
        const val DEFAULT_ALIAS = "windconnect.identity.wrap.v1"
        const val DEFAULT_FILE = "identity.key"
        const val FORMAT_KEYSTORE_AES_GCM: Byte = 2
        val MAGIC = byteArrayOf('W'.code.toByte(), 'C'.code.toByte(), 'I'.code.toByte(), 'D'.code.toByte())
        private val HEADER = MAGIC + FORMAT_KEYSTORE_AES_GCM
        const val HEADER_SIZE = 5
        const val IV_SIZE = 12
        const val TAG_SIZE = 16
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private val PROCESS_LOCK = Any()
    }
}

/** The stored identity key is unreadable; the store never replaces it. */
class IdentityStoreException(message: String, cause: Throwable? = null) : IOException(message, cause)
