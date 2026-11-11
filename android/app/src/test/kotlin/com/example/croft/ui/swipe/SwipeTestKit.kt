// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import com.example.croft.mail.FakeData
import com.example.croft.mail.MessageRowData
import com.example.croft.ui.components.MotionProbes
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.NOW

/** The Inbox's list has 12 dp of padding at each side; the test phone is mdpi, so 1 dp is 1 px. */
const val LIST_LEFT_DP = 12f

/** Where the left edge of the card of message [id] is, in dp from the window's left edge. */
fun ComposeContentTestRule.cardLeft(id: Long): Float = onNodeWithTag(InboxTags.row(id)).getUnclippedBoundsInRoot().left.value

/** How far the card of message [id] is from its resting place, in dp; positive to the right. */
fun ComposeContentTestRule.cardOffset(id: Long): Float = cardLeft(id) - LIST_LEFT_DP

/** The swipe probe [name] (`swipe-offset`, `swipe-collapse`, `swipe-fade`, `swipe-icon-scale`, `swipe-phase`) of the row of [id]. */
fun ComposeContentTestRule.swipeProbe(id: Long, name: String): Float {
    val node = onNode(
        SemanticsMatcher("host of the row $id with probe $name") { it.config.getOrNull(MotionProbes)?.containsKey(name) == true } and
            hasAnyDescendant(hasTestTag(InboxTags.row(id))),
        useUnmergedTree = true,
    )
    return node.fetchSemanticsNode().config[MotionProbes].getValue(name)()
}

/** A made-up message in the folder at [position] of account 1 (see [FakeData]). */
fun message(id: Long, position: Long, seen: Boolean = false, flagged: Boolean = false, minutesAgo: Long = id): MessageRowData =
    MessageRowData(
        id = id, account = 1, folder = FakeData.folderId(1, position), senderName = "Sam Rivera", senderAddress = "sam@example.org",
        subject = "Subject $id", snippet = "Preview of message $id", dateMs = NOW - minutesAgo * FakeData.MINUTE, seen = seen, flagged = flagged,
        hasAttachments = false,
    )
