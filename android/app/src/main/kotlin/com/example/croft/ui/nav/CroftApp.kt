// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.example.croft.CroftTheme
import com.example.croft.RustInfo
import com.example.croft.mail.Connectivity
import com.example.croft.mail.LocalMailSettings
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.mail.MailSettingsStore
import com.example.croft.mail.accounts.AccountOps
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.AccountPrefsStore
import com.example.croft.mail.followConnectivity
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.onboarding.OnboardingEnvironment
import com.example.croft.ui.swipe.LocalSwipeSettings
import com.example.croft.ui.swipe.SwipeSettingsStore
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The app as the activity shows it: the stored theme applied with [CroftTheme], and the navigation inside it. Nothing is
 * drawn until the stored settings have been read, so the app never flashes the default look first (the window background
 * of the activity theme shows meanwhile). A change made in Appearance shows at once and is written behind it; DataStore
 * keeps the writes in order, so the last one wins.
 */
@Composable
fun CroftApp(
    store: ThemeStore,
    info: RustInfo?,
    session: MailSession,
    modifier: Modifier = Modifier,
    startAt: Route = Route.HOME,
    swipeStore: SwipeSettingsStore? = null,
    /** The mail settings (S49 Undo time); the default time when null. */
    mailStore: MailSettingsStore? = null,
    /** The platform's connectivity callback, turned into `set_network_available` for the core; none when null (tests). */
    connectivity: Connectivity? = null,
    /** The sign-in flow's parts (the real core and secret store); none in tests of the other screens. */
    onboarding: OnboardingEnvironment? = null,
    /** What Accounts (S44), Account (S45) and About (S60) change in the core and the secret store, and the core's sample mailbox; none in tests of other screens. */
    accountOps: AccountOps? = null,
    /** The app's own account order, names and colours; defaults when null. */
    accountPrefsStore: AccountPrefsStore? = null,
    /** A `mailto:` link or a share that started or reached the app: compose opens on it. */
    composeRequest: ComposeRequest? = null,
    onComposeRequestHandled: () -> Unit = {},
) {
    // The session the Inbox shows: the one given, until the first account is added; then the one over the core.
    var active by remember(session) { mutableStateOf(session) }
    var settings by remember { mutableStateOf<ThemeSettings?>(null) }
    LaunchedEffect(store) { settings = store.settings.first() }
    var swipe by remember { mutableStateOf<SwipeSettings?>(null) }
    LaunchedEffect(swipeStore) { swipe = swipeStore?.settings?.first() ?: SwipeSettings.Default }
    var mail by remember { mutableStateOf<MailSettings?>(null) }
    LaunchedEffect(mailStore) { mail = mailStore?.settings?.first() ?: MailSettings() }
    // The Undo time goes to the snackbar and to the core, at the start and at every change.
    LaunchedEffect(active, mail) {
        val m = mail ?: return@LaunchedEffect
        active.undoWindowMs = m.undoWindowMs
        try { active.repository.setUndoWindowMs(m.undoWindowMs) } catch (e: com.example.croft.mail.MailException) { /* the core is closing */ }
    }
    // Whether the device has a network: the core shows Offline and sends nothing while it does not.
    if (connectivity != null) LaunchedEffect(active, connectivity) { active.followConnectivity(connectivity) }
    var accountPrefs by remember { mutableStateOf<AccountPrefs?>(null) }
    LaunchedEffect(accountPrefsStore) { accountPrefs = accountPrefsStore?.prefs?.first() ?: AccountPrefs() }
    val scope = rememberCoroutineScope()
    val prefsNow = accountPrefs
    val swipeNow = swipe
    val themeNow = settings
    val mailNow = mail
    if (themeNow != null && swipeNow != null && mailNow != null && prefsNow != null) {
        CroftTheme(themeNow) {
            CompositionLocalProvider(LocalSwipeSettings provides swipeNow, LocalMailSettings provides mailNow) {
                CroftNavHost(
                    info = info,
                    session = active,
                    settings = themeNow,
                    modifier = modifier,
                    startAt = startAt,
                    onSettingsChange = { next -> settings = next; scope.launch { store.set(next) } },
                    swipeSettings = swipeNow,
                    onSwipeSettingsChange = { next -> swipe = next; scope.launch { swipeStore?.set(next) } },
                    mailSettings = mailNow,
                    onMailSettingsChange = { next -> mail = next; scope.launch { mailStore?.set(next) } },
                    onboarding = onboarding,
                    onAccountReady = { onboarding?.let { active = it.openSession() } },
                    accountOps = accountOps,
                    composeRequest = composeRequest,
                    onComposeRequestHandled = onComposeRequestHandled,
                    accountPrefs = prefsNow,
                    onAccountPrefsChange = { next -> accountPrefs = next; scope.launch { accountPrefsStore?.set(next) } },
                    // The removal path waits for this: the entries of a removed account are gone from the stored prefs when it returns.
                    removeAccountPrefs = { id ->
                        accountPrefs = accountPrefs?.without(id)
                        accountPrefsStore?.update { it.without(id) }
                    },
                    // "Erase all Croft data": the core and the secrets are already gone; the settings stores go back to their defaults too.
                    onErased = {
                        store.set(ThemeSettings.Default); swipeStore?.set(SwipeSettings.Default); mailStore?.set(MailSettings()); accountPrefsStore?.set(AccountPrefs())
                        settings = ThemeSettings.Default; swipe = SwipeSettings.Default; mail = MailSettings(); accountPrefs = AccountPrefs()
                        // No core is opened here: an erase leaves no Croft file, and the next one is made when the person adds an account (the
                        // sign-in flow and "Try with sample mail" open the core when they first need it, then `onAccountReady` builds the session).
                        active = MailSession.unavailable()
                    },
                )
            }
        }
    }
}
