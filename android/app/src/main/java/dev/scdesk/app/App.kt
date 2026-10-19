package dev.scdesk.app

import kotlinx.coroutines.launch
import android.app.Application

class App : Application() {
    lateinit var store: Store
    lateinit var api: ScApi
    lateinit var engine: Engine
    lateinit var link: Link
    lateinit var rooms: Rooms
    lateinit var charts: Charts

    /** the covers of the tiles of the categories, found once */
    val fans = java.util.concurrent.ConcurrentHashMap<String, List<String?>>()
    lateinit var wave: Wave
    lateinit var eq: EqController

    /** the servers of the VPN subscription (given by the computer), and the state of asking for them */
    val vpnServers = kotlinx.coroutines.flow.MutableStateFlow<List<VpnServer>>(emptyList())
    val vpnWaiting = kotlinx.coroutines.flow.MutableStateFlow(false)
    val vpnNote = kotlinx.coroutines.flow.MutableStateFlow<String?>(null)

    /** work that goes on when a screen is left (the saving of all the likes) */
    val appScope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.SupervisorJob() + kotlinx.coroutines.Dispatchers.IO)

    /** the ids of the liked tracks (asked of the computer, which does the liking), and the sleep timer in minutes (0: off) */
    val liked = kotlinx.coroutines.flow.MutableStateFlow<Set<Long>>(emptySet())
    val sleepMin = kotlinx.coroutines.flow.MutableStateFlow(0)
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    private val sleepStop = Runnable { engine.pause(); sleepMin.value = 0 }

    /** The music stops after [min] minutes; 0 cancels. */
    fun setSleep(min: Int) {
        handler.removeCallbacks(sleepStop)
        sleepMin.value = min
        if (min > 0) handler.postDelayed(sleepStop, min * 60_000L)
    }

    override fun onCreate() {
        super.onCreate()
        store = Store(this)
        api = ScApi(store).also { it.dir = java.io.File(cacheDir, "api") }
        C.mode = store.theme
        C.followSystem = store.followSystem
        C.accentIdx = store.accent
        C.useCover = store.accentFromCover
        C.density = store.density
        C.corners = store.corners
        L.lang = store.lang.ifEmpty { L.systemDefault() }
        if (store.customBg != 0 && store.customText != 0) {
            C.customBg = androidx.compose.ui.graphics.Color(store.customBg)
            C.customText = androidx.compose.ui.graphics.Color(store.customText)
            C.customAccent = if (store.customAccent != 0) androidx.compose.ui.graphics.Color(store.customAccent) else null
        }
        C.squareButton = store.squareButton
        C.anim = store.animations
        Downloads.load(this, store)
        // a while after the start (it stays light): the anonymous count of users, once a day
        appScope.launch { kotlinx.coroutines.delay(60_000); Stats.maybeSend(store) }
        CoverCache.dir = java.io.File(cacheDir, "covers")
        engine = Engine(this)
        link = Link(this)
        rooms = Rooms(this)
        charts = Charts(this)
        wave = Wave(this)
        eq = EqController(store)
        // the equalizer set here goes to the computer; what the computer set is not sent back
        eq.onChanged = { s, fromRemote -> if (!fromRemote) link.sendEq(s) }
        engine.onQueueReplaced = { byWave -> wave.active = byWave }
        engine.onError = { msg -> android.os.Handler(mainLooper).post { android.widget.Toast.makeText(this, msg, android.widget.Toast.LENGTH_LONG).show() } }
        engine.onStarted = { t ->
            Thread { History.add(store, t) }.start()
            // quiet at night: a new track starts softer (the volume is the player's own, not the system's)
            if (store.nightQuiet) {
                val h = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
                if (h >= 23 || h < 7) engine.volume(0.6f)
            }
        }
        engine.onChanged = { link.onPlayerChanged() }
        engine.connect()
        // the engine of the server tunnel: what an earlier run left is stopped, the picked server is started again
        Tunnel.cleanLeftover(this)
        vpnServers.value = Tunnel.parse(store.vpnServers)
        val sel = store.vpnSelected
        if (sel >= 0 && Tunnel.available(this)) {
            Thread {
                vpnServers.value.getOrNull(sel)?.let { if (Tunnel.start(this, it)) Tunnel.selected = sel }
            }.start()
        }
        // what an earlier update downloaded does not stay
        Updater.cleanup(this)
        if (BuildConfig.DEBUG) {
            // a test aid: install the package put into the cache by hand (adb run-as), the way an update would
            val filter = android.content.IntentFilter("dev.scdesk.mobile.DEBUG_INSTALL")
            val r = object : android.content.BroadcastReceiver() {
                override fun onReceive(c: android.content.Context, i: android.content.Intent) {
                    Updater.install(c, java.io.File(c.cacheDir, "update/sc-desk.apk"))
                }
            }
            if (android.os.Build.VERSION.SDK_INT >= 33) registerReceiver(r, filter, android.content.Context.RECEIVER_EXPORTED) else registerReceiver(r, filter)
        }
        // without a sign-in the app still searches and plays: it needs only the public client_id of the site
        if (store.clientId.isEmpty()) {
            Thread {
                try { store.clientId = ScApi.discoverClientId() } catch (e: Exception) { store.clientId = FALLBACK_CLIENT_ID }
            }.start()
        }
    }
}
