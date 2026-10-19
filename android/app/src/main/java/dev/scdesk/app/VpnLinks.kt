package dev.scdesk.app

import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.net.URLDecoder

/**
 * A VPN subscription read on the phone itself, when there is no computer to take the servers from: the same as
 * src-tauri/src/net/vpn.rs does (links vless:// vmess:// trojan:// ss:// hysteria2:// into sing-box outbounds).
 */
object VpnLinks {
    private val CLIENTS = listOf("Happ/4.3.0", "v2rayN/7.0", "HiddifyNext/2.5.7", "NekoBox/1.3.0", "Streisand/1.6")

    private fun b64(s: String): String? {
        val clean = s.filter { !it.isWhitespace() && it != '=' }
        return runCatching { String(android.util.Base64.decode(clean, android.util.Base64.DEFAULT or android.util.Base64.URL_SAFE)) }.getOrNull()
    }

    private fun dec(s: String) = runCatching { URLDecoder.decode(s, "UTF-8") }.getOrDefault(s)

    private fun nameOf(link: String): String {
        if (link.startsWith("vmess://")) {
            b64(link.removePrefix("vmess://"))?.let { runCatching { JSONObject(it).optString("ps") }.getOrNull() }?.takeIf { it.isNotEmpty() }?.let { return it }
        }
        return link.substringAfterLast('#', "").takeIf { it.isNotEmpty() }?.let(::dec) ?: link.substringBefore("://")
    }

    private fun transport(o: JSONObject, kind: String, path: String, host: String, service: String): Boolean {
        when (kind) {
            "", "tcp", "raw" -> {}
            "ws" -> o.put("transport", JSONObject().put("type", "ws").put("path", path.ifEmpty { "/" }).put("headers", if (host.isEmpty()) JSONObject() else JSONObject().put("Host", host)))
            "grpc" -> o.put("transport", JSONObject().put("type", "grpc").put("service_name", service))
            "httpupgrade" -> o.put("transport", JSONObject().put("type", "httpupgrade").put("path", path).put("host", host))
            else -> return false // a transport sing-box lacks (xhttp ...)
        }
        return true
    }

    /** The sing-box outbound of one server link; null when it is not a link of a kind that is known. */
    fun outbound(link: String): JSONObject? {
        return try {
        if (link.startsWith("vmess://")) {
            val v = JSONObject(b64(link.removePrefix("vmess://")) ?: return null)
            val s = { k: String -> v.opt(k)?.toString().orEmpty() }
            val o = JSONObject().put("type", "vmess").put("server", s("add")).put("server_port", s("port").toIntOrNull() ?: return null).put("uuid", s("id"))
                .put("security", s("scy").ifEmpty { "auto" }).put("alter_id", s("aid").toIntOrNull() ?: 0)
            if (!transport(o, s("net"), s("path"), s("host"), s("path"))) return null
            if (s("tls") == "tls") o.put("tls", JSONObject().put("enabled", true).put("server_name", s("sni").ifEmpty { s("host") }).put("utls", JSONObject().put("enabled", true).put("fingerprint", "chrome")))
            o
        } else {
            val u = URI(link.substringBefore('#'))
            val query = (u.rawQuery ?: "").split('&').filter { it.contains('=') }.associate { it.substringBefore('=') to dec(it.substringAfter('=')) }
            val q = { k: String -> query[k].orEmpty() }
            val host = u.host?.trim('[', ']') ?: return null
            val port = u.port.takeIf { it > 0 } ?: return null
            val user = dec(u.rawUserInfo ?: "")
            val scheme = u.scheme
            when (scheme) {
                "ss" -> {
                    val creds = b64(user)?.takeIf { it.contains(':') } ?: user
                    if (!creds.contains(':')) return null
                    return JSONObject().put("type", "shadowsocks").put("server", host).put("server_port", port).put("method", creds.substringBefore(':')).put("password", dec(creds.substringAfter(':')))
                }
                "hysteria2", "hy2" -> {
                    val o = JSONObject().put("type", "hysteria2").put("server", host).put("server_port", port).put("password", user)
                        .put("tls", JSONObject().put("enabled", true).put("server_name", q("sni").ifEmpty { host }).put("insecure", q("insecure") == "1"))
                    if (q("obfs") == "salamander") o.put("obfs", JSONObject().put("type", "salamander").put("password", q("obfs-password")))
                    return o
                }
            }
            val o = when (scheme) {
                "vless" -> JSONObject().put("type", "vless").put("server", host).put("server_port", port).put("uuid", user).also { if (q("flow").isNotEmpty()) it.put("flow", q("flow")) }
                "trojan" -> JSONObject().put("type", "trojan").put("server", host).put("server_port", port).put("password", user)
                else -> return null
            }
            if (!transport(o, q("type"), q("path"), q("host"), q("serviceName"))) return null
            if (q("security") == "tls" || q("security") == "reality") {
                val tls = JSONObject().put("enabled", true).put("server_name", q("sni").ifEmpty { host }).put("utls", JSONObject().put("enabled", true).put("fingerprint", q("fp").ifEmpty { "chrome" }))
                if (q("alpn").isNotEmpty()) tls.put("alpn", JSONArray(q("alpn").split(',')))
                if (q("security") == "reality") tls.put("reality", JSONObject().put("enabled", true).put("public_key", q("pbk")).put("short_id", q("sid")))
                o.put("tls", tls)
            }
            o
        }
        } catch (e: Exception) {
            null
        }
    }

    /** The servers in the text of a subscription: links one to a line, or the base64 of such lines. */
    fun servers(body: String): List<VpnServer> {
        val text = if (body.contains("://")) body else b64(body.trim()).orEmpty()
        return text.lines().map { it.trim() }.filter { it.contains("://") }.mapNotNull { l -> outbound(l)?.let { VpnServer(nameOf(l), it) } }
    }

    /** A subscription address or one server link, pasted: its servers (blocking, the network). */
    fun fetch(input: String): List<VpnServer> {
        val text = input.trim()
        if (!(text.startsWith("http://") || text.startsWith("https://"))) return servers(text)
        var encrypted = false
        for (ua in CLIENTS) {
            val body = runCatching {
                http.newCall(Request.Builder().url(text).header("User-Agent", ua).build()).execute().use { r -> if (r.isSuccessful) r.body?.string() else null }
            }.getOrNull() ?: continue
            if (body.trimStart().startsWith("happ://crypt")) { encrypted = true; continue }
            val list = servers(body)
            // a stub instead of servers (an invitation to install an app): a few entries that point nowhere
            val stub = list.size <= 3 && list.all { s -> s.outbound.optString("server") in setOf("0.0.0.0", "127.0.0.1", "localhost", "") || listOf("установите", "обновите", "install", "update").any { s.name.lowercase().contains(it) } }
            if (list.isNotEmpty() && !stub) return list
        }
        throw java.io.IOException(
            if (encrypted) "подписка зашифрована для Happ: попросите обычную ссылку или вставьте ссылку на один сервер (vless://...)"
            else "сервис не отдал серверы: вставьте ссылку на один сервер (vless://...) из своей VPN-программы",
        )
    }
}
