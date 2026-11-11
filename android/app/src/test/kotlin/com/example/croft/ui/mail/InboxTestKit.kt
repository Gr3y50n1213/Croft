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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.FolderId
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.OpId
import com.example.croft.mail.RepositoryRowActions
import com.example.croft.mail.RowActions
import com.example.croft.swipe.SwipeSettings
import com.example.croft.ui.swipe.LocalBackGestureInset
import com.example.croft.ui.swipe.LocalSwipeSettings
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

    /** Every call a swipe, a menu or a custom action makes on the rows goes through here, in order. */
    val actions = RecordingRowActions(RepositoryRowActions(repo))
    val session = MailSession(repo, anchors, { clockMs }, ZoneId.of("UTC"), actions)
    val haptics = RecordingHaptics()

    /** The ids of the unified Inbox, newest first, as the repository holds them now. */
    fun inboxIds(): List<Long> = repo.snapshot(MessageScope.UnifiedInbox).map { it.id }

    /** The id of the newest message of the unified Inbox. */
    val topId: Long get() = inboxIds().first()
}

/**
 * Show the Inbox of [f] under the app theme at [level]; every haptic goes to the fixture's recorder. [swipe] is read in the
 * composition (it may read a state, so a test can change the settings while the screen is up); a test window has no system gesture
 * strips, so a 24 dp strip is put at each edge.
 */
fun ComposeContentTestRule.showInbox(
    f: InboxFixture,
    level: AnimationLevel = AnimationLevel.FULL,
    onSettings: () -> Unit = {},
    startDrawerOpen: Boolean = false,
    startSwitcherOpen: Boolean = false,
    startScope: MessageScope = MessageScope.UnifiedInbox,
    swipe: () -> SwipeSettings = { SwipeSettings.Default },
    onOpenMessage: (Long) -> Unit = {},
) {
    show(level, f.haptics) {
        CompositionLocalProvider(LocalSwipeSettings provides swipe(), LocalBackGestureInset provides 24.dp) {
            InboxScreen(f.session, onOpenSettings = onSettings, startScope = startScope, startDrawerOpen = startDrawerOpen, startSwitcherOpen = startSwitcherOpen, onOpenMessage = onOpenMessage)
        }
    }
}

/** A [RowActions] that remembers what it was asked, in order, and does it through [inner]. */
class RecordingRowActions(private val inner: RowActions) : RowActions {
    private val log = java.util.concurrent.CopyOnWriteArrayList<String>()

    /** "archive 7", "trash 7", "markSeen 7 true", ... in call order. */
    val calls: List<String> get() = log.toList()

    override suspend fun markSeen(row: MessageRowData, seen: Boolean) { log += "markSeen ${row.id} $seen"; inner.markSeen(row, seen) }
    override suspend fun setFlagged(row: MessageRowData, flagged: Boolean) { log += "setFlagged ${row.id} $flagged"; inner.setFlagged(row, flagged) }
    override suspend fun archive(row: MessageRowData): List<OpId> { log += "archive ${row.id}"; return inner.archive(row) }
    override suspend fun trash(row: MessageRowData): List<OpId> { log += "trash ${row.id}"; return inner.trash(row) }
    override suspend fun markSpam(row: MessageRowData): List<OpId> { log += "markSpam ${row.id}"; return inner.markSpam(row) }
    override suspend fun deleteForever(row: MessageRowData): List<OpId> { log += "deleteForever ${row.id}"; return inner.deleteForever(row) }
    override suspend fun moveTo(row: MessageRowData, folder: FolderId): List<OpId> { log += "moveTo ${row.id} $folder"; return inner.moveTo(row, folder) }
    override suspend fun undo(ops: List<OpId>) { log += "undo ${ops.size}"; inner.undo(ops) }
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
