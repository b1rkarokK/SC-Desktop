package dev.scdesk.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import androidx.core.content.ContextCompat

class MainActivity : ComponentActivity() {
    /** a pairing link the phone was opened with (the camera of the system found the QR) */
    private val pairLink = mutableStateOf<String?>(null)

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        android.util.Log.i("SCDesk", "new intent: ${intent.data?.scheme}://${intent.data?.host}")
        pairLink.value = intent.data?.toString()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // the notification of the player needs this on Android 13 and later
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { }.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        val app = application as App
        pairLink.value = intent?.data?.toString()
        setContent { SCTheme { Root(app, pairLink.value) } }
    }

    override fun onStart() {
        super.onStart()
        (application as App).link.setForeground(true)
    }

    override fun onStop() {
        (application as App).link.setForeground(false)
        super.onStop()
    }
}
