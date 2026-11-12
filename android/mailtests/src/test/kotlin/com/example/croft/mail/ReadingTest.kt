// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.reading.AttachmentData
import com.example.croft.mail.reading.BodyData
import com.example.croft.mail.reading.CoreReadingRepository
import com.example.croft.mail.reading.ReadFailure
import com.example.croft.mail.reading.ReadProblem
import com.example.croft.mail.reading.RemoteImages
import com.example.croft.mail.reading.toProblem
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.FfiAddress

/**
 * The reading calls of the app's mail layer ([CoreReadingRepository]) against the real core: a conversation opened from the in-process
 * IMAP server, a body that loads and is not marked read until the app says so, remote images blocked then allowed (the sample mailbox
 * has HTML with a remote picture), an attachment saved into its folder, and a blocked executable refused.
 */
class ReadingTest {
    private val rigs = mutableListOf<Rig>()
    private val dirs = mutableListOf<File>()

    private fun rig(messages: Int = 3): Rig = Rig(messages, undoWindowMs = 600, smtp = true).also { rigs += it }

    private fun tempDir(): File = Files.createTempDirectory("croft-shared-attachments").toFile().also { dirs += it }

    @After
    fun tearDown() {
        rigs.forEach { it.close() }
        dirs.forEach { it.deleteRecursively() }
    }

    // ---- Over the in-process IMAP server ----

    @Test(timeout = 60_000)
    fun aConversationOpensFromTheInboxAndItsBodyLoadsWithoutBeingMarkedRead() {
        val r = rig()
        val rows = r.awaitInbox(3)
        val reading = CoreReadingRepository(r.core)
        val row = rows.first()

        val conversation = r.run { reading.openConversation(row.id) }
        assertEquals(r.account, conversation.account)
        assertEquals(row.id, conversation.openedId)
        val message = conversation.opened!!
        assertEquals("Ann Example", message.senderName)
        assertEquals(FolderRole.INBOX, message.folderRole)
        assertFalse("the body is not stored by the sync", message.bodyStored)
        assertFalse(message.seen)
        assertFalse("a real account is not the sample mailbox", conversation.localOnly)
        assertTrue("the opened message starts expanded", row.id in conversation.initiallyExpanded)

        r.settle()
        val body = r.run { reading.loadBody(row.id, RemoteImages.ASK) }
        assertTrue(body.text.orEmpty().contains("Hello, this is the body."))
        assertEquals("a text-only message has no HTML", null, body.html)
        assertFalse(body.remoteAllowed)
        assertEquals(0, body.blockedRemoteCount)
        assertTrue("the body was fetched with PEEK", r.server.commands().any { it.contains("BODY.PEEK") })
        assertTrue("the fetch did not set a flag", r.changingCommands().none { it.contains("STORE") })
        assertTrue(r.run { reading.openConversation(row.id) }.opened!!.bodyStored)

        // Not read until the app says so, and the list agrees.
        fun onServer() = r.server.flagsOfSubject("INBOX", row.subject)
        assertFalse(onServer().contains("\\Seen"))
        assertFalse(r.inbox().first { it.id == row.id }.seen)
        Thread.sleep(2_000)
        assertFalse("still not read after waiting", onServer().contains("\\Seen"))

        r.run { r.repo.markSeen(listOf(row.id), true) }
        assertTrue(r.inbox().first { it.id == row.id }.seen)
        await("the server has \\Seen") { onServer().takeIf { "\\Seen" in it } }
    }

    @Test(timeout = 60_000)
    fun aMessageThatWasLoadedOpensOffline_andOneThatWasNotSaysOffline() {
        val r = rig()
        val rows = r.awaitInbox(3)
        val reading = CoreReadingRepository(r.core)
        r.run { reading.loadBody(rows[0].id, RemoteImages.ASK) }
        r.run { r.repo.setNetworkAvailable(false) }
        assertTrue(r.run { reading.loadBody(rows[0].id, RemoteImages.ASK) }.text.orEmpty().isNotEmpty())
        val e = try { r.run { reading.loadBody(rows[1].id, RemoteImages.ASK) }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.OFFLINE, e?.problem)
        assertTrue(ReadProblem.OFFLINE.canRetry)
        r.run { r.repo.setNetworkAvailable(true) }
        assertNotNull(r.run { reading.loadBody(rows[1].id, RemoteImages.ASK) }.text)
    }

    @Test(timeout = 60_000)
    fun aRefusalIsAFixedCategory() {
        val r = rig(1)
        r.awaitInbox(1)
        val reading = CoreReadingRepository(r.core)
        val gone = try { r.run { reading.loadBody(999_999, RemoteImages.ASK) }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.NO_SUCH_MESSAGE, gone?.problem)
        assertEquals("NO_SUCH_MESSAGE", gone?.message)
        val badAddress = try { r.run { reading.trustSender(r.account, "not an address") }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.INVALID_ADDRESS, badAddress?.problem)
    }

    // ---- Over the sample mailbox (the core's own: HTML with a remote picture, attachments, conversations) ----

    private class Sample(val rig: Rig, val reading: CoreReadingRepository, val account: Long, val rows: List<MessageRowData>)

    private fun sample(): Sample {
        val r = rig(0)
        val account = r.run { r.core.addDemoAccount() }.id
        val rows = await("the sample Inbox", 30_000) {
            val inbox = r.run { r.repo.listFolders(account) }.firstOrNull { it.role == FolderRole.INBOX } ?: return@await null
            r.run { r.repo.listMessages(MessageScope.Folder(account, inbox.id), null, 200) }.rows.takeIf { it.size >= 10 }
        }
        return Sample(r, CoreReadingRepository(r.core), account, rows)
    }

    private fun Sample.firstWith(predicate: (BodyData) -> Boolean): Pair<MessageRowData, BodyData> {
        for (row in rows) {
            val body = rig.run { reading.loadBody(row.id, RemoteImages.ASK) }
            if (predicate(body)) return row to body
        }
        fail("no sample message fits")
        error("unreachable")
    }

    @Test(timeout = 90_000)
    fun theSampleConversationIsMarkedNotSavedToTheServer_andListsItsMessagesOldestFirst() {
        val s = sample()
        val longest = s.rows.map { s.rig.run { s.reading.openConversation(it.id) } }.maxByOrNull { it.messages.size }!!
        assertTrue("the sample mailbox has a thread", longest.messages.size >= 2)
        assertTrue(longest.localOnly)
        assertEquals(longest.messages.sortedBy { it.dateMs }.map { it.id }, longest.messages.map { it.id })
        assertTrue(longest.opened != null)
    }

    @Test(timeout = 90_000)
    fun remoteImagesAreBlocked_thenShowImagesAllowsThem_andNeverWinsOverIt() {
        val s = sample()
        val (row, blocked) = s.firstWith { it.blockedRemoteCount > 0 }
        assertFalse(blocked.remoteAllowed)
        val html = blocked.html!!
        assertFalse("no remote address reaches the page", html.contains("src=\"https://"))
        assertFalse(html.contains("<script", ignoreCase = true))
        assertTrue(blocked.blockedRemoteCount > 0)

        s.rig.run { s.reading.allowRemoteContent(row.id) }
        val shown = s.rig.run { s.reading.loadBody(row.id, RemoteImages.ASK) }
        assertTrue(shown.remoteAllowed)
        assertEquals(0, shown.blockedRemoteCount)
        assertTrue(shown.html!!.contains("src=\"https://"))
        assertFalse("the page only names https pictures", shown.html!!.contains("src=\"http://"))

        // Never wins over the person's own tap.
        val never = s.rig.run { s.reading.loadBody(row.id, RemoteImages.NEVER) }
        assertFalse(never.remoteAllowed)
        assertTrue(never.blockedRemoteCount > 0)
        // Nothing of this marked the message read.
        assertFalse(s.rig.run { s.rig.repo.listMessages(MessageScope.UnifiedInbox, null, 200) }.rows.first { it.id == row.id }.seen)
    }

    @Test(timeout = 90_000)
    fun knownSendersAreListed_trusted_andUntrusted() {
        val s = sample()
        val (row, _) = s.firstWith { it.blockedRemoteCount > 0 }
        val sender = s.rig.run { s.reading.openConversation(row.id) }.opened!!.senderAddress!!
        assertEquals(emptyList<String>(), s.rig.run { s.reading.trustedSenders(s.account) })
        assertFalse(s.rig.run { s.reading.loadBody(row.id, RemoteImages.KNOWN_SENDERS) }.remoteAllowed)

        s.rig.run { s.reading.trustSender(s.account, sender.uppercase()) }
        assertEquals(listOf(sender.lowercase()), s.rig.run { s.reading.trustedSenders(s.account) })
        val known = s.rig.run { s.reading.loadBody(row.id, RemoteImages.KNOWN_SENDERS) }
        assertTrue(known.remoteAllowed && known.senderKnown)
        assertFalse("Ask does not use the list", s.rig.run { s.reading.loadBody(row.id, RemoteImages.ASK) }.remoteAllowed)

        assertTrue(s.rig.run { s.reading.untrustSender(s.account, sender) })
        assertFalse(s.rig.run { s.reading.untrustSender(s.account, sender) })
        assertEquals(emptyList<String>(), s.rig.run { s.reading.trustedSenders(s.account) })
    }

    @Test(timeout = 90_000)
    fun anAttachmentIsSavedInsideTheGivenFolderOnly_andAgainUnderANewName() {
        val s = sample()
        val (row, body) = s.firstWith { b -> b.chips.any { !it.blocked } }
        val attachment: AttachmentData = body.chips.first { !it.blocked }
        val dir = tempDir()
        val file = s.rig.run { s.reading.saveAttachment(row.id, attachment.id, dir) }
        assertEquals(dir.canonicalFile, file.canonicalFile.parentFile)
        assertTrue(file.length() > 0)
        val again = s.rig.run { s.reading.saveAttachment(row.id, attachment.id, dir) }
        assertTrue("a second save never overwrites", again.canonicalPath != file.canonicalPath && again.exists() && file.exists())
        assertEquals(2, dir.listFiles()!!.size)

        // A relative folder, and an attachment of another message, are refused.
        val relative = try { s.rig.run { s.reading.saveAttachment(row.id, attachment.id, File("relative/dir")) }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.BAD_DESTINATION, relative?.problem)
        val other = s.rows.first { it.id != row.id }
        val wrong = try { s.rig.run { s.reading.saveAttachment(other.id, attachment.id, dir) }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.NO_SUCH_ATTACHMENT, wrong?.problem)
    }

    // ---- A blocked executable, over the in-process server: a draft with a program attached is appended to Drafts ----

    @Test(timeout = 90_000)
    fun aBlockedExecutableIsFlaggedAndRefused_andNothingIsWritten() {
        val r = rig(1)
        r.awaitInbox(1)
        val reading = CoreReadingRepository(r.core)
        val exe = File(r.attachDir, "tool-copy.bin").also { it.writeBytes(byteArrayOf(0x4d, 0x5a, 0x00, 0x01)) }
        val pdf = File(r.attachDir, "report-copy.bin").also { it.writeBytes("%PDF-1.4 test".toByteArray()) }
        val draft = r.run {
            val saved = r.compose.saveDraft(
                r.account, null,
                DraftContent(null, listOf(FfiAddress(null, "bob@example.org")), emptyList(), emptyList(), "Tools", "See attached.", null, emptyList()),
            )
            r.compose.addAttachmentFile(r.account, saved.id, exe.path, "setup.exe", "application/octet-stream")
            r.compose.addAttachmentFile(r.account, saved.id, pdf.path, "report.pdf", "application/pdf")
            r.compose.syncDraftToServer(r.account, saved.id, true)
            saved.id
        }
        assertTrue(draft > 0)
        await("the draft on the server", 30_000) { r.server.subjects("Drafts").takeIf { it == listOf("Tools") } }
        r.run { r.repo.syncNow(r.account) }
        val row = await("the draft in the Drafts folder", 30_000) { r.rows("Drafts").firstOrNull { it.subject == "Tools" } }

        val body = r.run { reading.loadBody(row.id, RemoteImages.ASK) }
        val program = body.chips.first { it.fileName.endsWith(".exe") }
        val document = body.chips.first { it.fileName.endsWith(".pdf") }
        assertTrue("the program is flagged blocked", program.blocked)
        assertFalse(document.blocked)

        val dir = tempDir()
        val refused = try { r.run { reading.saveAttachment(row.id, program.id, dir) }; null } catch (e: ReadFailure) { e }
        assertEquals(ReadProblem.ATTACHMENT_BLOCKED, refused?.problem)
        assertEquals("nothing was written", 0, dir.listFiles()!!.size)
        r.run { reading.saveAttachment(row.id, document.id, dir) }
        assertEquals("only the document was written", listOf("report.pdf"), dir.listFiles()!!.map { it.name })
    }

    @Test
    fun everyVariantOfTheCoresReadErrorHasAProblem() {
        val all = uniffi.croft_ffi.ReadException::class.java.declaredClasses.filter { uniffi.croft_ffi.ReadException::class.java.isAssignableFrom(it) }
        assertTrue("found ${all.size} variants", all.size >= 17)
        val seen = HashSet<ReadProblem>()
        for (c in all) {
            val e: uniffi.croft_ffi.ReadException = when (c) {
                uniffi.croft_ffi.ReadException.TooLarge::class.java -> uniffi.croft_ffi.ReadException.TooLarge(1uL)
                uniffi.croft_ffi.ReadException.Storage::class.java -> uniffi.croft_ffi.ReadException.Storage(uniffi.croft_ffi.StorageReason.DATABASE)
                else -> c.getDeclaredConstructor().newInstance() as uniffi.croft_ffi.ReadException
            }
            seen += e.toProblem()
        }
        assertEquals("every problem has a variant", ReadProblem.entries.toSet(), seen)
    }
}
