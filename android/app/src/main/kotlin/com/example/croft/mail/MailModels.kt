// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

/** Local row id of an account (the core's `account` column). */
typealias AccountId = Long

/** Local row id of a folder. */
typealias FolderId = Long

/** Local row id of a message (what the actions take). */
typealias MessageId = Long

/** Id of a queued operation (`u64` in the core, always small enough for a `Long`). */
typealias OpId = Long

/**
 * An Inbox category (T35, S10's chips, S49). Decided by the core from the sender and the list headers, and by the person's "Always do
 * this" rules. [key] is the stored name, used in the settings file.
 */
enum class MailCategory(val key: String) {
    PRIMARY("primary"),
    UPDATES("updates"),
    PROMOTIONS("promotions"),
    SOCIAL("social");

    companion object {
        /** The chips shown by default (docs/ux/customisation.md): Social is off until turned on in S49. */
        val DEFAULT_SHOWN: Set<MailCategory> = setOf(PRIMARY, UPDATES, PROMOTIONS)

        fun fromKey(key: String?): MailCategory? = entries.firstOrNull { it.key == key }
    }
}

/**
 * Which messages a list shows. One folder of one account, or the Inbox of every account merged by date, as the core's
 * `MessageScope`. The core's third case, Starred (API version 5), is a [Folder] here whose id is a virtual one
 * ([VirtualFolders]): the drawer shows "Starred" like any mailbox.
 */
sealed interface MessageScope {
    /** One folder of one account. */
    data class Folder(val account: AccountId, val folderId: FolderId) : MessageScope

    /** The Inbox of every account (folders whose special use is `inbox`), merged by date. */
    data object UnifiedInbox : MessageScope

    /** A short stable text for this scope: the key of a persisted scroll anchor and of the saved screen state. */
    val key: String
        get() = when (this) {
            is Folder -> "folder:$account:$folderId"
            UnifiedInbox -> "unified"
        }

    companion object {
        /** The scope with this [key], or null for text that is not one. */
        fun fromKey(key: String?): MessageScope? {
            if (key == null) return null
            if (key == "unified") return UnifiedInbox
            val parts = key.split(':')
            if (parts.size == 3 && parts[0] == "folder") {
                val a = parts[1].toLongOrNull() ?: return null
                val f = parts[2].toLongOrNull() ?: return null
                return Folder(a, f)
            }
            return null
        }
    }
}

/** Where the next page starts: just after the row with this date and id. Opaque to the screens: pass back a page's `next`. */
data class RowCursor(val dateMs: Long, val id: Long)

/**
 * One list row, shaped for S10 as the core's `MessageRow` (T10, API version 5): id, account, folder, sender, subject, snippet,
 * date, seen, flagged, attachments, pending, thread count and recipients. [encrypted] is not in the surface (always false until
 * OpenPGP lands) and stays at its default.
 */
data class MessageRowData(
    val id: MessageId,
    val account: AccountId,
    val folder: FolderId,
    val senderName: String?,
    val senderAddress: String?,
    val subject: String,
    val snippet: String,
    val dateMs: Long,
    val seen: Boolean,
    val flagged: Boolean,
    val hasAttachments: Boolean,
    /** A local change has not been confirmed by the server yet (the clock beside the time). */
    val pending: Boolean = false,
    /** Messages in the conversation in this folder; 1 for a single message. */
    val threadCount: Int = 1,
    /** Encrypted (the lock). Not in the core surface yet: always false. */
    val encrypted: Boolean = false,
    /** Recipients (names, else addresses, joined), for the Sent and Drafts folders; null when the core has none stored. */
    val recipients: String? = null,
    /** The row is in the sample mailbox: every change stays on this device, and the row says "Not saved to server" (docs/ux/server-contract.md, read-only mailbox). */
    val localOnly: Boolean = false,
    /** T36: woken from a snooze in the last day: the row shows a Snoozed chip. */
    val backFromSnooze: Boolean = false,
    /** T36: in the Snoozed folder, when it comes back (Unix ms); null elsewhere. */
    val snoozedUntilMs: Long? = null,
) {
    /** What the list shows as the name: the sender's name, else the address. */
    val displayName: String get() = senderName?.takeIf { it.isNotBlank() } ?: senderAddress.orEmpty()

    /** The row's cursor. */
    val cursor: RowCursor get() = RowCursor(dateMs, id)

    // Message content and addresses are not printed (CLAUDE.md, security rules).
    override fun toString(): String = "MessageRowData(id=$id, account=$account, folder=$folder, dateMs=$dateMs)"
}

/** One page of a list, newest first. [next] is null when this was the last page. */
data class MessagePage(val rows: List<MessageRowData>, val next: RowCursor?)

/** The account statuses of the core (`AccountStatus`, T10). */
enum class AccountState { OK, SYNCING, OFFLINE, NEEDS_SIGN_IN, ERROR }

/**
 * An account as the list screens need it: the core's `AccountInfo` (id, label, email, status, last synced time: S19 and S13
 * show "Last synced").
 */
data class AccountInfo(
    val id: AccountId,
    val label: String,
    val email: String,
    val status: AccountState,
    val lastSyncedMs: Long? = null,
    /** The sample mailbox ("Try with sample mail"): fictional mail from the core, no server, no password. */
    val isDemo: Boolean = false,
) {
    // The address is shown on screen but never logged (CLAUDE.md).
    override fun toString(): String = "AccountInfo(id=$id, status=$status)"
}

/** What a folder is for, from its special use (`inbox`, `sent`, ...). */
enum class FolderRole(val specialUse: String) {
    INBOX("inbox"), STARRED("starred"), SNOOZED("snoozed"), SENT("sent"), DRAFTS("drafts"), OUTBOX("outbox"),
    ARCHIVE("archive"), SPAM("junk"), TRASH("trash"), OTHER("");

    companion object {
        /** The role of a folder with this special use; anything unknown is a custom folder ([OTHER]). */
        fun fromSpecialUse(use: String): FolderRole = entries.firstOrNull { it != OTHER && it.specialUse == use.lowercase() } ?: OTHER
    }
}

/** One folder as the core's `FolderInfo`, plus its account. */
data class FolderInfo(
    val id: FolderId,
    val account: AccountId,
    /** Full path with the server's delimiter. */
    val path: String,
    val specialUse: String,
    val unreadCount: Int,
    val totalCount: Int,
) {
    /** What the folder is for. */
    val role: FolderRole get() = FolderRole.fromSpecialUse(specialUse)

    /** A mailbox the core does not store as a folder (Starred): it has no server path and cannot be a move target. */
    val isVirtual: Boolean get() = VirtualFolders.isVirtual(id)

    /** The last segment of the path, for the drawer. */
    val name: String get() = path.substringAfterLast('/')
}

/** The ids of the mailboxes that are queries, not folders. Real folder ids are positive; these are negative and never collide. */
object VirtualFolders {
    /** The id of [account]'s Starred mailbox. */
    fun starred(account: AccountId): FolderId = -(account * 10 + 1)

    /** T36: the id of [account]'s Snoozed mailbox (what is snoozed on this device). */
    fun snoozed(account: AccountId): FolderId = -(account * 10 + 2)

    /** Whether [id] is a Snoozed mailbox. */
    fun isSnoozed(id: FolderId): Boolean = id < 0 && (-id) % 10 == 2L

    /** Whether [id] is a virtual mailbox. */
    fun isVirtual(id: FolderId): Boolean = id < 0
}

/** A folder named in a change notice. */
data class FolderRef(val account: AccountId, val folderId: FolderId)

/** Why a queued change was given up. Fixed categories, as in the core. */
enum class FailureReason { LOST_AFTER_MOVE, LOST_AFTER_RESET, AMBIGUOUS_COPY, REJECTED }

/**
 * A change that failed and was undone locally. [messages] are the messages of the call that made the operation, when the
 * repository knows them (the core names only the operation); the list marks those that are on screen with the red edge.
 */
data class OpFailure(val account: AccountId, val opId: OpId, val reason: FailureReason, val messages: List<MessageId> = emptyList())

/** What changed since the `after` the caller passed to `nextChange` (T10, option B: the screen re-runs what it shows). */
data class ChangeBatch(
    val seq: Long,
    /** True when the caller was too far behind to be told exactly: re-run everything. */
    val everything: Boolean = false,
    val folders: List<FolderRef> = emptyList(),
    val accounts: List<AccountId> = emptyList(),
    val failures: List<OpFailure> = emptyList(),
)

/** Typed refusals of an action (fieldless, like the core's errors). */
enum class MailError { NO_ARCHIVE_FOLDER, NO_TRASH_FOLDER, NO_SPAM_FOLDER, NOT_FOUND, FAILED }

/**
 * An action the repository refused. [account] is the account that has no Archive, Trash or Spam folder, when that is the reason
 * (the screen then asks which folder to use, once, and remembers it).
 */
class MailException(val error: MailError, val account: AccountId? = null) : Exception(error.name)
