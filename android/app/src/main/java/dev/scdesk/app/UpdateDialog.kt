package dev.scdesk.app

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** «Доступна версия X»: what changed, «Обновить» (download, install) and «Позже». */
@Composable
internal fun UpdateDialog(u: UpdateInfo, ctx: Context, onLater: () -> Unit) {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var progress by remember { mutableFloatStateOf(0f) }
    var error by remember { mutableStateOf<String?>(null) }
    var needAllow by remember { mutableStateOf(false) }
    val shape = RoundedCornerShape(22.dp)

    fun go() {
        if (!Updater.mayInstall(ctx)) {
            needAllow = true
            return
        }
        busy = true
        error = null
        scope.launch {
            try {
                val file = withContext(Dispatchers.IO) { Updater.download(ctx, u) { progress = it } }
                withContext(Dispatchers.IO) { Updater.install(ctx, file) }
            } catch (e: Exception) {
                error = "Не получилось обновить: ${e.message}"
                busy = false
            }
        }
    }

    Box(Modifier.fillMaxSize().background(Color(0xAA000000)).pressable { }, contentAlignment = Alignment.Center) {
        Column(Modifier.padding(24.dp).fillMaxWidth().clip(shape).background(C.panel).border(1.dp, C.border, shape).padding(22.dp)) {
            Text("Доступна версия ${u.versionName}", color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
            Text("Что нового", color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 14.dp, bottom = 4.dp))
            Text(u.notes.ifEmpty { "Улучшения и исправления." }, color = C.text, fontSize = 14.sp)
            Text("Размер около ${"%.1f".format(u.size / 1_048_576.0)} МБ", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 10.dp))
            if (busy) {
                Box(Modifier.padding(top = 16.dp).fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(C.panel2)) {
                    Box(Modifier.fillMaxWidth(progress).height(6.dp).background(C.accent))
                }
                Text(if (progress >= 1f) "Устанавливаем…" else "Загружаем…", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp))
            }
            error?.let { Text(it, color = C.accent, fontSize = 13.sp, modifier = Modifier.padding(top = 10.dp)) }
            if (needAllow) {
                Text("Android просит разрешить SC Desk устанавливать обновления: откроется страница настроек, включите переключатель и вернитесь.", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 10.dp))
            }
            Row(Modifier.padding(top = 18.dp).fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (!busy) {
                    Box(Modifier.weight(1f)) { BigButton("Позже", false, onLater) }
                    Box(Modifier.weight(1f)) {
                        BigButton(if (needAllow) "Разрешить" else "Обновить", true) {
                            if (needAllow) {
                                Updater.askToAllow(ctx)
                                needAllow = false
                            } else go()
                        }
                    }
                }
            }
        }
    }
}
