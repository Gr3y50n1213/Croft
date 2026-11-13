// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.contacts

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure rules of S40 and S42: sections A to Z, their order, and the editor (changed, can save, the form saved). Made-up names. */
class ContactRulesTest {
    private fun row(id: Long, key: String) = ContactRow(id, 1, key, key, null)

    @Test
    fun theSectionIsTheFirstLetterWithoutAccents_elseHash() {
        assertEquals("A", ContactSections.of("aoife"))
        assertEquals("E", ContactSections.of("émile"))
        assertEquals("O", ContactSections.of("okafor mira"))
        assertEquals("#", ContactSections.of("42 printing"))
        assertEquals("#", ContactSections.of(""))
        assertEquals("#", ContactSections.of("张"))
    }

    @Test
    fun rowsAreOrderedBySectionThenKeyIgnoringAccents_withHashLast() {
        val ordered = ContactSections.order(listOf(row(1, "zed"), row(2, "émile"), row(3, "eve"), row(4, "42 printing"), row(5, "adam")))
        assertEquals(listOf(5L, 2L, 3L, 1L, 4L), ordered.map { it.id })
    }

    @Test
    fun stateItemsPutAHeadingBeforeEachSection() {
        val s = ContactsState(rows = ContactSections.order(listOf(row(1, "adam"), row(2, "anna"), row(3, "bea"))))
        assertEquals(
            listOf(ContactsItem.Header("A"), ContactsItem.Row(s.rows[0]), ContactsItem.Row(s.rows[1]), ContactsItem.Header("B"), ContactsItem.Row(s.rows[2])),
            s.items,
        )
        assertEquals(setOf("A", "B"), s.letters)
    }

    @Test
    fun nothingChanged_soSaveIsOff_andANewContactNeedsANameOrAnAddress() {
        val editor = ContactEditor(ContactForm())
        assertFalse(editor.changed(editor.initial))
        assertFalse(editor.canSave(editor.initial))
        // Spaces are not a change.
        assertFalse(editor.changed(editor.initial.copy(first = "  ")))
        // A phone alone is a change, but not enough to save.
        val phoneOnly = editor.initial.copy(phones = listOf(LabeledValue("+44 7700 900123")))
        assertTrue(editor.changed(phoneOnly))
        assertFalse(editor.canSave(phoneOnly))
        assertTrue(editor.canSave(phoneOnly.copy(emails = listOf(LabeledValue("mira@example.org")))))
        assertTrue(editor.canSave(editor.initial.copy(first = "Mira")))
    }

    @Test
    fun aNewContactGetsItsFullNameFromFirstAndLast_andBlankRowsAreDropped() {
        val editor = ContactEditor(ContactForm())
        val form = editor.toForm(editor.initial.copy(first = " Mira ", last = "Okafor", emails = listOf(LabeledValue(""), LabeledValue("mira@example.org"))))
        assertEquals("Mira Okafor", form.formattedName)
        assertEquals("Mira", form.givenName)
        assertEquals("Okafor", form.familyName)
        assertEquals(listOf(LabeledValue("mira@example.org")), form.emails)
        assertTrue(form.phones.isEmpty())
        assertTrue(form.addresses.isEmpty())
    }

    @Test
    fun editingKeepsWhatS42DoesNotShow_andEachValueKeepsItsKinds() {
        val original = ContactForm(
            formattedName = "Dr. Mira Okafor", givenName = "Mira", familyName = "Okafor",
            emails = listOf(LabeledValue("mira@example.org", listOf("work"), preferred = true), LabeledValue("mira@home.example", listOf("home"))),
            addresses = listOf(PostalAddress(street = "1 Example Street", kinds = listOf("work")), PostalAddress(street = "2 Other Road", kinds = listOf("home"))),
            birthday = "1990-04-01",
        )
        val editor = ContactEditor(original)
        // Only the notes change: the full name, the birthday and the second postal address stay as they were.
        val notes = editor.toForm(editor.initial.copy(note = "Runs the DNS side"))
        assertEquals("Dr. Mira Okafor", notes.formattedName)
        assertEquals("1990-04-01", notes.birthday)
        assertEquals(original.addresses, notes.addresses)
        assertEquals("Runs the DNS side", notes.note)
        // The first address is removed: the second keeps its own kind ("home"), not the first one's.
        val removed = editor.toForm(editor.initial.copy(emails = editor.initial.emails.drop(1)))
        assertEquals(listOf(LabeledValue("mira@home.example", listOf("home"))), removed.emails)
        // Changing a name rebuilds the full name from first and last.
        assertEquals("Mira Lund", editor.toForm(editor.initial.copy(last = "Lund")).formattedName)
    }

    @Test
    fun aCardWithOnlyAFullNameShowsItAsTheFirstName_andKeepsItWhenUntouched() {
        val original = ContactForm(formattedName = "Hollis Printing", emails = listOf(LabeledValue("orders@hollis.example")))
        val editor = ContactEditor(original)
        assertEquals("Hollis Printing", editor.initial.first)
        val form = editor.toForm(editor.initial.copy(organization = "Hollis"))
        assertEquals("Hollis Printing", form.formattedName)
        assertEquals("", form.givenName)
        assertEquals("", form.familyName)
    }

    @Test
    fun postalAddressLinesSkipEmptyParts() {
        assertEquals(listOf("1 Example Street", "Exampleton EX1 2AB", "Example Land"), PostalAddress("1 Example Street", "Exampleton", "", "EX1 2AB", "Example Land").lines())
        assertTrue(PostalAddress().isBlank)
    }

    @Test
    fun nothingPersonalIsPrinted() {
        val form = ContactForm(formattedName = "Mira Okafor", emails = listOf(LabeledValue("mira@example.org")))
        val printed = listOf(form.toString(), ContactRow(1, 2, "Mira Okafor", "okafor mira", "mira@example.org").toString(), EditorFields.of(form).toString(), ContactCard(1, 2, 3, "Mira Okafor", form, false, false, false).toString())
        for (p in printed) {
            assertFalse(p, p.contains("Mira") || p.contains("example.org") || p.contains("okafor"))
        }
    }
}
