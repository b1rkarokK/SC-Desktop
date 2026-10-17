package dev.scdesk.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EqTest {
    @Test
    fun a_flat_equalizer_stays_flat_on_any_bands() {
        val dev = EqMap.toDevice(List(10) { 0f }, floatArrayOf(60f, 230f, 910f, 3600f, 14000f))
        assertTrue(dev.all { it == 0f })
    }

    @Test
    fun a_band_of_the_computer_reaches_the_phone_band_at_the_same_frequency() {
        val g = List(10) { if (it == 5) 6f else 0f } // +6 dB at 1 kHz
        val dev = EqMap.toDevice(g, floatArrayOf(1000f, 3600f, 60f))
        assertEquals(6f, dev[0], 1e-4f)
        assertTrue(dev[1] < 6f && dev[1] >= 0f)
        assertEquals(0f, dev[2], 1e-4f)
    }

    @Test
    fun outside_the_range_the_end_values_hold() {
        assertEquals(3f, EqMap.at(EqMap.pcHz, List(10) { if (it == 0) 3f else 0f }, 20f), 1e-6f)
        assertEquals(-2f, EqMap.at(EqMap.pcHz, List(10) { if (it == 9) -2f else 0f }, 20000f), 1e-6f)
    }

    @Test
    fun the_state_goes_over_the_wire_and_back() {
        val s = EqState(true, listOf(1f, 2f, 3f, 4f, 5f, 6f, 5f, 4f, 3f, -12f), "custom")
        val back = EqState.fromJson(s.toJson())!!
        assertEquals(s, back)
        // what the computer sends has the same fields (src-tauri EqConfig): 10 gains, enabled, preset
        assertEquals(10, s.toJson().getJSONArray("gains").length())
    }

    @Test
    fun a_message_with_the_wrong_number_of_bands_is_ignored() {
        val j = org.json.JSONObject("""{"enabled":true,"gains":[1,2,3],"preset":"x"}""")
        assertEquals(null, EqState.fromJson(j))
    }
}
