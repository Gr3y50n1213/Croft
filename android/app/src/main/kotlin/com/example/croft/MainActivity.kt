// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.LaunchedEffect
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.nav.CroftNavHost
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The only activity. It applies the stored theme and hosts the simple navigation. No orientation or size locks; Back goes
 * through the system back-callback API in [CroftNavHost]. It does not open the Rust core (that must not happen on the main
 * thread; later tasks open it off the main thread when a screen needs mail).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // A missing library or a version mismatch shows the error line instead of crashing.
        val info = runCatching { loadRustInfo() }.getOrNull()
        val store = ThemeStore.shared(applicationContext)
        setContent {
            // Nothing is drawn until the stored settings are read, so the app never flashes the default look first.
            var settings by remember { mutableStateOf<ThemeSettings?>(null) }
            LaunchedEffect(store) { settings = store.settings.first() }
            val scope = rememberCoroutineScope()
            settings?.let { current ->
                CroftTheme(current) {
                    CroftNavHost(
                        info = info,
                        settings = current,
                        // The screen shows the new value at once; the write follows in order (DataStore serialises them).
                        onSettingsChange = { next -> settings = next; scope.launch { store.set(next) } },
                    )
                }
            }
        }
    }
}
