// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import com.example.croft.ui.components.AccountStatus
import com.example.croft.ui.components.AttachmentState
import com.example.croft.ui.components.BannerKind
import com.example.croft.ui.components.FabState
import com.example.croft.ui.components.SendCondition
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A missing state is a visible gap (docs/ux/build-order.md, step 3). The states are kept as data and iterated: first the tables
 * of docs/ux/controls.md themselves, read from the file, then the enumerations of the controls. Adding a row to the
 * document, or a value to a control, fails these tests until the gallery shows it.
 */
class GalleryCompletenessTest {
    private val sections = GalleryCatalog.sections
    private val entryIds = sections.flatMap { s -> s.entries.map { it.id } }.toSet()
    private val docRows = sections.flatMap { s -> s.entries.flatMap { it.docRows } }.toSet()

    @Test
    fun everyEntryIdIsUniqueAndNamedSectionDotState() {
        val all = sections.flatMap { s -> s.entries.map { s.id to it.id } }
        assertEquals("duplicate ids", all.size, all.map { it.second }.toSet().size)
        for ((section, id) in all) assertTrue("$id should start with $section.", id.startsWith("$section."))
    }

    /** The first-column cells of the markdown tables of docs/ux/controls.md whose header is one of [headers]. */
    private fun firstColumn(headers: Set<String>): List<String> {
        val lines = File("../../docs/ux/controls.md").readLines()
        val cells = ArrayList<String>()
        var i = 0
        while (i < lines.size) {
            val header = lines[i].trim()
            if (header.startsWith("|") && lines.getOrNull(i + 1)?.trim()?.startsWith("| ---") == true &&
                header.trim('|').split('|').first().trim() in headers
            ) {
                var j = i + 2
                while (j < lines.size && lines[j].trim().startsWith("|")) {
                    cells += lines[j].trim().trim('|').split('|').first().trim()
                    j++
                }
                i = j
            } else i++
        }
        return cells
    }

    @Test
    fun theTablesOfControlsDotMdAreFound() {
        assertEquals("common states (8) and message row (11)", 19, firstColumn(setOf("State")).size)
        assertEquals(7, firstColumn(setOf("Button")).size)
        assertEquals(10, firstColumn(setOf("Condition")).size)
        assertEquals(4, firstColumn(setOf("Chip")).size)
    }

    @Test
    fun everyRowOfTheStateTablesHasAGalleryEntry() {
        val required = firstColumn(setOf("State", "Button", "Condition", "Chip"))
        assertTrue("found only ${required.size} rows; is the working directory the app module?", required.size >= 40)
        // The first word of a doc row may carry a detail in brackets that the gallery spells the same way; compare as written,
        // except where the doc adds words after a comma or "and".
        val missing = required.filter { row -> row !in docRows }
        assertTrue("controls.md rows without a gallery entry: $missing", missing.isEmpty())
    }

    @Test
    fun everyValueOfTheControlEnumerationsHasItsOwnEntry() {
        val expected = buildSet {
            SendCondition.entries.forEach { add("send.${it.name.lowercase()}") }
            AttachmentState.entries.forEach { add("attachment.${it.name.lowercase()}") }
            BannerKind.entries.forEach { add("banner.${it.name.lowercase()}") }
            FabState.entries.filter { it != FabState.HIDDEN }.forEach { add("fab.${it.name.lowercase()}") }
            add("fab.hidden")
            // Buttons: one entry per kind (the two danger kinds share one).
            addAll(listOf("buttons.filled", "buttons.tonal", "buttons.outlined", "buttons.text", "buttons.danger"))
            // Chips: one entry per chip, with all of its states side by side; every account status is drawn by the status entry.
            addAll(listOf("chips.filter", "chips.input", "chips.suggestion", "chips.account_status"))
            addAll(listOf("row.read", "row.unread", "row.pressed", "row.focused", "row.selected", "row.dragging", "row.new", "row.starred", "row.encrypted", "row.pending", "row.failed"))
        }
        val missing = expected - entryIds
        assertTrue("no gallery entry for: $missing", missing.isEmpty())
        assertEquals("every account status is a value the status entry loops over", 5, AccountStatus.entries.size)
    }

    @Test
    fun everyKindOfControlOfTheSpecHasASection() {
        val ids = sections.map { it.id }.toSet()
        // Spec: buttons, FAB, Send, chips, text fields, switch/radio/checkbox, message row, attachment chip, banner, snackbar,
        // dialog, bottom sheet, menus, icon-only button with tooltip, long-press; (T14) the markers a row has in the list, (T15) the swipe card and pill,
        // (T38b) S40's A to Z index, and (T39b) the calendar's day header and event card.
        val required = setOf("buttons", "fab", "send", "chips", "fields", "toggles", "row", "swipe", "listmarkers", "attachment", "banner", "snackbar", "dialog", "sheet", "menu", "iconbutton", "longpress", "common", "index", "calendar")
        assertEquals(required, ids)
    }

    @Test
    fun theTextFieldStatesOfTheSpecAreAllThere() {
        val fields = sections.first { it.id == "fields" }.entries.map { it.id.substringAfter('.') }.toSet()
        assertTrue(fields.containsAll(setOf("empty", "focused", "filled", "error", "disabled", "read_only", "icons", "supporting_counter", "password")))
    }

    @Test
    fun theSwitchRadioAndCheckboxStatesOfTheSpecAreAllThere() {
        val toggles = sections.first { it.id == "toggles" }.entries.map { it.id.substringAfter('.') }.toSet()
        assertTrue(toggles.containsAll(setOf("switch_off", "switch_on", "switch_pressed", "switch_disabled_on", "switch_disabled_off", "radio", "checkbox")))
    }
}
