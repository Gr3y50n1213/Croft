// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import uniffi.croft_ffi.DraftAttachment
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.FfiIdentity

/**
 * How compose is opened (S23). The reading screens (S20, S21), the Inbox, the Outbox and the system intents all use these; the
 * navigation takes one (`openCompose(start)`). A start never carries an address or text beyond what the caller already holds.
 */
sealed interface ComposeStart {
    /** A new message. [account] null: the first account the core lists. */
    data class New(val account: AccountId? = null) : ComposeStart

    /** A `mailto:` link or a share: the fields to fill in ([ComposeFill]) and files to attach (content URIs, copied by the screen). */
    data class Fill(val fill: ComposeFill, val account: AccountId? = null) : ComposeStart

    /** Reply to the stored message [messageId] of [account] (the core's `reply_draft`). */
    data class Reply(val account: AccountId, val messageId: Long) : ComposeStart

    /** Reply all (`reply_all_draft`). */
    data class ReplyAll(val account: AccountId, val messageId: Long) : ComposeStart

    /** Forward (`forward_draft`). */
    data class Forward(val account: AccountId, val messageId: Long) : ComposeStart

    /**
     * An existing draft: reopened from the Drafts list, from Edit in the Outbox or from Undo. [cursor] is where the cursor was in the
     * body (kept in memory for the session by [CursorMemory]); null puts it at the end.
     */
    data class Draft(val account: AccountId, val draftId: Long, val cursor: BodyCursor? = null) : ComposeStart
}

/** The body's selection (start, end) in characters. */
data class BodyCursor(val start: Int, val end: Int)

/**
 * What a `mailto:` link or a SEND intent filled in. [attachments] are content URIs as text (never `file:`); the screen copies each
 * one into the attachment folder as soon as compose opens, because a share's read grant lasts only as long as its task.
 */
data class ComposeFill(
    val to: List<String> = emptyList(),
    val cc: List<String> = emptyList(),
    val bcc: List<String> = emptyList(),
    val subject: String = "",
    val body: String = "",
    val attachments: List<String> = emptyList(),
) {
    // Recipients, subject and body are not printed (CLAUDE.md, security rules).
    override fun toString(): String = "ComposeFill(${to.size + cc.size + bcc.size} recipients, ${attachments.size} files)"
}

/** The three recipient fields. */
enum class RecipientField { TO, CC, BCC }

/** A recipient chip: the address as typed, and whether the core accepts it (`is_valid_address`). An invalid one is an error chip. */
data class RecipientChipData(val address: FfiAddress, val valid: Boolean) {
    /** What the chip says: the name when there is one, else the address. */
    val label: String get() = address.name?.takeIf { it.isNotBlank() } ?: address.address

    override fun toString(): String = "RecipientChipData(valid=$valid)"
}

/** What the Send button shows (docs/ux/controls.md, "Send button"). The screen maps each to its `SendCondition`. */
enum class SendState {
    NO_RECIPIENT, INVALID_RECIPIENT, OVER_SIZE_LIMIT, EMPTY_CONFIRM, READY, UPLOADING, OFFLINE_OR_SIGN_IN, SENDING, ENCRYPTION_NO_KEY,
}

/** The quiet line about the copy on the server ("Saved", "Saved on this device", "Waiting for connection"). */
enum class SaveStatus { NONE, SAVED, SAVED_ON_DEVICE, WAITING_FOR_CONNECTION, COULD_NOT_SAVE }

/** A question the screen must answer before the message goes. Send never proceeds without the person's choice. */
sealed interface SendPrompt {
    /** "Send without subject or text?" */
    data object EmptyConfirm : SendPrompt

    /** S24: encryption is required and [missing] have no key. Send unencrypted or Cancel. */
    data class EncryptionCheck(val missing: List<String>) : SendPrompt {
        override fun toString(): String = "EncryptionCheck(${missing.size})"
    }

    /** Tapping a disabled Send: the reason, once. */
    data class Explain(val state: SendState) : SendPrompt
}

/** Why a picked file was not attached. */
enum class AttachmentError { TOO_LARGE, TOO_MANY, TOTAL_TOO_LARGE, UNREADABLE, FAILED }

/** How compose ended, for the screen that opened it (the Inbox shows the snackbar). */
sealed interface ComposeClosed {
    /** Closed with nothing to keep. */
    data object Nothing : ComposeClosed

    /** Closed with a draft kept on the device ("Draft saved"). */
    data object DraftSaved : ComposeClosed

    /** The draft was discarded by the person. */
    data object Discarded : ComposeClosed

    /** Send was tapped: the message is in the Outbox until [sendAtMs] (the Undo window), then goes out. */
    data class Sent(val account: AccountId, val outboxId: Long, val sendAtMs: Long, val cursor: BodyCursor?) : ComposeClosed
}

/** The whole state of an open compose screen. Immutable; the controller replaces it at every change. */
data class ComposeState(
    val phase: Phase = Phase.LOADING,
    val account: AccountId = 0,
    val draftId: Long? = null,
    val identities: List<FfiIdentity> = emptyList(),
    val identity: Long? = null,
    val to: List<RecipientChipData> = emptyList(),
    val cc: List<RecipientChipData> = emptyList(),
    val bcc: List<RecipientChipData> = emptyList(),
    /** Cc and Bcc stay hidden behind a toggle until used. */
    val showCcBcc: Boolean = false,
    val subject: String = "",
    val body: String = "",
    /** Where the cursor goes when the body first shows (a reopened draft); null: the end. */
    val initialCursor: BodyCursor? = null,
    val attachments: List<DraftAttachment> = emptyList(),
    /** Files being copied now. */
    val attachmentsBusy: Int = 0,
    val attachmentError: AttachmentError? = null,
    val sendState: SendState = SendState.NO_RECIPIENT,
    val sizeLimitMb: Int = 25,
    val saveStatus: SaveStatus = SaveStatus.NONE,
    val prompt: SendPrompt? = null,
    /** The account is offline or needs sign-in (Send stays enabled; the Outbox shows Queued). */
    val offlineOrSignIn: Boolean = false,
    /** Encryption is required and a recipient has no key: Send shows the warning icon. */
    val encryptionWarning: Boolean = false,
    /** The reply's quote could not be filled (the body is not stored on the device yet). */
    val quoteMissing: Boolean = false,
    /** The last Send was refused for a reason the Send button cannot show (the account has no SMTP server, a failure of the core). */
    val sendProblem: ComposeProblem? = null,
    /** Set when compose is over; the screen closes and hands this to the one that opened it. */
    val closed: ComposeClosed? = null,
) {
    enum class Phase { LOADING, EDITING, SENDING, CLOSED, FAILED_TO_OPEN, LOCKED }

    /** Every recipient chip of all three fields. */
    val allRecipients: List<RecipientChipData> get() = to + cc + bcc

    fun chips(field: RecipientField): List<RecipientChipData> = when (field) {
        RecipientField.TO -> to
        RecipientField.CC -> cc
        RecipientField.BCC -> bcc
    }

    val canSwitchAccount: Boolean get() = attachments.isEmpty() && attachmentsBusy == 0
}

/** The cursor positions of this session's drafts, by draft id: kept in memory only, which is enough for Undo (a restart is a reopen). */
class CursorMemory {
    private val map = HashMap<Long, BodyCursor>()

    @Synchronized fun remember(draftId: Long, cursor: BodyCursor) { map[draftId] = cursor }

    @Synchronized fun recall(draftId: Long): BodyCursor? = map[draftId]

    @Synchronized fun forget(draftId: Long) { map.remove(draftId) }
}

/** Reading and checking addresses: the core's functions in the app, a plain stand-in in tests that cannot load the library. */
interface AddressTools {
    /** A pasted or typed list read into recipients (the core's `parse_recipients`). */
    fun parse(text: String): List<FfiAddress>

    /** Whether the bare address is one a server could deliver to (the core's `is_valid_address`). */
    fun isValid(address: String): Boolean
}

/** [AddressTools] over the core's own free functions. */
object CoreAddressTools : AddressTools {
    override fun parse(text: String): List<FfiAddress> = uniffi.croft_ffi.parseRecipients(text)

    override fun isValid(address: String): Boolean = uniffi.croft_ffi.isValidAddress(address)
}
