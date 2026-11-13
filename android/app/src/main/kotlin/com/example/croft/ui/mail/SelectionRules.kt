// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageRowData
import com.example.croft.ui.swipe.SwipeRules

/** What the selection bar can do (docs/ux/controls.md, "Toolbar enablement rules"; docs/ux/message-list.md, "Folder rules"). */
enum class SelectionAction {
    /** Move to the Archive folder. */
    ARCHIVE,

    /** Move to Trash. */
    DELETE,

    /** Delete for good (Trash and Spam), after a confirmation. */
    DELETE_FOREVER,

    /** Mark everything selected as read. */
    MARK_READ,

    /** Mark everything selected as unread. */
    MARK_UNREAD,

    /** Star everything selected. */
    STAR,

    /** Remove the star from everything selected. */
    UNSTAR,

    /** Move back to the Inbox: "Move to inbox" from Archive, "Not spam" from Spam, "Restore" from Trash. */
    RESTORE,

    /** Move to Spam (the swipe action Mark as spam has this route too). */
    SPAM,

    /** Move to a folder chosen in the Move to sheet (S30; the swipe action Move to has this route too). In the overflow menu. */
    MOVE_TO,

    /** T36: snooze until a time chosen in S29 (the swipe action Snooze has this route too). In the overflow menu. */
    SNOOZE,
}

/** The bar's buttons in order, and what goes in the overflow menu. */
data class SelectionToolbar(val primary: List<SelectionAction>, val overflow: List<SelectionAction>)

/** The rules of the selection bar, as a pure function so the table in the docs and the tests can be compared line by line. */
object SelectionRules {
    /**
     * The toolbar for [selected] rows shown in a folder of [role] (the unified Inbox counts as [FolderRole.INBOX]).
     * [archiveAvailable] is true when every account involved has an Archive folder.
     *
     * - Mark as read appears if at least one selected message is unread; otherwise Mark as unread.
     * - Archive is not offered in Archive, Sent, Drafts, Spam and Trash, nor when an account has no Archive folder; in
     *   Archive, Spam and Trash the Restore action (Move to inbox, Not spam, Restore) takes its place.
     * - Delete is always there; in Trash and Spam it is Delete forever.
     * - Mark as spam is in the overflow menu except in Spam, Trash, Sent, Drafts and Outbox, or where an account has no Spam folder.
     * - Move to is the last item of the overflow menu, when [moveAvailable] (everything selected is in one account: the sheet lists
     *   that account's folders).
     */
    fun toolbar(selected: List<MessageRowData>, role: FolderRole, archiveAvailable: Boolean, spamAvailable: Boolean = true, moveAvailable: Boolean = false): SelectionToolbar {
        val primary = ArrayList<SelectionAction>()
        when (role) {
            // T36: in Snoozed, Restore is Unsnooze.
            FolderRole.ARCHIVE, FolderRole.SPAM, FolderRole.TRASH, FolderRole.SNOOZED -> primary += SelectionAction.RESTORE
            FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX -> Unit
            else -> if (archiveAvailable) primary += SelectionAction.ARCHIVE
        }
        primary += if (role == FolderRole.TRASH || role == FolderRole.SPAM) SelectionAction.DELETE_FOREVER else SelectionAction.DELETE
        primary += if (selected.any { !it.seen }) SelectionAction.MARK_READ else SelectionAction.MARK_UNREAD
        val overflow = ArrayList<SelectionAction>()
        overflow += if (selected.all { it.flagged } && selected.isNotEmpty()) SelectionAction.UNSTAR else SelectionAction.STAR
        // Mark as spam is the swipe action that the bar did not have: it goes in the overflow menu, where the swipe's rules put it.
        if (spamAvailable && SwipeRules.available(com.example.croft.swipe.SwipeAction.SPAM, role, hasArchive = true, hasSpam = true)) overflow += SelectionAction.SPAM
        // T36: Snooze, where mail can be put away (not from Snoozed itself, Sent, Drafts, the Outbox, Trash or Spam).
        if (selected.isNotEmpty() && snoozeOffered(role)) overflow += SelectionAction.SNOOZE
        if (moveAvailable && selected.isNotEmpty()) overflow += SelectionAction.MOVE_TO
        return SelectionToolbar(primary, overflow)
    }

    /** T36: Snooze means nothing for mail that is sent, a draft, in the Outbox, deleted, spam or already snoozed. */
    fun snoozeOffered(role: FolderRole?): Boolean = role !in SNOOZE_NOT_IN

    private val SNOOZE_NOT_IN = setOf(FolderRole.SNOOZED, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX, FolderRole.TRASH, FolderRole.SPAM)

    /** More than this many messages in a bulk Archive or Delete ask first (docs/ux/message-list.md). */
    const val CONFIRM_ABOVE = 50

    /** Whether running [action] on [count] messages asks first: Delete forever always does, Archive and Delete above [CONFIRM_ABOVE]. */
    fun needsConfirmation(action: SelectionAction, count: Int): Boolean = when (action) {
        SelectionAction.DELETE_FOREVER -> true
        SelectionAction.ARCHIVE, SelectionAction.DELETE -> count > CONFIRM_ABOVE
        else -> false
    }
}
