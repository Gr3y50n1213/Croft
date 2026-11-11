// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import com.example.croft.RustInfo
import com.example.croft.debug.DebugGallery
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.ui.mail.InboxSettingsScreen
import com.example.croft.swipe.SwipeSettings
import com.example.croft.ui.swipe.SwipeSettingsFlow
import com.example.croft.ui.mail.InboxScreen
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.appearance.AppearanceFlow
import com.example.croft.ui.components.sharedAxisX
import com.example.croft.mail.accounts.AccountOps
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.onboarding.Step
import com.example.croft.ui.accounts.AboutLinks
import com.example.croft.ui.accounts.AboutScreen
import com.example.croft.ui.accounts.AccountScreen
import com.example.croft.ui.accounts.AccountsScreen
import com.example.croft.ui.accounts.LicencesScreen
import com.example.croft.ui.accounts.openInBrowser
import com.example.croft.ui.onboarding.OnboardingEnvironment
import com.example.croft.ui.onboarding.OnboardingHost
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The screens reachable today. S50 and its drill-down live inside [APPEARANCE] (see `AppearanceFlow`). S02 to S05 and S09 live inside [ONBOARDING] and [ADD_ACCOUNT]. */
enum class Route(val parent: Route?) {
    /** The sign-in flow (S01 to S05, S09). The first screen on a fresh install, while no account exists. It handles its own Back. */
    ONBOARDING(null),

    /** The start screen: the Inbox (S10), with selection mode, the drawer and the account switcher on top of it. */
    HOME(null),

    /** The settings list. */
    SETTINGS(HOME),

    /** S50 Appearance and its sub-screens. */
    APPEARANCE(SETTINGS),

    /** S47 Swipe actions and its picker S48. */
    SWIPE_ACTIONS(SETTINGS),

    /** S49 Inbox and categories (the Undo time so far). */
    INBOX_SETTINGS(SETTINGS),

    /** The component gallery (debug builds only). */
    GALLERY(SETTINGS),

    /** S44 Accounts. */
    ACCOUNTS(SETTINGS),

    /** S45 Account (the account is [CroftNavHost]'s `accountId`). */
    ACCOUNT(ACCOUNTS),

    /** S02 and on, to add another account from S44 or from the account switcher. It handles its own Back. */
    ADD_ACCOUNT(ACCOUNTS),

    /** S60 About and diagnostics. */
    ABOUT(SETTINGS),

    /** The open-source licences list, from S60. */
    LICENCES(ABOUT),
    ;

    /** How deep the screen is; going to a larger depth is "forward". */
    val depth: Int get() = generateSequence(this) { it.parent }.count() - 1
}

/**
 * The app's simple navigation: Home (the Inbox, S10 to S19), Settings (from the drawer), Appearance, and (in debug builds) the
 * component gallery. Back inside the Inbox (selection mode, the drawer, the account switcher) is handled by the Inbox first. Back goes to the
 * parent screen through the system back-callback API (`BackHandler` registers an `OnBackPressedCallback`); on Home it is
 * left to the system. Moving between screens uses the shared axis X transition at the animation level in force. The
 * route survives rotation and process death.
 */
@Composable
fun CroftNavHost(
    info: RustInfo?,
    session: MailSession,
    settings: ThemeSettings,
    onSettingsChange: (ThemeSettings) -> Unit,
    modifier: Modifier = Modifier,
    startAt: Route = Route.HOME,
    swipeSettings: SwipeSettings = SwipeSettings.Default,
    onSwipeSettingsChange: (SwipeSettings) -> Unit = {},
    mailSettings: MailSettings = MailSettings(),
    onMailSettingsChange: (MailSettings) -> Unit = {},
    /** The sign-in screens' parts. Without them (tests of the other screens) the route [Route.ONBOARDING] is not reachable. */
    onboarding: OnboardingEnvironment? = null,
    /** Called when accounts were added or removed, before the screens use them: the app builds a fresh session over the core. */
    onAccountReady: suspend () -> Unit = {},
    /** What S44, S45 and S60 change in the core and the secret store, and "Try with sample mail" (the core's own sample account). Without it the sample button just opens the Inbox (tests of other screens). */
    accountOps: AccountOps? = null,
    /** The app's own account order, names and colours (S44, S45). */
    accountPrefs: AccountPrefs = AccountPrefs(),
    onAccountPrefsChange: (AccountPrefs) -> Unit = {},
    /** After everything was erased in the core and the secret store: the app clears its settings stores and shows S01. */
    onErased: suspend () -> Unit = {},
) {
    var route by rememberSaveable { mutableStateOf(startAt) }
    var accountId by rememberSaveable { mutableStateOf(0L) }
    // Where Add account (S02) returns to: S44, or the Inbox when it was started from the account switcher.
    var addReturn by rememberSaveable { mutableStateOf(Route.ACCOUNTS) }
    var sampleProblem by remember { mutableStateOf(false) }
    var sampleBusy by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val prefsNow by rememberUpdatedState(accountPrefs)
    val sessionNow by rememberUpdatedState(session)
    if (route == Route.ONBOARDING && onboarding == null) route = Route.HOME
    val scope = rememberCoroutineScope()
    // A debug-only route can never be reached from a release build, even from saved state.
    if (route == Route.GALLERY && !DebugGallery.available) route = Route.SETTINGS
    // The sign-in flow has its own Back (one step back; on its first screen the system leaves the app).
    BackHandler(enabled = route != Route.HOME && route != Route.ONBOARDING && route != Route.ADD_ACCOUNT) { route = route.parent ?: Route.HOME }

    // Removing an account: its saved password and its rows in the core, then its entries in the app's own prefs, then a fresh session.
    // It runs to its end even if the screen goes away (a half removal would be worse than a late one). Returns how many accounts are
    // left, or null when it failed.
    val removeAccount: suspend (Long) -> Int? = remove@{ id ->
        val ops = accountOps ?: return@remove null
        withContext(NonCancellable) {
            try {
                ops.remove(id)
                onAccountPrefsChange(prefsNow.without(id))
                onAccountReady()
                sessionNow.repository.listAccounts().size
            } catch (e: Exception) {
                null
            }
        }
    }
    val eraseAll: suspend () -> Boolean = erase@{
        val ops = accountOps ?: return@erase false
        withContext(NonCancellable) {
            try {
                ops.eraseEverything()
                onErased()
                route = Route.ONBOARDING
                true
            } catch (e: Exception) {
                false
            }
        }
    }
    val trySample: () -> Unit = {
        val ops = accountOps
        if (ops == null) route = Route.HOME
        else if (!sampleBusy) {
            sampleBusy = true
            sampleProblem = false
            scope.launch {
                try {
                    withContext(NonCancellable) { ops.addSample(); onAccountReady() }
                    route = Route.HOME
                } catch (e: Exception) {
                    sampleProblem = true
                } finally {
                    sampleBusy = false
                }
            }
        }
    }
    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    AnimatedContent(
        targetState = route,
        modifier = modifier,
        label = "app-route",
        transitionSpec = { sharedAxisX(level, forward = targetState.depth > initialState.depth, rtl = rtl) },
    ) { current ->
        when (current) {
            Route.ONBOARDING -> if (onboarding != null) OnboardingHost(
                environment = onboarding,
                onTrySampleMail = trySample,
                onFinished = { scope.launch { onAccountReady(); route = Route.HOME } },
                sampleProblem = sampleProblem,
                onPrivacyPolicy = { context.openInBrowser(AboutLinks.PRIVACY_POLICY_URL) },
            )
            Route.ADD_ACCOUNT -> if (onboarding != null) OnboardingHost(
                environment = onboarding,
                onTrySampleMail = {},
                onFinished = { scope.launch { onAccountReady(); route = addReturn } },
                startAt = Step.ADD_ACCOUNT,
                onExit = { route = addReturn },
            )
            Route.HOME -> InboxScreen(
                session, onOpenSettings = { route = Route.SETTINGS },
                onAddAccount = if (onboarding != null) ({ addReturn = Route.HOME; route = Route.ADD_ACCOUNT }) else null,
                onManageAccounts = { route = Route.ACCOUNTS },
            )
            Route.SETTINGS -> SettingsHomeScreen(
                info = info,
                onOpenAppearance = { route = Route.APPEARANCE },
                onOpenSwipeActions = { route = Route.SWIPE_ACTIONS },
                onOpenInbox = { route = Route.INBOX_SETTINGS },
                onOpenGallery = if (DebugGallery.available) ({ route = Route.GALLERY }) else null,
                onOpenAccounts = { route = Route.ACCOUNTS },
                onOpenAbout = { route = Route.ABOUT },
                onBack = { route = Route.HOME },
            )
            Route.ACCOUNTS -> AccountsScreen(
                session = session, prefs = accountPrefs, onPrefsChange = onAccountPrefsChange,
                onAdd = { if (onboarding != null) { addReturn = Route.ACCOUNTS; route = Route.ADD_ACCOUNT } },
                onOpenAccount = { id -> accountId = id; route = Route.ACCOUNT },
                removeAccount = removeAccount, onNoAccounts = { route = Route.ONBOARDING },
                onBack = { route = Route.SETTINGS },
            )
            Route.ACCOUNT -> AccountScreen(
                session = session, accountId = accountId, prefs = accountPrefs, onPrefsChange = onAccountPrefsChange,
                removeAccount = removeAccount, onRemoved = { left -> route = if (left == 0) Route.ONBOARDING else Route.ACCOUNTS },
                eraseAll = eraseAll, onBack = { route = Route.ACCOUNTS },
            )
            Route.ABOUT -> AboutScreen(
                info = info, session = session, onOpenLicences = { route = Route.LICENCES }, eraseAll = eraseAll,
                onBack = { route = Route.SETTINGS },
            )
            Route.LICENCES -> LicencesScreen(onBack = { route = Route.ABOUT })
            Route.APPEARANCE -> AppearanceFlow(settings = settings, onChange = onSettingsChange, onBack = { route = Route.SETTINGS })
            Route.SWIPE_ACTIONS -> SwipeSettingsFlow(settings = swipeSettings, onChange = onSwipeSettingsChange, onBack = { route = Route.SETTINGS })
            Route.INBOX_SETTINGS -> InboxSettingsScreen(settings = mailSettings, onChange = onMailSettingsChange, onBack = { route = Route.SETTINGS })
            Route.GALLERY -> DebugGallery.Screen(onBack = { route = Route.SETTINGS })
        }
    }
}
