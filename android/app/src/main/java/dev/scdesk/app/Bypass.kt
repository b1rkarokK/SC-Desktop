package dev.scdesk.app

import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import kotlin.concurrent.thread

/**
 * Getting through the block of a provider that cuts the connections of apps (it let Chrome through and let
 * an app's own TLS greeting hang): the same thing the computer does (src-tauri/src/net/bypass.rs). A tiny HTTPS
 * proxy on 127.0.0.1 (CONNECT only) that every connection of the app goes through; it cuts the first message of
 * the TLS connection (ClientHello) into two TLS records, the cut inside the name of the site, each in its own TCP
 * packet, and then only copies the bytes. The traffic itself is not touched and goes straight to the site.
 */
object Bypass {
    @Volatile private var port = 0

    /** how the greeting is cut (tried out on the phone, see NetTest); 1 = the way the computer does it */
    @Volatile var strategy = 1

    @Synchronized
    fun port(): Int {
        if (port != 0) return port
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        port = server.localPort
        thread(isDaemon = true, name = "bypass") {
            while (true) {
                val client = try { server.accept() } catch (e: Exception) { break }
                thread(isDaemon = true, name = "bypass-conn") { serve(client) }
            }
        }
        return port
    }

    private fun serve(client: Socket) {
        try {
            client.tcpNoDelay = true
            val input = client.getInputStream()
            val out = client.getOutputStream()
            // "CONNECT host:port HTTP/1.1" and the headers up to the empty line
            val head = StringBuilder()
            while (!head.endsWith("\r\n\r\n")) {
                val b = input.read()
                if (b < 0) return
                head.append(b.toChar())
                if (head.length > 8192) return
            }
            val target = head.lineSequence().first().split(' ').getOrNull(1) ?: return
            val host = target.substringBeforeLast(':')
            android.util.Log.i("SCDesk", "bypass: CONNECT $target")
            val port = target.substringAfterLast(':', "443").toIntOrNull() ?: 443
            val server = connectTo(host, port)
            out.write("HTTP/1.1 200 Connection established\r\n\r\n".toByteArray())
            out.flush()
            // the greeting, cut inside the name of the site
            val buf = ByteArray(16 * 1024)
            val n = input.read(buf)
            if (n <= 0) return
            val so = server.getOutputStream()
            android.util.Log.i("SCDesk", "bypass: greeting $n bytes, first byte ${buf[0].toInt() and 0xff}")
            for ((part, wait) in plan(buf.copyOf(n), host.toByteArray(), strategy)) {
                so.write(part)
                so.flush()
                Thread.sleep(wait)
            }
            val back = thread(isDaemon = true) { pump(server.getInputStream(), out, client, server) }
            pump(input, so, client, server)
            back.join(1000)
        } catch (e: Exception) {
            android.util.Log.w("SCDesk", "bypass: ${e.javaClass.simpleName} ${e.message}")
        } finally {
            try { client.close() } catch (e: Exception) { /* closed */ }
        }
    }

    /** The host by the addresses DoH gave (the system DNS may point a blocked site to a stub), else as the system knows it. */
    private val lastGood = java.util.concurrent.ConcurrentHashMap<String, String>()

    private fun connectTo(host: String, port: Int): Socket {
        // a block of the provider often covers only some addresses of the CDN: the audio ones open, the pictures and the site do not.
        // The CDN serves any of its sites from any of its addresses (the name is in the TLS greeting), so these go first
        val own = if (Doh.wanted(host)) Doh.lookup(host) else emptyList()
        val h = host.lowercase()
        val open = if (h.endsWith(".sndcdn.com") || h == "soundcloud.com" || h.endsWith(".soundcloud.com")) Doh.lookup("cf-hls-media.sndcdn.com") else emptyList()
        val ips = (open + own).distinctBy { it.hostAddress }
        android.util.Log.i("SCDesk", "bypass: $host doh=${ips.joinToString { it.hostAddress ?: "" }}")
        var last: Exception? = null
        // the address that opened last time for this site goes first
        val good = lastGood[h]
        for (ip in if (good != null) ips.sortedByDescending { it.hostAddress == good } else ips) {
            val s = Socket()
            s.tcpNoDelay = true
            try {
                s.connect(InetSocketAddress(ip, port), 3_000)
                ip.hostAddress?.let { lastGood[h] = it }
                return s
            } catch (e: Exception) {
                last = e
                try { s.close() } catch (x: Exception) { /* closed */ }
            }
        }
        val s = Socket()
        s.tcpNoDelay = true
        try {
            s.connect(InetSocketAddress(host, port), 15_000)
        } catch (e: Exception) {
            throw last ?: e
        }
        return s
    }

    private fun pump(from: InputStream, to: OutputStream, a: Socket, b: Socket) {
        try {
            val buf = ByteArray(16 * 1024)
            while (true) {
                val n = from.read(buf)
                if (n < 0) break
                to.write(buf, 0, n)
                to.flush()
            }
        } catch (e: Exception) {
            // closed
        } finally {
            try { a.close() } catch (e: Exception) { /* closed */ }
            try { b.close() } catch (e: Exception) { /* closed */ }
        }
    }

    /** The pieces of the first message and the wait after each (milliseconds), by strategy. */
    fun plan(data: ByteArray, name: ByteArray, s: Int): List<Pair<ByteArray, Long>> {
        val sni = indexOf(data, name)
        val mid = if (sni < 0) data.size / 2 else sni + maxOf(name.size / 2, 1)
        fun tcp(vararg at: Int, wait: Long): List<Pair<ByteArray, Long>> {
            val cuts = (listOf(0) + at.toList().filter { it in 1 until data.size } + listOf(data.size)).distinct().sorted()
            return cuts.zipWithNext { a, b -> data.copyOfRange(a, b) to wait }
        }
        return when (s) {
            0 -> listOf(data to 0L)
            1 -> cut(data, name).map { it to 2L }
            2 -> recordSplit(data, intArrayOf(1)).map { it to 2L }
            3 -> tcp(mid, wait = 30)
            4 -> tcp(1, wait = 30)
            5 -> tcp(1, maxOf(sni, 2), mid, wait = 10)
            6 -> cut(data, name).map { it to 100L }
            7 -> recordSplit(data, intArrayOf(maxOf(sni - 1, 1), mid, if (sni < 0) mid + 1 else sni + name.size)).map { it to 5L }
            else -> listOf(data to 0L)
        }
    }

    /** The first record cut into records at the given places of its body. */
    fun recordSplit(data: ByteArray, at: IntArray): List<ByteArray> {
        if (data.size < 6 || data[0].toInt() != 0x16) return listOf(data)
        val len = ((data[3].toInt() and 0xff) shl 8) or (data[4].toInt() and 0xff)
        if (len < 2 || data.size < 5 + len) return listOf(data)
        val body = data.copyOfRange(5, 5 + len)
        val cuts = (listOf(0) + at.filter { it in 1 until len }.distinct().sorted() + listOf(len)).distinct()
        val out = ArrayList<ByteArray>()
        for ((a, b) in cuts.zipWithNext()) {
            val part = body.copyOfRange(a, b)
            out.add(byteArrayOf(0x16, data[1], data[2], (part.size shr 8).toByte(), part.size.toByte()) + part)
        }
        if (data.size > 5 + len) out.add(data.copyOfRange(5 + len, data.size))
        return out
    }

    /**
     * The first TLS record (a handshake) as two records, the cut inside [name] when it is found there.
     * Anything else is passed as it is.
     */
    fun cut(data: ByteArray, name: ByteArray): List<ByteArray> {
        if (data.size < 6 || data[0].toInt() != 0x16) return listOf(data)
        val len = ((data[3].toInt() and 0xff) shl 8) or (data[4].toInt() and 0xff)
        if (len < 2 || data.size < 5 + len) return listOf(data)
        val body = data.copyOfRange(5, 5 + len)
        var at = indexOf(body, name).let { if (it < 0) len / 2 else it + maxOf(name.size / 2, 1) }
        at = at.coerceIn(1, len - 1)
        fun record(part: ByteArray): ByteArray =
            byteArrayOf(0x16, data[1], data[2], (part.size shr 8).toByte(), part.size.toByte()) + part
        val out = arrayListOf(record(body.copyOfRange(0, at)), record(body.copyOfRange(at, len)))
        if (data.size > 5 + len) out.add(data.copyOfRange(5 + len, data.size))
        return out
    }

    private fun indexOf(hay: ByteArray, needle: ByteArray): Int {
        if (needle.isEmpty() || hay.size < needle.size) return -1
        outer@ for (i in 0..hay.size - needle.size) {
            for (j in needle.indices) if (hay[i + j] != needle[j]) continue@outer
            return i
        }
        return -1
    }
}
