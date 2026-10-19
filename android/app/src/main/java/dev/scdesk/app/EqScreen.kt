package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val LABELS = listOf("32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k")

/** The equalizer: the same ten bands and presets as on the computer, and the same settings on both (the link carries them). */
@Composable
internal fun EqScreen(app: App, onClose: () -> Unit) {
    var state by remember { mutableStateOf(app.eq.state) }
    val set: (EqState) -> Unit = { s ->
        state = s
        app.eq.set(s)
    }
    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding().pressable { }) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(44.dp).pressable(onClose), contentAlignment = Alignment.Center) { Icon(Ico.Back, 26.dp) }
            Text("Эквалайзер", color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            // the switch
            Box(
                Modifier.width(52.dp).size(width = 52.dp, height = 30.dp).clip(RoundedCornerShape(15.dp)).background(if (state.enabled) C.accent else C.panel2)
                    .pressable { set(state.copy(enabled = !state.enabled)) }.padding(3.dp),
                contentAlignment = if (state.enabled) Alignment.CenterEnd else Alignment.CenterStart,
            ) { Box(Modifier.size(24.dp).clip(RoundedCornerShape(12.dp)).background(if (state.enabled) C.bg else C.muted)) }
        }
        Text("Настройки общие с компьютером: что меняете здесь, меняется и там.", color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 2.dp))
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            EqMap.presets.forEach { (id, label, gains) -> Chip(label, state.preset == id) { set(EqState(true, gains, id)) } }
        }
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp)) {
            state.gains.forEachIndexed { i, g ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(LABELS[i], color = C.muted, fontSize = 13.sp, modifier = Modifier.width(38.dp))
                    Slider(
                        value = g, onValueChange = { v ->
                            val next = state.gains.toMutableList().also { it[i] = (v * 2).toInt() / 2f }
                            set(EqState(true, next, "custom"))
                        },
                        valueRange = -12f..12f, modifier = Modifier.weight(1f),
                        colors = SliderDefaults.colors(thumbColor = C.text, activeTrackColor = C.accent, inactiveTrackColor = C.panel2),
                    )
                    Text((if (g > 0) "+" else "") + "%.1f".format(g).replace(".0", ""), color = if (g != 0f) C.text else C.faint, fontSize = 13.sp, modifier = Modifier.width(44.dp))
                }
            }
            Text("У телефона полос меньше, чем у компьютера: значения между ними выравниваются.", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(vertical = 12.dp))
        }
    }
}
