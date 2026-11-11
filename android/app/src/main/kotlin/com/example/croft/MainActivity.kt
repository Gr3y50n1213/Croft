// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.example.croft.mail.AndroidConnectivity
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettingsStore
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.nav.CroftApp

/**
 * The only activity. It applies the stored theme and hosts the simple navigation ([CroftApp]). No orientation or size
 * locks; Back goes through the system back-callback API in the navigation. It opens the Rust core **off the main thread**
 * ([MailRuntime.session]: the database key from the Keystore, the credential provider over the secret store) and shows the
 * Inbox over it. While no account exists yet it shows the sample mailbox instead (onboarding is T17); nothing is drawn until
 * the session is ready, so the app never flashes the sample first and then the real mail.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // A missing library or a version mismatch shows the error line instead of crashing.
        val info = runCatching { loadRustInfo() }.getOrNull()
        val store = ThemeStore.shared(applicationContext)
        val mailStore = MailSettingsStore.shared(applicationContext)
        val connectivity = AndroidConnectivity(applicationContext)
        setContent {
            var session by remember { mutableStateOf<MailSession?>(null) }
            LaunchedEffect(Unit) { session = MailRuntime.session(applicationContext, mailStore) }
            session?.let { ready ->
                CroftApp(
                    store, info, ready,
                    swipeStore = com.example.croft.ui.swipe.SwipeSettingsStore.shared(applicationContext),
                    mailStore = mailStore,
                    connectivity = connectivity,
                )
            }
        }
    }
}
