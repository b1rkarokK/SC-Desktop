package dev.scdesk.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Secrets of the app (the SoundCloud login, the key of the link) are kept sealed with a key the system holds. */
private object Vault {
    private const val ALIAS = "scdesk-vault"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return gen.generateKey()
    }

    fun seal(text: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        return Base64.getEncoder().encodeToString(c.iv + c.doFinal(text.toByteArray()))
    }

    fun open(blob: String): String? = try {
        val raw = Base64.getDecoder().decode(blob)
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw, 0, 12)) }
        String(c.doFinal(raw, 12, raw.size - 12))
    } catch (e: Exception) {
        null
    }
}

/** What this phone knows: the login of the account (given by the computer over the QR pairing) and the link. */
class Store(ctx: Context) {
    private val p = ctx.getSharedPreferences("scdesk", Context.MODE_PRIVATE)

    private fun secret(k: String): String = p.getString(k, null)?.let { Vault.open(it) } ?: ""
    private fun putSecret(k: String, v: String) = p.edit().putString(k, if (v.isEmpty()) "" else Vault.seal(v)).apply()

    var clientId: String
        get() = secret("cid")
        set(v) = putSecret("cid", v)
    var oauthToken: String
        get() = secret("tok")
        set(v) = putSecret("tok", v)
    var streamToken: String
        get() = secret("stok")
        set(v) = putSecret("stok", v)
    /** the servers of the VPN subscription the computer gave: JSON of [{n, o}] */
    var vpnServers: String
        get() = secret("vpns")
        set(v) = putSecret("vpns", v)
    var linkKey: String
        get() = secret("lkey")
        set(v) = putSecret("lkey", v)

    /** the equalizer as JSON (the computer's format); not a secret */
    var eqJson: String
        get() = p.getString("eq", "") ?: ""
        set(v) = p.edit().putString("eq", v).apply()

    /** when the feed of updates was asked last (ms) */
    var updateCheckedAt: Long
        get() = p.getLong("upd", 0)
        set(v) = p.edit().putLong("upd", v).apply()

    /** the look beyond the theme: follows the system, the accent (an index of the swatches), the accent from the cover, the density of the lists (0 tight, 1, 2 roomy), the square play button, the animations, quiet at night */
    var followSystem: Boolean
        get() = p.getBoolean("fsys", false)
        set(v) = p.edit().putBoolean("fsys", v).apply()
    var accent: Int
        get() = p.getInt("accent", 0)
        set(v) = p.edit().putInt("accent", v).apply()
    var accentFromCover: Boolean
        get() = p.getBoolean("acov", false)
        set(v) = p.edit().putBoolean("acov", v).apply()
    /** the roundness of the corners: 0 sharp, 1 usual, 2 round */
    var corners: Int
        get() = p.getInt("corners", 1)
        set(v) = p.edit().putInt("corners", v).apply()
    /** the user's own colours (ARGB), 0: not set */
    var customBg: Int
        get() = p.getInt("cbg", 0)
        set(v) = p.edit().putInt("cbg", v).apply()
    var customText: Int
        get() = p.getInt("ctx", 0)
        set(v) = p.edit().putInt("ctx", v).apply()
    var customAccent: Int
        get() = p.getInt("cac", 0)
        set(v) = p.edit().putInt("cac", v).apply()
    var density: Int
        get() = p.getInt("dens", 1)
        set(v) = p.edit().putInt("dens", v).apply()
    var squareButton: Boolean
        get() = p.getBoolean("sqbtn", true)
        set(v) = p.edit().putBoolean("sqbtn", v).apply()
    var animations: Boolean
        get() = p.getBoolean("anim", true)
        set(v) = p.edit().putBoolean("anim", v).apply()
    var nightQuiet: Boolean
        get() = p.getBoolean("nightq", false)
        set(v) = p.edit().putBoolean("nightq", v).apply()

    /** the settings of the wave and its exclusions (JSON, see Wave.kt) */
    var waveJson: String
        get() = p.getString("wave", "") ?: ""
        set(v) = p.edit().putString("wave", v).apply()

    /** what the charts matched on SoundCloud: "key=id;key=id" (0: not found) */
    var chartMatches: String
        get() = p.getString("cmatch", "") ?: ""
        set(v) = p.edit().putString("cmatch", v).apply()

    /** the saved tracks (JSON, see Downloads.kt) and the room for them in gigabytes */
    var downloads: String
        get() = p.getString("dls", "") ?: ""
        set(v) = p.edit().putString("dls", v).apply()
    /** «Скачать все» only on Wi-Fi (the mobile data is not spent on gigabytes of music) */
    var downloadWifiOnly: Boolean
        get() = p.getBoolean("dlwifi", true)
        set(v) = p.edit().putBoolean("dlwifi", v).apply()
    /** counted in the anonymous number of users (Stats.kt): a random number, the version and the system, once a day */
    var statsAnon: Boolean
        get() = p.getBoolean("statsanon", true)
        set(v) = p.edit().putBoolean("statsanon", v).apply()
    var installId: String
        get() = p.getString("installid", "") ?: ""
        set(v) = p.edit().putString("installid", v).apply()
    var statsDay: String
        get() = p.getString("statsday", "") ?: ""
        set(v) = p.edit().putString("statsday", v).apply()
    var downloadLimitGb: Int
        get() = p.getInt("dlimit", 5)
        set(v) = p.edit().putInt("dlimit", v).apply()
    /** the look: 0 «Тёплая», 1 OLED, 2 светлая */
    var theme: Int
        get() = p.getInt("theme", 0)
        set(v) = p.edit().putInt("theme", v).apply()

    /** what was looked for lately (a JSON list in a string), the newest first */
    var recents: List<String>
        get() = try { val a = org.json.JSONArray(p.getString("recents", "[]")); (0 until a.length()).map { a.getString(it) } } catch (e: Exception) { emptyList() }
        set(v) = p.edit().putString("recents", org.json.JSONArray(v).toString()).apply()

    /** the plays of this phone (JSON, see History in Extra.kt); not a secret, never leaves the phone */
    var history: String
        get() = p.getString("hist", "") ?: ""
        set(v) = p.edit().putString("hist", v).apply()
    var autoUpdate: Boolean
        get() = p.getBoolean("autoupd", true)
        set(v) = p.edit().putBoolean("autoupd", v).apply()

    /** the picked server (an index into the list), -1: none */
    var vpnSelected: Int
        get() = p.getInt("vpnsel", -1)
        set(v) = p.edit().putInt("vpnsel", v).apply()

    var userId: Long
        get() = p.getLong("uid", 0)
        set(v) = p.edit().putLong("uid", v).apply()
    var userName: String
        get() = p.getString("uname", "") ?: ""
        set(v) = p.edit().putString("uname", v).apply()
    var linkChan: String
        get() = p.getString("lchan", "") ?: ""
        set(v) = p.edit().putString("lchan", v).apply()
    var pcDev: String
        get() = p.getString("pcdev", "") ?: ""
        set(v) = p.edit().putString("pcdev", v).apply()
    var pcName: String
        get() = p.getString("pcname", "") ?: ""
        set(v) = p.edit().putString("pcname", v).apply()

    /** this phone, for the other devices */
    val deviceId: String = p.getString("dev", null) ?: ("ph-" + java.util.UUID.randomUUID().toString().take(12)).also { p.edit().putString("dev", it).apply() }
    val deviceName: String = "${Build.MANUFACTURER.replaceFirstChar { it.uppercase() }} ${Build.MODEL}"

    /** the account is there (from the sign-in here, or from the computer) */
    val signedIn: Boolean get() = oauthToken.isNotEmpty()

    /** the computer is linked: the devices can see each other */
    val linked: Boolean get() = linkChan.isNotEmpty() && linkKey.isNotEmpty()

    fun signOut() {
        val dev = deviceId
        p.edit().clear().putString("dev", dev).apply()
    }
}
