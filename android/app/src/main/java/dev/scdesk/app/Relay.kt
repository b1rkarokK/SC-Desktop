package dev.scdesk.app

import okhttp3.Call
import okhttp3.ConnectionSpec
import okhttp3.EventListener
import okhttp3.Handshake
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Proxy
import okhttp3.Protocol
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.TlsVersion
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** The relay of the sync (server/src/index.js): a WebSocket per channel; what goes through is sealed (Crypto). */
const val RELAY = "https://sc-desk-sync.sc-desk-sync.workers.dev"

val http: OkHttpClient by lazy {
    OkHttpClient.Builder()
        .eventListener(object : EventListener() {
            val t0 = System.nanoTime()
            fun log(m: String) { android.util.Log.i("SCDesk", "net +${(System.nanoTime() - t0) / 1_000_000}ms $m") }
            override fun dnsStart(call: Call, domainName: String) = log("dns $domainName")
            override fun dnsEnd(call: Call, domainName: String, inetAddressList: List<InetAddress>) = log("dns ok ${inetAddressList.size}")
            override fun connectStart(call: Call, inetSocketAddress: InetSocketAddress, proxy: Proxy) = log("connect $inetSocketAddress")
            override fun secureConnectStart(call: Call) = log("tls start")
            override fun secureConnectEnd(call: Call, handshake: Handshake?) = log("tls ok ${handshake?.tlsVersion}")
            override fun connectFailed(call: Call, inetSocketAddress: InetSocketAddress, proxy: Proxy, protocol: Protocol?, ioe: IOException) = log("connect FAILED ${ioe.message}")
            override fun responseHeadersStart(call: Call) = log("headers")
            override fun callFailed(call: Call, ioe: IOException) = log("call FAILED ${ioe.javaClass.simpleName} ${ioe.message}")
        })
        // through the local proxy that cuts the TLS greeting (Bypass.kt): the provider hangs the plain one
        // through the server of the VPN subscription when one is picked (Tunnel.kt), else through the proxy that cuts the greeting
        .proxySelector(object : java.net.ProxySelector() {
            override fun select(uri: java.net.URI?): List<java.net.Proxy> =
                if (Tunnel.port != 0) listOf(java.net.Proxy(java.net.Proxy.Type.SOCKS, java.net.InetSocketAddress("127.0.0.1", Tunnel.port)))
                else listOf(java.net.Proxy(java.net.Proxy.Type.HTTP, java.net.InetSocketAddress("127.0.0.1", Bypass.port())))

            override fun connectFailed(uri: java.net.URI?, sa: java.net.SocketAddress?, ioe: java.io.IOException?) {}
        })
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .pingInterval(25, TimeUnit.SECONDS)
        .build()
}

/**
 * One channel. It reconnects by itself while it is open, hands every readable message to [onMessage]
 * and keeps what is sent while the link is down (a few, the newest) until it is back.
 */
class Channel(
    private val chan: String,
    private val key: ByteArray,
    private val onMessage: (JSONObject) -> Unit,
    private val onUp: () -> Unit = {},
    private val base: String = RELAY,
) {
    private val open = AtomicBoolean(false)
    private var ws: WebSocket? = null
    private val waiting = ArrayDeque<String>()
    private var backoff = 800L
    private var connected = false

    @Synchronized
    fun start() {
        if (open.getAndSet(true)) return
        connect()
    }

    private fun connect() {
        if (!open.get()) return
        val url = base.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://").trimEnd('/') + "/ch/$chan?since=0"
        ws = http.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { android.util.Log.i("SCDesk", "relay: open ${response.code}") }
            override fun onMessage(webSocket: WebSocket, text: String) {
                val w = try { JSONObject(text) } catch (e: Exception) { return }
                android.util.Log.i("SCDesk", "relay: ${w.optString("t")}")
                when (w.optString("t")) {
                    "hello" -> {
                        backoff = 800
                        synchronized(this@Channel) {
                            connected = true
                            while (waiting.isNotEmpty()) webSocket.send(waiting.removeFirst())
                        }
                        onUp()
                    }
                    "msg" -> {
                        val plain = Crypto.open(key, chan, w.optString("blob")) ?: return
                        try { onMessage(JSONObject(String(plain, Charsets.UTF_8))) } catch (e: Exception) { /* not ours */ }
                    }
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = down()
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                android.util.Log.w("SCDesk", "relay: ${t.javaClass.simpleName}: ${t.message}")
                down()
            }
        })
    }

    private fun down() {
        synchronized(this) { connected = false }
        if (!open.get()) return
        val wait = backoff
        backoff = (backoff * 2).coerceAtMost(15_000)
        Thread { Thread.sleep(wait); connect() }.apply { isDaemon = true }.start()
    }

    @Synchronized
    fun send(msg: JSONObject) {
        val wire = JSONObject().put("t", "send").put("blob", Crypto.seal(key, chan, msg.toString().toByteArray(Charsets.UTF_8))).put("keep", false).toString()
        if (connected && ws?.send(wire) == true) return
        waiting.addLast(wire)
        while (waiting.size > 20) waiting.removeFirst()
    }

    fun close() {
        if (!open.getAndSet(false)) return
        ws?.close(1000, null)
        ws = null
    }
}
