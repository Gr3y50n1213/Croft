// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import android.content.res.Resources
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
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
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.FolderRole
import com.example.croft.mail.ListAnchor
import com.example.croft.mail.MailError
import com.example.croft.mail.MailException
import com.example.croft.mail.MailSession
import com.example.croft.mail.MessageId
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.MessageScope
import com.example.croft.mail.OpId
import com.example.croft.mail.RowCursor
import com.example.croft.theme.CroftTimings
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
    val refreshFailed: String get() = res.getString(R.string.mail_snack_refresh_failed)
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

    /** The one snackbar of the screen. */
    val snackbar = CroftSnackbarState()

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
            val archiveOk = chosen.map { it.account }.distinct().all { a -> folders.any { it.account == a && it.role == FolderRole.ARCHIVE } }
            return SelectionRules.toolbar(chosen, scopeRole, archiveOk)
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
        get() = if (phase != LoadPhase.LOADED || rows.isNotEmpty()) null else when (scopeRole) {
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
        launch { refreshAccountsAndFolders() }
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
        dismissed = dismissed.filterTo(HashSet()) { kind -> activeKinds().contains(kind) }
        if (batch.failures.isNotEmpty()) {
            haptics.perform(HapticEvent.Failure)
            snackbar.show(texts.failed)
        }
        if (batch.everything || affects(batch)) reloadHead()
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
        pendingAnchor = session.anchors.load(target.key)
        restoring = true
        restoreTarget = null
        scope.launch { loadFirst() }
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
        selected = rows.mapTo(LinkedHashSet()) { it.id }
        allInFolderSelected = false
    }

    /** "Select all N in this folder". */
    fun selectAllInFolder() {
        haptics.perform(HapticEvent.SelectionTick)
        selected = rows.mapTo(LinkedHashSet()) { it.id }
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
                SelectionAction.DELETE_FOREVER -> {
                    prepareFocus(ids)
                    removedByMe += ids
                    repository.deleteForever(ids)
                    snackbar.show(texts.deletedForever(ids.size))
                }
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
            haptics.perform(HapticEvent.Rejected)
            snackbar.show(
                when (e.error) {
                    MailError.NO_ARCHIVE_FOLDER -> texts.noArchive
                    MailError.NO_TRASH_FOLDER -> texts.noTrash
                    MailError.NOT_FOUND -> texts.failed
                },
            )
        }
    }

    private suspend fun removeWithUndo(ids: List<MessageId>, text: String, block: suspend () -> List<OpId>) {
        prepareFocus(ids)
        removedByMe += ids
        val ops = block()
        snackbar.show(text, texts.undo, stayMs = UNDO_WINDOW_MS) {
            scope.launch {
                removedByMe -= ids.toSet()
                ops.forEach { repository.undo(it) }
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

        /** The Undo window of the snackbar, from the same default the core uses. */
        private val UNDO_WINDOW_MS: Long = CroftSnackbarState.DefaultUndoWindowMs
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
