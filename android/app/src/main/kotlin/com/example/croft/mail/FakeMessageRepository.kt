// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first

/** One call of [FakeMessageRepository.listMessages], for tests that count requests. */
data class ListRequest(val scope: MessageScope, val before: RowCursor?, val limit: Int)

/**
 * A [MessageRepository] in memory, for T14 (and for tests and previews later): it holds any number of rows (5,000 and more),
 * several accounts and every row state, and it can inject new mail, go offline, and flip an account to Needs sign-in or to a
 * server problem. T16 replaces it with the core; the screens do not change.
 *
 * It behaves as the core is documented to: keyset paging by (date, id) newest first, a change log that [nextChange] reads,
 * actions applied at once with `pending` set while offline, moves that can be undone inside the undo window (by the injected
 * clock). Calls are serialised with a lock; none of them waits for anything but the optional hold below.
 */
class FakeMessageRepository(
    rows: List<MessageRowData> = FakeData.rows(200, System.currentTimeMillis()),
    private val clock: () -> Long = System::currentTimeMillis,
    accounts: List<AccountInfo> = FakeData.accounts(clock()),
) : MessageRepository {
    private val lock = Any()

    /** Newest first by (date, id). */
    private val all: MutableList<MessageRowData> = rows.sortedWith(ORDER).toMutableList()
    private val accountList: MutableList<AccountInfo> = accounts.toMutableList()
    private val folderList: List<FolderInfo> = accounts.flatMap { a -> FakeData.folderSpecs.map { (pos, spec) -> FolderInfo(FakeData.folderId(a.id, pos), a.id, spec.second, spec.first, 0, 0) } }
    private var nextId: Long = (rows.maxOfOrNull { it.id } ?: 0L) + 1
    private var nextOp = 1L
    private var undoWindowMs = DEFAULT_UNDO_WINDOW_MS
    private var offline = false

    private class Op(val moves: List<Pair<MessageId, FolderId>>, val expiresAt: Long)
    private val ops = HashMap<OpId, Op>()

    private class Change(val seq: Long, val folders: Set<FolderRef>, val accounts: Set<AccountId>)
    private val log = ArrayList<Change>()
    private val seq = MutableStateFlow(0L)

    private var hold: CompletableDeferred<Unit>? = null
    private val requestLog = ArrayList<ListRequest>()

    /** Every `listMessages` call so far, oldest first. */
    val requests: List<ListRequest> get() = synchronized(lock) { requestLog.toList() }

    /** The number of rows held, in every folder. */
    val size: Int get() = synchronized(lock) { all.size }

    /** The rows of [scope] right now, newest first (for tests). */
    fun snapshot(scope: MessageScope): List<MessageRowData> = synchronized(lock) { scopeRows(scope).toList() }

    // ---- Controls for tests and the debug app ----

    /** Make `listMessages` wait until [release] (a slow first load, a slow refresh). */
    fun holdQueries() { synchronized(lock) { if (hold == null) hold = CompletableDeferred() } }

    /** Let the held queries answer. */
    fun release() { synchronized(lock) { hold?.complete(Unit); hold = null } }

    /** Set the undo window in ms (the default is 5 s). */
    fun setUndoWindow(ms: Long) { synchronized(lock) { undoWindowMs = ms } }

    /**
     * New mail: [count] unread rows in the Inbox of [account], each newer than every row there is. Returns their ids,
     * newest first.
     */
    fun injectNewMail(count: Int = 1, account: AccountId = accountList.first().id): List<MessageId> = synchronized(lock) {
        val newest = all.firstOrNull()?.dateMs ?: clock()
        val base = maxOf(clock(), newest + 1000)
        val fresh = (0 until count).map { i ->
            val id = nextId++
            MessageRowData(
                id = id, account = account, folder = FakeData.folderId(account, FakeData.INBOX),
                senderName = "New Sender ${id}", senderAddress = "new$id@example.org",
                subject = "New message $id", snippet = "Just arrived.", dateMs = base + i * 1000L,
                seen = false, flagged = false, hasAttachments = false,
            )
        }
        all.addAll(fresh); all.sortWith(ORDER)
        changed(setOf(FolderRef(account, FakeData.folderId(account, FakeData.INBOX))), emptySet())
        fresh.map { it.id }.reversed()
    }

    /** Airplane mode: every account goes Offline (and the queued changes wait, shown with the clock) or comes back. */
    fun setOffline(value: Boolean) = synchronized(lock) {
        offline = value
        val touched = HashSet<AccountId>()
        accountList.replaceAll { a ->
            when {
                value && (a.status == AccountState.OK || a.status == AccountState.SYNCING) -> { touched += a.id; a.copy(status = AccountState.OFFLINE) }
                !value && a.status == AccountState.OFFLINE -> { touched += a.id; a.copy(status = AccountState.OK, lastSyncedMs = clock()) }
                else -> a
            }
        }
        if (!value) confirmPending()
        changed(emptySet(), touched)
    }

    /** Flip one account to [state] (Needs sign-in, a server problem, Syncing...). */
    fun setAccountStatus(account: AccountId, state: AccountState) = synchronized(lock) {
        accountList.replaceAll { if (it.id == account) it.copy(status = state) else it }
        changed(emptySet(), setOf(account))
    }

    // ---- MessageRepository ----

    override suspend fun listAccounts(): List<AccountInfo> = synchronized(lock) { accountList.toList() }

    override suspend fun listFolders(account: AccountId): List<FolderInfo> = synchronized(lock) {
        folderList.filter { it.account == account }.map { f ->
            val rows = scopeRows(MessageScope.Folder(account, f.id))
            f.copy(unreadCount = rows.count { !it.seen }, totalCount = rows.size)
        }
    }

    override suspend fun listMessages(scope: MessageScope, before: RowCursor?, limit: Int): MessagePage {
        synchronized(lock) { requestLog += ListRequest(scope, before, limit) }
        hold?.await()
        return synchronized(lock) {
            val rows = scopeRows(scope).asSequence()
                .dropWhile { before != null && !(it.dateMs < before.dateMs || (it.dateMs == before.dateMs && it.id < before.id)) }
                .take(limit + 1).toList()
            val page = rows.take(limit)
            MessagePage(page, if (rows.size > limit) page.last().cursor else null)
        }
    }

    override suspend fun nextChange(after: Long): ChangeBatch {
        val now = seq.first { it > after }
        return synchronized(lock) {
            val pending = log.filter { it.seq > after }
            ChangeBatch(
                seq = now,
                everything = pending.isEmpty() || log.first().seq > after + 1,
                folders = pending.flatMap { it.folders }.distinct(),
                accounts = pending.flatMap { it.accounts }.distinct(),
            )
        }
    }

    override suspend fun markSeen(ids: List<MessageId>, value: Boolean) = synchronized(lock) {
        update(ids) { it.copy(seen = value, pending = offline) }
    }

    override suspend fun setFlagged(ids: List<MessageId>, value: Boolean) = synchronized(lock) {
        update(ids) { it.copy(flagged = value, pending = offline) }
    }

    override suspend fun archive(ids: List<MessageId>): List<OpId> = synchronized(lock) { moveByRole(ids, FakeData.ARCHIVE, MailError.NO_ARCHIVE_FOLDER) }

    override suspend fun trash(ids: List<MessageId>): List<OpId> = synchronized(lock) { moveByRole(ids, FakeData.TRASH, MailError.NO_TRASH_FOLDER) }

    override suspend fun moveTo(ids: List<MessageId>, folderId: FolderId): List<OpId> = synchronized(lock) {
        val target = folderList.firstOrNull { it.id == folderId } ?: throw MailException(MailError.NOT_FOUND)
        move(ids) { target.id }
    }

    override suspend fun deleteForever(ids: List<MessageId>): List<OpId> = synchronized(lock) {
        val set = ids.toSet()
        val gone = all.filter { it.id in set }
        if (gone.isEmpty()) throw MailException(MailError.NOT_FOUND)
        all.removeAll(gone.toSet())
        changed(gone.map { FolderRef(it.account, it.folder) }.toSet(), emptySet())
        // A permanent delete has no undo: the operation id exists but is already past its window.
        gone.map { nextOp++ }
    }

    override suspend fun undo(opId: OpId): Boolean = synchronized(lock) {
        val op = ops.remove(opId) ?: return false
        if (clock() > op.expiresAt) return false
        val back = op.moves.toMap()
        val touched = HashSet<FolderRef>()
        all.replaceAll { r ->
            val old = back[r.id] ?: return@replaceAll r
            touched += FolderRef(r.account, r.folder); touched += FolderRef(r.account, old)
            r.copy(folder = old)
        }
        changed(touched, emptySet())
        true
    }

    override suspend fun syncNow(account: AccountId) = synchronized(lock) {
        accountList.replaceAll { a ->
            when {
                a.id != account -> a
                a.status == AccountState.ERROR || a.status == AccountState.SYNCING || a.status == AccountState.OK -> a.copy(status = AccountState.OK, lastSyncedMs = clock())
                else -> a
            }
        }
        changed(emptySet(), setOf(account))
    }

    override suspend fun setNetworkAvailable(available: Boolean) = setOffline(!available)

    override suspend fun signedIn(account: AccountId) = synchronized(lock) {
        accountList.replaceAll { if (it.id == account) it.copy(status = if (offline) AccountState.OFFLINE else AccountState.OK, lastSyncedMs = clock()) else it }
        changed(emptySet(), setOf(account))
    }

    // ---- Internals (always under the lock) ----

    private fun scopeRows(scope: MessageScope): List<MessageRowData> = when (scope) {
        MessageScope.UnifiedInbox -> {
            val inboxes = folderList.filter { it.role == FolderRole.INBOX }.map { it.id }.toSet()
            all.filter { it.folder in inboxes }
        }
        is MessageScope.Folder -> {
            val role = folderList.firstOrNull { it.id == scope.folderId }?.role
            when (role) {
                // Virtual folders: starred mail of the account (not in Trash or Spam), and nothing snoozed.
                FolderRole.STARRED -> all.filter { it.account == scope.account && it.flagged && !inFolderOf(it, FolderRole.TRASH) && !inFolderOf(it, FolderRole.SPAM) }
                FolderRole.SNOOZED -> emptyList()
                else -> all.filter { it.folder == scope.folderId }
            }
        }
    }

    private fun inFolderOf(row: MessageRowData, role: FolderRole) = folderList.firstOrNull { it.id == row.folder }?.role == role

    private fun update(ids: List<MessageId>, f: (MessageRowData) -> MessageRowData) {
        val set = ids.toSet()
        val touched = HashSet<FolderRef>()
        all.replaceAll { r -> if (r.id in set) { touched += FolderRef(r.account, r.folder); f(r) } else r }
        changed(touched, emptySet())
    }

    private fun moveByRole(ids: List<MessageId>, position: Long, error: MailError): List<OpId> {
        val set = ids.toSet()
        val rows = all.filter { it.id in set }
        if (rows.isEmpty()) throw MailException(MailError.NOT_FOUND)
        // Validated before the first change, like the core: every account needs the folder.
        for (a in rows.map { it.account }.toSet()) {
            if (accountsWithout[a]?.contains(position) == true) throw MailException(error)
        }
        return move(ids) { row -> FakeData.folderId(row.account, position) }
    }

    private val accountsWithout = HashMap<AccountId, Set<Long>>()

    /** Make [account] have no folder at [position] (for example [FakeData.ARCHIVE]): the action is then refused with a typed error. */
    fun removeFolder(account: AccountId, position: Long) = synchronized(lock) { accountsWithout[account] = (accountsWithout[account] ?: emptySet()) + position }

    private fun move(ids: List<MessageId>, destination: (MessageRowData) -> FolderId?): List<OpId> {
        val set = ids.toSet()
        val moves = ArrayList<Pair<MessageId, FolderId>>()
        val touched = HashSet<FolderRef>()
        all.replaceAll { r ->
            if (r.id !in set) return@replaceAll r
            val to = destination(r) ?: return@replaceAll r
            // A message that is already there is skipped, no error (T10).
            if (to == r.folder) return@replaceAll r
            moves += r.id to r.folder
            touched += FolderRef(r.account, r.folder); touched += FolderRef(r.account, to)
            r.copy(folder = to, pending = offline)
        }
        val op = nextOp++
        ops[op] = Op(moves, clock() + undoWindowMs)
        changed(touched, emptySet())
        return listOf(op)
    }

    private fun confirmPending() {
        val touched = HashSet<FolderRef>()
        all.replaceAll { r -> if (r.pending) { touched += FolderRef(r.account, r.folder); r.copy(pending = false) } else r }
        changed(touched, emptySet())
    }

    private fun changed(folders: Set<FolderRef>, accounts: Set<AccountId>) {
        val next = seq.value + 1
        log += Change(next, folders, accounts)
        if (log.size > LOG_SIZE) log.removeAt(0)
        seq.value = next
    }

    private companion object {
        const val DEFAULT_UNDO_WINDOW_MS = 5_000L
        const val LOG_SIZE = 512
        val ORDER: Comparator<MessageRowData> = compareByDescending<MessageRowData> { it.dateMs }.thenByDescending { it.id }
    }
}
