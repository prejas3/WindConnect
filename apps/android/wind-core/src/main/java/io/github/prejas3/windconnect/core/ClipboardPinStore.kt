package io.github.prejas3.windconnect.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * CCS/1 SEC:pinned clipboard entries (WN-117) on disk only encrypted: AES-256-GCM, key in Android Keystore (never leaves it),
 * file `clipboard-pins.bin` in noBackupFilesDir (no cloud/device backup), header `WCPN` 1, then IV and ciphertext.
 * RULE:a damaged, newer or undecryptable file (a lost key) starts without pins and stays as it is; no pins removes it.
 */
internal class ClipboardPinStore(context: Context, private val file: File = File(context.noBackupFilesDir, FILE)) {
    fun read(): ByteArray? = try {
        if (!file.exists()) null else {
            val bytes = file.readBytes()
            if (bytes.size < HEADER.size + 1 + IV || !bytes.copyOfRange(0, HEADER.size).contentEquals(HEADER) || bytes[HEADER.size] != FORMAT) null
            else {
                val key = existingKey() ?: return null
                val start = HEADER.size + 1
                val cipher = Cipher.getInstance(TRANSFORM)
                cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes, start, IV))
                cipher.updateAAD(HEADER + FORMAT)
                cipher.doFinal(bytes, start + IV, bytes.size - start - IV)
            }
        }
    } catch (_: Exception) { null }

    /** Writes the pins (atomically: a temporary file, then renamed); empty or null deletes the file. */
    fun write(plain: ByteArray?): Boolean = try {
        if (plain == null) { file.delete(); true } else {
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.ENCRYPT_MODE, key())
            cipher.updateAAD(HEADER + FORMAT)
            val sealed = cipher.doFinal(plain)
            val iv = cipher.iv
            check(iv.size == IV)
            val temporary = File(file.parentFile, "$FILE.tmp")
            temporary.writeBytes(HEADER + FORMAT + iv + sealed)
            temporary.renameTo(file) || run { temporary.delete(); false }
        }
    } catch (_: Exception) { false } finally { plain?.fill(0) }

    private fun existingKey(): SecretKey? = KeyStore.getInstance(KEYSTORE).apply { load(null) }.getKey(ALIAS, null) as? SecretKey
    private fun key(): SecretKey = existingKey() ?: KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).run {
        init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build())
        generateKey()
    }

    companion object {
        const val FILE = "clipboard-pins.bin"
        private const val ALIAS = "windconnect-clipboard-pins"
        private const val KEYSTORE = "AndroidKeyStore"
        private const val TRANSFORM = "AES/GCM/NoPadding"
        private val HEADER = "WCPN".toByteArray(Charsets.US_ASCII)
        private const val FORMAT: Byte = 1
        private const val IV = 12
    }
}
