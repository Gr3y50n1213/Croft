// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

/**
 * What a swipe, a long-press menu item or a TalkBack custom action does to one message row. The screens call only this;
 * T16 implements it on the core (the operation queue: applied locally at once, sent after the undo window for moves,
 * deletes and spam, after 1.5 s for read and star). [RepositoryRowActions] is the implementation over [MessageRepository],
 * which is what runs until T16; tests wrap it to record the calls.
 *
 * Every call is `suspend` because the core's calls must not run on the main thread, and throws [MailException] when the
 * account cannot do it (no Archive, Trash or Spam folder). The functions that move a message return the operation ids that
 * [undo] cancels inside the undo window.
 */
interface RowActions {
    /** Mark [row] read ([seen] true) or unread. Quiet: no snackbar. */
    suspend fun markSeen(row: MessageRowData, seen: Boolean)

    /** Star or unstar [row]. Quiet: no snackbar. */
    suspend fun setFlagged(row: MessageRowData, flagged: Boolean)

    /** Move [row] to the Archive of its account. */
    suspend fun archive(row: MessageRowData): List<OpId>

    /** Move [row] to Trash. */
    suspend fun trash(row: MessageRowData): List<OpId>

    /** Move [row] to Spam. */
    suspend fun markSpam(row: MessageRowData): List<OpId>

    /** Delete [row] for good (the screen has asked first). Cannot be undone. */
    suspend fun deleteForever(row: MessageRowData): List<OpId>

    /** Move [row] to [folder] (the Move to sheet, S30, from T16 on). */
    suspend fun moveTo(row: MessageRowData, folder: FolderId): List<OpId>

    /** Cancel the queued operations [ops] inside their undo window. */
    suspend fun undo(ops: List<OpId>)
}

/** [RowActions] over a [MessageRepository]: the same calls the selection toolbar makes. */
class RepositoryRowActions(private val repository: MessageRepository) : RowActions {
    override suspend fun markSeen(row: MessageRowData, seen: Boolean) = repository.markSeen(listOf(row.id), seen)

    override suspend fun setFlagged(row: MessageRowData, flagged: Boolean) = repository.setFlagged(listOf(row.id), flagged)

    override suspend fun archive(row: MessageRowData): List<OpId> = repository.archive(listOf(row.id))

    override suspend fun trash(row: MessageRowData): List<OpId> = repository.trash(listOf(row.id))

    override suspend fun markSpam(row: MessageRowData): List<OpId> {
        val spam = repository.listFolders(row.account).firstOrNull { it.role == FolderRole.SPAM } ?: throw MailException(MailError.NO_SPAM_FOLDER)
        return repository.moveTo(listOf(row.id), spam.id)
    }

    override suspend fun deleteForever(row: MessageRowData): List<OpId> = repository.deleteForever(listOf(row.id))

    override suspend fun moveTo(row: MessageRowData, folder: FolderId): List<OpId> = repository.moveTo(listOf(row.id), folder)

    override suspend fun undo(ops: List<OpId>) { ops.forEach { repository.undo(it) } }
}
