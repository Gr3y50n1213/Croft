// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.example.croft.mail.AndroidConnectivity
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettingsStore
import com.example.croft.mail.accounts.AccountPrefsStore
import com.example.croft.mail.companion.CompanionRuntime
import com.example.croft.push.PushRuntime
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.compose.ComposeIntents
import com.example.croft.notify.CheckSchedule
import com.example.croft.notify.NotificationIntents
import com.example.croft.notify.NotificationOpen
import com.example.croft.notify.NotifySettingsStore
import com.example.croft.ui.nav.ComposeRequest
import com.example.croft.ui.nav.NotifiedMessage
import com.example.croft.ui.nav.OpenMessageRequest
import com.example.croft.ui.nav.CroftApp
import com.example.croft.ui.nav.Route
import com.example.croft.ui.nav.StartProblemScreen
import com.example.croft.ui.accounts.appAccountOps
import com.example.croft.ui.onboarding.appOnboardingEnvironment
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * The only activity. It applies the stored theme and hosts the simple navigation ([CroftApp]). No orientation or size
 * locks; Back goes through the system back-callback API in the navigation. It opens the Rust core **off the main thread**
 * ([MailRuntime.session]: the database key from the Keystore, the credential provider over the secret store) and shows the
 * Inbox over it. While no account exists (the sample mailbox counts as one) the app starts at the sign-in flow (S01); nothing is drawn until
 * the session and the start screen are known, so the app never flashes the wrong screen.
 */
class MainActivity : ComponentActivity() {
    /** A `mailto:` link or a share that opened (or reached) the app, until the navigation has taken it. */
    private var composeRequest by mutableStateOf<ComposeRequest?>(null)

    /** A tap on a new-mail notification (S32), until the navigation has taken it. */
    private var openRequest by mutableStateOf<OpenMessageRequest?>(null)

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        take(intent)
    }

    /** What [intent] asks for: compose (a link, a share, a notification's Reply) or a message (a notification's tap). */
    private fun take(intent: Intent?) {
        when (val open = NotificationIntents.parse(intent, this)) {
            is NotificationOpen.Message -> openRequest = OpenMessageRequest(NotifiedMessage(open.account, open.messageId, open.uid), reply = false)
            is NotificationOpen.Reply -> openRequest = OpenMessageRequest(NotifiedMessage(open.account, open.messageId, open.uid), reply = true)
            NotificationOpen.Inbox -> openRequest = OpenMessageRequest(null, reply = false)
            is NotificationOpen.Event -> openRequest = OpenMessageRequest(null, reply = false, event = com.example.croft.ui.nav.OpenEvent(open.account, open.eventId, open.keyMs))
            null -> ComposeIntents.parse(intent, this)?.let { composeRequest = ComposeRequest(it) }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // Only on a first start: after a rotation the intent is still the old one and the request was already used.
        if (savedInstanceState == null) take(intent)
        // A missing library or a version mismatch shows the error line instead of crashing.
        val info = runCatching { loadRustInfo() }.getOrNull()
        val store = ThemeStore.shared(applicationContext)
        val mailStore = MailSettingsStore.shared(applicationContext)
        val connectivity = AndroidConnectivity(applicationContext)
        val onboarding = appOnboardingEnvironment(applicationContext, mailStore)
        val accountOps = appAccountOps(applicationContext)
        val notifyStore = NotifySettingsStore.shared(applicationContext)
        setContent {
            var session by remember { mutableStateOf<MailSession?>(null) }
            var startAt by remember { mutableStateOf<Route?>(null) }
            var problem by remember { mutableStateOf(false) }
            var attempt by remember { mutableIntStateOf(0) }
            LaunchedEffect(attempt) {
                problem = false
                val ready = MailRuntime.session(applicationContext, mailStore)
                // No account at all (a fresh install, or everything was removed) starts at S01; with one (the sample mailbox too) the app starts on
                // the Inbox. A list that could not be read is not "no account": the person sees a retry screen, never S01 by mistake.
                val count = if (ready.coreUnavailable) 0 else MailRuntime.accountCount(applicationContext)
                if (count == null) { problem = true; return@LaunchedEffect }
                startAt = if (count > 0) Route.HOME else Route.ONBOARDING
                session = ready
                // The periodic check (T20): kept as it is when it already runs (KEEP), and only while an account can notify (the sample
                // mailbox never does): otherwise it is stopped, so it never creates Croft files on its own.
                val real = !ready.coreUnavailable && runCatching { ready.repository.listAccounts().any { !it.isDemo } }.getOrDefault(false)
                CheckSchedule.forAccounts(applicationContext, notifyStore.current(), real)
                // T34d: with a Companion connected, make sure push is registered (a new token or endpoint is subscribed again). Never
                // asks the person anything from here: choosing a UnifiedPush distributor is S58's.
                if (!ready.coreUnavailable) {
                    launch(Dispatchers.IO) {
                        if (CompanionRuntime.connection(applicationContext)?.isConnected() == true) PushRuntime.start(applicationContext)
                    }
                }
            }
            val ready = session
            val start = startAt
            if (problem) {
                CroftTheme { StartProblemScreen(onRetry = { attempt++ }) }
            } else if (ready != null && start != null) {
                CroftApp(
                    store, info, ready,
                    startAt = start,
                    onboarding = onboarding,
                    accountOps = accountOps,
                    accountPrefsStore = AccountPrefsStore.shared(applicationContext),
                    swipeStore = com.example.croft.ui.swipe.SwipeSettingsStore.shared(applicationContext),
                    mailStore = mailStore,
                    connectivity = connectivity,
                    composeRequest = composeRequest,
                    onComposeRequestHandled = { composeRequest = null },
                    notifyStore = notifyStore,
                    openRequest = openRequest,
                    onOpenRequestHandled = { openRequest = null },
                )
            }
        }
    }
}
