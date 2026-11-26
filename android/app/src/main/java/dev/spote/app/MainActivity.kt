package dev.spote.app

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.spote.app.setup.SetupLinkBus
import dev.spote.app.ui.SpoteRoot
import dev.spote.app.ui.theme.SpoteTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        publishSetupLink(intent)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.parseColor("#070708")),
        )
        setContent {
            SpoteRoot()
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        publishSetupLink(intent)
    }

    /**
     * Publica el enlace en el bus de proceso; no navega ni toca ViewModels.
     * [SetupLinkBus.publish] ignora lo que no sea un `spote://setup` válido.
     */
    private fun publishSetupLink(intent: Intent?) {
        SetupLinkBus.publish(intent?.dataString)
    }
}
