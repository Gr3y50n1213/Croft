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
import com.example.croft.mail.accounts.AccountPrefsStore
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.nav.CroftApp
import com.example.croft.ui.nav.Route
import com.example.croft.ui.accounts.appAccountOps
import com.example.croft.ui.onboarding.appOnboardingEnvironment

/**
 * The only activity. It applies the stored theme and hosts the simple navigation ([CroftApp]). No orientation or size
 * locks; Back goes through the system back-callback API in the navigation. It opens the Rust core **off the main thread**
 * ([MailRuntime.session]: the database key from the Keystore, the credential provider over the secret store) and shows the
 * Inbox over it. While no account exists (the sample mailbox counts as one) the app starts at the sign-in flow (S01); nothing is drawn until
 * the session and the start screen are known, so the app never flashes the wrong screen.
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
        val onboarding = appOnboardingEnvironment(applicationContext, mailStore)
        val accountOps = appAccountOps(applicationContext)
        setContent {
            var session by remember { mutableStateOf<MailSession?>(null) }
            var startAt by remember { mutableStateOf<Route?>(null) }
            LaunchedEffect(Unit) {
                val ready = MailRuntime.session(applicationContext, mailStore)
                // No account at all (a fresh install, or everything was removed) starts at S01; with one (the sample mailbox too) the app starts on the Inbox.
                startAt = if (!ready.coreUnavailable && MailRuntime.hasAccounts(applicationContext)) Route.HOME else Route.ONBOARDING
                session = ready
            }
            val ready = session
            val start = startAt
            if (ready != null && start != null) {
                CroftApp(
                    store, info, ready,
                    startAt = start,
                    onboarding = onboarding,
                    accountOps = accountOps,
                    accountPrefsStore = AccountPrefsStore.shared(applicationContext),
                    swipeStore = com.example.croft.ui.swipe.SwipeSettingsStore.shared(applicationContext),
                    mailStore = mailStore,
                    connectivity = connectivity,
                )
            }
        }
    }
}
