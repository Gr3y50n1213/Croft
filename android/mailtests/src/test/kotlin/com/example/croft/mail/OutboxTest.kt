// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.ComposeProblem
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.FakeComposeBackend
import com.example.croft.mail.compose.OutboxController
import com.example.croft.mail.compose.OutboxPrompt
import com.example.croft.mail.compose.RecipientField
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.OutboxItem
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.OutboxReason
import uniffi.croft_ffi.outboxReasonKey

/** S26 on the real core (Queued, Edit, Discard) and on the scripted backend (Failed, Sending, "may have been sent"). */
class OutboxTest {
    private val rigs = mutableListOf<Rig>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After
    fun tearDown() {
        scope.cancel()
        rigs.forEach { it.close() }
    }

    private fun item(id: ULong, state: OutboxItemState, reason: OutboxReason? = null, maybe: Boolean = false, at: Long = id.toLong() * 1000) =
        OutboxItem(id, state, at, 1u, reason, maybe, id.toLong(), "Subject $id", 1u, listOf("x@example.org"))

    private fun controller(backend: FakeComposeBackend, changes: suspend (Long) -> ChangeBatch = { kotlinx.coroutines.awaitCancellation() }) =
        OutboxController(backend, { listOf(1L, 2L) }, changes, scope)

    // ---- states ----

    @Test
    fun queuedSendingAndFailedRowsAreListed_withTheirReasons() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(1u, OutboxItemState.QUEUED, OutboxReason.TRYING_AGAIN))
        fake.addOutboxItem(1, item(2u, OutboxItemState.SENDING))
        fake.addOutboxItem(1, item(3u, OutboxItemState.FAILED, OutboxReason.RECIPIENT_REFUSED))
        val c = controller(fake)
        c.refresh()
        val rows = c.state.value.rows
        assertEquals(listOf(OutboxItemState.QUEUED, OutboxItemState.SENDING, OutboxItemState.FAILED), rows.map { it.item.state })
        assertEquals("outbox.reason.recipient_refused", c.reasonKey(rows[2]))
        assertNull(c.reasonKey(rows[1]))
    }

    @Test
    fun everyReasonHasAKeyTheScreensCanTranslate() {
        val keys = OutboxReason.entries.map { outboxReasonKey(it) }
        assertEquals(OutboxReason.entries.size, keys.toSet().size)
        assertTrue(keys.all { it.startsWith("outbox.reason.") })
    }

    // ---- actions ----

    @Test
    fun retryOnAMessageThatMayHaveBeenSentNeedsConfirmation() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(1u, OutboxItemState.FAILED, OutboxReason.MAY_NOT_HAVE_BEEN_SENT, maybe = true))
        val c = controller(fake)
        c.refresh()
        val row = c.state.value.rows.single()

        c.retry(row)
        assertTrue("asks first", c.state.value.prompt is OutboxPrompt.ConfirmRetry)
        assertTrue("the core was not called", fake.calls.none { it.startsWith("retry") })

        c.cancelPrompt()
        c.confirmRetry()
        assertTrue("Cancel leaves it alone, and a confirm without a question does nothing", fake.calls.none { it.startsWith("retry") })

        c.retry(row)
        c.confirmRetry()
        assertEquals("only now, and with confirmed = true", listOf("retryConfirmed"), fake.calls.filter { it.startsWith("retry") })
        assertEquals(OutboxItemState.QUEUED, c.state.value.rows.single().item.state)
    }

    @Test
    fun retryOnAnOrdinaryFailedMessageGoesAtOnceUnconfirmed() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(1u, OutboxItemState.FAILED, OutboxReason.GAVE_UP))
        val c = controller(fake)
        c.refresh()
        c.retry(c.state.value.rows.single())
        assertNull(c.state.value.prompt)
        assertEquals(listOf("retry"), fake.calls.filter { it.startsWith("retry") })
    }

    @Test
    fun ifTheCoreAsksForConfirmationAnywayTheScreenAsks() = runBlocking {
        val fake = FakeComposeBackend()
        // The list said "ordinary", the core knows better by now.
        fake.addOutboxItem(1, item(1u, OutboxItemState.FAILED, OutboxReason.OTHER, maybe = false))
        fake.failRetryWith = ComposeProblem.NEEDS_CONFIRMATION
        val c = controller(fake)
        c.refresh()
        c.retry(c.state.value.rows.single())
        assertTrue(c.state.value.prompt is OutboxPrompt.ConfirmRetry)
        assertEquals(listOf("retry"), fake.calls.filter { it.startsWith("retry") })
    }

    @Test
    fun discardAsksFirst() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(1u, OutboxItemState.FAILED, OutboxReason.OTHER))
        val c = controller(fake)
        c.refresh()
        c.discard(c.state.value.rows.single())
        assertTrue(c.state.value.prompt is OutboxPrompt.ConfirmDiscard)
        assertTrue(fake.calls.none { it == "discardOutbox" })
        c.cancelPrompt()
        assertTrue(fake.calls.none { it == "discardOutbox" })
        c.discard(c.state.value.rows.single())
        c.confirmDiscard()
        assertEquals(listOf("discardOutbox"), fake.calls.filter { it == "discardOutbox" })
        assertTrue(c.state.value.rows.isEmpty())
    }

    @Test
    fun aMessageBeingSentCannotBeEditedOrDiscarded() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(1u, OutboxItemState.SENDING))
        val c = controller(fake)
        c.refresh()
        val row = c.state.value.rows.single()
        c.discard(row)
        assertNull("no question: it is not offered", c.state.value.prompt)
        assertNull(c.edit(row))
        assertEquals(ComposeProblem.WRONG_STATE, c.state.value.problem)
    }

    @Test
    fun editGoesBackToTheDraft() = runBlocking {
        val fake = FakeComposeBackend()
        fake.addOutboxItem(1, item(7u, OutboxItemState.FAILED, OutboxReason.MESSAGE_REFUSED))
        val c = controller(fake)
        c.refresh()
        val start = c.edit(c.state.value.rows.single())
        assertEquals(ComposeStart.Draft(1, 7), start)
        assertTrue("it left the Outbox", c.state.value.rows.isEmpty())
    }

    // ---- refresh from the change notices ----

    @Test
    fun theListFollowsTheCoresChangeNotices() {
        val fake = FakeComposeBackend()
        val batches = Channel<ChangeBatch>(Channel.UNLIMITED)
        val c = controller(fake) { after -> batches.receive().also { assert(it.seq >= after) } }
        val job: Job = scope.launch { c.run() }
        await("first load") { c.state.value.takeIf { it.loaded } }
        assertTrue(c.state.value.rows.isEmpty())

        fake.addOutboxItem(2, item(1u, OutboxItemState.QUEUED))
        // A notice for another account: nothing re-read for account 2.
        batches.trySend(ChangeBatch(seq = 1, accounts = listOf(1)))
        Thread.sleep(300)
        assertTrue("account 2 was not named", c.state.value.rows.isEmpty())
        batches.trySend(ChangeBatch(seq = 2, accounts = listOf(2)))
        await("the row of account 2") { c.state.value.rows.takeIf { it.size == 1 } }

        fake.addOutboxItem(1, item(2u, OutboxItemState.QUEUED))
        batches.trySend(ChangeBatch(seq = 3, everything = true))
        await("everything re-read") { c.state.value.rows.takeIf { it.size == 2 } }
        job.cancel()
    }

    // ---- on the real core ----

    @Test(timeout = 60_000)
    fun aSendShowsAsQueued_editReturnsTheDraft_discardAsksAndDrops() {
        val r = Rig(messages = 3, undoWindowMs = 5_000, smtp = true).also { rigs += it }
        val h = ComposeHarness(r)
        val c = h.open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("First"); c.onBody("Body"); c.requestSend() }
        val outbox = OutboxController(r.compose, { listOf(r.account) }, { r.repo.nextChange(it) }, scope)
        r.run { outbox.refresh() }
        val row = outbox.state.value.rows.single()
        assertEquals(OutboxItemState.QUEUED, row.item.state)
        assertEquals("First", row.item.subject)

        val start = r.run { outbox.edit(row) }!!
        assertEquals(0, outbox.state.value.rows.size)
        val reopened = h.open(start)
        assertEquals("First", reopened.state.value.subject)
        r.run { reopened.requestSend() }

        r.run { outbox.refresh() }
        val again = outbox.state.value.rows.single()
        outbox.discard(again)
        assertNotNull(outbox.state.value.prompt)
        assertEquals("still there until confirmed", 1, r.run { r.compose.listOutbox(r.account) }.size)
        r.run { outbox.confirmDiscard() }
        assertEquals(0, r.run { r.compose.listOutbox(r.account) }.size)
        assertEquals("the draft went with it", emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })
        h.close()
    }

    @Test(timeout = 60_000)
    fun theOutboxListFollowsTheCoresNotices() {
        val r = Rig(messages = 3, undoWindowMs = 5_000, smtp = true).also { rigs += it }
        val outbox = OutboxController(r.compose, { listOf(r.account) }, { r.repo.nextChange(it) }, scope)
        val job = scope.launch { outbox.run() }
        await("loaded") { outbox.state.value.takeIf { it.loaded } }
        val h = ComposeHarness(r)
        val c = h.open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("Live"); c.onBody("B"); c.requestSend() }
        await("the new row, by the core's own notice") { outbox.state.value.rows.takeIf { it.size == 1 } }
        job.cancel()
        h.close()
    }
}
