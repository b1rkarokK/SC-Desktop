package dev.scdesk.app

import android.media.audiofx.Equalizer
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.ln
import kotlin.math.roundToInt

/** The equalizer as the computer has it: 10 bands from 32 Hz to 16 kHz, ±12 dB (src-tauri/src/config.rs EqConfig). */
data class EqState(val enabled: Boolean = false, val gains: List<Float> = List(10) { 0f }, val preset: String = "flat") {
    fun toJson(): JSONObject = JSONObject().put("enabled", enabled).put("preamp", 0.0).put("gains", JSONArray(gains.map { it.toDouble() })).put("preset", preset)

    companion object {
        fun fromJson(j: JSONObject): EqState? {
            val g = j.optJSONArray("gains") ?: return null
            if (g.length() != 10) return null
            return EqState(j.optBoolean("enabled"), (0 until 10).map { g.optDouble(it).toFloat().coerceIn(-12f, 12f) }, j.optString("preset", "custom"))
        }
    }
}

object EqMap {
    val pcHz = floatArrayOf(32f, 64f, 125f, 250f, 500f, 1000f, 2000f, 4000f, 8000f, 16000f)

    /** The same presets as the computer's (src/ui/eq_popover.ts). */
    val presets: List<Triple<String, String, List<Float>>> = listOf(
        Triple("flat", "Ровно", listOf(0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f)),
        Triple("bass", "Бас +", listOf(6f, 5f, 3f, 1f, 0f, -1f, 0f, 1f, 2f, 2f)),
        Triple("vocal", "Вокал", listOf(-2f, -2f, -1f, 1f, 3f, 4f, 3f, 1f, 0f, -1f)),
        Triple("rock", "Рок", listOf(5f, 3f, 1f, -1f, -2f, -1f, 1f, 3f, 4f, 5f)),
        Triple("electronic", "Электроника", listOf(5f, 4f, 1f, 0f, -2f, 1f, 0f, 2f, 4f, 5f)),
        Triple("acoustic", "Акустика", listOf(3f, 2f, 1f, 1f, 2f, 2f, 3f, 3f, 2f, 1f)),
        Triple("night", "Ночь (тихо)", listOf(-3f, -2f, 0f, 1f, 2f, 2f, 1f, 0f, -2f, -4f)),
    )

    /** A value at [hz] from the points ([xs] in Hz, [ys]): straight between neighbours on the log-frequency scale. */
    fun at(xs: FloatArray, ys: List<Float>, hz: Float): Float {
        if (hz <= xs.first()) return ys.first()
        if (hz >= xs.last()) return ys.last()
        val i = xs.indexOfLast { it <= hz }
        val t = (ln(hz) - ln(xs[i])) / (ln(xs[i + 1]) - ln(xs[i]))
        return ys[i] + (ys[i + 1] - ys[i]) * t
    }

    /** What the bands of the phone's equalizer (their centres in Hz) get from the ten of the computer's, in dB. */
    fun toDevice(gains10: List<Float>, centres: FloatArray): List<Float> = centres.map { at(pcHz, gains10, it) }

    /** The computer's ten from the phone's own bands, in dB (for what the phone sets). */
    fun fromDevice(devGains: List<Float>, centres: FloatArray): List<Float> = pcHz.map { at(centres, devGains, it).coerceIn(-12f, 12f) }
}

/** The equalizer of the phone: the system's Equalizer on the audio session of the player, set from [state]. */
class EqController(private val store: Store) {
    var state: EqState = load()
        private set

    /** the user (or the computer) changed it: the screens and the link know */
    var onChanged: ((EqState, Boolean) -> Unit)? = null

    private var eq: Equalizer? = null

    private fun load(): EqState = try {
        EqState.fromJson(JSONObject(store.eqJson)) ?: EqState()
    } catch (e: Exception) {
        EqState()
    }

    /** The player has an audio session: the equalizer goes onto it (the service calls this). */
    @Synchronized
    fun attach(sessionId: Int) {
        try { eq?.release() } catch (e: Exception) { /* gone */ }
        eq = try { Equalizer(0, sessionId) } catch (e: Exception) { null }
        apply()
    }

    @Synchronized
    private fun apply() {
        val e = eq ?: return
        try {
            val n = e.numberOfBands.toInt()
            val centres = FloatArray(n) { e.getCenterFreq(it.toShort()) / 1000f }
            val range = e.bandLevelRange
            val dev = EqMap.toDevice(state.gains, centres)
            for (i in 0 until n) e.setBandLevel(i.toShort(), (dev[i] * 100).roundToInt().coerceIn(range[0].toInt(), range[1].toInt()).toShort())
            e.enabled = state.enabled
        } catch (x: Exception) {
            // an effect the device refuses: the music plays without it
        }
    }

    /** [fromRemote]: the computer set it, so it is not told back. */
    fun set(s: EqState, fromRemote: Boolean = false) {
        state = s
        store.eqJson = s.toJson().toString()
        apply()
        onChanged?.invoke(s, fromRemote)
    }
}
