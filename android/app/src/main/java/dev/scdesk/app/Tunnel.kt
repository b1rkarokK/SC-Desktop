package dev.scdesk.app

import android.content.Context
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket

/** A server of the VPN subscription (given by the computer): its name and the sing-box outbound. */
data class VpnServer(val name: String, val outbound: JSONObject)

/**
 * The way out through a server of the VPN subscription, the way the computer's «Свой сервер» does it
 * (src-tauri/src/net/vpn.rs): sing-box runs as a program of its own (the native library libsingbox.so, not the VPN of
 * the whole phone) and listens on 127.0.0.1 as a SOCKS proxy; every connection of the app then goes through it. It
 * speaks what the subscriptions use (VLESS with Reality, VMess, Trojan, Shadowsocks, Hysteria2 and the like), so a
 * server that looks like an allowed site gets through where the provider lets only such sites in.
 */
object Tunnel {
    /** the port of the running engine, 0 when it is not running */
    @Volatile var port = 0
    @Volatile var selected = -1
    private var proc: Process? = null

    private fun exe(ctx: Context) = File(ctx.applicationInfo.nativeLibraryDir, "libsingbox.so")

    /** The engine is in this build (the 32-bit build has none). */
    fun available(ctx: Context): Boolean = exe(ctx).exists()

    fun parse(text: String): List<VpnServer> = try {
        val a = JSONArray(text)
        (0 until a.length()).map { a.getJSONObject(it) }.map { VpnServer(it.optString("n"), it.getJSONObject("o")) }
    } catch (e: Exception) {
        emptyList()
    }

    /** What an earlier run left (the app was killed): stopped. */
    fun cleanLeftover(ctx: Context) {
        val f = File(ctx.filesDir, "singbox.pid")
        runCatching { f.readText().trim().toInt() }.getOrNull()?.let { runCatching { android.os.Process.killProcess(it) } }
        f.delete()
    }

    @Synchronized
    fun stop(ctx: Context? = null) {
        port = 0
        proc?.destroy()
        proc = null
        ctx?.let { File(it.filesDir, "singbox.pid").delete() }
    }

    /** Starts the engine with this server (the earlier run is stopped); true when it listens. Blocking. */
    @Synchronized
    fun start(ctx: Context, server: VpnServer): Boolean {
        stop(ctx)
        if (!available(ctx)) return false
        val p = ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { it.localPort }
        val out = JSONObject(server.outbound.toString()).put("tag", "proxy")
        val config = JSONObject()
            .put("log", JSONObject().put("level", "warn"))
            // the name of the server and the sites are looked up over HTTPS: the provider's DNS may lie or be cut
            .put("dns", JSONObject().put("servers", JSONArray().put(JSONObject().put("type", "https").put("server", "1.1.1.1").put("tag", "local"))))
            .put("route", JSONObject().put("default_domain_resolver", "local"))
            .put("inbounds", JSONArray().put(JSONObject().put("type", "socks").put("listen", "127.0.0.1").put("listen_port", p)))
            .put("outbounds", JSONArray().put(out))
        val cfg = File(ctx.filesDir, "singbox.json").also { it.writeText(config.toString()) }
        val child = ProcessBuilder(exe(ctx).absolutePath, "run", "-c", cfg.absolutePath)
            .directory(ctx.filesDir).redirectErrorStream(true).start()
        proc = child
        // what it says is read and dropped (and kept short in the log): it must not stall on a full pipe
        Thread {
            runCatching { child.inputStream.bufferedReader().forEachLine { if (it.isNotBlank()) android.util.Log.i("SCDesk", "sing-box: ${it.take(200)}") } }
        }.apply { isDaemon = true }.start()
        runCatching { File(ctx.filesDir, "singbox.pid").writeText(pidOf(child).toString()) }
        repeat(60) {
            try {
                Socket().use { it.connect(InetSocketAddress("127.0.0.1", p), 200) }
                port = p
                return true
            } catch (e: Exception) {
                if (!child.isAlive) {
                    stop(ctx)
                    return false
                }
                Thread.sleep(100)
            }
        }
        stop(ctx)
        return false
    }

    private fun pidOf(p: Process): Int = try {
        val f = p.javaClass.getDeclaredField("pid")
        f.isAccessible = true
        f.getInt(p)
    } catch (e: Exception) {
        -1
    }

    /**
     * Whether the site opens through the running engine: the thing the whole thing is for. Returns the milliseconds,
     * or -1 (blocking).
     */
    fun test(): Long {
        val p = port
        if (p == 0) return -1
        val c = http.newBuilder().callTimeout(12, java.util.concurrent.TimeUnit.SECONDS).build()
        val t0 = System.currentTimeMillis()
        return try {
            c.newCall(Request.Builder().url("https://soundcloud.com/").header("User-Agent", USER_AGENT).build()).execute().use { if (it.code < 500) System.currentTimeMillis() - t0 else -1 }
        } catch (e: Exception) {
            -1
        }
    }
}
