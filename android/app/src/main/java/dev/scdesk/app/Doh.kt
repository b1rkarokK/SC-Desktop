package dev.scdesk.app

import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.net.InetAddress
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/**
 * Addresses of the blocked hosts asked over HTTPS (DoH), as the computer does (src-tauri/src/net/doh.rs): a
 * provider that blocks a site often answers the usual DNS with an address of its own. The question goes to
 * Cloudflare and Google by their IP addresses (nothing to look up first); the answers are kept for 10 minutes.
 */
object Doh {
    private val servers = listOf("https://1.1.1.1/dns-query", "https://8.8.8.8/resolve")
    private val keep = TimeUnit.MINUTES.toMillis(10)
    private val cache = ConcurrentHashMap<String, Pair<List<InetAddress>, Long>>()
    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder().proxy(java.net.Proxy.NO_PROXY).connectTimeout(4, TimeUnit.SECONDS).readTimeout(4, TimeUnit.SECONDS).callTimeout(6, TimeUnit.SECONDS).build()
    }

    /** The hosts the cutting proxy asks DoH for: the ones that are blocked at home. */
    fun wanted(host: String): Boolean {
        val h = host.trimEnd('.').lowercase()
        return listOf("soundcloud.com", "sndcdn.com", "soundcloud.cloud", "workers.dev").any { h == it || h.endsWith(".$it") }
    }

    /** IPv4 addresses of [host] from DoH; empty when nobody answered. */
    fun lookup(host: String): List<InetAddress> {
        cache[host]?.let { (ips, at) -> if (System.currentTimeMillis() - at < keep) return ips }
        for (server in servers) {
            try {
                val req = Request.Builder().url("$server?name=$host&type=A").header("accept", "application/dns-json").build()
                val body = client.newCall(req).execute().use { it.body?.string() } ?: continue
                val answers = JSONObject(body).optJSONArray("Answer") ?: continue
                val ips = (0 until answers.length()).map { answers.getJSONObject(it) }.filter { it.optInt("type") == 1 }
                    .mapNotNull { runCatching { InetAddress.getByName(it.getString("data")) }.getOrNull() }
                if (ips.isNotEmpty()) {
                    cache[host] = ips to System.currentTimeMillis()
                    return ips
                }
            } catch (e: Exception) {
                // the next server
            }
        }
        return emptyList()
    }
}
