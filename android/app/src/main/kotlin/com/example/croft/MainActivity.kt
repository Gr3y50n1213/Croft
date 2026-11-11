// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.example.croft.mail.MailSession
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.nav.CroftApp

/**
 * The only activity. It applies the stored theme and hosts the simple navigation ([CroftApp]). No orientation or size
 * locks; Back goes through the system back-callback API in the navigation. It does not open the Rust core (that must not
 * happen on the main thread; later tasks open it off the main thread when a screen needs mail).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // A missing library or a version mismatch shows the error line instead of crashing.
        val info = runCatching { loadRustInfo() }.getOrNull()
        val store = ThemeStore.shared(applicationContext)
        val session = MailSession.shared(applicationContext)
        setContent { CroftApp(store, info, session, swipeStore = com.example.croft.ui.swipe.SwipeSettingsStore.shared(applicationContext)) }
    }
}
