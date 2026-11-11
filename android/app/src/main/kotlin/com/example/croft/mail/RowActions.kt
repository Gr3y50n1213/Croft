// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

/**
 * What a swipe, a long-press menu item or a TalkBack custom action does to one message row. The screens call only this;
 * T16 implements it on the core ([CoreRowActions]; the operation queue: applied locally at once, sent after the undo window for moves,
 * deletes and spam, after 1.5 s for read and star). [RepositoryRowActions] is the implementation over [MessageRepository],
 * which is what runs for the sample mailbox; [CoreRowActions] is T16's, over the core; tests wrap either to record the calls.
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
open class RepositoryRowActions(private val repository: MessageRepository) : RowActions {
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

/**
 * [RowActions] over the core (T16): the swipe, the menus and the TalkBack actions call the core's queue through
 * [CoreMessageRepository]. A move, delete or spam is applied to the row at once, sent after the Undo window, and cancelled by
 * [undo] inside it; read and star are sent after 1.5 s and undone by the opposite call. What it adds to the plain delegation:
 *
 * - **Mark as spam** goes to the account's Junk folder, or to the folder the person chose for it ([FolderMapping]); an account
 *   with neither throws [MailError.NO_SPAM_FOLDER] with the account (the swipe is then unavailable, and the screen asks).
 * - **Move to** remembers the destination as a recent one for the Move to sheet (S30).
 * Archive and Trash find their folder in [CoreMessageRepository] (special use, then the mapping, then [MailError]).
 */
class CoreRowActions(
    private val repository: MessageRepository,
    private val mapping: FolderMapping,
) : RepositoryRowActions(repository) {
    override suspend fun markSpam(row: MessageRowData): List<OpId> {
        val folders = repository.listFolders(row.account)
        val spam = folders.firstOrNull { it.role == FolderRole.SPAM }?.id
            ?: mapping.mapped(row.account, FolderRole.SPAM)?.takeIf { id -> folders.any { it.id == id } }
            ?: throw MailException(MailError.NO_SPAM_FOLDER, row.account)
        return repository.moveTo(listOf(row.id), spam)
    }

    override suspend fun moveTo(row: MessageRowData, folder: FolderId): List<OpId> {
        val ops = repository.moveTo(listOf(row.id), folder)
        mapping.recordRecent(row.account, folder)
        return ops
    }
}
