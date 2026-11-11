// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.Dp
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.AnchorStore
import com.example.croft.mail.FakeData
import com.example.croft.mail.FakeMessageRepository
import com.example.croft.mail.InMemoryAnchorStore
import com.example.croft.mail.MailSession
import com.example.croft.mail.MessageScope
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.isInteractive
import com.example.croft.ui.components.show
import java.time.ZoneId

/** The fixed "now" of the mail tests: times, headers and screenshots do not depend on the day the tests run. */
const val NOW = 1_800_000_000_000L

/**
 * A mailbox for a test: a fake repository with [rows] made-up rows (the newest six are the wireframe rows), an in-memory
 * anchor store (or [anchors]), a fixed clock and UTC. The top row of the unified Inbox has the id [rows].
 */
class InboxFixture(
    val rows: Int = 300,
    val anchors: AnchorStore = InMemoryAnchorStore(),
    data: List<com.example.croft.mail.MessageRowData> = FakeData.rows(rows, NOW),
) {
    var clockMs = NOW
    val repo = FakeMessageRepository(data, { clockMs }, FakeData.accounts(NOW))
    val session = MailSession(repo, anchors, { clockMs }, ZoneId.of("UTC"))
    val haptics = RecordingHaptics()

    /** The ids of the unified Inbox, newest first, as the repository holds them now. */
    fun inboxIds(): List<Long> = repo.snapshot(MessageScope.UnifiedInbox).map { it.id }

    /** The id of the newest message of the unified Inbox. */
    val topId: Long get() = inboxIds().first()
}

/** Show the Inbox of [f] under the app theme at [level]; every haptic goes to the fixture's recorder. */
fun ComposeContentTestRule.showInbox(
    f: InboxFixture,
    level: AnimationLevel = AnimationLevel.FULL,
    onSettings: () -> Unit = {},
    startDrawerOpen: Boolean = false,
    startSwitcherOpen: Boolean = false,
) {
    show(level, f.haptics) { InboxScreen(f.session, onOpenSettings = onSettings, startDrawerOpen = startDrawerOpen, startSwitcherOpen = startSwitcherOpen) }
}

/** Wait until the row of message [id] is on screen. */
fun ComposeContentTestRule.waitForRow(id: Long, timeoutMs: Long = 10_000) {
    waitUntil(timeoutMs) { onAllNodes(hasTestTag(InboxTags.row(id))).fetchSemanticsNodes().isNotEmpty() }
}

/** The tag of any row. */
val isAnyRow: SemanticsMatcher = SemanticsMatcher("is a message row") { it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("row-") == true }

/** The number of rows that are composed right now (the lazy list composes only what is on screen, plus a margin). */
fun ComposeContentTestRule.composedRows(): Int = onAllNodes(isAnyRow).fetchSemanticsNodes().size

/** The interactive card of the row of message [id] (not the layout box that carries the tag). */
fun ComposeContentTestRule.rowCard(id: Long): SemanticsNodeInteraction = onNode(isInteractive and hasAnyAncestor(hasTestTag(InboxTags.row(id))))

/** The row's top edge in dp from the top of the window, unclipped (a row scrolled partly away has a negative value). */
fun ComposeContentTestRule.rowTop(id: Long): Dp = onNodeWithTag(InboxTags.row(id)).getUnclippedBoundsInRoot().top

/** Scroll the list so that the row of [id] is composed (it ends up at the top). */
fun ComposeContentTestRule.scrollToRow(id: Long) {
    onNodeWithTag(InboxTags.LIST).performScrollToNode(hasTestTag(InboxTags.row(id)))
    waitForIdle()
}

/** What a screen reader says for the row of [id]. */
fun ComposeContentTestRule.spoken(id: Long): String = rowCard(id).fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
