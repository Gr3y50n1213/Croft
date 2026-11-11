// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.assertCountEquals
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Chips, text fields, switches, radios and checkboxes: docs/ux/controls.md. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ChipsFieldsTogglesTest {
    @get:Rule
    val rule = createComposeRule()

    private val haptics = RecordingHaptics()
    private fun tag(t: String) = rule.onNodeWithTag(t)

    // ---- Chips ----

    @Test
    fun aFilterChipSelectsOnTap_andReadsItsCountWithItsLabel() {
        var selected by mutableStateOf(false)
        rule.show { CroftFilterChip("Updates", selected, { selected = !selected }, Modifier.testTag("chip"), count = 12) }
        tag("chip").assertIsNotSelected().assertTextContains("Updates", substring = true).assertTextContains("12", substring = true)
        tag("chip").performClick()
        tag("chip").assertIsSelected()
    }

    @Test
    fun aDisabledFilterChipIsInTheTreeAsDisabled() {
        rule.show { CroftFilterChip("Unread", false, {}, Modifier.testTag("chip"), enabled = false) }
        tag("chip").assertIsNotEnabled()
    }

    @Test
    fun aSuggestionChipTaps() {
        var taps = 0
        rule.show { CroftSuggestionChip("Thanks!", { taps++ }, Modifier.testTag("chip")) }
        tag("chip").performClick()
        assertEquals(1, taps)
    }

    @Test
    fun anInputChipInErrorSaysSoInWords_andNotJustInColour() {
        rule.show {
            Column {
                RecipientChip("mira@@example", RecipientChipState.ERROR, {}, Modifier.testTag("error"))
                RecipientChip("Mira Okafor", RecipientChipState.NORMAL, {}, Modifier.testTag("normal"), hasKey = true)
                RecipientChip("Dan", RecipientChipState.SELECTED, {}, Modifier.testTag("selected"))
            }
        }
        tag("error").assertTextContains("Invalid address", substring = true)
        assertEquals("Invalid address", tag("error").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
        assertTrue(tag("normal").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single().contains("key found"))
        assertEquals("Selected", tag("selected").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
    }

    @Test
    fun anInputChipOpensItsMenuOnTap_andLiftsToDragOnLongPress_withTheDragHaptic() {
        var taps = 0; var lifts = 0
        rule.show(haptics = haptics) { RecipientChip("Mira Okafor", RecipientChipState.NORMAL, { taps++ }, Modifier.testTag("chip"), onLongPress = { lifts++ }) }
        tag("chip").performClick()
        assertEquals(1, taps)
        tag("chip").performTouchInput { longClick() }
        assertEquals(1, lifts)
        assertEquals(1, taps)
        assertEquals(listOf(HapticEvent.DragStart), haptics.events)
    }

    @Test
    fun theAccountStatusChipIsHiddenWhenConnected_andReadsAsAccountAndStatusOtherwise() {
        rule.show {
            Column {
                for (s in AccountStatus.entries) AccountStatusChip("Sam", s, {}, Modifier.testTag("status-${s.name}"))
            }
        }
        tag("status-CONNECTED").assertDoesNotExistNow()
        val expected = mapOf(
            AccountStatus.SYNCING to "Account Sam, Syncing",
            AccountStatus.OFFLINE to "Account Sam, Offline",
            AccountStatus.NEEDS_SIGN_IN to "Account Sam, Needs sign-in",
            AccountStatus.SERVER_PROBLEM to "Account Sam, Server problem",
        )
        for ((s, text) in expected) {
            assertEquals(text, tag("status-${s.name}").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single())
        }
    }

    private fun androidx.compose.ui.test.SemanticsNodeInteraction.assertDoesNotExistNow() = assertDoesNotExist()

    // ---- Text fields ----

    @Test
    fun validationRunsWhenTheFieldLosesFocus_notWhileTyping_andThenUpdatesLive() {
        val v = FieldValidation { if ("@" in it) null else "Enter a full address" }
        v.onFocusChange(true)
        v.onValueChange("sam")
        assertNull("not while typing", v.error)
        v.onFocusChange(false)
        assertEquals("when the field loses focus", "Enter a full address", v.error)
        v.onFocusChange(true)
        v.onValueChange("sam@")
        assertNull("after a first error it updates live, so it clears as soon as it is fixed", v.error)
        v.onValueChange("sam")
        assertEquals("and comes back live", "Enter a full address", v.error)
    }

    @Test
    fun aFieldThatWasNeverEnteredIsNotCheckedByLeaving() {
        val v = FieldValidation { "bad" }
        v.onFocusChange(false)
        assertNull(v.error)
    }

    @Test
    fun aFieldWithNoErrorStaysSilentWhileTypingAfterALeaveThatPassed() {
        val v = FieldValidation { if (it.length >= 3) null else "short" }
        v.onFocusChange(true); v.onValueChange("abc"); v.onFocusChange(false)
        assertNull(v.error)
        v.onFocusChange(true); v.onValueChange("a")
        assertNull("no live checking until a first error has been shown", v.error)
    }

    @Test
    fun anErrorFieldShowsAnIconAndAMessage_andIsMarkedAsAnErrorForScreenReaders() {
        rule.show { CroftTextField("sam@", {}, "Email address", Modifier.testTag("field"), errorText = "Enter a full address like sam@example.org") }
        rule.onNodeWithText("Enter a full address like sam@example.org").assertIsDisplayed()
        assertEquals("Enter a full address like sam@example.org", tag("field").fetchSemanticsNode().config[SemanticsProperties.Error])
    }

    @Test
    fun disabledAndReadOnlyFieldsAreInTheTree() {
        rule.show {
            Column {
                CroftTextField("a", {}, "Disabled", Modifier.testTag("disabled"), enabled = false)
                CroftTextField("b", {}, "Read only", Modifier.testTag("readonly"), readOnly = true)
            }
        }
        tag("disabled").assertIsNotEnabled()
        tag("readonly").assertIsEnabled()
    }

    @Test
    fun theCounterReadsHowManyOfHowMany() {
        rule.show { CroftTextField("Lunch", {}, "Subject", maxLength = 78) }
        rule.onNodeWithText("5 of 78").assertIsDisplayed()
    }

    @Test
    fun aPasswordFieldHasAShowHideToggleThatIsAnnounced() {
        rule.show { CroftTextField("secret (made up)", {}, "Password", password = true) }
        rule.onNodeWithContentDescription("Show password").assertIsDisplayed()
        assertEquals("Password hidden", rule.onNodeWithContentDescription("Show password").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
        rule.onNodeWithContentDescription("Show password").performClick()
        rule.onAllNodesWithContentDescription("Show password").assertCountEquals(0)
        assertEquals("Password shown", rule.onNodeWithContentDescription("Hide password").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
    }

    @Test
    fun aFieldCanTellAutofillWhatItIs() {
        rule.show { CroftTextField("", {}, "Email", Modifier.testTag("field"), contentType = androidx.compose.ui.autofill.ContentType.EmailAddress) }
        assertNotNull(tag("field").fetchSemanticsNode().config.getOrNull(SemanticsProperties.ContentType))
    }

    // ---- Switches, radios, checkboxes ----

    @Test
    fun aSwitchRowTogglesFromAnywhereOnTheRow_andPlaysTheToggleHaptics() {
        var on by mutableStateOf(false)
        rule.show(haptics = haptics) { CroftSwitchRow("Sync over mobile data", on, { on = it }, Modifier.testTag("switch"), hint = "Takes effect after Croft restarts") }
        tag("switch").assertIsOff()
        rule.onNodeWithText("Takes effect after Croft restarts").assertIsDisplayed()
        tag("switch").performClick()
        tag("switch").assertIsOn()
        tag("switch").performClick()
        tag("switch").assertIsOff()
        assertEquals(listOf(HapticEvent.ToggleOn, HapticEvent.ToggleOff), haptics.events)
    }

    @Test
    fun aDisabledSwitchStaysInTheTreeAsDisabled_onAndOff() {
        rule.show {
            Column {
                CroftSwitchRow("A", true, {}, Modifier.testTag("on"), enabled = false)
                CroftSwitchRow("B", false, {}, Modifier.testTag("off"), enabled = false)
            }
        }
        tag("on").assertIsNotEnabled().assertIsOn()
        tag("off").assertIsNotEnabled().assertIsOff()
    }

    @Test
    fun aRadioRowSelectsFromAnywhereOnTheRow() {
        var selected by mutableStateOf(0)
        rule.show {
            Column {
                for (i in 0..2) CroftRadioRow("Option $i", selected == i, { selected = i }, Modifier.testTag("radio-$i"))
            }
        }
        tag("radio-0").assertIsSelected()
        tag("radio-2").performClick()
        tag("radio-2").assertIsSelected()
        tag("radio-0").assertIsNotSelected()
    }

    @Test
    fun aCheckboxCanBeIndeterminate_andATapOnItSelectsAll() {
        var state by mutableStateOf(ToggleableState.Indeterminate)
        var requested: Boolean? = null
        rule.show(haptics = haptics) { CroftCheckboxRow("Select all", state, { requested = it; state = if (it) ToggleableState.On else ToggleableState.Off }, Modifier.testTag("all")) }
        assertEquals(ToggleableState.Indeterminate, tag("all").fetchSemanticsNode().config[SemanticsProperties.ToggleableState])
        tag("all").performClick()
        assertEquals(true, requested)
        tag("all").assertIsOn()
        assertEquals(listOf(HapticEvent.SelectionTick), haptics.events)
    }
}
