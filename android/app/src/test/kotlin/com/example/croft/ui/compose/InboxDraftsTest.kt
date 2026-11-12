// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.FakeData
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.MessageScope
import com.example.croft.mail.compose.ComposeBackend
import com.example.croft.mail.compose.FakeComposeBackend
import com.example.croft.ui.mail.InboxController
import com.example.croft.ui.mail.InboxTexts
import com.example.croft.ui.mail.LoadPhase
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.FfiAddress

/**
 * The Drafts folder with the drafts of this device above it (the review's M5), on the Inbox's own controller: a server row that repeats a
 * local draft is hidden and is never selected by Select all; a draft read that finishes after the person moved to another folder hides
 * nothing there.
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class InboxDraftsTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @After
    fun tearDown() = scope.cancel()

    private val drafts = FakeData.folderId(1, FakeData.DRAFTS)
    private val inbox = FakeData.folderId(1, FakeData.INBOX)

    private fun row(id: Long, folder: Long, subject: String, snippet: String) = MessageRowData(
        id = id, account = 1, folder = folder, senderName = "Sam", senderAddress = "sam@example.org", subject = subject, snippet = snippet,
        dateMs = 1_800_000_000_000L - id * 1000, seen = true, flagged = false, hasAttachments = false,
    )

    private fun data() = listOf(
        row(901, drafts, "Plans", "Let us meet at noon on Friday and bring the files"), // the server's copy of the local draft below
        row(902, drafts, "Plans", "A different draft with the same subject"),
        row(903, drafts, "Ideas", ""), // no text at all
        row(1, inbox, "Hello", "Hi there"),
    )

    private fun controller(f: ComposeFixture, start: MessageScope) =
        InboxController(f.session, scope, start, InboxTexts(RuntimeEnvironment.getApplication().resources))

    private fun wait(what: String, check: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < end) { if (check()) return; Thread.sleep(25) }
        throw AssertionError("timed out waiting for: $what")
    }

    private fun localDraft(f: ComposeFixture, subject: String, body: String) = runBlocking {
        f.backend.saveDraft(1, null, DraftContent(null, listOf(FfiAddress(null, "ann@example.org")), emptyList(), emptyList(), subject, body, null, emptyList()))
    }

    @Test
    fun theServerCopyIsHiddenAndSelectAllNeverSelectsIt() {
        val f = ComposeFixture(data = data())
        localDraft(f, "Plans", "Let us meet at noon on Friday")
        val c = controller(f, MessageScope.Folder(1, drafts))
        scope.launch { c.run() }
        wait("the folder and the local drafts") { c.phase == LoadPhase.LOADED && c.localDrafts.size == 1 && c.rows.size == 3 }

        assertEquals("the copy is not shown", setOf(902L, 903L), c.shownRows.map { it.id }.toSet())
        c.selectAllLoaded()
        assertEquals("only what is shown is selected", setOf(902L, 903L), c.selected)
        c.clearSelection()
        c.selectAllInFolder()
        assertFalse(901L in c.selected)
    }

    @Test
    fun aDraftWithoutTextHidesNothing() {
        val f = ComposeFixture(data = data())
        localDraft(f, "Ideas", "")
        val c = controller(f, MessageScope.Folder(1, drafts))
        scope.launch { c.run() }
        wait("loaded") { c.phase == LoadPhase.LOADED && c.localDrafts.size == 1 }
        assertTrue("an empty snippet matches nothing", 903L in c.shownRows.map { it.id })
        assertEquals(3, c.shownRows.size)
    }

    @Test
    fun aSlowDraftReadDoesNotHideRowsOfTheFolderThePersonMovedTo() {
        val fake = FakeComposeBackend()
        val slow = object : ComposeBackend by fake {
            override suspend fun listDrafts(account: Long): List<uniffi.croft_ffi.DraftSummary> { delay(700); return fake.listDrafts(account) }
        }
        val f = ComposeFixture(data = data(), session = { slow })
        runBlocking { fake.saveDraft(1, null, DraftContent(null, emptyList(), emptyList(), emptyList(), "Hello", "Hi there", null, emptyList())) }
        val c = controller(f, MessageScope.Folder(1, drafts))
        scope.launch { c.run() }
        wait("the folders") { c.folders.isNotEmpty() }
        // Move to the Inbox while the read of the drafts is on its way.
        c.openScope(MessageScope.Folder(1, inbox))
        Thread.sleep(1_500)
        assertTrue("no drafts for the Inbox", c.localDrafts.isEmpty())
        wait("the Inbox rows") { c.phase == LoadPhase.LOADED && c.rows.isNotEmpty() }
        assertEquals("the Inbox row \"Hello\" is shown even though a local draft has the same subject and text", listOf(1L), c.shownRows.map { it.id })
    }
}
