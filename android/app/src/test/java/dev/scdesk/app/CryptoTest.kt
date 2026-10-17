package dev.scdesk.app

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
import org.junit.Test
import java.io.File
import java.util.Base64

class CryptoTest {
    private fun fixture(): JSONObject = JSONObject(File("../../shared-tests/crypto.json").readText())

    @Test
    fun hchacha20_matches_the_reference_vector() {
        // draft-irtf-cfrg-xchacha, section 2.2.1
        val key = ByteArray(32) { it.toByte() }
        val nonce = byteArrayOf(0, 0, 0, 0x09, 0, 0, 0, 0x4a, 0, 0, 0, 0, 0x31, 0x41, 0x59, 0x27)
        val hex = Crypto.hchacha20(key, nonce).joinToString("") { "%02x".format(it) }
        assertEquals("82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc", hex)
    }

    @Test
    fun what_the_computer_sealed_opens_here() {
        val f = fixture()
        val key = Base64.getDecoder().decode(f.getString("key_b64"))
        val cases = f.getJSONArray("cases")
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val plain = Crypto.open(key, c.getString("channel"), c.getString("blob"))
            assertNotNull("case $i", plain)
            assertEquals(c.getString("plain"), String(plain!!, Charsets.UTF_8))
        }
    }

    @Test
    fun a_wrong_channel_or_a_changed_blob_opens_nothing() {
        val f = fixture()
        val key = Base64.getDecoder().decode(f.getString("key_b64"))
        val c = f.getJSONArray("cases").getJSONObject(0)
        assertNull(Crypto.open(key, "another-channel", c.getString("blob")))
        val raw = Base64.getDecoder().decode(c.getString("blob"))
        raw[raw.size - 1] = (raw[raw.size - 1].toInt() xor 1).toByte()
        assertNull(Crypto.open(key, c.getString("channel"), Base64.getEncoder().encodeToString(raw)))
    }

    @Test
    fun sealing_and_opening_round_trip() {
        val key = Crypto.randomKey()
        val sealed = Crypto.seal(key, "ch", "привет".toByteArray())
        assertEquals("привет", String(Crypto.open(key, "ch", sealed)!!, Charsets.UTF_8))
    }
}
