package dev.scdesk.app

import android.net.Uri
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Signing in to SoundCloud here, without the computer: the site's own sign-in page in a window, then the login
 * token is taken from its cookie (the way the computer does it). A sign-in through Google or Apple does not work
 * in an embedded window (they forbid it): for those the QR code from the computer is the way.
 */
@Composable
fun LoginScreen(app: App, onBack: () -> Unit, onQr: () -> Unit, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    var status by remember { mutableStateOf("Войдите в SoundCloud как обычно") }
    var finishing by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var web by remember { mutableStateOf<WebView?>(null) }

    // the token appears in the cookie when the sign-in is over (the page is a one-page app: look about once a second)
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            if (finishing) continue
            val cookies = CookieManager.getInstance().getCookie("https://soundcloud.com") ?: continue
            val token = Regex("(?:^|;\\s*)oauth_token=([^;]+)").find(cookies)?.groupValues?.get(1)?.let(Uri::decode) ?: continue
            finishing = true
            status = "Входим…"
            scope.launch {
                try {
                    val (id, name) = withContext(Dispatchers.IO) {
                        app.store.oauthToken = token
                        app.store.clientId = ScApi.discoverClientId()
                        app.api.me()
                    }
                    app.store.userId = id
                    app.store.userName = name
                    onDone()
                } catch (e: Exception) {
                    app.store.oauthToken = ""
                    status = "Не получилось войти: ${e.message}"
                    finishing = false
                }
            }
        }
    }

    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(44.dp).pressable(onBack), contentAlignment = Alignment.Center) { Icon(Ico.Back, 26.dp) }
            Text("Вход в SoundCloud", color = C.text, fontSize = 20.sp)
        }
        Text(status, color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
        if (failed) Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Войти по QR с компьютера", color = C.accent, fontSize = 14.sp, modifier = Modifier.pressable(onQr).padding(8.dp))
            Text("Повторить", color = C.accent, fontSize = 14.sp, modifier = Modifier.pressable { failed = false; status = "Войдите в SoundCloud как обычно"; web?.reload() }.padding(8.dp))
        }
        DisposableEffect(Unit) {
            onDispose {
                web?.destroy()
                runCatching { if (androidx.webkit.WebViewFeature.isFeatureSupported(androidx.webkit.WebViewFeature.PROXY_OVERRIDE)) androidx.webkit.ProxyController.getInstance().clearProxyOverride({ it.run() }, {}) }
            }
        }
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { c ->
                WebView(c).also { w ->
                    w.settings.javaScriptEnabled = true
                    w.settings.domStorageEnabled = true
                    w.settings.userAgentString = USER_AGENT
                    CookieManager.getInstance().setAcceptCookie(true)
                    CookieManager.getInstance().setAcceptThirdPartyCookies(w, true)
                    w.webViewClient = object : WebViewClient() {
                        override fun onPageFinished(v: WebView?, url: String?) { if (!failed) status = "Войдите в SoundCloud как обычно" }
                        override fun onReceivedError(v: WebView, r: android.webkit.WebResourceRequest, e: android.webkit.WebResourceError) {
                            // only the page itself (not a picture or a script): the site is cut off here, the QR code from the computer still works
                            if (r.isForMainFrame) { failed = true; status = "Сайт SoundCloud не открывается с этого телефона (провайдер его закрывает)." }
                        }
                    }
                    w.setBackgroundColor(0xFF161414.toInt())
                    web = w
                    // the page goes the way the rest of the app goes (the bypass of the blocking, or the picked server):
                    // without it soundcloud.com does not open where it is cut off
                    val viaProxy = runCatching {
                        if (!androidx.webkit.WebViewFeature.isFeatureSupported(androidx.webkit.WebViewFeature.PROXY_OVERRIDE)) false
                        else {
                            val rule = if (Tunnel.port != 0) "socks5://127.0.0.1:${Tunnel.port}" else "127.0.0.1:${Bypass.port()}"
                            androidx.webkit.ProxyController.getInstance().setProxyOverride(
                                androidx.webkit.ProxyConfig.Builder().addProxyRule(rule).build(), { it.run() }, { w.loadUrl("https://soundcloud.com/signin") },
                            )
                            true
                        }
                    }.getOrDefault(false)
                    if (!viaProxy) w.loadUrl("https://soundcloud.com/signin")
                }
            },
        )
    }
}
