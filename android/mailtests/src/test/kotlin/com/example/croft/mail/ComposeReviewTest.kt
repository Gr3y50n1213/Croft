// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.AttachmentError
import com.example.croft.mail.compose.AttachmentIntake
import com.example.croft.mail.compose.AutosaveTiming
import com.example.croft.mail.compose.ComposeBackend
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.ComposeState
import com.example.croft.mail.compose.FakeComposeBackend
import com.example.croft.mail.compose.LocalDraftMatch
import com.example.croft.mail.compose.Mailto
import com.example.croft.mail.compose.PickedFile
import com.example.croft.mail.compose.RecipientChipData
import com.example.croft.mail.compose.RecipientField
import com.example.croft.mail.compose.SendState
import com.example.croft.mail.compose.SimpleAddressTools
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.nio.file.Files
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.ComposeLimits
import uniffi.croft_ffi.DraftSummary
import uniffi.croft_ffi.FfiAddress

/**
 * The opus review of T18d, on the logic that has no Android in it: Send waits for files still being added from a share (M3), the text still
 * in a recipient field is not lost (M6), a stuck provider cannot hold anything up (L1), recipients and texts from outside are capped (L2,
 * L3), a share is kept by Back (L6), the limits hold for several files (L7), a removed attachment can be taken back (L8), chips show the
 * address (M2) and the Drafts rule never hides other mail (M5). Over the in-memory backend: the real core is in the other tests.
 */
class ComposeReviewTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dirs = mutableListOf<File>()

    @After
    fun tearDown() {
        scope.cancel()
        dirs.forEach { it.deleteRecursively() }
    }

    private fun dir() = Files.createTempDirectory("croft-review-test").toFile().also { dirs += it }

    private val accounts: suspend () -> List<AccountInfo> = { listOf(AccountInfo(1, "Sam", "sam@example.org", AccountState.OK)) }

    private fun controller(
        backend: ComposeBackend,
        dir: File = dir(),
        files: (String) -> PickedFile? = { null },
        timing: AutosaveTiming = AutosaveTiming(localMs = 60_000, serverIdleMs = 60_000, removeUndoMs = 400),
        stallMs: Long = 30_000,
    ) = ComposeController(
        backend = backend, scope = scope, accounts = accounts, intake = AttachmentIntake(backend, dir, stallMs = stallMs),
        tools = SimpleAddressTools, timing = timing, files = files, background = scope,
    )

    /** A file whose content arrives when [gate] opens. */
    private class GatedFile(private val bytes: ByteArray, val gate: CountDownLatch = CountDownLatch(1), override val displayName: String? = "shared.txt") : PickedFile {
        override val mimeType: String? = "text/plain"
        override val declaredSize: Long? = bytes.size.toLong()
        override fun open(): InputStream {
            gate.await(30, TimeUnit.SECONDS)
            return ByteArrayInputStream(bytes)
        }
    }

    /** A file that never answers `open` (a stuck provider) until the test ends. */
    private class StuckFile(val release: CountDownLatch = CountDownLatch(1)) : PickedFile {
        override val displayName: String? = "stuck.bin"
        override val mimeType: String? = null
        override val declaredSize: Long? = null
        override fun open(): InputStream? { release.await(30, TimeUnit.SECONDS); return null }
    }

    /** A stream that gives a few bytes and then blocks in `read` until it is closed. */
    private class StallingStream : InputStream() {
        private val closed = CountDownLatch(1)
        private var gave = false
        override fun read(): Int = if (!gave) { gave = true; 1 } else { closed.await(30, TimeUnit.SECONDS); -1 }
        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (!gave) { gave = true; b[off] = 1; return 1 }
            closed.await(30, TimeUnit.SECONDS)
            return -1
        }
        override fun close() { closed.countDown() }
    }

    private fun wait(what: String, timeoutMs: Long = 20_000, check: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < end) { if (check()) return; Thread.sleep(25) }
        throw AssertionError("timed out waiting for: $what")
    }

    // ---- M3: Send waits for a share's files ----

    @Test(timeout = 60_000)
    fun sendWaitsForAFileStillBeingCopiedFromAShare_andNeverGoesWithoutIt() = runBlocking {
        val fake = FakeComposeBackend()
        val file = GatedFile("shared bytes".toByteArray())
        val c = controller(fake, files = { file })
        c.open(ComposeStart.Fill(ComposeFill(to = listOf("ann@example.org"), subject = "Shared", body = "See file", attachments = listOf("content://other/1")), 1))
        // The copy has begun: the button says so at once, and nothing blocks the person.
        assertEquals(SendState.UPLOADING, c.state.value.sendState)
        assertEquals(1, c.state.value.attachmentsBusy)

        val sending = scope.async { c.requestSend() }
        delay(400)
        assertFalse("not sent while the file is on its way", fake.calls.contains("send"))
        assertTrue(sending.isActive)

        file.gate.countDown()
        withTimeout(20_000) { sending.await() }
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        val calls = synchronized(fake.calls) { fake.calls.toList() }
        assertTrue("the file was added before the message went: $calls", calls.indexOf("addAttachmentFile") in 0 until calls.indexOf("send"))
        assertEquals(1, fake.loadDraft(1, 1).attachments.size)
    }

    // ---- M6: typed text not yet a chip ----

    @Test(timeout = 60_000)
    fun textTypedInARecipientFieldIsAChipWhenSendIsTapped() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake)
        c.open(ComposeStart.New(1))
        c.onSubject("Typed")
        c.onBody("Body")
        // The screen holds what was typed in To and has not turned it into a chip (no comma, no focus change).
        c.setPendingRecipientText(RecipientField.TO, "ann@example.org")
        c.requestSend()
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(listOf("ann@example.org"), fake.loadDraft(1, 1).content.to.map { it.address })
        assertEquals("the field is told to clear its text", 1, c.state.value.recipientTextTick)
    }

    @Test(timeout = 60_000)
    fun textTypedInARecipientFieldCountsAsARecipientForTheSendButton() = runBlocking {
        val c = controller(FakeComposeBackend())
        c.open(ComposeStart.New(1))
        c.onSubject("Typed")
        c.onBody("Body")
        c.commitRecipients(RecipientField.TO, "ann@example.org")
        c.removeRecipient(RecipientField.TO, c.state.value.to.single())
        assertEquals(SendState.NO_RECIPIENT, c.state.value.sendState)
        c.setPendingRecipientText(RecipientField.TO, "bob@example.org")
        wait("the button counts the typed address") { c.state.value.sendState == SendState.READY }
        c.setPendingRecipientText(RecipientField.TO, "")
        wait("and stops counting it when it is gone") { c.state.value.sendState == SendState.NO_RECIPIENT }
    }

    @Test(timeout = 60_000)
    fun textTypedInARecipientFieldIsKeptByBack() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake)
        c.open(ComposeStart.New(1))
        c.setPendingRecipientText(RecipientField.CC, "bob@example.org")
        c.back()
        assertEquals(ComposeClosed.DraftSaved, c.state.value.closed)
        assertEquals(listOf("bob@example.org"), fake.loadDraft(1, 1).content.cc.map { it.address })
    }

    // ---- L1: a stuck provider ----

    @Test(timeout = 60_000)
    fun aProviderThatNeverAnswersIsGivenUpAfterTheStallTimeout() = runBlocking {
        val fake = FakeComposeBackend()
        val d = dir()
        val stuck = StuckFile()
        val intake = AttachmentIntake(fake, d, stallMs = 400)
        val started = System.currentTimeMillis()
        val result = withTimeout(20_000) { intake.add(1, 1, stuck) }
        assertEquals(AttachmentIntake.Result.Refused(AttachmentError.TIMED_OUT), result)
        assertTrue("given up soon after the timeout, not after the provider", System.currentTimeMillis() - started < 10_000)
        stuck.release.countDown()
        wait("the partial copy is deleted by the worker") { d.list()!!.isEmpty() }
    }

    @Test(timeout = 60_000)
    fun aStreamThatStopsMidwayIsClosedAndItsPartialCopyDeleted() = runBlocking {
        val fake = FakeComposeBackend()
        val d = dir()
        val stream = StallingStream()
        val file = object : PickedFile {
            override val displayName: String? = "half.bin"
            override val mimeType: String? = null
            override val declaredSize: Long? = null
            override fun open(): InputStream = stream
        }
        val result = withTimeout(20_000) { AttachmentIntake(fake, d, stallMs = 400).add(1, 1, file) }
        assertEquals(AttachmentIntake.Result.Refused(AttachmentError.TIMED_OUT), result)
        wait("no file left") { d.list()!!.isEmpty() }
        assertFalse(fake.calls.contains("addAttachmentFile"))
    }

    @Test(timeout = 60_000)
    fun backDoesNotWaitForACopy_andTheStuckCopyLeavesNothing() = runBlocking {
        val fake = FakeComposeBackend()
        val d = dir()
        val stuck = StuckFile()
        val c = controller(fake, dir = d, files = { stuck })
        c.open(ComposeStart.Fill(ComposeFill(to = listOf("ann@example.org"), subject = "S", attachments = listOf("content://other/1")), 1))
        assertEquals(1, c.state.value.attachmentsBusy)
        val started = System.currentTimeMillis()
        withTimeout(10_000) { c.back() }
        assertTrue("Back returned at once", System.currentTimeMillis() - started < 5_000)
        assertEquals(ComposeClosed.DraftSaved, c.state.value.closed)
        stuck.release.countDown()
        wait("the pending copy is gone") { c.state.value.attachmentsBusy == 0 && d.list()!!.isEmpty() }
    }

    @Test(timeout = 60_000)
    fun aCopyCanBeCancelledFromItsChip() = runBlocking {
        val fake = FakeComposeBackend()
        val d = dir()
        val stuck = StuckFile()
        val c = controller(fake, dir = d)
        c.open(ComposeStart.New(1))
        c.addFiles(listOf(stuck))
        wait("a pending copy") { c.state.value.pending.isNotEmpty() }
        c.cancelPending(c.state.value.pending.single().id)
        wait("no pending copy") { c.state.value.pending.isEmpty() }
        assertEquals(0, c.state.value.attachments.size)
        stuck.release.countDown()
        wait("no file left") { d.list()!!.isEmpty() }
    }

    // ---- L2, L3: limits on what comes from outside ----

    @Test(timeout = 60_000)
    fun recipientsFromALinkAreCapped() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake)
        val many = (1..1000).map { "u$it@example.org" }
        c.open(ComposeStart.Fill(ComposeFill(to = many, cc = many, bcc = many), 1))
        val cap = fake.limits().maxRecipients.toInt()
        assertEquals(cap, c.state.value.allRecipients.size)
        assertEquals(cap, c.state.value.to.size)
        // And the link parser itself never returns more than a hundred a field.
        val link = Mailto.parse("mailto:" + many.joinToString(","))!!
        assertEquals(Mailto.MAX_RECIPIENTS, link.to.size)
    }

    @Test
    fun aTextThatWasCutIsFlagged() {
        val long = "x".repeat(Mailto.MAX_BODY + 5)
        val cut = Mailto.parse("mailto:a@example.org?body=$long")!!
        assertEquals(Mailto.MAX_BODY, cut.body.length)
        assertTrue(cut.truncated)
        assertFalse(Mailto.parse("mailto:a@example.org?body=short")!!.truncated)
        runBlocking {
            val c = controller(FakeComposeBackend())
            c.open(ComposeStart.Fill(cut, 1))
            assertTrue(c.state.value.truncatedNotice)
            c.truncatedNoticeShown()
            assertFalse(c.state.value.truncatedNotice)
        }
    }

    // ---- L6: Back on a share ----

    @Test(timeout = 60_000)
    fun backOnAShareThatWasNotTypedInKeepsItAsADraft_butNewAndReplyAreLeftAlone() = runBlocking {
        val fake = FakeComposeBackend()
        val shared = controller(fake)
        shared.open(ComposeStart.Fill(ComposeFill(subject = "Look at this", body = "https://example.org/x"), 1))
        shared.back()
        assertEquals(ComposeClosed.DraftSaved, shared.state.value.closed)
        assertEquals("Look at this", fake.listDrafts(1).single().subject)

        val empty = controller(fake)
        empty.open(ComposeStart.New(1))
        empty.back()
        assertEquals(ComposeClosed.Nothing, empty.state.value.closed)
        assertEquals(1, fake.listDrafts(1).size)

        fake.reply = uniffi.croft_ffi.ReplyDraft(FakeComposeBackend.emptyContent().copy(to = listOf(FfiAddress(null, "ann@example.org")), subject = "Re: x", body = "> quote"), true)
        val reply = controller(fake)
        reply.open(ComposeStart.Reply(1, 5))
        reply.back()
        assertEquals(ComposeClosed.Nothing, reply.state.value.closed)
        assertEquals("a reply nobody wrote in leaves no draft", 1, fake.listDrafts(1).size)
    }

    // ---- L7: limits hold for several files at once ----

    @Test(timeout = 60_000)
    fun severalFilesAtOnceCannotPassTheTotalLimitTogether() = runBlocking {
        val fake = FakeComposeBackend()
        val small = object : ComposeBackend by fake {
            override fun limits() = ComposeLimits(
                attachmentWarnBytes = 25u, maxAttachmentBytes = 10u, maxAttachmentsTotalBytes = 10u, maxAttachments = 50u,
                maxRecipients = 200u, maxSubjectChars = 998u, maxBodyBytes = 1_000_000u, maxSuggestions = 20u,
            )
        }
        val c = controller(small)
        c.open(ComposeStart.New(1))
        c.addFiles(listOf(1, 2, 3).map { GatedFile("123456".toByteArray(), CountDownLatch(0), displayName = "f$it.txt") })
        c.awaitAttachments()
        assertEquals("only the first 6 bytes fit in 10", 1, c.state.value.attachments.size)
        assertEquals(AttachmentError.TOTAL_TOO_LARGE, c.state.value.attachmentError)
    }

    // ---- L8: Undo on removing an attachment ----

    @Test(timeout = 60_000)
    fun aRemovedAttachmentCanBeTakenBackInsideTheWindow_andIsGoneForGoodAfterIt() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake)
        c.open(ComposeStart.New(1))
        c.addFromFile(GatedFile("abc".toByteArray(), CountDownLatch(0), "a.txt"))
        val a = c.state.value.attachments.single()

        c.removeAttachment(a.id)
        assertEquals("the chip is gone", 0, c.state.value.attachments.size)
        assertEquals(a, c.state.value.removedAttachment)
        assertFalse("but the core still has it", fake.calls.contains("removeAttachment"))
        c.undoRemoveAttachment()
        assertEquals(listOf(a), c.state.value.attachments)
        assertNull(c.state.value.removedAttachment)

        c.removeAttachment(a.id)
        wait("the removal is final after the window") { synchronized(fake.calls) { fake.calls.contains("removeAttachment") } && c.state.value.removedAttachment == null }
        assertEquals(0, fake.loadDraft(1, 1).attachments.size)
    }

    @Test(timeout = 60_000)
    fun sendMakesARemovalFinalFirst_soTheMessageNeverCarriesARemovedFile() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake, timing = AutosaveTiming(localMs = 60_000, serverIdleMs = 60_000, removeUndoMs = 60_000))
        c.open(ComposeStart.New(1))
        c.commitRecipients(RecipientField.TO, "ann@example.org")
        c.onSubject("S")
        c.addFromFile(GatedFile("abc".toByteArray(), CountDownLatch(0), "a.txt"))
        c.removeAttachment(c.state.value.attachments.single().id)
        c.requestSend()
        assertTrue(c.state.value.closed is ComposeClosed.Sent)
        assertEquals(0, fake.loadDraft(1, 1).attachments.size)
    }

    // ---- M2: chips show the address ----

    @Test
    fun aChipAlwaysShowsTheAddress() {
        val tools = SimpleAddressTools
        fun label(raw: String) = tools.parse(raw).single().let { RecipientChipData(it, tools.isValid(it.address)).label }
        // The link of the review: a name that reads as another address.
        val hostile = Mailto.parse("mailto:%22boss%40corp.com%22%20%3Cattacker%40evil.example%3E")!!.to.single()
        assertTrue(label(hostile).contains("attacker@evil.example"))
        assertEquals("boss@corp.com <attacker@evil.example>", label(hostile))
        assertEquals("ann@example.org", label("ann@example.org"))
        assertEquals("Ann <ann@example.org>", label("Ann <ann@example.org>"))
        assertEquals("a name equal to the address shows once", "ann@example.org", RecipientChipData(FfiAddress("ann@example.org", "ann@example.org"), true).label)
    }

    // ---- M5: the Drafts rule never hides other mail ----

    private fun row(subject: String, snippet: String) = MessageRowData(
        id = 1, account = 1, folder = 5, senderName = null, senderAddress = null, subject = subject, snippet = snippet,
        dateMs = 0, seen = true, flagged = false, hasAttachments = false,
    )

    private fun draft(subject: String, snippet: String) = DraftSummary(7, 1, subject, snippet, emptyList(), false, null, 0)

    @Test
    fun anEmptySnippetNeverMatches() {
        assertFalse("a draft without text must not hide every row of that subject", LocalDraftMatch.isCopyOf(row("Plans", "Anything at all"), draft("Plans", "")))
        assertFalse(LocalDraftMatch.isCopyOf(row("Plans", ""), draft("Plans", "Hello")))
        assertFalse(LocalDraftMatch.isCopyOf(row("Plans", ""), draft("Plans", "")))
        assertEquals(1, LocalDraftMatch.withoutCopies(listOf(row("Plans", "x")), listOf(draft("Plans", ""))).size)
    }

    // ---- Info: nothing of the message in a printout ----

    @Test(timeout = 60_000)
    fun stateAndResultsDoNotPrintSubjectBodyOrNames() = runBlocking {
        val fake = FakeComposeBackend()
        val c = controller(fake)
        c.open(ComposeStart.Fill(ComposeFill(to = listOf("secret.person@example.org"), subject = "Top secret subject", body = "Top secret body"), 1))
        c.addFromFile(GatedFile("abc".toByteArray(), CountDownLatch(0), "secret-plan.txt"))
        val text = c.state.value.toString() + ComposeStart.Fill(ComposeFill(subject = "Top secret subject")) + c.state.value.attachments.size
        assertFalse(text.contains("secret"))
        assertFalse(AttachmentIntake.Result.Added(c.state.value.attachments.single()).toString().contains("secret"))
    }

    @Test
    fun displayNamesLoseEveryFormatCharacter() {
        assertEquals("abc.txt", AttachmentIntake.cleanName("a​b‮c.txt"))
        assertEquals("fdp.exe", AttachmentIntake.cleanName("‮fdp.exe"))
        assertEquals("attachment", AttachmentIntake.cleanName("​⁠"))
        assertEquals("a b", AttachmentIntake.cleanName("a b "))
    }
}
