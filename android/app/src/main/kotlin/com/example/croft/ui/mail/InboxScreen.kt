// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.PermanentDrawerSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.pulltorefresh.pullToRefresh
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MailSession
import com.example.croft.mail.MessageId
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.MessageScope
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeDirection
import com.example.croft.theme.ListStyle
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.MotionLevel
import com.example.croft.ui.components.BannerHost
import com.example.croft.ui.components.BannerKind
import com.example.croft.ui.components.CroftExtendedFab
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.DestructiveDialog
import com.example.croft.ui.components.EncryptionMark
import com.example.croft.ui.components.FabState
import com.example.croft.ui.components.MessageRow
import com.example.croft.ui.components.MessageRowState
import com.example.croft.ui.components.RowAction
import com.example.croft.ui.swipe.LocalSwipeSettings
import com.example.croft.ui.swipe.SwipeCallbacks
import com.example.croft.ui.swipe.SwipeRowHost
import com.example.croft.ui.swipe.swipeIcon
import com.example.croft.ui.swipe.swipeLabel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import java.text.NumberFormat
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

/**
 * The mail screens S10 to S19 in one place: the Inbox (S10) with its search pill, banners, category chips, list, extended FAB
 * and bottom bar; selection mode (S11); the navigation drawer (S12); the account switcher (S13); pull to refresh and the
 * "N new" pill (S17); the offline, sign-in and server banners (S18); the empty states (S19).
 *
 * It talks to the mail core only through [MailSession.repository] (the fake until T16). Back goes through the system's
 * back-callback API: the account switcher and the drawer close first, then selection mode, then the navigation leaves the
 * screen (the handlers are registered in that order of priority: the last registered is asked first).
 */
@OptIn(ExperimentalFoundationApi::class, androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
fun InboxScreen(
    session: MailSession,
    onOpenSettings: () -> Unit,
    modifier: Modifier = Modifier,
    startScope: MessageScope = MessageScope.UnifiedInbox,
    startDrawerOpen: Boolean = false,
    startSwitcherOpen: Boolean = false,
    onOpenMessage: (MessageId) -> Unit = {},
    onCompose: () -> Unit = {},
    /** "Add another account" of the switcher (S02); a note that it comes later shows when null (tests of the Inbox alone). */
    onAddAccount: (() -> Unit)? = null,
    /** "Manage accounts" of the switcher (S44); a note shows when null. */
    onManageAccounts: (() -> Unit)? = null,
) {
    val controller = rememberInboxController(session, startScope)
    controller.haptics = LocalHaptics.current
    controller.swipeSettings = LocalSwipeSettings.current
    controller.density = LocalDensity.current.density
    LaunchedEffect(controller) { controller.run() }

    val scope = rememberCoroutineScope()
    val drawerState = androidx.compose.material3.rememberDrawerState(if (startDrawerOpen) DrawerValue.Open else DrawerValue.Closed)
    var switcherOpen by rememberSaveable { mutableStateOf(startSwitcherOpen) }
    val laterMessage = stringResource(R.string.mail_snack_account_screens)

    // Back: selection first (registered first, so asked last), then the drawer, then the switcher.
    BackHandler(enabled = controller.isSelecting) { controller.clearSelection() }

    val closeDrawer: () -> Unit = { scope.launch { drawerState.close() } }
    val openScope: (MessageScope) -> Unit = { target -> closeDrawer(); controller.openScope(target) }
    val drawerContent: @Composable () -> Unit = {
        val drawerAccount = controller.drawerAccount
        MailDrawerContent(
            folders = controller.folders.filter { it.account == drawerAccount },
            scope = controller.messageScope,
            unifiedUnread = controller.unreadInAllInboxes,
            accountLabel = (controller.messageScope as? MessageScope.Folder)?.let { s -> controller.accounts.firstOrNull { it.id == s.account }?.label },
            onSelect = { f ->
                val target = if (f.role == FolderRole.INBOX && controller.messageScope == MessageScope.UnifiedInbox) MessageScope.UnifiedInbox else MessageScope.Folder(f.account, f.id)
                openScope(target)
            },
            onSettings = { closeDrawer(); onOpenSettings() },
            // The answer (with its Undo) is a snackbar on the list: the drawer closes so that its scrim does not cover the button.
            onMarkAllRead = { f ->
                closeDrawer()
                controller.markMailboxRead(f, unified = f.role == FolderRole.INBOX && controller.messageScope == MessageScope.UnifiedInbox)
            },
            onSyncNow = { f -> closeDrawer(); controller.syncMailbox(f) },
            onMailboxSettings = { closeDrawer(); controller.snackbar.show(laterMessage) },
        )
    }

    // The test tags are exposed as resource ids so that the macrobenchmark (UiAutomator) can find the list.
    BoxWithConstraints(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(InboxTags.SCREEN).semantics { testTagsAsResourceId = true }) {
        val expanded = maxWidth >= EXPANDED_FROM_DP.dp
        if (expanded) {
            Row(Modifier.fillMaxSize()) {
                PermanentDrawerSheet(Modifier.width(DRAWER_WIDTH)) { drawerContent() }
                InboxBody(controller, session, showMenu = false, showBottomBar = false, onMenu = {}, onAccount = { switcherOpen = true }, onOpenMessage = onOpenMessage, onCompose = onCompose, Modifier.weight(1f))
            }
        } else {
            ModalNavigationDrawer(
                drawerContent = { ModalDrawerSheet { drawerContent() } },
                drawerState = drawerState,
                gesturesEnabled = drawerState.isOpen || !controller.isSelecting,
            ) {
                InboxBody(controller, session, showMenu = true, showBottomBar = true, onMenu = { scope.launch { drawerState.open() } }, onAccount = { switcherOpen = true }, onOpenMessage = onOpenMessage, onCompose = onCompose, Modifier.fillMaxSize())
            }
            BackHandler(enabled = drawerState.isOpen) { closeDrawer() }
        }

        val locale = LocalConfiguration.current.locales[0]
        AccountSwitcher(
            visible = switcherOpen,
            accounts = controller.accounts,
            scope = controller.messageScope,
            unreadAll = controller.unreadInAllInboxes,
            formatSynced = { ms -> formatTime(session, ms, locale) },
            onSelectAll = { switcherOpen = false; controller.openScope(MessageScope.UnifiedInbox) },
            onSelectAccount = { a ->
                switcherOpen = false
                controller.inboxOf(a.id)?.let { controller.openScope(MessageScope.Folder(a.id, it)) }
            },
            onFixStatus = { a ->
                when (a.status) {
                    AccountState.ERROR -> controller.retry(a.id)
                    AccountState.NEEDS_SIGN_IN -> controller.requestSignIn(a.id)
                    else -> Unit
                }
            },
            onAddAccount = { if (onAddAccount != null) { switcherOpen = false; onAddAccount() } else controller.snackbar.show(laterMessage) },
            onManageAccounts = { if (onManageAccounts != null) { switcherOpen = false; onManageAccounts() } else controller.snackbar.show(laterMessage) },
            onClose = { switcherOpen = false },
        )
    }
}

private val DRAWER_WIDTH = 300.dp

private fun formatTime(session: MailSession, ms: Long, locale: java.util.Locale): String =
    DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale).format(java.time.Instant.ofEpochMilli(ms).atZone(session.zone))

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun InboxBody(
    controller: InboxController,
    session: MailSession,
    showMenu: Boolean,
    showBottomBar: Boolean,
    onMenu: () -> Unit,
    onAccount: () -> Unit,
    onOpenMessage: (MessageId) -> Unit,
    onCompose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val listState = controller.listState
    val settings = LocalThemeSettings.current
    val level = LocalMotionLevel.current
    val res = LocalContext.current.resources
    val locale = LocalConfiguration.current.locales[0]
    val haptics = LocalHaptics.current
    val selecting = controller.isSelecting
    val laterMessage = stringResource(R.string.mail_snack_account_screens)

    // The items: headers and rows, rebuilt when the rows change.
    val rows = controller.rows
    val nowMs = remember(rows) { session.clock() }
    val items = remember(rows) { buildListItems(rows, nowMs, session.zone) }
    val rowsUpTo = remember(items) {
        // rowsUpTo[i] = rows among items[0..i], to count the rows after the last visible one.
        var n = 0
        IntArray(items.size) { i -> if (items[i] is ListItem.Row) n++; n }
    }

    // ---- Effects: paging, new mail, place ----
    LaunchedEffect(controller, items) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
            .distinctUntilChanged()
            .collect { last ->
                if (rowsUpTo.isNotEmpty()) controller.loadMoreIfNear(rowsUpTo.last() - rowsUpTo[last.coerceIn(0, rowsUpTo.lastIndex)])
            }
    }
    LaunchedEffect(controller.scrollToTopTick) { if (controller.scrollToTopTick > 0) listState.scrollToItem(0) }
    LaunchedEffect(controller, listState) {
        snapshotFlow { controller.atTop }.distinctUntilChanged().filter { it }.collect { controller.clearNew() }
    }
    LaunchedEffect(controller, listState) {
        snapshotFlow { listState.isScrollInProgress }.distinctUntilChanged().filter { !it }.collect { controller.persistAnchor() }
    }
    DisposableEffect(controller) { onDispose { controller.persistAnchor() } }
    var shownScope by remember { mutableStateOf(controller.messageScope) }
    LaunchedEffect(controller.messageScope) {
        if (controller.messageScope != shownScope) { shownScope = controller.messageScope; listState.scrollToItem(0) }
    }
    val target = controller.restoreTarget
    LaunchedEffect(target, items) {
        if (target != null) {
            val row = controller.resolveAnchor(target)
            val index = row?.let { r -> items.indexOfFirst { it is ListItem.Row && it.row.id == r.id } } ?: -1
            if (index >= 0) {
                listState.scrollToItem(index)
                listState.scrollBy(-target.offsetDp * controller.density)
                controller.restored()
            } else if (controller.phase == LoadPhase.LOADED && rows.isEmpty()) controller.restored()
        }
    }

    // FAB: collapses on scroll down, grows on scroll up.
    var fabCollapsed by remember { mutableStateOf(false) }
    LaunchedEffect(listState) {
        var previous = listState.firstVisibleItemIndex * FAB_INDEX_WEIGHT + listState.firstVisibleItemScrollOffset
        snapshotFlow { listState.firstVisibleItemIndex * FAB_INDEX_WEIGHT + listState.firstVisibleItemScrollOffset }.collect { position ->
            if (position > previous) fabCollapsed = true else if (position < previous) fabCollapsed = false
            previous = position
        }
    }
    val fabState = when {
        selecting -> FabState.HIDDEN
        fabCollapsed && !controller.atTop -> FabState.COLLAPSED
        else -> FabState.EXPANDED
    }

    // Focus after a removal.
    val focusRequesters = remember(controller.messageScope) { HashMap<MessageId, FocusRequester>() }
    val focusTarget = controller.focusTarget
    LaunchedEffect(focusTarget, items) {
        if (focusTarget != null && rows.any { it.id == focusTarget }) {
            androidx.compose.runtime.withFrameNanos { }
            try { focusRequesters[focusTarget]?.requestFocus(); controller.focusTarget = null } catch (_: IllegalStateException) { }
        }
    }

    val role = controller.scopeRole
    val outgoing = role == FolderRole.SENT || role == FolderRole.DRAFTS || role == FolderRole.OUTBOX
    val avatarAccount = (controller.messageScope as? MessageScope.Folder)?.let { s -> controller.accounts.firstOrNull { it.id == s.account } } ?: controller.accounts.firstOrNull()
    // The avatar is read with its own account's status when that account needs attention ("Account Sam, Needs sign-in").
    val statusText = avatarAccount?.takeIf { it.status != AccountState.OK && it.status != AccountState.SYNCING }?.let { stringResource(statusWord(it.status.toChip())) }

    Column(modifier.windowInsetsPadding(WindowInsets.statusBars)) {
        Box(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
            if (selecting) {
                val tb = controller.toolbar
                SelectionBar(
                    count = controller.selectedCount,
                    role = role,
                    toolbar = tb,
                    onClose = controller::clearSelection,
                    onAction = controller::perform,
                    offerSelectAllInFolder = controller.offerSelectAllInFolder,
                    totalInFolder = controller.totalInScope,
                    allInFolderSelected = controller.allInFolderSelected,
                    onSelectAll = controller::selectAllLoaded,
                    onSelectAllInFolder = controller::selectAllInFolder,
                )
            } else {
                SearchPill(
                    accountLabel = avatarAccount?.label ?: "",
                    accountStatusText = statusText,
                    onMenu = onMenu,
                    onSearch = { controller.snackbar.show(laterMessage) },
                    onAccount = onAccount,
                    showMenu = showMenu,
                )
            }
        }
        val banner = controller.banner
        BannerHost(
            active = listOfNotNull(banner?.first),
            onAction = { kind ->
                when (kind) {
                    BannerKind.SERVER_PROBLEM -> banner?.second?.let { controller.retry(it.id) }
                    BannerKind.NEEDS_SIGN_IN -> banner?.second?.let { controller.requestSignIn(it.id) }
                    else -> Unit
                }
            },
            onDismiss = { controller.dismissBanner(it) },
            accountName = banner?.second?.label ?: "",
        )
        if (!selecting) CategoryChipsRow(Modifier.padding(bottom = 8.dp))

        // The list area: pull to refresh, the list or its skeleton or empty state, the pill, the FAB and the snackbar.
        val pullState = rememberPullToRefreshState()
        PullRefreshHaptics(pullState, controller.refreshing, haptics)
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .padding(horizontal = 12.dp)
                .pullToRefresh(isRefreshing = controller.refreshing, state = pullState, enabled = !selecting, threshold = PULL_ARM_DP, onRefresh = controller::refresh),
        ) {
            val emptyKind = controller.emptyKind
            when {
                controller.phase == LoadPhase.FIRST_LOAD -> SkeletonList()
                emptyKind != null -> EmptyState(
                    kind = emptyKind,
                    lastSynced = controller.lastSyncedMs?.let { formatTime(session, it, locale) },
                    onCompose = onCompose,
                    onSearchServer = { controller.snackbar.show(laterMessage) },
                )
                else -> {
                    val flat = settings.listStyle == ListStyle.FLAT
                    val listContainer = if (flat) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceContainerLow
                    Surface(Modifier.fillMaxSize(), shape = if (flat) androidx.compose.ui.graphics.RectangleShape else MaterialTheme.shapes.extraLarge, color = listContainer) {
                        LazyColumn(
                            state = listState,
                            modifier = Modifier.fillMaxSize().testTag(InboxTags.LIST).semantics { contentDescription = res.getString(R.string.mail_list_description) },
                            contentPadding = PaddingValues(bottom = LIST_BOTTOM_PADDING),
                        ) {
                            for (item in items) when (item) {
                                // The header takes no item animation: a sticky header and a placement animation fight over its position.
                                is ListItem.Header -> stickyHeader(key = item.key, contentType = "header") {
                                    DateHeader(item.bucket.label(res, nowMs, session.zone, locale), listContainer, Modifier.testTag(InboxTags.header(item.bucket.id)))
                                }
                                is ListItem.Row -> item(key = item.key, contentType = "row") {
                                    RowItem(
                                        row = item.row, controller = controller, nowMs = nowMs, zone = session.zone, locale = locale,
                                        role = role, outgoing = outgoing, selecting = selecting,
                                        focusRequester = focusRequesters.getOrPut(item.row.id) { FocusRequester() },
                                        onOpen = onOpenMessage,
                                        listScrolling = { listState.isScrollInProgress },
                                    )
                                }
                            }
                        }
                    }
                    FastScroller(listState, items, labelOf = { it.label(res, nowMs, session.zone, locale) }, haptics = haptics, modifier = Modifier.align(Alignment.CenterEnd), onDraggingChange = { controller.fastScrolling = it })
                }
            }
            NewMailPill(
                count = controller.newCount,
                onClick = {
                    controller.clearNew()
                    scope.launch { if (level == MotionLevel.FULL) listState.animateScrollToItem(0) else listState.scrollToItem(0) }
                },
                modifier = Modifier.align(Alignment.TopCenter).padding(top = 8.dp),
            )
            PullIndicator(pullState, controller.refreshing, Modifier.align(Alignment.TopCenter))
            Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth(), horizontalAlignment = Alignment.End) {
                Box(Modifier.padding(16.dp)) {
                    CroftExtendedFab(stringResource(R.string.mail_compose), onCompose, fabState, Modifier.testTag(InboxTags.FAB))
                }
                CroftSnackbarHost(controller.snackbar, Modifier.align(Alignment.CenterHorizontally))
            }
        }
        if (showBottomBar) {
            MailBottomBar(
                enabled = !selecting,
                onMail = {
                    if (!controller.atTop) scope.launch { if (level == MotionLevel.FULL) listState.animateScrollToItem(0) else listState.scrollToItem(0) }
                    else controller.refresh()
                },
            )
        } else {
            Box(Modifier.windowInsetsPadding(WindowInsets.navigationBars))
        }
    }

    controller.parkedRow?.let {
        // A swiped Delete forever: the card waits at 40 percent while this is open; Cancel springs it back.
        DestructiveDialog(
            title = pluralStringResource(R.plurals.mail_confirm_title_delete_forever, 1, 1),
            message = stringResource(R.string.mail_confirm_message_forever),
            confirmLabel = stringResource(R.string.mail_action_delete_forever),
            onConfirm = { controller.resolveParked(true) },
            onDismiss = { controller.resolveParked(false) },
        )
    }
    // S30 Move to (the toolbar's Move to, a swipe on Move to, the row's own action).
    controller.moveRequest?.let { request ->
        MoveToSheet(
            title = stringResource(R.string.move_title),
            folders = controller.moveFolders,
            recent = controller.moveRecents,
            excluding = controller.moveExcluding,
            onPick = controller::pickMove,
            onDismiss = controller::dismissMove,
        )
    }
    // The first Archive or Delete of an account with no such folder: which folder is yours? (remembered, asked once)
    controller.folderRequest?.let { request ->
        MoveToSheet(
            title = stringResource(if (request.role == FolderRole.TRASH) R.string.choose_trash_title else R.string.choose_archive_title),
            folders = controller.folders.filter { it.account == request.account },
            recent = emptyList(),
            excluding = emptySet(),
            onPick = controller::answerFolderRequest,
            onDismiss = controller::dismissFolderRequest,
            footer = if (request.role == FolderRole.ARCHIVE) ({ DeclineFolderButton(stringResource(R.string.choose_decline_archive), controller::declineFolderRequest) }) else null,
        )
    }
    controller.pendingConfirm?.let { pending ->
        val titleRes = when (pending.action) {
            SelectionAction.ARCHIVE -> R.plurals.mail_confirm_title_archive
            SelectionAction.DELETE_FOREVER -> R.plurals.mail_confirm_title_delete_forever
            else -> R.plurals.mail_confirm_title_delete
        }
        DestructiveDialog(
            title = pluralStringResource(titleRes, pending.count, pending.count),
            message = stringResource(if (pending.action == SelectionAction.DELETE_FOREVER) R.string.mail_confirm_message_forever else R.string.mail_confirm_message_undo),
            confirmLabel = pending.action.label(role),
            onConfirm = controller::confirm,
            onDismiss = controller::cancelConfirm,
        )
    }
}

private val LIST_BOTTOM_PADDING = 96.dp
private const val FAB_INDEX_WEIGHT = 100_000

@Composable
private fun statusWord(chip: com.example.croft.ui.components.AccountStatus): Int = when (chip) {
    com.example.croft.ui.components.AccountStatus.OFFLINE -> R.string.status_offline
    com.example.croft.ui.components.AccountStatus.NEEDS_SIGN_IN -> R.string.status_needs_sign_in
    com.example.croft.ui.components.AccountStatus.SERVER_PROBLEM -> R.string.status_server_problem
    else -> R.string.status_syncing
}

/**
 * One row of the list: the shared [MessageRow] fed from the message data, inside the swipe host (the card, the pill, the gesture and
 * the commit sequence) with its divider. The row's custom actions for screen readers and switch users are the configured swipe actions
 * (so they follow S47), then Select, Star and Reply: nobody needs a swipe, a long-press or a menu.
 */
@Composable
private fun LazyItemScope.RowItem(
    row: MessageRowData,
    controller: InboxController,
    nowMs: Long,
    zone: java.time.ZoneId,
    locale: java.util.Locale,
    role: FolderRole,
    outgoing: Boolean,
    selecting: Boolean,
    focusRequester: FocusRequester,
    onOpen: (MessageId) -> Unit,
    listScrolling: () -> Boolean,
) {
    val res = LocalContext.current.resources
    val shown = RowTimes.shown(row.dateMs, nowMs, zone, locale)
    val spoken = RowTimes.spoken(res, row.dateMs, nowMs, shown)
    val state = MessageRowState(
        unread = !row.seen,
        selected = row.id in controller.selected,
        starred = row.flagged,
        encryption = if (row.encrypted) EncryptionMark.ENCRYPTED else EncryptionMark.NONE,
        pendingOffline = row.pending,
        localOnly = row.localOnly,
        failed = row.id in controller.failedRows,
        isNew = row.id in controller.arrived,
        hasAttachment = row.hasAttachments,
        threadCount = row.threadCount,
    )
    val settings = controller.swipeSettings
    // The custom actions: every configured swipe action this row can do (Star is the row's own action already), then Reply.
    val configured = listOf(SwipeDirection.START_TO_END, SwipeDirection.END_TO_START)
        .map { settings.actionOf(it) }
        .filter { it != SwipeAction.NONE && it != SwipeAction.TOGGLE_STAR }
        .distinct()
        .filter { controller.canDo(row, it) }
    val extra = configured.map { a -> RowAction(swipeLabel(res, a, row, role)) { controller.act(row, a) } } +
        RowAction(res.getString(R.string.mail_action_reply)) { controller.reply(row) }
    val rowNow by rememberUpdatedState(row)
    val callbacks = remember(controller) {
        SwipeCallbacks(
            onKeep = { a -> controller.keepAction(rowNow, a) },
            onRemoved = { a, onFailed -> controller.removeAction(rowNow, a, onFailed) },
            onPark = { a -> controller.swipeParked(rowNow, a) },
            onNotAvailable = { controller.swipeNotAvailable() },
        )
    }
    Column(listItemMotion()) {
        SwipeRowHost(
            settings = settings,
            enabled = !selecting && !controller.refreshing && !controller.fastScrolling,
            alive = controller.isPresent(row.id),
            slotOf = { d -> controller.slotOf(row, d) },
            iconOf = { a -> swipeIcon(a, row) },
            listScrolling = listScrolling,
            callbacks = callbacks,
            parkResolution = controller.parkResolution?.takeIf { it.first == row.id }?.second,
            onParkResolutionHandled = { controller.parkHandled(row.id) },
            trailing = { HorizontalDivider(Modifier.padding(horizontal = 16.dp), color = MaterialTheme.colorScheme.outlineVariant) },
        ) { card ->
            MessageRow(
                sender = if (outgoing) row.recipients ?: row.displayName else row.displayName,
                subject = row.subject,
                preview = row.snippet,
                time = shown,
                state = state,
                onClick = { if (selecting) controller.toggle(row.id) else onOpen(row.id) },
                modifier = Modifier.testTag(InboxTags.row(row.id)),
                onLongClick = { controller.longPress(row.id) },
                onAvatarClick = { controller.toggle(row.id) },
                onToggleStar = { controller.toggleStar(row) },
                card = card,
                timeDescription = spoken,
                outgoing = outgoing,
                swipeEnabled = !selecting,
                extraActions = extra,
                focusRequester = focusRequester,
            )
        }
    }
}

