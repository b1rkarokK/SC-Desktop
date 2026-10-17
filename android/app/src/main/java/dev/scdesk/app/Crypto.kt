package dev.scdesk.app

import java.util.Base64
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * The encryption of the sync, the same as on the computer (src-tauri/src/sync/crypto.rs): XChaCha20-Poly1305 with a
 * random 24-byte nonce and the channel id as associated data; on the wire it is base64(nonce + ciphertext + tag).
 * The system has ChaCha20-Poly1305 (12-byte nonce) since Android 9; XChaCha is made of it with HChaCha20.
 */
object Crypto {
    private val rng = SecureRandom()

    fun randomKey(): ByteArray = ByteArray(32).also { rng.nextBytes(it) }

    private fun rotl(x: Int, n: Int) = (x shl n) or (x ushr (32 - n))

    private fun le(b: ByteArray, at: Int) =
        (b[at].toInt() and 0xff) or ((b[at + 1].toInt() and 0xff) shl 8) or ((b[at + 2].toInt() and 0xff) shl 16) or ((b[at + 3].toInt() and 0xff) shl 24)

    /** The 256-bit subkey that XChaCha derives from the key and the first 16 bytes of the nonce. */
    fun hchacha20(key: ByteArray, nonce16: ByteArray): ByteArray {
        val s = IntArray(16)
        s[0] = 0x61707865; s[1] = 0x3320646e; s[2] = 0x79622d32; s[3] = 0x6b206574
        for (i in 0 until 8) s[4 + i] = le(key, i * 4)
        for (i in 0 until 4) s[12 + i] = le(nonce16, i * 4)
        fun q(a: Int, b: Int, c: Int, d: Int) {
            s[a] += s[b]; s[d] = rotl(s[d] xor s[a], 16)
            s[c] += s[d]; s[b] = rotl(s[b] xor s[c], 12)
            s[a] += s[b]; s[d] = rotl(s[d] xor s[a], 8)
            s[c] += s[d]; s[b] = rotl(s[b] xor s[c], 7)
        }
        repeat(10) {
            q(0, 4, 8, 12); q(1, 5, 9, 13); q(2, 6, 10, 14); q(3, 7, 11, 15)
            q(0, 5, 10, 15); q(1, 6, 11, 12); q(2, 7, 8, 13); q(3, 4, 9, 14)
        }
        val out = ByteArray(32)
        val words = intArrayOf(s[0], s[1], s[2], s[3], s[12], s[13], s[14], s[15])
        for (i in words.indices) for (j in 0 until 4) out[i * 4 + j] = (words[i] ushr (8 * j)).toByte()
        return out
    }

    private fun aead(mode: Int, key: ByteArray, nonce24: ByteArray, channel: String, input: ByteArray): ByteArray {
        val sub = hchacha20(key, nonce24.copyOfRange(0, 16))
        val nonce12 = ByteArray(12)
        System.arraycopy(nonce24, 16, nonce12, 4, 8)
        val cipher = Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(mode, SecretKeySpec(sub, "ChaCha20"), IvParameterSpec(nonce12))
        cipher.updateAAD(channel.toByteArray(Charsets.UTF_8))
        return cipher.doFinal(input)
    }

    fun seal(key: ByteArray, channel: String, plain: ByteArray): String {
        val nonce = ByteArray(24).also { rng.nextBytes(it) }
        val ct = aead(Cipher.ENCRYPT_MODE, key, nonce, channel, plain)
        return Base64.getEncoder().encodeToString(nonce + ct)
    }

    /** null for anything that was not made with this key for this channel, or was changed. */
    fun open(key: ByteArray, channel: String, blob: String): ByteArray? = try {
        val raw = Base64.getDecoder().decode(blob)
        if (raw.size < 24 + 16) null
        else aead(Cipher.DECRYPT_MODE, key, raw.copyOfRange(0, 24), channel, raw.copyOfRange(24, raw.size))
    } catch (e: Exception) {
        null
    }
}
