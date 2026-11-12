// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import android.content.res.Resources
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.ChangeBatch
import com.example.croft.mail.FolderId
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.FolderRole
import com.example.croft.mail.ListAnchor
import com.example.croft.mail.MailError
import com.example.croft.mail.MailEvent
import com.example.croft.mail.MailException
import com.example.croft.mail.MailSession
import com.example.croft.mail.OpFailure
import com.example.croft.mail.MessageId
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.MessageScope
import com.example.croft.mail.OpId
import com.example.croft.mail.RowCursor
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipeSettings
import com.example.croft.swipe.SwipeSlot
import com.example.croft.theme.CroftTimings
import com.example.croft.ui.swipe.SwipeRules
import com.example.croft.ui.components.BannerKind
import com.example.croft.ui.components.CroftSnackbarState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Whether the folder has been read yet. [FIRST_LOAD] is the only time skeleton rows show. */
enum class LoadPhase { FIRST_LOAD, LOADED }

/** The copy of S19, by what is empty. */
sealed interface EmptyKind {
    /** The Inbox is empty: "You're all caught up", last synced, Compose. */
    data object InboxZero : EmptyKind

    /** Trash is empty. */
    data object Trash : EmptyKind

    /** Spam is empty. */
    data object Spam : EmptyKind

    /** Any other empty folder. */
    data object Folder : EmptyKind

    /** A search found nothing; [serverToo] is true when the server was searched as well as the device. */
    data class Search(val query: String, val serverToo: Boolean) : EmptyKind
}

/** A bulk action waiting for the person to confirm it. */
data class PendingConfirm(val action: SelectionAction, val count: Int)

/**
 * The Move to sheet (S30) is open for [rows], all in [account]. [fromSwipe]: the card of the single row is parked at 40 percent behind
 * the sheet (a swipe on Move to) and goes or returns with the answer.
 */
data class MoveRequest(val rows: List<MessageRowData>, val account: AccountId, val fromSwipe: Boolean)

/**
 * The first use of an action for an account that has no folder for it (docs/ux/message-list.md, "Folder rules"): the sheet asks
 * which folder to use as [role] (Archive or Trash), once; the answer is remembered ([com.example.croft.mail.FolderMapping]) and [retry]
 * runs the action again.
 */
data class FolderRequest(val role: FolderRole, val account: AccountId, val retry: suspend () -> Unit)

/** Strings the controller needs for snackbars; built from the resources so the controller itself holds no Context. */
class InboxTexts(private val res: Resources) {
    fun archived(n: Int): String = res.getQuantityString(R.plurals.mail_snack_archived, n, n)
    fun deleted(n: Int): String = res.getQuantityString(R.plurals.mail_snack_deleted, n, n)
    fun deletedForever(n: Int): String = res.getQuantityString(R.plurals.mail_snack_deleted_forever, n, n)
    fun moved(n: Int): String = res.getQuantityString(R.plurals.mail_snack_moved, n, n)
    val undo: String get() = res.getString(R.string.mail_undo)
    val noArchive: String get() = res.getString(R.string.mail_snack_no_archive)
    val noTrash: String get() = res.getString(R.string.mail_snack_no_trash)
    val failed: String get() = res.getString(R.string.mail_snack_failed)
    val changeFailed: String get() = res.getString(R.string.mail_snack_change_failed)
    val retry: String get() = res.getString(R.string.mail_retry)
    val refreshFailed: String get() = res.getString(R.string.mail_snack_refresh_failed)
    fun spammed(n: Int): String = res.getQuantityString(R.plurals.mail_snack_spam, n, n)
    fun markedRead(n: Int): String = res.getQuantityString(R.plurals.mail_snack_marked_read, n, n)
    val noSpam: String get() = res.getString(R.string.mail_snack_no_spam)
    val notAvailable: String get() = res.getString(R.string.swipe_not_available)
    val nothingUnread: String get() = res.getString(R.string.mail_snack_nothing_unread)
    val synced: String get() = res.getString(R.string.mail_snack_synced)
    val replyLater: String get() = res.getString(R.string.mail_snack_reply_later)
    val accountScreensLater: String get() = res.getString(R.string.mail_snack_account_screens)
    val draftSaved: String get() = res.getString(R.string.compose_snack_draft_saved)
    val sending: String get() = res.getString(R.string.compose_snack_sending)
    val tooLateToUndo: String get() = res.getString(R.string.compose_snack_too_late)
    val undoFailed: String get() = res.getString(R.string.compose_snack_undo_failed)
}

/**
 * The state and the logic of the message list (S10 to S19), apart from drawing it. It reads and acts through a
 * [com.example.croft.mail.MessageRepository] only; nothing here knows whether that is the fake or the core.
 *
 * - **Paging**: 50 rows first ([FIRST_PAGE]), then [PAGE] at a time when the list is [PREFETCH_DISTANCE] rows from the end
 *   ([loadMoreIfNear]). Skeleton rows only while [phase] is [LoadPhase.FIRST_LOAD]; a refresh keeps the rows.
 * - **Changes**: the repository's change notices re-run the head of the list. Rows that arrive above the person's place are
 *   counted in [newCount] (the pill) instead of moving the list; at the very top they are marked in [arrived] and the list
 *   shows them ([scrollToTopTick]).
 * - **Selection**: [selected], select all ([selectAllLoaded], [selectAllInFolder]), the toolbar ([toolbar]) and the bulk actions
 *   ([perform]) with their Undo snackbar and confirmations.
 * - **Place**: [currentAnchor] (a message id and a dp offset) is what is saved, never a pixel position.
 *
 * It lives with the composition ([rememberInboxController]); the persisted anchors and the per-folder cache live in the
 * [MailSession].
 */
@Stable
class InboxController(
    private val session: MailSession,
    private val scope: CoroutineScope,
    startScope: MessageScope,
    private val texts: InboxTexts,
    restoredSelection: Set<MessageId> = emptySet(),
    /** True when the state was restored (rotation, process death): [restoredAnchor] is then the whole truth, even when null (top). */
    restored: Boolean = false,
    restoredAnchor: ListAnchor? = null,
) {
    private val repository get() = session.repository

    /** The list's scroll state. The controller owns it so the place can be saved and restored by message id. */
    val listState = LazyListState()

    /** The display density, set by the screen, to turn dp into pixels and back. */
    var density: Float = 1f

    /** Every haptic goes through this (set by the screen). */
    var haptics: Haptics = NoHaptics

    /** What a swipe does (S47), set by the screen from the stored settings. */
    var swipeSettings: SwipeSettings = SwipeSettings.Default

    /** The fast scroller's thumb is being dragged: swipes are off meanwhile. */
    var fastScrolling by mutableStateOf(false)

    /** The one snackbar of the screen. */
    val snackbar = CroftSnackbarState()

    /**
     * What Reply does on a row. Set by the screen when compose is wired in (T18d): it opens compose as a reply. Without it (tests of the
     * list alone) the row says that Reply comes later.
     */
    var replyHandler: ((MessageRowData) -> Unit)? = null

    /** The drafts of this device shown at the top of a Drafts folder (the core's local drafts, those not in the Outbox); empty elsewhere. */
    var localDrafts by mutableStateOf<List<uniffi.croft_ffi.DraftSummary>>(emptyList())
        private set

    /** The rows the list shows: [rows] without the server's copy of a draft of this device (see [LocalDraftMatch]). What is selected is chosen from these. */
    val shownRows: List<MessageRowData> get() = com.example.croft.mail.compose.LocalDraftMatch.withoutCopies(rows, localDrafts)

    /** After Send, focus goes to the first row of the list (docs/ux/accessibility.md): set until the rows are there and focus was asked for. */
    var focusFirstPending by mutableStateOf(false)

    var messageScope by mutableStateOf(startScope)
        private set

    /** The rows loaded so far, newest first. */
    var rows by mutableStateOf<List<MessageRowData>>(emptyList())
        private set

    /** The first load of this folder is not done yet (skeleton rows). */
    var phase by mutableStateOf(LoadPhase.FIRST_LOAD)
        private set

    /** Pull to refresh or Retry is running (the rows stay). */
    var refreshing by mutableStateOf(false)
        private set

    /** Every account with its status. */
    var accounts by mutableStateOf<List<AccountInfo>>(emptyList())
        private set

    /** Folders of every account, with counters. */
    var folders by mutableStateOf<List<FolderInfo>>(emptyList())
        private set

    /** New rows above the person's place that the pill counts. */
    var newCount by mutableIntStateOf(0)
        private set

    /** Raised when new rows arrived while the list was at the very top: the screen shows them (they slide in). */
    var scrollToTopTick by mutableIntStateOf(0)
        private set

    /** Ids of rows that just arrived at the top and carry the 1.2 s tint. */
    var arrived by mutableStateOf<Set<MessageId>>(emptySet())
        private set

    /** The selected messages; selection mode is on while this is not empty. */
    var selected by mutableStateOf(restoredSelection)
        private set

    /** "Select all N in this folder" was chosen: every message of the folder is selected, not only the loaded ones. */
    var allInFolderSelected by mutableStateOf(false)
        private set

    /** A bulk action that asks first. */
    var pendingConfirm by mutableStateOf<PendingConfirm?>(null)
        private set

    /** Banners the person dismissed (until the problem goes away). */
    private var dismissed by mutableStateOf<Set<BannerKind>>(emptySet())

    /** Messages whose change the server refused for good: back in the list with the red edge, until Retry or another action on them. */
    var failedRows by mutableStateOf<Set<MessageId>>(emptySet())
        private set

    /** The Move to sheet (S30), while it is open. */
    var moveRequest by mutableStateOf<MoveRequest?>(null)
        private set

    /** The "which folder is your Archive (or Trash)" question, while it is open. */
    var folderRequest by mutableStateOf<FolderRequest?>(null)
        private set

    /** The folder each parked Move to swipe was given, for the row to take when its card has left. */
    private val chosenMoves = HashMap<MessageId, FolderId>()

    /** The row that should take focus after a removal. */
    var focusTarget by mutableStateOf<MessageId?>(null)

    /** Where the person left this folder, still to be restored (null once restored or when there was nothing). */
    var restoreTarget by mutableStateOf<ListAnchor?>(null)
        private set

    /** True from the start until the saved place has been restored (or found to be unneeded): the place is not saved meanwhile. */
    var restoring by mutableStateOf(true)
        private set

    private var next: RowCursor? = null
    private var generation = 0
    private val loadLock = Mutex()
    private var loadingMore = false
    private val removedByMe = HashSet<MessageId>()
    private var pendingAnchor: ListAnchor? = if (restored) restoredAnchor else session.anchors.load(startScope.key)

    // ---- Derived state ----

    /** True while at least one message is selected. */
    val isSelecting: Boolean get() = selected.isNotEmpty()

    /** The accounts the list shows: one, or all in the unified Inbox. */
    val scopeAccounts: List<AccountInfo>
        get() = when (val s = messageScope) {
            MessageScope.UnifiedInbox -> accounts
            is MessageScope.Folder -> accounts.filter { it.id == s.account }
        }

    /** What the folder shown is for. */
    val scopeRole: FolderRole
        get() = when (val s = messageScope) {
            MessageScope.UnifiedInbox -> FolderRole.INBOX
            is MessageScope.Folder -> folders.firstOrNull { it.id == s.folderId }?.role ?: FolderRole.OTHER
        }

    /** All messages in the folder shown, from the folder counters. */
    val totalInScope: Int
        get() = when (val s = messageScope) {
            MessageScope.UnifiedInbox -> folders.filter { it.role == FolderRole.INBOX }.sumOf { it.totalCount }
            is MessageScope.Folder -> folders.firstOrNull { it.id == s.folderId }?.totalCount ?: rows.size
        }

    /** How many are selected, counting the whole folder after "Select all N". */
    val selectedCount: Int get() = if (allInFolderSelected) totalInScope else selected.size

    /** True when more messages exist than are loaded and "Select all N" should be offered. */
    val offerSelectAllInFolder: Boolean get() = isSelecting && !allInFolderSelected && next != null && totalInScope > rows.size

    /** The selection bar's buttons for the selected rows. */
    val toolbar: SelectionToolbar
        get() {
            val chosen = rows.filter { it.id in selected }
            val accountsInvolved = chosen.map { it.account }.distinct()
            val archiveOk = accountsInvolved.all { a -> hasFolder(a, FolderRole.ARCHIVE) }
            val spamOk = accountsInvolved.all { a -> folders.any { it.account == a && it.role == FolderRole.SPAM } }
            // Move to needs one account: its sheet lists that account's folders.
            return SelectionRules.toolbar(chosen, scopeRole, archiveOk, spamOk, moveAvailable = accountsInvolved.size == 1)
        }

    /** The banner now shown (one at a time, the most urgent) and the account it is about. */
    val banner: Pair<BannerKind, AccountInfo?>?
        get() {
            val accs = scopeAccounts
            val active = LinkedHashMap<BannerKind, AccountInfo?>()
            accs.firstOrNull { it.status == AccountState.NEEDS_SIGN_IN }?.let { active[BannerKind.NEEDS_SIGN_IN] = it }
            accs.firstOrNull { it.status == AccountState.ERROR }?.let { active[BannerKind.SERVER_PROBLEM] = it }
            accs.firstOrNull { it.status == AccountState.OFFLINE }?.let { active[BannerKind.OFFLINE] = it }
            val shown = com.example.croft.ui.components.mostUrgentBanner(active.keys.filter { it !in dismissed }) ?: return null
            return shown to active[shown]
        }

    /** Why the list is empty, or null when it is not. */
    val emptyKind: EmptyKind?
        get() = if (phase != LoadPhase.LOADED || rows.isNotEmpty() || localDrafts.isNotEmpty()) null else when (scopeRole) {
            FolderRole.INBOX -> EmptyKind.InboxZero
            FolderRole.TRASH -> EmptyKind.Trash
            FolderRole.SPAM -> EmptyKind.Spam
            else -> EmptyKind.Folder
        }

    /** When the accounts of the list were last synced (the newest of them), or null. */
    val lastSyncedMs: Long? get() = scopeAccounts.mapNotNull { it.lastSyncedMs }.maxOrNull()

    /** True when the list is scrolled to its very top. */
    val atTop: Boolean get() = listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset == 0

    /** Whether more rows can be loaded. */
    val hasMore: Boolean get() = next != null

    // ---- Running ----

    /** Loads the folder, then follows the repository's changes until cancelled. Call from a `LaunchedEffect`. */
    suspend fun run() = coroutineScope {
        // The folders first: the empty state and the toolbar need to know what the folder is.
        refreshAccountsAndFolders()
        reloadLocalDrafts()
        launch { loadFirst() }
        launch { observeChanges() }
    }

    private suspend fun refreshAccountsAndFolders() {
        val accs = repository.listAccounts()
        val all = accs.flatMap { repository.listFolders(it.id) }
        accounts = accs
        folders = all
    }

    private suspend fun observeChanges() {
        var after = 0L
        while (true) {
            val batch = repository.nextChange(after)
            after = batch.seq
            handle(batch)
        }
    }

    private suspend fun handle(batch: ChangeBatch) {
        refreshAccountsAndFolders()
        reloadLocalDrafts()
        dismissed = dismissed.filterTo(HashSet()) { kind -> activeKinds().contains(kind) }
        if (batch.failures.isNotEmpty()) onFailures(batch.failures)
        if (batch.everything || affects(batch)) reloadHead()
    }

    /**
     * The server refused changes for good (or the core could not find the message again): the core has put the rows back, so they
     * get the red edge, the Rejected haptic plays, and the snackbar says so with Retry, which runs the same actions again.
     */
    private fun onFailures(failures: List<OpFailure>) {
        haptics.perform(HapticEvent.Rejected)
        failedRows = failedRows + failures.flatMap { it.messages }
        val ops = failures.map { it.opId }.distinct()
        snackbar.show(texts.changeFailed, texts.retry, stayMs = session.undoWindowMs) { retryFailed(ops) }
    }

    private fun retryFailed(ops: List<OpId>) {
        failedRows = emptySet()
        scope.launch {
            try {
                ops.forEach { repository.retryFailed(it) }
            } catch (e: MailException) {
                if (!askForFolder(e) { retryFailed(ops) }) {
                    haptics.perform(HapticEvent.Rejected)
                    snackbar.show(errorText(e.error))
                }
            }
        }
    }

    /** The person pressed Sign in on the banner: the app is told (the sign-in screen, T17, listens); without a listener the old note shows. */
    fun requestSignIn(account: AccountId) {
        session.raise(MailEvent.SignInRequested(account))
        if (!session.hasEventListener) snackbar.show(texts.accountScreensLater)
    }

    private fun activeKinds(): Set<BannerKind> = buildSet {
        val accs = scopeAccounts
        if (accs.any { it.status == AccountState.NEEDS_SIGN_IN }) add(BannerKind.NEEDS_SIGN_IN)
        if (accs.any { it.status == AccountState.ERROR }) add(BannerKind.SERVER_PROBLEM)
        if (accs.any { it.status == AccountState.OFFLINE }) add(BannerKind.OFFLINE)
    }

    private fun affects(batch: ChangeBatch): Boolean {
        val s = messageScope
        return when (s) {
            MessageScope.UnifiedInbox -> batch.folders.any { ref -> folders.any { it.id == ref.folderId && it.role == FolderRole.INBOX } }
            is MessageScope.Folder -> when (scopeRole) {
                FolderRole.STARRED, FolderRole.SNOOZED -> batch.folders.any { it.account == s.account }
                else -> batch.folders.any { it.folderId == s.folderId }
            }
        }
    }

    private suspend fun loadFirst() {
        val s = messageScope
        val key = s.key
        val gen = generation
        val cached = session.cached(key)
        if (cached != null) {
            rows = cached.rows
            next = cached.next
            phase = LoadPhase.LOADED
        } else {
            rows = emptyList()
            next = null
            phase = LoadPhase.FIRST_LOAD
        }
        val limit = maxOf(cached?.rows?.size ?: 0, FIRST_PAGE)
        loadLock.withLock {
            val page = repository.listMessages(s, null, limit)
            if (gen != generation) return
            rows = page.rows
            next = page.next
            phase = LoadPhase.LOADED
            session.remember(key, rows, next)
        }
        val anchor = pendingAnchor
        if (anchor != null) {
            loadThrough(anchor)
            restoreTarget = anchor
            pendingAnchor = null
        } else restoring = false
    }

    /** Pages forward until the row of [anchor] (or the first older one) is loaded, or the folder ends. */
    private suspend fun loadThrough(anchor: ListAnchor) {
        var guard = 0
        while (guard++ < MAX_RESTORE_PAGES) {
            val last = rows.lastOrNull() ?: return
            if (next == null || isAtOrBefore(last, anchor)) return
            appendPage()
        }
    }

    private fun isAtOrBefore(row: MessageRowData, anchor: ListAnchor) =
        row.dateMs < anchor.dateMs || (row.dateMs == anchor.dateMs && row.id <= anchor.messageId)

    /** The row the saved [ListAnchor] points at, or the nearest older one when it is gone. */
    fun resolveAnchor(anchor: ListAnchor): MessageRowData? = rows.firstOrNull { isAtOrBefore(it, anchor) } ?: rows.lastOrNull()

    /** The screen restored the place: from now on it is saved again. */
    fun restored() {
        restoreTarget = null
        restoring = false
    }

    /** Ask for the next page if the list is within [PREFETCH_DISTANCE] rows of its end. [rowsAfterLastVisible] counts rows only. */
    fun loadMoreIfNear(rowsAfterLastVisible: Int) {
        if (rowsAfterLastVisible <= PREFETCH_DISTANCE) loadMore()
    }

    /** Load the next [PAGE] rows. */
    fun loadMore() {
        if (loadingMore || next == null || phase != LoadPhase.LOADED) return
        loadingMore = true
        scope.launch {
            try { appendPage() } finally { loadingMore = false }
        }
    }

    private suspend fun appendPage() {
        val key = messageScope.key
        val gen = generation
        loadLock.withLock {
            val cursor = next ?: return
            val page = repository.listMessages(messageScope, cursor, PAGE)
            if (gen != generation) return
            val known = HashSet<MessageId>(rows.size * 2)
            rows.forEach { known += it.id }
            rows = rows + page.rows.filter { it.id !in known }
            next = page.next
            session.remember(key, rows, next)
        }
    }

    private suspend fun reloadHead() {
        val key = messageScope.key
        val gen = generation
        loadLock.withLock {
            if (phase != LoadPhase.LOADED) return
            val old = rows
            val page = repository.listMessages(messageScope, null, maxOf(old.size, FIRST_PAGE))
            if (gen != generation) return
            val arrivals = if (old.isEmpty()) emptyList() else {
                val known = HashSet<MessageId>(old.size * 2)
                old.forEach { known += it.id }
                val newest = old.first().dateMs
                page.rows.filter { it.id !in known && it.dateMs >= newest && it.id !in removedByMe }
            }
            rows = page.rows
            next = page.next
            if (!allInFolderSelected && selected.isNotEmpty()) {
                val present = page.rows.mapTo(HashSet()) { it.id }
                val kept = selected.filterTo(LinkedHashSet()) { it in present }
                if (kept.size != selected.size) selected = kept
            }
            session.remember(key, rows, next)
            if (arrivals.isNotEmpty()) onArrivals(arrivals)
        }
    }

    private fun onArrivals(arrivals: List<MessageRowData>) {
        if (atTop) {
            val ids = arrivals.mapTo(HashSet()) { it.id }
            arrived = arrived + ids
            scrollToTopTick++
            scope.launch {
                delay(CroftTimings.NEW_ROW_MARK_MS)
                arrived = arrived - ids
            }
        } else {
            newCount += arrivals.size
        }
    }

    /** The person is at the very top again (or tapped the pill): the pill goes. */
    fun clearNew() { if (newCount != 0) newCount = 0 }

    // ---- Refresh ----

    /** Pull to refresh, the active bottom-bar item tapped at the top, Retry on the banner. */
    fun refresh() {
        if (refreshing) return
        refreshing = true
        scope.launch {
            try {
                scopeAccounts.forEach { repository.syncNow(it.id) }
                refreshAccountsAndFolders()
                reloadHead()
                if (scopeAccounts.any { it.status != AccountState.OK && it.status != AccountState.SYNCING }) {
                    haptics.perform(HapticEvent.Rejected)
                    snackbar.show(texts.refreshFailed)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                haptics.perform(HapticEvent.Rejected)
                snackbar.show(texts.refreshFailed)
            } finally {
                refreshing = false
            }
        }
    }

    /** Retry on the server problem banner. */
    fun retry(account: AccountId) { scope.launch { repository.syncNow(account) } }

    /** The X on a banner. It stays away until the problem is gone and comes back. */
    fun dismissBanner(kind: BannerKind) { dismissed = dismissed + kind }

    // ---- Scope ----

    /** Show another folder (drawer) or account (switcher). The place of the folder left is saved first. */
    fun openScope(target: MessageScope) {
        if (target == messageScope) return
        persistAnchor()
        generation++
        clearSelection()
        newCount = 0
        arrived = emptySet()
        messageScope = target
        // The drafts of the folder left must not hide rows of the one entered, not even for a moment.
        if (localDrafts.isNotEmpty()) localDrafts = emptyList()
        pendingAnchor = session.anchors.load(target.key)
        restoring = true
        restoreTarget = null
        scope.launch { loadFirst() }
        scope.launch { reloadLocalDrafts() }
    }

    /** The inbox folder id of [account]. */
    fun inboxOf(account: AccountId): Long? = folders.firstOrNull { it.account == account && it.role == FolderRole.INBOX }?.id

    // ---- Place ----

    /** The first message on screen and how far its top is from the top of the list, or null at the very top. */
    fun currentAnchor(): ListAnchor? {
        if (atTop) return null
        val info = listState.layoutInfo
        val first = info.visibleItemsInfo.firstOrNull { it.key is Long && it.offset + it.size > info.viewportStartOffset } ?: return null
        val row = rows.firstOrNull { it.id == first.key } ?: return null
        return ListAnchor(row.id, row.dateMs, (first.offset - info.viewportStartOffset) / density)
    }

    /** Save the place of this folder (not while a saved place is still being restored). */
    fun persistAnchor() {
        if (restoring || phase != LoadPhase.LOADED || rows.isEmpty()) return
        session.anchors.save(messageScope.key, currentAnchor())
    }

    // ---- Selection ----

    /** Long-press on a row: selection mode starts with it (the long-press haptic already played). */
    fun longPress(id: MessageId) { if (id !in selected) { selected = selected + id; allInFolderSelected = false } }

    /** Avatar tap or a tap while selecting: toggle [id]. */
    fun toggle(id: MessageId) {
        haptics.perform(HapticEvent.SelectionTick)
        selected = if (id in selected) selected - id else selected + id
        allInFolderSelected = false
    }

    /** Select every loaded row. */
    fun selectAllLoaded() {
        haptics.perform(HapticEvent.SelectionTick)
        selected = shownRows.mapTo(LinkedHashSet()) { it.id }
        allInFolderSelected = false
    }

    /** "Select all N in this folder". */
    fun selectAllInFolder() {
        haptics.perform(HapticEvent.SelectionTick)
        selected = shownRows.mapTo(LinkedHashSet()) { it.id }
        allInFolderSelected = true
    }

    /** Leave selection mode. */
    fun clearSelection() {
        if (selected.isNotEmpty()) selected = emptySet()
        allInFolderSelected = false
    }

    // ---- Actions ----

    /** Star or unstar one row (the star in its accessibility actions). */
    fun toggleStar(row: MessageRowData) { scope.launch { repository.setFlagged(listOf(row.id), !row.flagged) } }

    /** Mark one row read or unread (its accessibility actions). */
    fun toggleRead(row: MessageRowData) { scope.launch { repository.markSeen(listOf(row.id), !row.seen) } }

    /** Archive or delete one row from its accessibility actions: the same as selecting it and using the bar. */
    fun actOn(row: MessageRowData, action: SelectionAction) {
        scope.launch { execute(action, listOf(row.id)) }
    }

    /** Run [action] on the selection, after a confirmation when the rules say so. */
    fun perform(action: SelectionAction) {
        val count = selectedCount
        if (count == 0) return
        if (action == SelectionAction.MOVE_TO) { openMoveSheet(rows.filter { it.id in selected }, fromSwipe = false); return }
        if (SelectionRules.needsConfirmation(action, count)) {
            pendingConfirm = PendingConfirm(action, count)
            return
        }
        scope.launch { execute(action, selectedIds()); clearSelection() }
    }

    /** The person confirmed the pending action. */
    fun confirm() {
        val pending = pendingConfirm ?: return
        pendingConfirm = null
        scope.launch { execute(pending.action, selectedIds()); clearSelection() }
    }

    /** The person cancelled the pending action. */
    fun cancelConfirm() { pendingConfirm = null }

    private suspend fun selectedIds(): List<MessageId> {
        if (!allInFolderSelected) return selected.toList()
        val ids = ArrayList<MessageId>()
        var cursor: RowCursor? = null
        do {
            val page = repository.listMessages(messageScope, cursor, ALL_IDS_PAGE)
            page.rows.forEach { ids += it.id }
            cursor = page.next
        } while (cursor != null)
        return ids
    }

    private suspend fun execute(action: SelectionAction, ids: List<MessageId>) {
        if (ids.isEmpty()) return
        try {
            when (action) {
                SelectionAction.MARK_READ -> repository.markSeen(ids, true)
                SelectionAction.MARK_UNREAD -> repository.markSeen(ids, false)
                SelectionAction.STAR -> repository.setFlagged(ids, true)
                SelectionAction.UNSTAR -> repository.setFlagged(ids, false)
                SelectionAction.ARCHIVE -> removeWithUndo(ids, texts.archived(ids.size)) { repository.archive(ids) }
                SelectionAction.DELETE -> removeWithUndo(ids, texts.deleted(ids.size)) { repository.trash(ids) }
                SelectionAction.SPAM -> {
                    val byAccount = rows.filter { it.id in ids }.groupBy { it.account }
                    removeWithUndo(ids, texts.spammed(ids.size)) {
                        byAccount.flatMap { (account, rs) ->
                            val spam = folders.firstOrNull { it.account == account && it.role == FolderRole.SPAM } ?: throw MailException(MailError.NO_SPAM_FOLDER, account)
                            repository.moveTo(rs.map { it.id }, spam.id)
                        }
                    }
                }
                SelectionAction.DELETE_FOREVER -> {
                    prepareFocus(ids)
                    removedByMe += ids
                    repository.deleteForever(ids)
                    snackbar.show(texts.deletedForever(ids.size))
                }
                SelectionAction.MOVE_TO -> Unit
                SelectionAction.RESTORE -> {
                    val byAccount = rows.filter { it.id in ids }.groupBy { it.account }
                    removeWithUndo(ids, texts.moved(ids.size)) {
                        byAccount.flatMap { (account, rs) ->
                            val inbox = inboxOf(account) ?: throw MailException(MailError.NOT_FOUND)
                            repository.moveTo(rs.map { it.id }, inbox)
                        }
                    }
                }
            }
            haptics.perform(HapticEvent.Success)
        } catch (e: MailException) {
            // Removed rows are put back by the failed call itself (the core changed nothing); the question or the message follows.
            removedByMe -= ids.toSet()
            if (!askForFolder(e) { execute(action, ids) }) {
                haptics.perform(HapticEvent.Rejected)
                snackbar.show(errorText(e.error))
            }
        }
    }

    private fun errorText(error: MailError): String = when (error) {
        MailError.NO_ARCHIVE_FOLDER -> texts.noArchive
        MailError.NO_TRASH_FOLDER -> texts.noTrash
        MailError.NO_SPAM_FOLDER -> texts.noSpam
        MailError.NOT_FOUND, MailError.FAILED -> texts.failed
    }

    /**
     * An account with no Archive or Trash folder (and none chosen yet): opens the question "which folder do you use?" and, once
     * it is answered, runs [retry]. False when [e] is not that kind of refusal (the caller shows its message).
     */
    private fun askForFolder(e: MailException, retry: suspend () -> Unit): Boolean {
        val account = e.account ?: return false
        val role = when (e.error) {
            MailError.NO_ARCHIVE_FOLDER -> FolderRole.ARCHIVE
            MailError.NO_TRASH_FOLDER -> FolderRole.TRASH
            else -> return false
        }
        if (session.folderMapping.isDeclined(account, role)) return false
        haptics.perform(HapticEvent.Rejected)
        folderRequest = FolderRequest(role, account, retry)
        return true
    }

    /** The person chose [folder] as the folder to use for the pending question: it is remembered and the action runs again. */
    fun answerFolderRequest(folder: FolderInfo) {
        val request = folderRequest ?: return
        folderRequest = null
        session.folderMapping.remember(request.account, request.role, folder.id)
        scope.launch { request.retry() }
    }

    /** "Don't offer Archive for this account": remembered; Archive is hidden for the account from now on. */
    fun declineFolderRequest() {
        val request = folderRequest ?: return
        folderRequest = null
        session.folderMapping.decline(request.account, request.role)
        snackbar.show(errorText(if (request.role == FolderRole.TRASH) MailError.NO_TRASH_FOLDER else MailError.NO_ARCHIVE_FOLDER))
    }

    /** The question was dismissed without an answer: nothing is remembered, the action did not run. */
    fun dismissFolderRequest() {
        val request = folderRequest ?: return
        folderRequest = null
        snackbar.show(errorText(if (request.role == FolderRole.TRASH) MailError.NO_TRASH_FOLDER else MailError.NO_ARCHIVE_FOLDER))
    }

    private suspend fun removeWithUndo(ids: List<MessageId>, text: String, block: suspend () -> List<OpId>) {
        prepareFocus(ids)
        removedByMe += ids
        val ops = block()
        snackbar.show(text, texts.undo, stayMs = session.undoWindowMs) {
            // The ids stay in removedByMe: a message that comes back through Undo is not new mail, whatever its date.
            scope.launch {
                session.rowActions.undo(ops)
                // Undo returns the row with the list's own motion and a brief highlight (the new-row tint, which Reduced keeps and Off drops).
                arrived = arrived + ids
                delay(CroftTimings.NEW_ROW_MARK_MS)
                arrived = arrived - ids.toSet()
            }
        }
    }

    /** After the rows [removed] go, focus moves to the next row, or the one before when they were the last. */
    private fun prepareFocus(removed: List<MessageId>) {
        val gone = removed.toHashSet()
        val list = rows
        val firstGone = list.indexOfFirst { it.id in gone }
        if (firstGone < 0) return
        val after = list.drop(firstGone).firstOrNull { it.id !in gone }
        focusTarget = (after ?: list.take(firstGone).lastOrNull { it.id !in gone })?.id
    }

    // ---- Swipe, row actions and mailbox menus ----

    private val rowIds by derivedStateOf { rows.mapTo(HashSet(rows.size * 2)) { it.id } }

    /** Whether the row of [id] is still in the list: a row that a sync removed ends the drag on it without an action. */
    fun isPresent(id: MessageId): Boolean = id in rowIds

    /**
     * Whether [account] can do the action that needs a folder of [role]. An account with no Archive (or Trash) folder can still do it
     * the first time: the person is asked once which folder to use (docs/ux/message-list.md), unless they chose one already (then it
     * is that folder) or declined (then Archive is hidden for the account).
     */
    private fun hasFolder(account: AccountId, role: FolderRole): Boolean {
        if (folders.any { it.account == account && it.role == role }) return true
        if (role != FolderRole.ARCHIVE) return false
        val mapping = session.folderMapping
        return mapping.mapped(account, role) != null || (folders.any { it.account == account } && !mapping.isDeclined(account, role))
    }

    /** What a swipe in [direction] does for [row] here: the configured action, and whether this row can do it (its account and folder). */
    fun slotOf(row: MessageRowData, direction: SwipeDirection): SwipeSlot =
        SwipeRules.slot(swipeSettings.actionOf(direction), scopeRole, hasFolder(row.account, FolderRole.ARCHIVE), hasFolder(row.account, FolderRole.SPAM))

    /** Whether [action] can run on [row] here (the custom actions of a row use the same rule as its swipe). */
    fun canDo(row: MessageRowData, action: SwipeAction): Boolean =
        SwipeRules.available(action, scopeRole, hasFolder(row.account, FolderRole.ARCHIVE), hasFolder(row.account, FolderRole.SPAM))

    /** The row whose Delete forever swipe waits for its confirmation dialog (the card is parked at 40 percent). */
    var parkedRow by mutableStateOf<MessageRowData?>(null)
        private set

    /** The answer to a parked card, for its row to take: (message id, confirmed). */
    var parkResolution by mutableStateOf<Pair<MessageId, Boolean>?>(null)
        private set

    /** An armed swipe parked its card. Only Delete forever has a dialog yet; Snooze and Move to wait for their sheets, and are not offered. */
    fun swipeParked(row: MessageRowData, action: SwipeAction) {
        when {
            SwipeRules.deletesForever(action, scopeRole) -> { parkedFromSwipe = true; parkedRow = row }
            action == SwipeAction.MOVE_TO -> openMoveSheet(listOf(row), fromSwipe = true)
            else -> parkResolution = row.id to false
        }
    }

    // ---- Move to (S30) ----

    private fun openMoveSheet(chosen: List<MessageRowData>, fromSwipe: Boolean) {
        val account = chosen.map { it.account }.distinct().singleOrNull()
        if (account == null) { if (fromSwipe) chosen.firstOrNull()?.let { parkResolution = it.id to false }; return }
        moveRequest = MoveRequest(chosen, account, fromSwipe)
    }

    /** The folders of the account of the open Move to sheet, and which of them the messages are in already. */
    val moveFolders: List<FolderInfo> get() = moveRequest?.let { r -> folders.filter { it.account == r.account } }.orEmpty()

    /** The folders the messages of the open sheet are in (they are not offered). */
    val moveExcluding: Set<FolderId> get() = moveRequest?.rows?.mapTo(HashSet()) { it.folder }.orEmpty()

    /** The recent destinations of the account of the open sheet. */
    val moveRecents: List<FolderId> get() = moveRequest?.let { session.folderMapping.recents(it.account) }.orEmpty()

    /** A folder was chosen in the Move to sheet: the mail moves there, with Undo. A parked card now leaves. */
    fun pickMove(folder: FolderInfo) {
        val request = moveRequest ?: return
        moveRequest = null
        val single = request.rows.singleOrNull()
        if (request.fromSwipe && single != null) {
            // The card slides out; the move is run when it has (removeAction).
            chosenMoves[single.id] = folder.id
            parkResolution = single.id to true
        } else {
            scope.launch { moveRows(request.rows, folder) }
            clearSelection()
        }
    }

    /** The sheet was dismissed: nothing moves and a parked card springs back. */
    fun dismissMove() {
        val request = moveRequest ?: return
        moveRequest = null
        if (request.fromSwipe) request.rows.singleOrNull()?.let { parkResolution = it.id to false }
    }

    private suspend fun moveRows(rowsToMove: List<MessageRowData>, folder: FolderInfo, onFailed: () -> Unit = {}) {
        val ids = rowsToMove.map { it.id }
        try {
            clearFailed(ids)
            removeWithUndo(ids, texts.moved(ids.size)) {
                rowsToMove.flatMap { session.rowActions.moveTo(it, folder.id) }
            }
            haptics.perform(HapticEvent.Success)
        } catch (e: MailException) {
            removedByMe -= ids.toSet()
            focusTarget = null
            haptics.perform(HapticEvent.Rejected)
            snackbar.show(errorText(e.error))
            onFailed()
        }
    }

    private fun clearFailed(ids: Collection<MessageId>) { if (failedRows.isNotEmpty()) failedRows = failedRows - ids.toSet() }

    private var parkedFromSwipe = false

    /** The dialog of a parked card was answered (or the dialog of a Delete forever from the row's accessibility actions, which has no card). */
    fun resolveParked(confirmed: Boolean) {
        val row = parkedRow ?: return
        parkedRow = null
        if (parkedFromSwipe) parkResolution = row.id to confirmed
        else if (confirmed) removeAction(row, SwipeAction.DELETE)
    }

    /** The row took the answer. */
    fun parkHandled(id: MessageId) { if (parkResolution?.first == id) parkResolution = null }

    /** A swipe asked for something this row cannot do: Rejected has played; the snackbar says so (and screen readers hear it). */
    fun swipeNotAvailable() { snackbar.show(texts.notAvailable) }

    /** A swipe that leaves the row in place (read, star), or the same from the row's accessibility actions: quiet, no snackbar. */
    fun keepAction(row: MessageRowData, action: SwipeAction) {
        scope.launch {
            try {
                when (action) {
                    SwipeAction.TOGGLE_READ -> session.rowActions.markSeen(row, !row.seen)
                    SwipeAction.TOGGLE_STAR -> session.rowActions.setFlagged(row, !row.flagged)
                    else -> Unit
                }
            } catch (e: MailException) {
                haptics.perform(HapticEvent.Rejected)
                snackbar.show(errorText(e.error))
            }
        }
    }

    /**
     * A removing action finished its slide-out and collapse (or came from the row's accessibility actions): run it through
     * [com.example.croft.mail.RowActions]. Archive, Delete and Spam show the Undo snackbar; Delete forever (Trash, Spam) does not
     * and has been confirmed. If the account cannot do it [onFailed] brings the row back with the Rejected haptic and a snackbar.
     */
    fun removeAction(row: MessageRowData, action: SwipeAction, onFailed: () -> Unit = {}) {
        scope.launch {
            clearFailed(listOf(row.id))
            try {
                when {
                    action == SwipeAction.MOVE_TO -> {
                        val target = chosenMoves.remove(row.id)?.let { id -> folders.firstOrNull { it.id == id } } ?: throw MailException(MailError.NOT_FOUND)
                        moveRows(listOf(row), target, onFailed)
                    }
                    SwipeRules.deletesForever(action, scopeRole) -> {
                        prepareFocus(listOf(row.id))
                        removedByMe += row.id
                        session.rowActions.deleteForever(row)
                        snackbar.show(texts.deletedForever(1))
                    }
                    action == SwipeAction.DELETE -> removeWithUndo(listOf(row.id), texts.deleted(1)) { session.rowActions.trash(row) }
                    action == SwipeAction.ARCHIVE -> removeWithUndo(listOf(row.id), texts.archived(1)) { session.rowActions.archive(row) }
                    action == SwipeAction.SPAM -> removeWithUndo(listOf(row.id), texts.spammed(1)) { session.rowActions.markSpam(row) }
                    else -> throw MailException(MailError.NOT_FOUND)
                }
            } catch (e: MailException) {
                removedByMe -= row.id
                focusTarget = null
                onFailed()
                if (!askForFolder(e) { removeAction(row, action) }) {
                    haptics.perform(HapticEvent.Rejected)
                    snackbar.show(errorText(e.error))
                }
            }
        }
    }

    /** Run [action] on [row] from a place that is not a swipe (TalkBack custom action, a menu): leaves or keeps the row as the action does. */
    fun act(row: MessageRowData, action: SwipeAction) {
        if (action == SwipeAction.NONE) return
        if (!canDo(row, action)) { haptics.perform(HapticEvent.Rejected); swipeNotAvailable(); return }
        if (action == SwipeAction.MOVE_TO) { openMoveSheet(listOf(row), fromSwipe = false); return }
        if (action.removesRow) {
            if (SwipeRules.deletesForever(action, scopeRole)) { parkedFromSwipe = false; parkedRow = row; return }
            removeAction(row, action)
        } else keepAction(row, action)
    }

    /** Reply: opens compose as a reply when the screen wired it (T18d); otherwise the row says that Reply comes later. */
    fun reply(row: MessageRowData) {
        val handler = replyHandler
        if (handler != null) handler(row) else snackbar.show(texts.replyLater)
    }

    /** Re-reads the local drafts when the folder shown is a Drafts folder (a local read); clears them elsewhere. */
    private suspend fun reloadLocalDrafts() {
        val compose = session.compose
        val s = messageScope
        if (compose == null || s !is MessageScope.Folder || scopeRole != FolderRole.DRAFTS) {
            if (localDrafts.isNotEmpty()) localDrafts = emptyList()
            return
        }
        val found = try {
            compose.listDrafts(s.account).filter { it.outboxId == null }.sortedByDescending { it.updatedMs }
        } catch (e: com.example.croft.mail.compose.ComposeFailure) {
            emptyList()
        }
        // The read took time: if the person moved to another folder meanwhile, these drafts are not for it (they would hide rows of that folder).
        localDrafts = if (messageScope == s && scopeRole == FolderRole.DRAFTS) found else emptyList()
    }

    /**
     * Compose closed with [closed]: "Draft saved", or "Sending..." with Undo for the Undo window. Undo takes the send back
     * (`undo_send`) and [onUndo] reopens compose on that draft with the cursor where it was; if the window has closed it says so.
     * After Send, focus goes to the first row.
     */
    fun showNotice(closed: com.example.croft.mail.compose.ComposeClosed, onUndo: (com.example.croft.mail.compose.ComposeStart.Draft) -> Unit) {
        when (closed) {
            com.example.croft.mail.compose.ComposeClosed.DraftSaved -> { snackbar.show(texts.draftSaved); scope.launch { reloadLocalDrafts() } }
            is com.example.croft.mail.compose.ComposeClosed.Sent -> {
                focusFirstPending = true
                val remaining = (closed.sendAtMs - session.clock()).coerceIn(MIN_UNDO_SNACK_MS, session.undoWindowMs + MIN_UNDO_SNACK_MS)
                snackbar.show(texts.sending, texts.undo, stayMs = remaining) {
                    scope.launch {
                        val backend = session.compose
                        try {
                            val draftId = backend?.undoSend(closed.account, closed.outboxId) ?: return@launch
                            onUndo(com.example.croft.mail.compose.ComposeStart.Draft(closed.account, draftId, closed.cursor))
                        } catch (e: com.example.croft.mail.compose.ComposeFailure) {
                            snackbar.show(if (e.problem == com.example.croft.mail.compose.ComposeProblem.TOO_LATE) texts.tooLateToUndo else texts.undoFailed)
                        }
                    }
                }
            }
            else -> scope.launch { reloadLocalDrafts() }
        }
    }

    /**
     * The conversation (S20) closed after an action: shows [text] with Undo for the Undo window, and Undo cancels [ops]. [ops] null or
     * empty: no Undo button (delete forever cannot be undone).
     */
    fun showConversationNotice(text: String, ops: List<OpId>?) {
        if (ops.isNullOrEmpty()) { snackbar.show(text); return }
        snackbar.show(text, texts.undo, stayMs = session.undoWindowMs) { scope.launch { session.rowActions.undo(ops) } }
    }

    /** Mailbox menu: mark every unread message of [folder] (of every account's Inbox when [unified]) as read, with Undo. */
    fun markMailboxRead(folder: FolderInfo, unified: Boolean = false) {
        scope.launch {
            val targets = if (unified) folders.filter { it.role == FolderRole.INBOX } else listOf(folder)
            val ids = ArrayList<MessageId>()
            for (f in targets) {
                var cursor: RowCursor? = null
                do {
                    val page = repository.listMessages(MessageScope.Folder(f.account, f.id), cursor, ALL_IDS_PAGE)
                    page.rows.forEach { if (!it.seen) ids += it.id }
                    cursor = page.next
                } while (cursor != null)
            }
            if (ids.isEmpty()) { snackbar.show(texts.nothingUnread); return@launch }
            repository.markSeen(ids, true)
            snackbar.show(texts.markedRead(ids.size), texts.undo, stayMs = session.undoWindowMs) { scope.launch { repository.markSeen(ids, false) } }
        }
    }

    /** Mailbox menu: sync [folder]'s account now. */
    fun syncMailbox(folder: FolderInfo) {
        scope.launch { repository.syncNow(folder.account) }
        snackbar.show(texts.synced)
    }

    // ---- Drawer and switcher helpers ----

    /** The account whose mailboxes the drawer lists: the account shown, or the first one in the unified Inbox. */
    val drawerAccount: AccountId?
        get() = when (val s = messageScope) {
            is MessageScope.Folder -> s.account
            MessageScope.UnifiedInbox -> accounts.firstOrNull()?.id
        }

    /** Unread messages in the Inbox of every account (the switcher's "All accounts"). */
    val unreadInAllInboxes: Int get() = folders.filter { it.role == FolderRole.INBOX }.sumOf { it.unreadCount }

    companion object {
        /** Rows in the first page. */
        const val FIRST_PAGE = 50

        /** Rows in every later page. */
        const val PAGE = 100

        /** Another page is asked for when the list is this many rows from its end. */
        const val PREFETCH_DISTANCE = 15

        private const val MAX_RESTORE_PAGES = 100
        private const val ALL_IDS_PAGE = 500
        private const val MIN_UNDO_SNACK_MS = 1_000L
    }
}

/** The controller of the list, kept across rotation and process death by its saved parts (scope, selection, place). */
@Composable
fun rememberInboxController(session: MailSession, startScope: MessageScope = MessageScope.UnifiedInbox): InboxController {
    val coroutineScope = rememberCoroutineScope()
    val resources = LocalContext.current.resources
    val texts = remember(resources) { InboxTexts(resources) }
    val saver = remember(session, coroutineScope, texts) {
        listSaver<InboxController, Any?>(
            save = { c ->
                val a = c.currentAnchor() ?: c.restoreTarget
                listOf(c.messageScope.key, a?.let { listOf(it.messageId, it.dateMs, it.offsetDp) }, ArrayList(c.selected))
            },
            restore = { saved ->
                val key = saved[0] as String
                @Suppress("UNCHECKED_CAST")
                val anchor = (saved[1] as List<Any?>?)?.let { ListAnchor(it[0] as Long, it[1] as Long, it[2] as Float) }
                @Suppress("UNCHECKED_CAST")
                val selection = (saved[2] as List<Long>).toCollection(LinkedHashSet())
                InboxController(session, coroutineScope, MessageScope.fromKey(key) ?: startScope, texts, selection, restored = true, restoredAnchor = anchor)
            },
        )
    }
    return rememberSaveable(session, saver = saver) { InboxController(session, coroutineScope, startScope, texts) }
}
