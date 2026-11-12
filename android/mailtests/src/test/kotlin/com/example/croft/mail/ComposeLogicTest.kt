// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.AttachmentError
import com.example.croft.mail.compose.AttachmentFolder
import com.example.croft.mail.compose.AttachmentIntake
import com.example.croft.mail.compose.Mailto
import com.example.croft.mail.compose.RecipientField
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.isValidAddress
import uniffi.croft_ffi.parseRecipients

/** The parts of compose that need no server: `mailto:` links, the attachment copy rules and the recipient chips. */
class ComposeLogicTest {
    private val dirs = mutableListOf<File>()

    private fun tmp() = Files.createTempDirectory("croft-attach-test").toFile().also { dirs += it }

    @After
    fun tearDown() = dirs.forEach { it.deleteRecursively() }

    // ---- mailto ----

    @Test
    fun mailtoFillsRecipientsSubjectAndBody() {
        val fill = Mailto.parse("mailto:ann@example.org,bob@example.org?cc=cy%40example.org&bcc=di@example.org&subject=Lunch%20at%20noon&body=See%20you%0Athere%20%E2%9C%93")!!
        assertEquals(listOf("ann@example.org", "bob@example.org"), fill.to)
        assertEquals(listOf("cy@example.org"), fill.cc)
        assertEquals(listOf("di@example.org"), fill.bcc)
        assertEquals("Lunch at noon", fill.subject)
        assertEquals("See you\nthere ✓", fill.body)
    }

    @Test
    fun mailtoKeepsAPlusAndIgnoresAttachAndUnknownFields() {
        val fill = Mailto.parse("mailto:a+tag@example.org?attach=%2Fdata%2Fsecret&in-reply-to=x&subject=a+b")!!
        assertEquals(listOf("a+tag@example.org"), fill.to)
        assertEquals("a+b", fill.subject)
        assertTrue("a link never attaches a file", fill.attachments.isEmpty())
    }

    @Test
    fun mailtoWithOnlyHeadersAndTheSubjectCannotInjectHeaders() {
        val fill = Mailto.parse("MAILTO:?to=x@example.org&subject=Hi%0D%0ABcc:%20evil@example.org")!!
        assertEquals(listOf("x@example.org"), fill.to)
        assertFalse(fill.subject.contains('\n') || fill.subject.contains('\r'))
        assertNull(Mailto.parse("https://example.org/?to=a@b.c"))
        assertNull(Mailto.parse("file:///data/x"))
    }

    @Test
    fun aBadEscapeStaysAsItIs() {
        assertEquals("100%", Mailto.parse("mailto:a@example.org?subject=100%")!!.subject)
        assertEquals("%zz", Mailto.parse("mailto:a@example.org?subject=%25zz")!!.subject)
    }

    // ---- the core's recipient helpers (the chips use them) ----

    @Test
    fun theCoreReadsAndChecksAddresses() {
        val list = parseRecipients("Ann <ann@example.org>; bob@example.org")
        assertEquals(listOf("ann@example.org", "bob@example.org"), list.map { it.address })
        assertTrue(isValidAddress("ann@example.org"))
        assertFalse(isValidAddress("ann@"))
        assertFalse(isValidAddress("not an address"))
    }

    // ---- attachments ----

    @Test
    fun anOverLimitFileIsRefusedBeforeAnythingIsCopied() = runBlocking {
        val fake = com.example.croft.mail.compose.FakeComposeBackend()
        val dir = tmp()
        var opened = false
        val file = ClaimedFile(fake.limits().maxAttachmentBytes.toLong() + 1) { opened = true; ByteArrayInputStream(ByteArray(1)) }
        val result = AttachmentIntake(fake, dir).add(1, 1, file)
        assertEquals(AttachmentIntake.Result.Refused(AttachmentError.TOO_LARGE), result)
        assertFalse("the content was never opened", opened)
        assertEquals("nothing written", 0, dir.list()!!.size)
        assertFalse("the core was not asked", fake.calls.contains("addAttachmentFile"))
    }

    @Test
    fun aFileThatLiesAboutItsSizeStopsAtTheLimit() = runBlocking {
        val fake = com.example.croft.mail.compose.FakeComposeBackend()
        val dir = tmp()
        val limit = fake.limits().maxAttachmentBytes.toLong()
        // Declares nothing, delivers more than the limit.
        val endless = object : InputStream() { override fun read(): Int = 1 }
        val result = AttachmentIntake(fake, dir).add(1, 1, ClaimedFile(null) { endless })
        assertEquals(AttachmentIntake.Result.Refused(AttachmentError.TOO_LARGE), result)
        assertEquals("the partial copy is deleted", 0, dir.list()!!.size)
        assertTrue(limit > 0)
    }

    @Test
    fun theCopyHasAGeneratedNameAndIsDeletedAfterTheCoreHasReadIt() = runBlocking {
        val fake = com.example.croft.mail.compose.FakeComposeBackend()
        fake.saveDraft(1, null, com.example.croft.mail.compose.FakeComposeBackend.emptyContent())
        val dir = tmp()
        val seen = mutableListOf<String>()
        val recording = object : com.example.croft.mail.compose.ComposeBackend by fake {
            override suspend fun addAttachmentFile(account: AccountId, draftId: Long, path: String, name: String, mimeType: String): uniffi.croft_ffi.DraftAttachment {
                seen += File(path).name
                assertTrue("the copy is under the folder", File(path).parentFile == dir)
                assertEquals(6, File(path).length())
                return fake.addAttachmentFile(account, draftId, path, name, mimeType)
            }
        }
        val result = AttachmentIntake(recording, dir).add(1, 1, MemoryFile("abcdef".toByteArray(), displayName = "../../etc/passwd‮.txt", mimeType = null))
        val added = (result as AttachmentIntake.Result.Added).attachment
        assertEquals("the shown name is a clean name, not a path", "passwd.txt", added.name)
        assertEquals("application/octet-stream", added.mimeType)
        assertTrue("generated name", seen.single().matches(Regex("att-[0-9a-f]{32}")))
        assertEquals("the copy is gone", 0, dir.list()!!.size)
    }

    @Test
    fun theAttachmentFolderIsClearedAtStart() {
        val dir = File(tmp(), "compose-attach")
        dir.mkdirs()
        File(dir, "att-old").writeText("left by a crash")
        File(dir, "sub").mkdirs()
        AttachmentFolder.prepare(dir)
        assertEquals(0, dir.list()!!.size)
        val missing = File(tmp(), "new/compose-attach")
        AttachmentFolder.prepare(missing)
        assertTrue(missing.isDirectory)
    }

    @Test
    fun displayNamesAreCleaned() {
        assertEquals("attachment", AttachmentIntake.cleanName(null))
        assertEquals("attachment", AttachmentIntake.cleanName("  \n"))
        assertEquals("b.txt", AttachmentIntake.cleanName("a\\b.txt"))
        assertEquals(120, AttachmentIntake.cleanName("x".repeat(500)).length)
        assertEquals("application/octet-stream", AttachmentIntake.cleanType("bad type"))
        assertEquals("image/png", AttachmentIntake.cleanType("image/png"))
    }

    @Test
    fun recipientFieldsAreThree() = assertEquals(3, RecipientField.entries.size)
}
