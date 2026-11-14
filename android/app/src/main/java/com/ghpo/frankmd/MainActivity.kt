package com.ghpo.frankmd

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge

class MainActivity : ComponentActivity() {
    private lateinit var vault: VaultStore

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        window.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        vault = VaultStore(filesDir)
        val settings = SettingsStore(applicationContext)
        PdfShare.clearExports(applicationContext)
        SyncScheduler.schedule(applicationContext)
        SyncScheduler.syncNow(applicationContext)
        setContent {
            val themeMode = androidx.compose.runtime.remember {
                androidx.compose.runtime.mutableStateOf(settings.themeMode)
            }
            FrankTheme(themeMode.value) {
                FrankApp(
                    vault = vault,
                    settings = settings,
                    themeMode = themeMode.value,
                    onThemeModeChange = { mode ->
                        settings.themeMode = mode
                        themeMode.value = mode
                    },
                )
            }
        }
    }

    override fun onDestroy() {
        vault.close()
        super.onDestroy()
    }
}
