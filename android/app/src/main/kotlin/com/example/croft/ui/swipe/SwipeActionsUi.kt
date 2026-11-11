// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import android.content.res.Resources
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Star
import androidx.compose.ui.graphics.vector.ImageVector
import com.example.croft.R
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageRowData
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSlot
import com.example.croft.ui.components.CroftIcons

/**
 * Which swipe actions a row can do where it is (docs/ux/swipe.md, "Blocked and special cases"), as a pure function so a
 * table of folders and the toolbar rules can be compared with it. The rules are the toolbar enablement rules of
 * docs/ux/controls.md: the swipe, the toolbar, the menu and the keyboard cannot disagree.
 */
object SwipeRules {
    private val outgoing = setOf(FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX)

    /**
     * Whether [action] can run for a row in a folder of [role]. Archive: not in Archive, Sent, Drafts, Outbox, Spam, Trash,
     * and only where the account has an Archive folder. Mark as spam: not in Spam, Sent, Drafts, Outbox, Trash, and only
     * where the account has a Spam folder. Everything else is always available.
     */
    fun available(action: SwipeAction, role: FolderRole, hasArchive: Boolean, hasSpam: Boolean): Boolean = when (action) {
        SwipeAction.ARCHIVE -> hasArchive && role !in outgoing && role != FolderRole.ARCHIVE && role != FolderRole.SPAM && role != FolderRole.TRASH
        SwipeAction.SPAM -> hasSpam && role !in outgoing && role != FolderRole.SPAM && role != FolderRole.TRASH
        else -> true
    }

    /** Delete in Trash and Spam deletes forever and asks first: the card parks behind the confirmation. */
    fun deletesForever(action: SwipeAction, role: FolderRole): Boolean = action == SwipeAction.DELETE && (role == FolderRole.TRASH || role == FolderRole.SPAM)

    /** The slot the machine gets for [action] in [role]. */
    fun slot(action: SwipeAction, role: FolderRole, hasArchive: Boolean, hasSpam: Boolean): SwipeSlot =
        SwipeSlot(action, available(action, role, hasArchive, hasSpam), parks = action.parks || deletesForever(action, role))
}

/**
 * The icon in the pill: it shows what the swipe will do. Mark read on an unread row is the open envelope, on a read row the
 * closed envelope; Star on an unstarred row the outline, on a starred row the filled star (the swipe removes it).
 */
fun swipeIcon(action: SwipeAction, row: MessageRowData?): ImageVector = when (action) {
    SwipeAction.TOGGLE_READ -> if (row == null || !row.seen) CroftIcons.EnvelopeOpen else Icons.Filled.Email
    SwipeAction.DELETE -> Icons.Filled.Delete
    SwipeAction.ARCHIVE -> CroftIcons.Archive
    SwipeAction.SNOOZE -> CroftIcons.Schedule
    SwipeAction.MOVE_TO -> CroftIcons.Folder
    SwipeAction.TOGGLE_STAR -> if (row != null && row.flagged) Icons.Filled.Star else CroftIcons.StarOutline
    SwipeAction.SPAM -> CroftIcons.Shield
    SwipeAction.NONE -> CroftIcons.Dash
}

/** The words for [action] on [row] (what a screen reader hears as the custom action, and the menu label). */
fun swipeLabel(res: Resources, action: SwipeAction, row: MessageRowData?, role: FolderRole): String = res.getString(
    when (action) {
        SwipeAction.TOGGLE_READ -> if (row != null && row.seen) R.string.mail_action_mark_unread else R.string.mail_action_mark_read
        SwipeAction.DELETE -> if (SwipeRules.deletesForever(action, role)) R.string.mail_action_delete_forever else R.string.mail_action_delete
        SwipeAction.ARCHIVE -> R.string.mail_action_archive
        SwipeAction.SNOOZE -> R.string.swipe_action_snooze
        SwipeAction.MOVE_TO -> R.string.swipe_action_move_to
        SwipeAction.TOGGLE_STAR -> if (row != null && row.flagged) R.string.mail_action_unstar else R.string.mail_action_star
        SwipeAction.SPAM -> R.string.mail_action_spam
        SwipeAction.NONE -> R.string.swipe_action_none
    },
)

/** The name of [action] in S47 and S48, which does not depend on a row: Mark read or unread, Star. */
fun swipeSettingName(res: Resources, action: SwipeAction): String = res.getString(
    when (action) {
        SwipeAction.TOGGLE_READ -> R.string.swipe_name_read
        SwipeAction.DELETE -> R.string.mail_action_delete
        SwipeAction.ARCHIVE -> R.string.mail_action_archive
        SwipeAction.SNOOZE -> R.string.swipe_action_snooze
        SwipeAction.MOVE_TO -> R.string.swipe_action_move_to
        SwipeAction.TOGGLE_STAR -> R.string.swipe_name_star
        SwipeAction.SPAM -> R.string.mail_action_spam
        SwipeAction.NONE -> R.string.swipe_action_none
    },
)
