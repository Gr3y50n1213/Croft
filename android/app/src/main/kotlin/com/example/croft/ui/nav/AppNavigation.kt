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
import com.example.croft.mail.companion.CompanionRuntime
import com.example.croft.ui.companion.CompanionScreen
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.ui.mail.InboxSettingsScreen
import com.example.croft.ui.mail.InboxNotice
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.ui.compose.ComposeHost
import com.example.croft.ui.compose.OutboxHost
import com.example.croft.ui.reading.ConversationHost
import com.example.croft.ui.reading.ConversationNotice
import com.example.croft.ui.compose.rememberComposeRetained
import androidx.compose.runtime.LaunchedEffect
import com.example.croft.swipe.SwipeSettings
import com.example.croft.ui.swipe.SwipeSettingsFlow
import com.example.croft.ui.mail.InboxScreen
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.appearance.AppearanceFlow
import com.example.croft.ui.components.sharedAxisX
import com.example.croft.mail.accounts.AccountOps
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.REMOVED_COUNT_UNKNOWN
import com.example.croft.mail.accounts.SavedSignIn
import com.example.croft.mail.onboarding.Step
import com.example.croft.ui.accounts.AboutLinks
import com.example.croft.ui.accounts.AboutScreen
import com.example.croft.ui.accounts.AccountScreen
import com.example.croft.ui.accounts.AccountsScreen
import com.example.croft.ui.accounts.LicencesScreen
import com.example.croft.ui.accounts.openInBrowser
import com.example.croft.ui.onboarding.OnboardingEnvironment
import com.example.croft.ui.onboarding.OnboardingHost
import com.example.croft.notify.CheckAccount
import com.example.croft.notify.CheckSchedule
import com.example.croft.notify.NotifySettingsStore
import com.example.croft.ui.notify.NotificationSettingsHost
import com.example.croft.ui.notify.NotifyIntroHost
import androidx.compose.runtime.collectAsState
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A request from outside to open compose ([ComposeIntents]). Compared by identity, so the same link opened twice is two requests. */
class ComposeRequest(val start: ComposeStart)

/** The message a notification was posted for: its row id and its Inbox UID in that account. */
data class NotifiedMessage(val account: Long, val messageId: Long, val uid: UInt)

/**
 * A tap on a notification (S32): [message] with the Inbox under it (or compose with a reply to it when [reply]), or the Inbox when null.
 * Compared by identity.
 */
class OpenMessageRequest(val message: NotifiedMessage?, val reply: Boolean)

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

    /** S06 Connect a Companion, and S58 Companion and devices once connected (T34b). */
    COMPANION(SETTINGS),

    /** The open-source licences list, from S60. */
    LICENCES(ABOUT),

    /** S23 Compose (what it opens is [CroftNavHost]'s `composeStart`). Back, Send and Discard return to where it was opened. */
    COMPOSE(HOME),

    /** S26 Outbox, from the drawer. */
    OUTBOX(HOME),

    /** S20 Conversation and S21 Message (the message is [CroftNavHost]'s `conversationId`): a tap on a row of any folder. Back returns to the list. */
    CONVERSATION(HOME),

    /** S27 Search and S28 results (T19), from the Inbox's search pill. It handles its own Back (results first, then the Inbox). */
    SEARCH(HOME),

    /** S07 Notifications set-up (T20): once, after the first account. Back is Not now. Then the Inbox. */
    NOTIFY_INTRO(null),

    /** S46 Notifications (T20). */
    NOTIFICATIONS(SETTINGS),
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
    /** Drops an account's entries from the stored prefs and waits until that is written (the removal path waits for it). */
    removeAccountPrefs: suspend (Long) -> Unit = {},
    /** After everything was erased in the core and the secret store: the app clears its settings stores and shows S01. */
    onErased: suspend () -> Unit = {},
    /** A request from outside the app (a `mailto:` link, a share): compose opens on it once, as soon as the app is past sign-in. */
    composeRequest: ComposeRequest? = null,
    onComposeRequestHandled: () -> Unit = {},
    /** The notification settings (S07, S46). Without them neither screen is reachable (tests of other screens). */
    notifyStore: NotifySettingsStore? = null,
    /** A tap on a notification: opens once, as soon as the app is past sign-in. */
    openRequest: OpenMessageRequest? = null,
    onOpenRequestHandled: () -> Unit = {},
) {
    var route by rememberSaveable { mutableStateOf(startAt) }
    val notifySettings by (notifyStore?.settings ?: kotlinx.coroutines.flow.flowOf(null)).collectAsState(initial = null)
    if ((route == Route.NOTIFY_INTRO || route == Route.NOTIFICATIONS) && notifyStore == null) route = Route.HOME
    var accountId by rememberSaveable { mutableStateOf(0L) }
    // Compose (T18d). What it opens is kept in memory only: after process death the route falls back to the Inbox, and the draft that
    // was open is in the Drafts folder (it was autosaved).
    // The compose screen's start and controller are kept by the activity's retained holder, so a rotation keeps compose open with its content.
    val retained = rememberComposeRetained()
    var composeReturn by rememberSaveable { mutableStateOf(Route.HOME) }
    // S20: the message that was opened, and what its actions leave for the Inbox to show once ("Archived", with Undo).
    var conversationId by rememberSaveable { mutableStateOf(0L) }
    // Where Back from a conversation goes: the Inbox, or the search results it was opened from.
    var conversationReturn by rememberSaveable { mutableStateOf(Route.HOME) }
    var conversationNotice by remember { mutableStateOf<ConversationNotice?>(null) }
    var notice by remember { mutableStateOf<InboxNotice?>(null) }
    val openCompose: (ComposeStart) -> Unit = { start ->
        // Compose returns to where it was opened: the Outbox or the conversation (a reply), else the Inbox.
        composeReturn = if (route == Route.OUTBOX || route == Route.CONVERSATION) route else Route.HOME
        retained.begin(start)
        route = Route.COMPOSE
    }
    // After a process restart nothing of compose is kept in memory: the Inbox (the draft was saved by autosave).
    if (route == Route.COMPOSE && retained.start == null) route = Route.HOME
    // Where Add account (S02) returns to: S44, or the Inbox when it was started from the account switcher.
    var addReturn by rememberSaveable { mutableStateOf(Route.ACCOUNTS) }
    // "Sign in" on the Needs sign-in banner: the add-account flow for that account's address (kept in memory only: after a restart
    // the person taps the banner again).
    var signInAgainEmail by remember { mutableStateOf<String?>(null) }
    var sampleProblem by remember { mutableStateOf(false) }
    var sampleBusy by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val sessionNow by rememberUpdatedState(session)
    if (route == Route.ONBOARDING && onboarding == null) route = Route.HOME
    val scope = rememberCoroutineScope()
    // S27 and S28 (T19): one controller per session, kept while the person opens a result and comes back.
    val searchController = remember(session) {
        session.search?.let { backend ->
            com.example.croft.mail.search.SearchController(
                backend,
                scope,
                com.example.croft.mail.search.StoredSearchHistory(java.io.File(context.noBackupFilesDir, com.example.croft.mail.search.StoredSearchHistory.FILE_NAME)),
            )
        }
    }
    // After a process restart the search controller is new: start from the Inbox.
    if (route == Route.SEARCH && searchController == null) route = Route.HOME
    // A debug-only route can never be reached from a release build, even from saved state.
    if (route == Route.GALLERY && !DebugGallery.available) route = Route.SETTINGS
    LaunchedEffect(composeRequest, route) {
        if (composeRequest != null && route != Route.ONBOARDING && route != Route.ADD_ACCOUNT) {
            if (route == Route.COMPOSE && retained.start != null) {
                // A link or a share while compose is open: what is being written is saved first (a "Draft saved" shows), then the new one opens.
                retained.replaceWith(composeRequest.start)
            } else {
                composeReturn = Route.HOME
                retained.begin(composeRequest.start)
                route = Route.COMPOSE
            }
            onComposeRequestHandled()
        }
    }
    // The banner's "Sign in" (any screen that raises it): S04 again for that account, then back where it was.
    if (onboarding != null) LaunchedEffect(session) {
        session.events.collect { event ->
            if (event is com.example.croft.mail.MailEvent.SignInRequested) {
                val email = runCatching { session.repository.listAccounts() }.getOrDefault(emptyList()).firstOrNull { it.id == event.account && !it.isDemo }?.email
                if (email != null && route != Route.ONBOARDING && route != Route.ADD_ACCOUNT) {
                    signInAgainEmail = email
                    addReturn = if (route == Route.ACCOUNT || route == Route.ACCOUNTS) route else Route.HOME
                    route = Route.ADD_ACCOUNT
                }
            }
        }
    }
    // A tap on a notification: the message over the Inbox (Back returns to the Inbox), or the Inbox.
    LaunchedEffect(openRequest, route) {
        val open = openRequest ?: return@LaunchedEffect
        if (route == Route.ONBOARDING || route == Route.ADD_ACCOUNT || route == Route.COMPOSE) return@LaunchedEffect
        val m = open.message
        // The row must still be the message the notification was posted for (a deleted message's row id can be reused): else the Inbox.
        val same = m != null && withContext(kotlinx.coroutines.Dispatchers.IO) {
            runCatching { com.example.croft.mail.MailRuntime.core?.core?.notifiedMessageIs(m.account, m.messageId, m.uid) == true }.getOrDefault(false)
        }
        onOpenRequestHandled()
        when {
            m == null || !same -> route = Route.HOME
            open.reply -> openCompose(ComposeStart.Reply(m.account, m.messageId))
            else -> { conversationId = m.messageId; conversationReturn = Route.HOME; route = Route.CONVERSATION }
        }
    }
    // After an account was added: S07 once (not for the sample mailbox, which never notifies), else where the flow returns to.
    val afterAccountAdded: suspend (Route) -> Unit = { next ->
        val intro = notifyStore != null && !(runCatching { notifyStore.current().introDone }.getOrDefault(true))
        route = if (intro) Route.NOTIFY_INTRO else next
        // A real account exists now: the periodic check runs (it is kept as it is if it already did).
        if (notifyStore != null) runCatching { CheckSchedule.forAccounts(context, notifyStore.current(), hasRealAccount(sessionNow)) }
    }
    // The sign-in flow has its own Back (one step back; on its first screen the system leaves the app).
    // Compose and the Outbox handle their own Back (compose saves first).
    BackHandler(enabled = route != Route.HOME && route != Route.ONBOARDING && route != Route.ADD_ACCOUNT && route != Route.COMPOSE && route != Route.OUTBOX && route != Route.CONVERSATION && route != Route.SEARCH && route != Route.NOTIFY_INTRO) { route = route.parent ?: Route.HOME }

    // Removing an account: the account in the core first (its task stops, nothing new is sent), then its saved secret (`AccountOps.remove`),
    // then the app's own entries for it (folder choices, order, name, colour: the prefs are written and waited for), then a fresh session.
    // It runs to its end even if the screen goes away (a half removal would be worse than a late one). Returns how many accounts are
    // left ([REMOVED_COUNT_UNKNOWN] when that could not be read), or null when the removal itself failed. Only `ops.remove` failing means
    // "couldn't remove": after it succeeded the account is gone, and a failure of the housekeeping below is not reported as one (the
    // start-up sweep finishes it).
    val removeAccount: suspend (Long) -> Int? = remove@{ id ->
        val ops = accountOps ?: return@remove null
        withContext(NonCancellable) {
            try {
                ops.remove(id)
            } catch (e: Exception) {
                return@withContext null
            }
            try { sessionNow.folderMapping.forgetAccount(id) } catch (e: Exception) { /* swept at the next start */ }
            try { sessionNow.anchors.forgetAccount(id) } catch (e: Exception) { /* swept at the next start */ }
            try { removeAccountPrefs(id) } catch (e: Exception) { /* swept at the next start */ }
            // T20: its notification switches, its notifications and its channel.
            try { notifyStore?.update { it.forget(id) } } catch (e: Exception) { /* harmless: an id that is never used again */ }
            try { com.example.croft.notify.AndroidNotifier(context).forgetAccount(id) } catch (e: Exception) { /* nothing showing */ }
            try { onAccountReady() } catch (e: Exception) { /* the next screen reads the core again */ }
            // No real account left: the periodic check stops.
            if (notifyStore != null) try { CheckSchedule.forAccounts(context, notifyStore.current(), hasRealAccount(sessionNow)) } catch (e: Exception) { /* the next start */ }
            try { sessionNow.repository.listAccounts().size } catch (e: Exception) { REMOVED_COUNT_UNKNOWN }
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
                // The erase stopped part way (nothing is claimed): the core may have been closed, so the app builds its session again (a new
                // core on the same files) and the person sees the failure and can try again.
                try { onAccountReady() } catch (ignored: Exception) { /* the screens read the core again when it is back */ }
                false
            }
        }
    }
    val savedSignIn: suspend (Long) -> SavedSignIn = { id -> accountOps?.savedSignIn(id) ?: SavedSignIn.NONE }
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
                onFinished = { scope.launch { onAccountReady(); afterAccountAdded(Route.HOME) } },
                sampleProblem = sampleProblem,
                onPrivacyPolicy = { context.openInBrowser(AboutLinks.PRIVACY_POLICY_URL) },
            )
            Route.ADD_ACCOUNT -> if (onboarding != null) OnboardingHost(
                environment = onboarding,
                onTrySampleMail = {},
                onFinished = { scope.launch { signInAgainEmail = null; onAccountReady(); afterAccountAdded(addReturn) } },
                startAt = Step.ADD_ACCOUNT,
                onExit = { signInAgainEmail = null; route = addReturn },
                signInAgainEmail = signInAgainEmail,
            )
            Route.HOME -> InboxScreen(
                session, onOpenSettings = { route = Route.SETTINGS },
                onAddAccount = if (onboarding != null) ({ addReturn = Route.HOME; route = Route.ADD_ACCOUNT }) else null,
                onManageAccounts = { route = Route.ACCOUNTS },
                onCompose = { openCompose(ComposeStart.New()) },
                onReply = { account, message -> openCompose(ComposeStart.Reply(account, message)) },
                notice = notice,
                onNoticeShown = { notice = null },
                onUndoSend = { openCompose(it) },
                onOpenOutbox = if (session.compose != null) ({ route = Route.OUTBOX }) else null,
                onOpenDraft = { account, draft -> openCompose(ComposeStart.Draft(account, draft)) },
                onOpenMessage = { id -> conversationId = id; conversationReturn = Route.HOME; route = Route.CONVERSATION },
                onOpenSearch = if (searchController != null) ({ route = Route.SEARCH }) else null,
                conversationNotice = conversationNotice,
                onConversationNoticeShown = { conversationNotice = null },
            )
            Route.SEARCH -> if (searchController != null) {
                var searchAccounts by remember { mutableStateOf<List<com.example.croft.mail.AccountInfo>>(emptyList()) }
                LaunchedEffect(session) { searchAccounts = runCatching { session.repository.listAccounts() }.getOrDefault(emptyList()) }
                com.example.croft.ui.search.SearchScreen(
                    controller = searchController,
                    accounts = searchAccounts,
                    onOpenMessage = { id -> conversationId = id; conversationReturn = Route.SEARCH; route = Route.CONVERSATION },
                    onBack = { searchController.clear(); route = Route.HOME },
                    zone = session.zone,
                )
            }
            Route.CONVERSATION -> ConversationHost(
                session = session,
                messageId = conversationId,
                settings = mailSettings,
                onBack = { route = conversationReturn },
                onClosed = { notice -> if (conversationReturn == Route.HOME) conversationNotice = notice; route = conversationReturn },
                onCompose = { openCompose(it) },
            )
            Route.COMPOSE -> retained.start?.let { start ->
                ComposeHost(
                    session = session,
                    start = start,
                    onClosed = { closed ->
                        when (closed) {
                            // Nothing was kept, or it was thrown away: back to where compose was opened from.
                            ComposeClosed.Nothing, ComposeClosed.Discarded -> route = composeReturn
                            // A kept draft or a send: the Inbox says so ("Draft saved", "Sending..." with Undo).
                            else -> { notice = InboxNotice(closed); route = Route.HOME }
                        }
                    },
                )
            }
            Route.OUTBOX -> OutboxHost(session, onBack = { route = Route.HOME }, onEdit = { openCompose(it) })
            Route.SETTINGS -> SettingsHomeScreen(
                info = info,
                onOpenAppearance = { route = Route.APPEARANCE },
                onOpenSwipeActions = { route = Route.SWIPE_ACTIONS },
                onOpenInbox = { route = Route.INBOX_SETTINGS },
                onOpenGallery = if (DebugGallery.available) ({ route = Route.GALLERY }) else null,
                onOpenAccounts = { route = Route.ACCOUNTS },
                onOpenAbout = { route = Route.ABOUT },
                onOpenNotifications = if (notifyStore != null) ({ route = Route.NOTIFICATIONS }) else null,
                onOpenCompanion = if (CompanionRuntime.connection(context) != null) ({ route = Route.COMPANION }) else null,
                onBack = { route = Route.HOME },
            )
            Route.COMPANION -> CompanionRuntime.connection(context)?.let { c ->
                CompanionScreen(connection = c, onBack = { route = Route.SETTINGS })
            }
            Route.ACCOUNTS -> AccountsScreen(
                session = session, prefs = accountPrefs, onPrefsChange = onAccountPrefsChange,
                onAdd = { if (onboarding != null) { addReturn = Route.ACCOUNTS; route = Route.ADD_ACCOUNT } },
                onOpenAccount = { id -> accountId = id; route = Route.ACCOUNT },
                removeAccount = removeAccount, savedSignIn = savedSignIn, onNoAccounts = { route = Route.ONBOARDING },
                onBack = { route = Route.SETTINGS },
            )
            Route.ACCOUNT -> AccountScreen(
                session = session, accountId = accountId, prefs = accountPrefs, onPrefsChange = onAccountPrefsChange,
                removeAccount = removeAccount, savedSignIn = savedSignIn, onRemoved = { left -> route = if (left == 0) Route.ONBOARDING else Route.ACCOUNTS },
                eraseAll = eraseAll, onBack = { route = Route.ACCOUNTS },
            )
            Route.ABOUT -> AboutScreen(
                info = info, session = session, onOpenLicences = { route = Route.LICENCES }, eraseAll = eraseAll,
                onBack = { route = Route.SETTINGS },
            )
            Route.LICENCES -> LicencesScreen(onBack = { route = Route.ABOUT })
            Route.APPEARANCE -> AppearanceFlow(settings = settings, onChange = onSettingsChange, onBack = { route = Route.SETTINGS })
            Route.SWIPE_ACTIONS -> SwipeSettingsFlow(settings = swipeSettings, onChange = onSwipeSettingsChange, onBack = { route = Route.SETTINGS })
            Route.INBOX_SETTINGS -> InboxSettingsScreen(settings = mailSettings, onChange = onMailSettingsChange, onBack = { route = Route.SETTINGS }, session = session)
            Route.GALLERY -> DebugGallery.Screen(onBack = { route = Route.SETTINGS })
            Route.NOTIFY_INTRO -> {
                val ns = notifySettings
                if (notifyStore != null && ns != null) NotifyIntroHost(
                    settings = ns,
                    onChange = { next -> notifyStore.set(next); CheckSchedule.apply(context, next, changed = true) },
                    accounts = { notifyAccounts(sessionNow) },
                    onFinished = { route = Route.HOME },
                )
            }
            Route.NOTIFICATIONS -> {
                val ns = notifySettings
                if (notifyStore != null && ns != null) NotificationSettingsHost(
                    settings = ns,
                    onChange = { next ->
                        scope.launch {
                            notifyStore.set(next)
                            if (next.frequency != ns.frequency || next.batteryNotLow != ns.batteryNotLow) CheckSchedule.apply(context, next, changed = true)
                        }
                    },
                    accounts = { notifyAccounts(sessionNow) },
                    onBack = { route = Route.SETTINGS },
                )
            }
        }
    }
}

/** The accounts S07 and S46 list, from the session (the sample mailbox is marked and never notifies). */
private suspend fun notifyAccounts(session: MailSession): List<CheckAccount> =
    session.repository.listAccounts().map { CheckAccount(it.id, it.label.ifBlank { it.email }, isDemo = it.isDemo) }

/** Whether the session has an account that can notify (the sample mailbox never does). */
private suspend fun hasRealAccount(session: MailSession): Boolean =
    runCatching { session.repository.listAccounts().any { !it.isDemo } }.getOrDefault(false)
