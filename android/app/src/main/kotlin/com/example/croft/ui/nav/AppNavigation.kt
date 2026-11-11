// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
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
import com.example.croft.ui.onboarding.OnboardingEnvironment
import com.example.croft.ui.onboarding.OnboardingHost
import kotlinx.coroutines.launch

/** The screens reachable today. S50 and its drill-down live inside [APPEARANCE] (see `AppearanceFlow`). */
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
    ;

    /** How deep the screen is; going to a larger depth is "forward". */
    val depth: Int get() = generateSequence(this) { it.parent }.count() - 1
}

/**
 * The one place "Try with sample mail" (S01) goes through. For now it opens the existing fake-data Inbox: while no account exists the
 * app's session is already the sample mailbox ([com.example.croft.mail.MailSession.usesFakeData]), so this just goes Home. A follow-up task
 * points it at the core's own sample account instead; only this function changes then.
 */
internal fun onTrySampleMail(goHome: () -> Unit) = goHome()

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
    /** Called when an account was added, before the Inbox shows: the app swaps the sample session for the one over the core. */
    onAccountReady: suspend () -> Unit = {},
) {
    var route by rememberSaveable { mutableStateOf(startAt) }
    if (route == Route.ONBOARDING && onboarding == null) route = Route.HOME
    val scope = rememberCoroutineScope()
    // A debug-only route can never be reached from a release build, even from saved state.
    if (route == Route.GALLERY && !DebugGallery.available) route = Route.SETTINGS
    // The sign-in flow has its own Back (one step back; on its first screen the system leaves the app).
    BackHandler(enabled = route != Route.HOME && route != Route.ONBOARDING) { route = route.parent ?: Route.HOME }
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
                onTrySampleMail = { onTrySampleMail { route = Route.HOME } },
                onFinished = { scope.launch { onAccountReady(); route = Route.HOME } },
            )
            Route.HOME -> InboxScreen(session, onOpenSettings = { route = Route.SETTINGS })
            Route.SETTINGS -> SettingsHomeScreen(
                info = info,
                onOpenAppearance = { route = Route.APPEARANCE },
                onOpenSwipeActions = { route = Route.SWIPE_ACTIONS },
                onOpenInbox = { route = Route.INBOX_SETTINGS },
                onOpenGallery = if (DebugGallery.available) ({ route = Route.GALLERY }) else null,
                onBack = { route = Route.HOME },
            )
            Route.APPEARANCE -> AppearanceFlow(settings = settings, onChange = onSettingsChange, onBack = { route = Route.SETTINGS })
            Route.SWIPE_ACTIONS -> SwipeSettingsFlow(settings = swipeSettings, onChange = onSwipeSettingsChange, onBack = { route = Route.SETTINGS })
            Route.INBOX_SETTINGS -> InboxSettingsScreen(settings = mailSettings, onChange = onMailSettingsChange, onBack = { route = Route.SETTINGS })
            Route.GALLERY -> DebugGallery.Screen(onBack = { route = Route.SETTINGS })
        }
    }
}
