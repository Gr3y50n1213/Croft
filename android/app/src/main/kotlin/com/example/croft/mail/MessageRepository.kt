// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

/**
 * Everything the list screens ask of the mail core, shaped like the T10 surface (`core/ffi`, docs/tasks/T10) so that T16 can
 * implement it over `uniffi.croft_ffi.Core` and change nothing else:
 *
 * | Here | Core |
 * |---|---|
 * | [listAccounts], [listFolders] | `list_accounts`, `list_folders` |
 * | [listMessages] (keyset paging, newest first) | `list_messages(scope, before, limit)` |
 * | [nextChange] | `next_change(after)` (returns when something changed after `after`) |
 * | [markSeen], [setFlagged], [archive], [trash], [moveTo], [deleteForever], [undo] | the same names |
 * | [syncNow], [setNetworkAvailable], [signedIn] | `sync_now`, `set_network_available`, `signed_in` |
 * | [setUndoWindowMs] | `set_undo_window_ms` |
 *
 * Two implementations: [FakeMessageRepository] (the sample mailbox, tests, previews) and [CoreMessageRepository] (the real core).
 *
 * The functions are `suspend` because the core's calls must not run on the main thread; the fake answers at once.
 * Every change a person makes is applied locally first and queued (CLAUDE.md): the list shows it as soon as the next change
 * notice arrives, with `pending` set until the server confirms. Nothing here waits for the network.
 */
interface MessageRepository {
    /** Every account with its status. */
    suspend fun listAccounts(): List<AccountInfo>

    /** The folders of [account] with their counters. */
    suspend fun listFolders(account: AccountId): List<FolderInfo>

    /**
     * One page of [scope], newest first, keyset by (date, id): the rows strictly after [before] (from the start when null),
     * at most [limit] of them. [MessagePage.next] is null when this was the last page.
     */
    suspend fun listMessages(scope: MessageScope, before: RowCursor?, limit: Int): MessagePage

    /**
     * T35: [listMessages] with only the messages of [category] in the Inbox scopes (`null`: all of them). An implementation without
     * categories lists everything.
     */
    suspend fun listMessagesIn(scope: MessageScope, category: MailCategory?, before: RowCursor?, limit: Int): MessagePage =
        listMessages(scope, before, limit)

    /** T35: unread messages per category in the Inbox of [account], or of every account (`null`). */
    suspend fun categoryCounts(account: AccountId?): Map<MailCategory, Long> = emptyMap()

    /** T35: moves a message to [category]; with [always], the sender's mail goes there from now on ("Always do this"). */
    suspend fun setCategory(id: MessageId, category: MailCategory, always: Boolean) {}

    /** T36: snoozes messages on this device until [untilMs] (S29). */
    suspend fun snooze(ids: List<MessageId>, untilMs: Long) {}

    /** T36: brings snoozed messages back at once (Undo, Unsnooze). */
    suspend fun unsnooze(ids: List<MessageId>) {}

    /** T36: wakes the messages whose time has come; their ids (for "Back in your inbox"). */
    suspend fun wakeDueSnoozes(): List<MessageId> = emptyList()

    /** T36: the soonest wake time of anything snoozed, for the wake-up job. */
    suspend fun nextSnoozeWake(): Long? = null

    /** T36b: pins or unpins [ids] (a keyword that syncs); the operation ids, for Undo. */
    suspend fun setPinned(ids: List<MessageId>, value: Boolean): List<OpId> = emptyList()

    /** T36b: marks [ids] to follow up, or clears the mark ("Done"); the operation ids, for Undo. */
    suspend fun setFollowUp(ids: List<MessageId>, value: Boolean): List<OpId> = emptyList()

    /** T36b: the pinned messages in the Inbox of [account], or of every account (`null`), newest first: the Pinned section. */
    suspend fun listPinned(account: AccountId?): List<MessageRowData> = emptyList()

    /** Returns as soon as anything changed after [after] (at once if [after] is old). Pass the batch's `seq` next time. */
    suspend fun nextChange(after: Long): ChangeBatch

    /** Mark [ids] read or unread (applied now, sent after 1.5 s; Undo is the opposite call). */
    suspend fun markSeen(ids: List<MessageId>, value: Boolean)

    /** Star or unstar [ids]. */
    suspend fun setFlagged(ids: List<MessageId>, value: Boolean)

    /** Move [ids] to the Archive of their accounts; [MailError.NO_ARCHIVE_FOLDER] if one has none. Returns the operation ids. */
    suspend fun archive(ids: List<MessageId>): List<OpId>

    /** Move [ids] to Trash; [MailError.NO_TRASH_FOLDER] if an account has none. */
    suspend fun trash(ids: List<MessageId>): List<OpId>

    /** Move [ids] to [folderId]. */
    suspend fun moveTo(ids: List<MessageId>, folderId: FolderId): List<OpId>

    /** Delete [ids] for good (asked about first by the screen). */
    suspend fun deleteForever(ids: List<MessageId>): List<OpId>

    /** Cancel a queued operation inside its undo window; false when it is too late. */
    suspend fun undo(opId: OpId): Boolean

    /**
     * Run again what operation [opId] was (a failed archive, delete or move), for the Retry of the failure snackbar. Returns the
     * new operation ids; [MailError.NOT_FOUND] when the repository no longer knows the operation.
     */
    suspend fun retryFailed(opId: OpId): List<OpId>

    /** The Undo window for moves, deletes and spam made from now on, in ms (S49: 3, 5 or 10 s). */
    suspend fun setUndoWindowMs(ms: Long)

    /** Sync [account] now (pull to refresh, Retry). */
    suspend fun syncNow(account: AccountId)

    /** Tell the core whether the network is there (every account). */
    suspend fun setNetworkAvailable(available: Boolean)

    /** The person fixed the sign-in of [account]. */
    suspend fun signedIn(account: AccountId)
}
