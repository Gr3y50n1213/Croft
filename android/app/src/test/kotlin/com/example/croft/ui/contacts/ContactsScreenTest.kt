// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import com.example.croft.CroftRobolectricRunner
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.contacts.ContactRow
import com.example.croft.mail.contacts.ContactSections
import com.example.croft.mail.contacts.ContactsAccount
import com.example.croft.mail.contacts.ContactsController
import com.example.croft.mail.contacts.ContactsSync
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** S40 (Robolectric, in-memory backend): A to Z with headings, search, the account chips, the index, the syncs and the empty states. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ContactsScreenTest {
    @get:Rule
    val rule = createComposeRule()

    private val accounts = listOf(ContactsAccount(1, "Sam"), ContactsAccount(2, "Work"), ContactsAccount(3, "Sample", isDemo = true))
    private val opened = ArrayList<ContactRow>()
    private var added = 0
    private lateinit var controller: ContactsController

    private fun backend() = FakeContactsBackend().apply {
        book(1, 10, "Personal")
        book(2, 20, "Team")
        add(1, 1, "Aoife", "Brennan", "aoife@example.org")
        add(1, 2, "Hollis Printing", email = "orders@hollis.example")
        add(1, 3, "Mira", "Okafor", "mira@example.org")
        add(2, 4, "Mira", "Lund", "mlund@acme.example")
        add(2, 5, "Nora", "Lindqvist", "nora@example.org")
    }

    private fun open(b: FakeContactsBackend, height: Int = 1600, haptics: RecordingHaptics = RecordingHaptics()) {
        ScreenshotKit.configure(360, height, false, 1.0f)
        rule.show(AnimationLevel.OFF, haptics) {
            val scope = rememberCoroutineScope()
            val c = remember { ContactsController(b, { accounts }, scope).also { it.start() } }
            controller = c
            ContactsScreen(c, onOpen = { opened += it }, onAdd = { added++ }, onMail = {})
        }
        rule.waitUntil(10_000) { ::controller.isInitialized && controller.state.value.syncedOnce }
        rule.waitForIdle()
    }

    private fun rowExists(account: Long, id: Long) = rule.onAllNodes(hasTestTag(ContactsTags.row(account, id))).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun theListIsAToZByFamilyName_withAHeadingPerLetter() {
        open(backend())
        for (letter in listOf("B", "H", "L", "O")) rule.onNodeWithTag(ContactsTags.header(letter)).assertExists()
        assertEquals(listOf(1L, 2L, 5L, 4L, 3L), controller.state.value.rows.map { it.id })
        rule.onNodeWithText("aoife@example.org").assertExists()
    }

    @Test
    fun openingSyncsEveryAccountButTheSampleOne() {
        val b = backend()
        open(b)
        assertEquals(listOf(1L, 2L), b.synced)
    }

    @Test
    fun searchNarrowsTheList() {
        open(backend())
        rule.onNodeWithTag(ContactsTags.SEARCH).performTextInput("mira")
        rule.waitUntil(5_000) { !rowExists(1, 1) }
        assertTrue(rowExists(1, 3))
        assertTrue(rowExists(2, 4))
        rule.onNodeWithTag(ContactsTags.SEARCH).performTextInput("zzz")
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("No contacts match")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun anAccountChipShowsOnlyThatAccount_andAllShowsEveryone() {
        open(backend())
        rule.onNodeWithTag(ContactsTags.chip(2)).performClick()
        rule.waitUntil(5_000) { !rowExists(1, 1) }
        assertTrue(rowExists(2, 4))
        rule.onNodeWithTag(ContactsTags.chip(null)).performClick()
        rule.waitUntil(5_000) { rowExists(1, 1) }
        // The sample account has no address book, so it has no chip.
        assertTrue(rule.onAllNodes(hasTestTag(ContactsTags.chip(3))).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun aTapOpensTheContact_andAddOpensTheEditor() {
        open(backend())
        rule.onNodeWithTag(ContactsTags.row(1, 3)).performClick()
        assertEquals(listOf(3L), opened.map { it.id })
        rule.onNodeWithTag(ContactsTags.ADD).performClick()
        assertEquals(1, added)
    }

    @Test
    fun theIndexJumpsToALetter_withATickAtEachNewLetter() {
        val b = FakeContactsBackend().apply {
            book(1, 10, "Personal")
            var id = 1L
            for (family in listOf("Abbot", "Baker", "Carter", "Dunne", "Ellis", "Fisher", "Gray", "Hale", "Irwin", "Jones", "Keane", "Lowe", "Moss", "Nolan", "Owens", "Price", "Quinn", "Reed", "Shaw", "Todd", "Upton", "Vance", "Walsh", "Young", "Zeller")) {
                for (given in listOf("Ann", "Ben")) add(1, id++, given, family)
            }
        }
        val haptics = RecordingHaptics()
        open(b, height = 640, haptics = haptics)
        rule.onNodeWithTag(ContactsTags.header("A")).assertIsDisplayed()
        assertTrue(rule.onAllNodes(hasTestTag(ContactsTags.header("Z"))).fetchSemanticsNodes().isEmpty())
        // Touch the strip at the slot of Z (the 26th of 27 letters), then drag up to the slot of Y.
        val slots = ContactSections.INDEX.size.toFloat()
        rule.onNodeWithTag(AlphabetIndexTags.STRIP).performTouchInput {
            down(Offset(centerX, height * (25.5f / slots)))
            moveTo(Offset(centerX, height * (24.5f / slots)))
            up()
        }
        rule.waitForIdle()
        rule.onNodeWithTag(ContactsTags.header("Y")).assertIsDisplayed()
        assertEquals(listOf(HapticEvent.Tick, HapticEvent.Tick), haptics.events.filter { it == HapticEvent.Tick })
    }

    @Test
    fun aLetterWithoutContactsGoesToTheNextSection() {
        assertEquals("M", nearestSection("K", ContactSections.INDEX, setOf("A", "M")))
        assertEquals("M", nearestSection("Z", ContactSections.INDEX, setOf("A", "M")))
        assertEquals("#", nearestSection("Q", ContactSections.INDEX, setOf("A", "#")))
    }

    @Test
    fun aServerThatCannotBeReachedIsNamed_andSignInToo() {
        val b = backend().apply {
            syncResults[1] = ContactsSync.SIGN_IN_NEEDED
            syncResults[2] = ContactsSync.UNREACHABLE
        }
        open(b)
        rule.onNodeWithText("Couldn't reach Work. Pull down to try again.").assertExists()
        rule.onNodeWithText("Sam needs you to sign in again.").assertExists()
    }

    @Test
    fun noContactsYet_saysSo() {
        val b = FakeContactsBackend().apply { book(1, 10, "Personal") }
        open(b)
        rule.onNodeWithText("No contacts yet").assertExists()
    }

    @Test
    fun noAddressBookAnywhere_saysTheAccountsDoNotOfferContacts() {
        val b = FakeContactsBackend().apply { syncResults[1] = ContactsSync.NOT_OFFERED; syncResults[2] = ContactsSync.NOT_OFFERED }
        open(b)
        rule.onNodeWithText("Your accounts don't offer contacts").assertExists()
        // Not offered is not a problem to report.
        assertTrue(rule.onAllNodes(hasTestTag(ContactsTags.PROBLEMS)).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun moreThanAPageIsReadPageByPage() {
        val b = backend().apply { pageSize = 2 }
        open(b)
        assertEquals(5, controller.state.value.rows.size)
    }
}
