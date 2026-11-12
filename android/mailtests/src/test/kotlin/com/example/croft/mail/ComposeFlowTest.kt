// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.AutosaveTiming
import com.example.croft.mail.compose.BodyCursor
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeFailure
import com.example.croft.mail.compose.ComposeProblem
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.ComposeState
import com.example.croft.mail.compose.EncryptionPolicy
import com.example.croft.mail.compose.RecipientField
import com.example.croft.mail.compose.RequiredKeyMissingPolicy
import com.example.croft.mail.compose.SaveStatus
import com.example.croft.mail.compose.SendPrompt
import com.example.croft.mail.compose.SendRules
import com.example.croft.mail.compose.SendState
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.DraftProblem
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.OutboxItemState

/**
 * T18 Part B "Done when", on the real Rust core with a real in-process IMAP server: reply, reply all and forward fill the right fields;
 * the Send button's condition table, one test per row; autosave survives a simulated crash (kill after 1 s: the local draft is restored;
 * after 5 s: the server has the draft); Undo returns to the draft with the cursor; the fake encryption policy never sends silently.
 * The account has an SMTP server on a port nobody listens on, so a send is accepted, waits its Undo window and is then retried: the
 * tests look only at what the core says about the Outbox.
 */
class ComposeFlowTest {
    private val rigs = mutableListOf<Rig>()
    private val harnesses = mutableListOf<ComposeHarness>()

    private fun rig(window: Long = 1_000): Rig = Rig(messages = 3, undoWindowMs = window, smtp = true).also { rigs += it }

    private fun harness(r: Rig) = ComposeHarness(r).also { harnesses += it }

    @After
    fun tearDown() {
        harnesses.forEach { it.close() }
        rigs.forEach { it.close() }
    }

    private fun addr(a: String) = FfiAddress(null, a)

    // ---- reply, reply all, forward ----

    @Test(timeout = 60_000)
    fun replyReplyAllAndForwardFillTheRightFields() {
        val r = rig()
        val row = r.awaitInbox(3).first()
        val h = harness(r)

        val reply = h.open(ComposeStart.Reply(r.account, row.id)).state.value
        assertEquals("To is the sender", listOf("ann@example.org"), reply.to.map { it.address.address })
        assertTrue("Re: once", reply.subject.startsWith("Re: ") && !reply.subject.startsWith("Re: Re:"))
        assertEquals(row.subject, reply.subject.removePrefix("Re: "))
        assertTrue(reply.cc.isEmpty() && reply.bcc.isEmpty())
        assertNull("nothing is stored until the person writes", reply.draftId)
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })
        assertEquals(BodyCursor(0, 0), reply.initialCursor)

        val all = h.open(ComposeStart.ReplyAll(r.account, row.id)).state.value
        assertEquals(listOf("ann@example.org"), all.to.map { it.address.address })
        assertFalse("never the person's own address", (all.to + all.cc).any { it.address.address == "alice@example.org" })

        val forward = h.open(ComposeStart.Forward(r.account, row.id)).state.value
        assertTrue("Fwd: once", forward.subject.startsWith("Fwd: "))
        assertEquals(row.subject, forward.subject.removePrefix("Fwd: "))
        assertTrue("a forward has no recipient yet", forward.to.isEmpty())
        assertEquals(SendState.NO_RECIPIENT, forward.sendState)
    }

    @Test(timeout = 60_000)
    fun aReplyThatIsLeftUntouchedLeavesNoDraft() {
        val r = rig()
        val row = r.awaitInbox(3).first()
        val c = harness(r).open(ComposeStart.Reply(r.account, row.id))
        r.run { c.back() }
        assertEquals(ComposeClosed.Nothing, c.state.value.closed)
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })
    }

    // ---- the Send button, row by row (docs/ux/controls.md) ----

    private fun stateAfter(r: Rig, build: suspend (com.example.croft.mail.compose.ComposeController) -> Unit): ComposeState {
        val c = harness(r).open(ComposeStart.New(r.account))
        r.run { build(c); c.flush() }
        return c.state.value
    }

    @Test(timeout = 60_000)
    fun sendRow_noRecipient_isDisabled() {
        val r = rig()
        val s = stateAfter(r) { it.onSubject("Hi"); it.onBody("Body") }
        assertEquals(SendState.NO_RECIPIENT, s.sendState)
        assertTrue(SendRules.isDisabled(s.sendState))
    }

    @Test(timeout = 60_000)
    fun sendRow_anInvalidRecipient_isDisabledAndTheChipIsAnErrorChip() {
        val r = rig()
        val s = stateAfter(r) { it.commitRecipients(RecipientField.TO, "ann@example.org, not-an-address"); it.onBody("Body") }
        assertEquals(SendState.INVALID_RECIPIENT, s.sendState)
        assertEquals(listOf(true, false), s.to.map { it.valid })
        assertTrue(SendRules.isDisabled(s.sendState))
    }

    @Test(timeout = 60_000)
    fun sendRow_recipientButEmptySubjectAndBody_isEnabledAndAsks() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org") }
        assertEquals(SendState.EMPTY_CONFIRM, c.state.value.sendState)
        assertFalse(SendRules.isDisabled(SendState.EMPTY_CONFIRM))
        r.run { c.requestSend() }
        assertEquals(SendPrompt.EmptyConfirm, c.state.value.prompt)
        assertEquals("nothing went out", 0, r.run { r.compose.listOutbox(r.account) }.size)
        r.run { c.confirmEmpty() }
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(1, r.run { r.compose.listOutbox(r.account) }.size)
    }

    @Test(timeout = 60_000)
    fun sendRow_everythingPresent_isReady_andSendsToTheOutbox() {
        val r = rig(window = 5_000)
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("Lunch"); c.onBody("Noon?", BodyCursor(5, 5)); c.flush() }
        assertEquals(SendState.READY, c.state.value.sendState)
        r.run { c.requestSend() }
        val closed = c.state.value.closed as ComposeClosed.Sent
        assertEquals(BodyCursor(5, 5), closed.cursor)
        val item = r.run { r.compose.listOutbox(r.account) }.single()
        assertEquals(OutboxItemState.QUEUED, item.state)
        assertEquals(1u, item.recipientCount)
        assertEquals("Lunch", item.subject)
    }

    @Test(timeout = 60_000)
    fun sendRow_attachmentsOverTheLimit_isDisabled() {
        val r = rig()
        val c = harness(r).open()
        r.run {
            c.commitRecipients(RecipientField.TO, "ann@example.org")
            c.onSubject("Big")
            // 26 MB: accepted by the core (it keeps what is under 40 MB) but over the 25 MB at which Send is disabled.
            c.addFromFile(MemoryFile(ByteArray(26 * 1024 * 1024)))
        }
        val s = c.state.value
        assertEquals(1, s.attachments.size)
        assertEquals(SendState.OVER_SIZE_LIMIT, s.sendState)
        assertEquals(25, s.sizeLimitMb)
        r.run { c.requestSend() }
        assertTrue("a tap only explains", c.state.value.prompt is SendPrompt.Explain)
        assertEquals(0, r.run { r.compose.listOutbox(r.account) }.size)
    }

    @Test(timeout = 60_000)
    fun sendRow_attachmentStillUploading_isEnabledAndNeverBlocks() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("S"); c.flush() }
        val check = r.run { r.compose.checkDraft(r.account, c.state.value.draftId!!) }
        assertTrue(check.problems.isEmpty())
        assertEquals(SendState.UPLOADING, SendRules.stateOf(check, uploading = true))
        assertFalse(SendRules.isDisabled(SendState.UPLOADING))
    }

    @Test(timeout = 60_000)
    fun sendRow_offlineOrNeedsSignIn_isEnabledAndQueues() {
        val r = rig()
        for (status in listOf(AccountState.OFFLINE, AccountState.NEEDS_SIGN_IN)) {
            val c = harness(r).controller(accounts = { listOf(AccountInfo(r.account, "Alice", "alice@example.org", status)) })
            r.run { c.open(ComposeStart.New(r.account)); c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("S"); c.onBody("B"); c.flush() }
            assertEquals(status.name, SendState.OFFLINE_OR_SIGN_IN, c.state.value.sendState)
            assertFalse(SendRules.isDisabled(c.state.value.sendState))
        }
    }

    @Test(timeout = 60_000)
    fun sendRow_tapped_isSendingThenClosed() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("S"); c.onBody("B"); c.flush() }
        assertEquals(SendState.READY, c.state.value.sendState)
        r.run { c.requestSend() }
        assertEquals(ComposeState.Phase.CLOSED, c.state.value.phase)
        assertTrue(SendRules.isDisabled(SendState.SENDING))
    }

    @Test(timeout = 60_000)
    fun sendRow_theTableAsAFunction_everyRowIsReachable() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("S"); c.flush() }
        val ok = r.run { r.compose.checkDraft(r.account, c.state.value.draftId!!) }
        assertEquals(SendState.READY, SendRules.stateOf(ok))
        assertEquals(SendState.ENCRYPTION_NO_KEY, SendRules.stateOf(ok, encryptionWarning = true))
        assertEquals(SendState.OFFLINE_OR_SIGN_IN, SendRules.stateOf(ok, offlineOrSignIn = true))
        assertEquals(SendState.NO_RECIPIENT, SendRules.stateOf(ok.copy(problems = listOf(DraftProblem.NO_RECIPIENT, DraftProblem.INVALID_RECIPIENT)), encryptionWarning = true))
        assertEquals(SendState.OVER_SIZE_LIMIT, SendRules.stateOf(ok.copy(problems = listOf(DraftProblem.ATTACHMENTS_TOO_LARGE)), uploading = true))
        assertEquals(SendState.EMPTY_CONFIRM, SendRules.stateOf(ok.copy(subjectEmpty = true, bodyEmpty = true)))
    }

    @Test(timeout = 60_000)
    fun aMailtoOrShareFill_hasItsSendButtonBeforeAnythingIsTypedOrSaved() {
        val r = rig()
        val ok = harness(r).open(ComposeStart.Fill(com.example.croft.mail.compose.ComposeFill(to = listOf("ann@example.org"), subject = "Hi", body = "Text"), r.account)).state.value
        assertEquals(SendState.READY, ok.sendState)
        assertNull("nothing stored until the person writes", ok.draftId)
        val bad = harness(r).open(ComposeStart.Fill(com.example.croft.mail.compose.ComposeFill(to = listOf("ann@example.org", "nope")), r.account)).state.value
        assertEquals(SendState.INVALID_RECIPIENT, bad.sendState)
        val bare = harness(r).open(ComposeStart.Fill(com.example.croft.mail.compose.ComposeFill(to = listOf("ann@example.org")), r.account)).state.value
        assertEquals(SendState.EMPTY_CONFIRM, bare.sendState)
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })
        // A fill from a link or a share is content: Back keeps it as a draft even when nothing was typed (the review's L6). A new compose and
        // a reply that were left alone leave nothing behind (see the tests above).
        val c = harness(r).open(ComposeStart.Fill(com.example.croft.mail.compose.ComposeFill(to = listOf("ann@example.org"), subject = "Hi"), r.account))
        r.run { c.back() }
        assertEquals(ComposeClosed.DraftSaved, c.state.value.closed)
        assertEquals("Hi", r.run { r.compose.listDrafts(r.account) }.single().subject)
    }

    // ---- autosave survives a crash ----

    @Test(timeout = 90_000)
    fun killAfterOneSecond_theLocalDraftIsRestored_andAfterFiveSecondsTheServerHasIt() {
        val r = rig()
        r.awaitInbox(3)
        val h = harness(r)
        val c = h.open(ComposeStart.New(r.account), timing = AutosaveTiming(localMs = 1_000, serverIdleMs = 5_000, serverMinIntervalMs = 20_000))
        c.onSubject("Crash test subject")
        c.onBody("Written before the crash", BodyCursor(7, 7))
        // The device copy is made 1 s after typing stops; before that the core knows nothing.
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })
        val draftId = await("the draft on the device", 10_000) { r.run { r.compose.listDrafts(r.account) }.firstOrNull() }.id
        assertEquals("not on the server yet", emptyList<String>(), r.server.subjects("Drafts"))

        // The app dies here: the core is closed with whatever it holds and opened again on the same database.
        h.scope.coroutineContext[kotlinx.coroutines.Job]!!.children.forEach { it.cancel() }
        r.killAndReopen()
        val restored = r.run { r.compose.loadDraft(r.account, draftId) }
        assertEquals("Crash test subject", restored.content.subject)
        assertEquals("Written before the crash", restored.content.body)

        // Reopened in compose, and left alone for 5 s: the server has the draft.
        val c2 = harness(r).open(ComposeStart.Draft(r.account, draftId))
        assertEquals("Written before the crash", c2.state.value.body)
        c2.onBody("Written before the crash, and a bit more", BodyCursor(40, 40))
        await("the draft on the server", 30_000) { r.server.subjects("Drafts").takeIf { it == listOf("Crash test subject") } }
        assertEquals(1, r.server.subjects("Drafts").size)
        // The reopened core copies the device draft to the server at start by itself, and the core paces copies of one draft
        // to one per 20 s: the screen's own copy can be deferred until that interval ends, so "Saved" may take up to ~20 s
        // after the server already has the draft. The wait covers that interval with room for a slow CI machine.
        await("the quiet status", 45_000) { c2.state.value.saveStatus.takeIf { it == SaveStatus.SAVED } }
    }

    @Test(timeout = 60_000)
    fun theAppPausing_copiesTheDraftToTheServerAtOnce() {
        val r = rig()
        r.awaitInbox(3)
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("Paused subject"); c.onBody("Text") }
        r.run { c.flush() }
        await("the draft on the server") { r.server.subjects("Drafts").takeIf { it == listOf("Paused subject") } }
    }

    // ---- Back rules ----

    @Test(timeout = 60_000)
    fun backOnAnEmptyComposeClosesWithoutADraft_andOnContentSavesIt() {
        val r = rig()
        val empty = harness(r).open()
        r.run { empty.back() }
        assertEquals(ComposeClosed.Nothing, empty.state.value.closed)
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })

        // Typed and then emptied again: the draft that was saved is discarded.
        val emptied = harness(r).open()
        r.run { emptied.commitRecipients(RecipientField.TO, "ann@example.org") }
        assertEquals(1, r.run { r.compose.listDrafts(r.account) }.size)
        r.run { emptied.removeRecipient(RecipientField.TO, emptied.state.value.to.single()); emptied.back() }
        assertEquals(ComposeClosed.Nothing, emptied.state.value.closed)
        assertEquals(emptyList<Any>(), r.run { r.compose.listDrafts(r.account) })

        val full = harness(r).open()
        r.run { full.onBody("Something worth keeping") }
        r.run { full.back() }
        assertEquals(ComposeClosed.DraftSaved, full.state.value.closed)
        assertEquals("Something worth keeping", r.run { r.compose.listDrafts(r.account) }.single().snippet)
    }

    // ---- Undo, with the cursor ----

    @Test(timeout = 60_000)
    fun undoInsideTheWindowReturnsToTheDraftWithTheCursorWhereItWas() {
        val r = rig(window = 5_000)
        val h = harness(r)
        val c = h.open()
        r.run { c.commitRecipients(RecipientField.TO, "ann@example.org"); c.onSubject("Undo me"); c.onBody("Hello there, friend", BodyCursor(5, 11)); c.requestSend() }
        val sent = c.state.value.closed as ComposeClosed.Sent
        assertEquals(1, r.run { r.compose.listOutbox(r.account) }.size)
        // What the Inbox's Undo snackbar does.
        val draftId = r.run { r.compose.undoSend(sent.account, sent.outboxId) }
        assertEquals("nothing is in the Outbox any more", 0, r.run { r.compose.listOutbox(r.account) }.size)
        val reopened = h.open(ComposeStart.Draft(sent.account, draftId, sent.cursor))
        assertEquals("Hello there, friend", reopened.state.value.body)
        assertEquals(BodyCursor(5, 11), reopened.state.value.initialCursor)
        assertEquals("Undo me", reopened.state.value.subject)
        // And from the session's memory alone (no cursor passed):
        val again = h.open(ComposeStart.Draft(sent.account, draftId))
        assertEquals(BodyCursor(5, 11), again.state.value.initialCursor)
    }

    // ---- the encryption seam ----

    @Test(timeout = 60_000)
    fun withTheFakePolicyANoKeyRecipientNeverGoesSilently() {
        val r = rig()
        val c = harness(r).open(policy = RequiredKeyMissingPolicy(listOf("dan@oldmail.org")))
        r.run { c.commitRecipients(RecipientField.TO, "dan@oldmail.org"); c.onSubject("Secret"); c.onBody("Body"); c.flush() }
        assertEquals("the warning shows on the button", SendState.ENCRYPTION_NO_KEY, c.state.value.sendState)

        r.run { c.requestSend() }
        assertEquals(SendPrompt.EncryptionCheck(listOf("dan@oldmail.org")), c.state.value.prompt)
        assertEquals("no send without a choice", 0, r.run { r.compose.listOutbox(r.account) }.size)
        assertNull(c.state.value.closed)

        // A stray "confirm empty" is not an answer to the encryption question.
        r.run { c.confirmEmpty() }
        assertEquals(0, r.run { r.compose.listOutbox(r.account) }.size)

        // Cancel: still nothing, compose stays open.
        c.cancelPrompt()
        assertNull(c.state.value.prompt)
        assertEquals(0, r.run { r.compose.listOutbox(r.account) }.size)
        assertEquals(ComposeState.Phase.EDITING, c.state.value.phase)

        // Ask again and choose Send unencrypted: now it goes, once.
        r.run { c.requestSend() }
        assertNotNull(c.state.value.prompt)
        r.run { c.confirmUnencrypted() }
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(1, r.run { r.compose.listOutbox(r.account) }.size)
    }

    @Test(timeout = 60_000)
    fun withTheDefaultPolicyNothingIsAsked() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.commitRecipients(RecipientField.TO, "dan@oldmail.org"); c.onSubject("Plain"); c.onBody("Body"); c.requestSend() }
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
    }

    @Test(timeout = 60_000)
    fun aPolicyThatThrowsOrSaysRequiredNeverLetsAMessageThroughOnItsOwn() {
        val r = rig()
        val all = EncryptionPolicy { _, recipients -> com.example.croft.mail.compose.EncryptionDecision.RequiredKeysMissing(recipients) }
        val c = harness(r).open(policy = all)
        r.run { c.commitRecipients(RecipientField.TO, "a@example.org"); c.commitRecipients(RecipientField.CC, "b@example.org"); c.onSubject("S"); c.onBody("B"); c.requestSend() }
        assertEquals(SendPrompt.EncryptionCheck(listOf("a@example.org", "b@example.org")), c.state.value.prompt)
        assertEquals(0, r.run { r.compose.listOutbox(r.account) }.size)
    }

    // ---- attachments on the real core ----

    @Test(timeout = 60_000)
    fun aPickedFileBecomesAnAttachmentOfTheDraft_andTheCopyIsGone() {
        val r = rig()
        val c = harness(r).open()
        r.run { c.addFromFile(MemoryFile("hello attachment".toByteArray(), displayName = "notes.txt")) }
        val s = c.state.value
        assertEquals(listOf("notes.txt"), s.attachments.map { it.name })
        assertEquals(16uL, s.attachments.single().sizeBytes)
        assertNotNull(s.draftId)
        assertEquals("the copy is deleted", 0, r.attachDir.list()!!.size)
        val loaded = r.run { r.compose.loadDraft(r.account, s.draftId!!) }
        assertEquals(1, loaded.attachments.size)
        r.run { c.removeAttachment(s.attachments.single().id) }
        assertEquals(0, c.state.value.attachments.size)
    }

    @Test(timeout = 60_000)
    fun aFileOverTheCoresLimitIsRefusedBeforeTheCopy() {
        val r = rig()
        val c = harness(r).open()
        val max = r.compose.limits().maxAttachmentBytes.toLong()
        var opened = false
        r.run { c.addFromFile(ClaimedFile(max + 1) { opened = true; null }) }
        assertEquals(com.example.croft.mail.compose.AttachmentError.TOO_LARGE, c.state.value.attachmentError)
        assertFalse(opened)
        assertEquals(0, c.state.value.attachments.size)
    }

    @Test(timeout = 60_000)
    fun theRegisteredFolderIsNotTheDatabaseFolder() {
        val r = rig()
        try {
            r.core.setAttachmentDirectory(r.dir.path)
            throw AssertionError("the core must refuse a folder holding the database")
        } catch (e: uniffi.croft_ffi.ComposeException) {
            assertTrue(e is uniffi.croft_ffi.ComposeException.BadAttachmentDirectory)
        } finally {
            r.core.setAttachmentDirectory(r.attachDir.path)
        }
    }
}
