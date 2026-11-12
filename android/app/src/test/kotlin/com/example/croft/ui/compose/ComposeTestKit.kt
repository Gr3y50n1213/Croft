// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.hasText
import com.example.croft.mail.MailSession
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.EncryptionPolicy
import com.example.croft.mail.compose.FakeComposeBackend
import com.example.croft.mail.compose.NoEncryptionPolicy
import com.example.croft.mail.compose.SimpleAddressTools
import com.example.croft.ui.mail.InboxFixture
import java.time.ZoneId
import uniffi.croft_ffi.OutboxItem
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.OutboxReason

/** A mailbox with the compose calls on an in-memory backend: the Inbox rows of [InboxFixture], accounts 1 to 3, and a [FakeComposeBackend]. */
class ComposeFixture(
    val policy: EncryptionPolicy = NoEncryptionPolicy,
    backend: FakeComposeBackend? = null,
    rows: Int = 30,
) {
    val inbox = InboxFixture(rows)

    /** On the fixture's fixed clock, so that the end of an Undo window is a known time after "now". */
    val backend: FakeComposeBackend = backend ?: FakeComposeBackend(clock = { inbox.clockMs })
    val session = MailSession(
        inbox.repo, inbox.anchors, { inbox.clockMs }, ZoneId.of("UTC"), inbox.actions,
        compose = backend, encryptionPolicy = policy,
    )
    val closed = mutableListOf<ComposeClosed>()
}

/** An Outbox item as the core lists it. */
fun outboxItem(id: ULong, state: OutboxItemState, reason: OutboxReason? = null, maybe: Boolean = false, subject: String = "Subject $id") =
    OutboxItem(id, state, 1_800_000_000_000L + id.toLong() * 1000, 1u, reason, maybe, id.toLong(), subject, 1u, listOf("x@example.org"))

/** Wait until [text] shows anywhere. */
fun ComposeContentTestRule.waitForText(text: String, timeoutMs: Long = 10_000) {
    waitUntil(timeoutMs) { onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }
}

/** The text in the body field (S23). */
fun ComposeContentTestRule.bodyText(): String =
    onNodeWithTag(ComposeTags.BODY).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

fun ComposeContentTestRule.subjectText(): String =
    onNodeWithTag(ComposeTags.SUBJECT).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

fun SemanticsNodeInteraction.selection() = fetchSemanticsNode().config[SemanticsProperties.TextSelectionRange]

/** The address tools of the UI tests: no native library needed. */
val UiTestAddressTools = SimpleAddressTools
