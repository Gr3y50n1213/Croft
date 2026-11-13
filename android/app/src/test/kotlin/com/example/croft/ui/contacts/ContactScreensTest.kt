// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import android.content.Intent
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.contacts.ContactCard
import com.example.croft.mail.contacts.ContactsAccount
import com.example.croft.mail.contacts.LabeledValue
import com.example.croft.mail.search.LocalSearchResult
import com.example.croft.mail.search.SearchBackend
import com.example.croft.mail.search.SearchHitData
import com.example.croft.mail.search.ServerSearchResult
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** S41 and S42 (Robolectric, in-memory backend): the contact's fields and actions, Delete, and the editor's rules. Made-up data. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ContactScreensTest {
    @get:Rule
    val rule = createComposeRule()

    private fun backend() = FakeContactsBackend().apply {
        book(1, 10, "Personal")
        book(2, 20, "Team")
        add(1, 3, "Mira", "Okafor", "mira@example.org", phone = "+44 7700 900123", book = 10, hasKey = true, organization = "Example Org")
    }

    /** Messages from or to the contact, as the device's index would find them. */
    private class FakeSearch(private val rows: List<MessageRowData>) : SearchBackend {
        val queries = ArrayList<String>()
        override suspend fun local(account: Long?, query: String, limit: Int): LocalSearchResult {
            queries += query
            return LocalSearchResult(rows.filter { query.startsWith("from:") }.take(limit).map { SearchHitData(it) }, bodiesPartial = false)
        }
        override suspend fun server(account: Long?, query: String) = ServerSearchResult(emptyList())
    }

    private fun message(id: Long, subject: String, date: Long) =
        MessageRowData(id, 1, 1, "Mira Okafor", "mira@example.org", subject, "Thanks, I will test the new DNS", date, seen = true, flagged = false, hasAttachments = false)

    // ---- S41 ----

    private var edited: ContactCard? = null
    private val mailed = ArrayList<Pair<Long, String>>()
    private val openedMessages = ArrayList<Long>()
    private var deletedFrom: Long? = null

    private fun openContact(b: FakeContactsBackend, search: SearchBackend? = null, id: Long = 3) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            ContactScreen(
                backend = b, account = 1, id = id, search = search, onBack = {},
                onEdit = { edited = it }, onMail = { a, address -> mailed += a to address },
                onOpenMessage = { openedMessages += it }, onDeleted = { deletedFrom = it },
            )
        }
        rule.waitForIdle()
    }

    @Test
    fun theContactShowsItsFields_theKey_andItsRecentConversations() {
        val search = FakeSearch(listOf(message(7, "Re: Server migration notes", 2_000), message(8, "Older", 1_000)))
        openContact(backend(), search)
        rule.onNodeWithTag(ContactTags.NAME).assert(hasText("Mira Okafor"))
        rule.onNodeWithText("Example Org").assertExists()
        rule.onNodeWithTag(ContactTags.email(0)).assert(hasText("mira@example.org", substring = true))
        rule.onNodeWithText("Work").assertExists()
        rule.onNodeWithText("Mobile").assertExists()
        rule.onNodeWithTag(ContactTags.KEY).assertExists()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(ContactTags.recent(7))).fetchSemanticsNodes().isNotEmpty() }
        assertEquals(listOf("from:\"mira@example.org\"", "to:\"mira@example.org\""), search.queries)
        rule.onNodeWithTag(ContactTags.recent(7)).performScrollTo().performClick()
        assertEquals(listOf(7L), openedMessages)
    }

    @Test
    fun mailWritesToTheFirstAddress_andCallOpensTheDialler() {
        openContact(backend())
        rule.onNodeWithTag(ContactTags.MAIL).performClick()
        assertEquals(listOf(1L to "mira@example.org"), mailed)
        rule.onNodeWithTag(ContactTags.CALL).performClick()
        val started = shadowOf(RuntimeEnvironment.getApplication()).nextStartedActivity
        assertEquals(Intent.ACTION_DIAL, started.action)
        assertEquals("tel", started.data?.scheme)
        assertEquals("+44 7700 900123", started.data?.schemeSpecificPart)
    }

    @Test
    fun editHandsOverTheContact() {
        openContact(backend())
        rule.onNodeWithTag(ContactTags.EDIT).performClick()
        assertEquals(3L, edited?.id)
    }

    @Test
    fun deleteAsksFirst_thenDeletes() {
        val b = backend()
        openContact(b)
        rule.onNodeWithTag(ContactTags.MORE).performClick()
        rule.onNodeWithText("Delete contact").performClick()
        rule.onNodeWithText("Delete this contact?").assertExists()
        // Cancel keeps it.
        rule.onNodeWithText("Cancel").performClick()
        assertTrue(b.deleted.isEmpty())
        rule.onNodeWithTag(ContactTags.MORE).performClick()
        rule.onNodeWithText("Delete contact").performClick()
        rule.onNodeWithText("Delete").performClick()
        rule.waitUntil(5_000) { deletedFrom != null }
        assertEquals(listOf(1L to 3L), b.deleted)
        assertEquals(1L, deletedFrom)
    }

    @Test
    fun aContactThatIsGoneSaysSo() {
        openContact(backend(), id = 99)
        rule.onNodeWithTag(ContactTags.GONE).assertExists()
    }

    @Test
    fun aConflictCopyAndAnUnsentChangeAreExplained() {
        val b = backend().apply {
            add(1, 4, "Mira", "Okafor", "mira@example.org", conflict = true)
            contacts[contacts.lastIndex] = contacts.last().copy(pending = true)
        }
        openContact(b, id = 4)
        rule.onNodeWithTag(ContactTags.CONFLICT).assertExists()
        rule.onNodeWithTag(ContactTags.PENDING).assertExists()
    }

    // ---- S42 ----

    private var savedAs: Pair<Long, Long>? = null
    private var closed = 0

    private fun openEditor(b: FakeContactsBackend, account: Long?, id: Long?) {
        ScreenshotKit.configure(360, 2000, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            EditContactScreen(
                backend = b, account = account, id = id,
                accounts = listOf(ContactsAccount(1, "Sam"), ContactsAccount(2, "Work")),
                books = b.books,
                onSaved = { a, i -> savedAs = a to i }, onClose = { closed++ },
            )
        }
        rule.waitForIdle()
    }

    @Test
    fun saveIsOffUntilSomethingChanges_andANewContactGoesToTheChosenBook() {
        val b = backend()
        openEditor(b, null, null)
        rule.onNodeWithTag(EditContactTags.SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(EditContactTags.FIRST).performTextInput("Nora")
        rule.onNodeWithTag(EditContactTags.LAST).performTextInput("Lindqvist")
        rule.onNodeWithTag(EditContactTags.email(0)).performTextInput("nora@example.org")
        rule.onNodeWithTag(EditContactTags.SAVE).assertIsEnabled()
        // The default book is the first account's first; pick the second account's.
        rule.onNodeWithTag(EditContactTags.SAVE_TO).performScrollTo().assert(hasText("Personal (Sam)", substring = true)).performClick()
        rule.onNodeWithText("Team (Work)").performClick()
        rule.onNodeWithTag(EditContactTags.SAVE).performClick()
        rule.waitUntil(5_000) { savedAs != null }
        assertEquals(Triple(2L, null, 20L), b.saved.single())
        val id = savedAs!!.second
        assertEquals(2L, savedAs!!.first)
        val form = b.form(2, id)
        assertEquals("Nora Lindqvist", form.formattedName)
        assertEquals(listOf(LabeledValue("nora@example.org")), form.emails)
        assertTrue(form.phones.isEmpty())
    }

    @Test
    fun editingKeepsTheIdAndTheBook_andAddsANumber() {
        val b = backend()
        openEditor(b, 1, 3)
        rule.onNodeWithTag(EditContactTags.FIRST).assert(hasText("Mira", substring = true))
        // There is no "Save to" for a contact that already has a book.
        assertTrue(rule.onAllNodes(hasTestTag(EditContactTags.SAVE_TO)).fetchSemanticsNodes().isEmpty())
        rule.onNodeWithTag(EditContactTags.ADD_PHONE).performScrollTo().performClick()
        rule.onNodeWithTag(EditContactTags.phone(1)).performScrollTo().performTextInput("+44 7700 900456")
        rule.onNodeWithTag(EditContactTags.SAVE).performClick()
        rule.waitUntil(5_000) { savedAs != null }
        assertEquals(Triple(1L, 3L, 10L), b.saved.single())
        assertEquals(1L to 3L, savedAs)
        val form = b.form(1, 3)
        assertEquals(listOf("+44 7700 900123", "+44 7700 900456"), form.phones.map { it.value })
        // The first number keeps its kind.
        assertEquals(listOf("cell"), form.phones[0].kinds)
    }

    @Test
    fun closingWithChangesAsksBeforeDiscarding_andWithoutChangesJustCloses() {
        val b = backend()
        openEditor(b, 1, 3)
        rule.onNodeWithTag(EditContactTags.CLOSE).performClick()
        assertEquals(1, closed)
        rule.onNodeWithTag(EditContactTags.NOTES).performScrollTo().performTextInput("Runs the DNS side")
        rule.onNodeWithTag(EditContactTags.CLOSE).performClick()
        rule.onNodeWithText("Discard changes?").assertExists()
        rule.onNodeWithText("Cancel").performClick()
        assertEquals(1, closed)
        rule.onNodeWithTag(EditContactTags.CLOSE).performClick()
        rule.onNodeWithText("Discard").performClick()
        assertEquals(2, closed)
        assertTrue(b.saved.isEmpty())
    }

    @Test
    fun clearingEveryNameAndAddressTurnsSaveOff() {
        val b = backend()
        openEditor(b, 1, 3)
        rule.onNodeWithTag(EditContactTags.FIRST).performTextClearance()
        rule.onNodeWithTag(EditContactTags.LAST).performTextClearance()
        rule.onNodeWithTag(EditContactTags.email(0)).performTextClearance()
        rule.onNodeWithTag(EditContactTags.SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(EditContactTags.FIRST).performTextInput("Mira")
        rule.onNodeWithTag(EditContactTags.SAVE).assertIsEnabled()
    }

    @Test
    fun aSaveThatFailsKeepsTheForm_andSaysSo() {
        val b = backend().apply { failSave = true }
        openEditor(b, 1, 3)
        rule.onNodeWithTag(EditContactTags.ORGANIZATION).performScrollTo().performTextInput(" Ltd")
        rule.onNodeWithTag(EditContactTags.SAVE).performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Couldn't save the contact")).fetchSemanticsNodes().isNotEmpty() }
        assertNull(savedAs)
        rule.onNodeWithTag(EditContactTags.SAVE).assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.Disabled))
    }
}
