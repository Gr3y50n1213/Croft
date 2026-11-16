// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.mail.calendar.EventPerson
import com.example.croft.mail.calendar.RsvpChoice
import com.example.croft.mail.calendar.RsvpProblem
import com.example.croft.mail.calendar.RsvpResult
import com.example.croft.mail.reading.CalendarChoice
import com.example.croft.mail.reading.InvitationClashData
import com.example.croft.mail.reading.InvitationData
import com.example.croft.mail.reading.InvitationKind
import com.example.croft.ui.calendar.RsvpTags
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** T40b: S22's invitation card: what it shows, nothing called until a tap, the answers and their outcomes. Made-up people only. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class InvitationTest {
    @get:Rule
    val rule = createComposeRule()

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun textExists(text: String) = rule.onAllNodes(hasText(text, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private val noon = 1_794_744_000_000L // 2026-11-06T12:00:00Z

    private fun invite(
        kind: InvitationKind = InvitationKind.REQUEST,
        organizerIsSender: Boolean = true,
        canRespond: Boolean = true,
        outdated: Boolean = false,
        canRemove: Boolean = false,
        storedEvent: Long? = null,
        myAnswer: RsvpChoice? = null,
    ) = InvitationData(
        messageId = NEWEST, account = 1, kind = kind, title = "Lunch with Mira", location = "The Mill",
        startMs = noon, endMs = noon + 3_600_000, allDay = false, repeats = false,
        organizer = EventPerson("mira@example.net", "Mira Okafor", "ACCEPTED"), organizerIsSender = organizerIsSender, ownEvent = false,
        attendees = listOf(
            EventPerson("mira@example.net", "Mira Okafor", "ACCEPTED"),
            EventPerson("sam@example.org", "Sam", "NEEDS-ACTION"),
            EventPerson("ben@example.com", "Ben", null),
        ),
        attendeeCount = 5, invitedAs = "sam@example.org", myAnswer = myAnswer,
        clashes = listOf(InvitationClashData(77, "Standup", noon + 1_800_000, noon + 2_700_000)),
        storedEvent = storedEvent, storedCalendar = null, outdated = outdated, organizerDiffers = false,
        canRespond = canRespond, canRemove = canRemove, cancelsAll = kind == InvitationKind.CANCEL, repliedBy = null,
    )

    private fun fixture(i: InvitationData): ReadingFixture = ReadingFixture().also { f ->
        f.reading.invitations[NEWEST] = i
        f.reading.calendarChoices += CalendarChoice(10, "Personal", null)
        f.reading.calendarChoices += CalendarChoice(11, "Work", "#3A7D44")
    }

    private fun answers(f: ReadingFixture) = f.reading.calls.filter { it.startsWith("respond:") || it.startsWith("cancel:") }

    @Test
    fun aRequestShowsWhatWhenWhereWhoAndTheClash_andNothingIsSentOnOpen() {
        val f = fixture(invite())
        f.show(rule)
        rule.waitForTag(InvitationTags.CARD)
        assertTrue(textExists("Mira Okafor invited you"))
        assertTrue(textExists("Lunch with Mira"))
        assertTrue(textExists("The Mill"))
        assertTrue("the person is 'you'", textExists("Mira Okafor, you, Ben and 2 more"))
        assertTrue(exists(InvitationTags.CLASH) && textExists("Conflicts with Standup"))
        assertTrue(exists(InvitationTags.FOOTNOTE))
        assertTrue("two calendars to choose from", exists(InvitationTags.calendar(10)) && exists(InvitationTags.calendar(11)))
        assertTrue(!exists(InvitationTags.NOT_SENDER) && !exists(InvitationTags.OUTDATED))
        assertTrue("nothing sent or stored on open", answers(f).isEmpty())
    }

    @Test
    fun yesAnswersWithTheChosenCalendarAndWordsInThePersonsLanguage_andTheCardShowsIt() {
        val f = fixture(invite())
        f.show(rule)
        rule.waitForTag(InvitationTags.CARD)
        rule.waitForTag(InvitationTags.calendar(11))
        rule.onNodeWithTag(InvitationTags.calendar(11)).performScrollTo().performClick()
        // The choice shows before answering.
        rule.waitUntil(10_000) {
            rule.onAllNodes(hasTestTag(InvitationTags.calendar(11)).and(SemanticsMatcher.expectValue(SemanticsProperties.Selected, true)))
                .fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNodeWithTag(RsvpTags.answer(RsvpChoice.YES)).performScrollTo().performClick()
        rule.waitUntil(10_000) { answers(f).isNotEmpty() }
        assertEquals(listOf("respond:$NEWEST:YES:11"), answers(f))
        assertEquals("Accepted: Lunch with Mira", f.reading.lastWords?.subject)
        rule.waitUntil(10_000) { textExists("You're going") }
        // The card reloads with the answer chosen.
        rule.waitUntil(10_000) {
            rule.onAllNodes(hasTestTag(RsvpTags.answer(RsvpChoice.YES)).and(SemanticsMatcher.expectValue(SemanticsProperties.Selected, true)))
                .fetchSemanticsNodes().isNotEmpty()
        }
    }

    @Test
    fun aProblemIsSaidPlainly() {
        val f = fixture(invite())
        f.reading.rsvpResult = RsvpResult(sent = false, eventId = 900, problem = RsvpProblem.CANNOT_SEND)
        f.show(rule)
        rule.waitForTag(InvitationTags.CARD)
        rule.onNodeWithTag(RsvpTags.answer(RsvpChoice.MAYBE)).performScrollTo().performClick()
        rule.waitUntil(10_000) { textExists("can't send mail") }
    }

    @Test
    fun notesSayWhenTheOrganiserIsNotTheSender_andAnOutdatedInvitationHasNoButtons() {
        val f = fixture(invite(organizerIsSender = false))
        f.reading.invitations[NEWEST] = invite(organizerIsSender = false).copy(organizer = EventPerson("boss@example.com", "Boss", null))
        f.show(rule)
        rule.waitForTag(InvitationTags.CARD)
        assertTrue(exists(InvitationTags.NOT_SENDER))
        assertTrue("names both addresses", textExists("Sent by mira@example.net on behalf of boss@example.com"))
    }

    @Test
    fun anOutdatedInvitationSaysSoAndOffersNoAnswer() {
        val f = fixture(invite(canRespond = false, outdated = true))
        f.show(rule)
        rule.waitForTag(InvitationTags.CARD)
        assertTrue(exists(InvitationTags.OUTDATED))
        assertTrue(!exists(RsvpTags.answer(RsvpChoice.YES)) && !exists(InvitationTags.FOOTNOTE))
    }

    @Test
    fun aSharedEventIsAdded() {
        val f = fixture(invite(kind = InvitationKind.PUBLISH))
        f.show(rule)
        rule.waitForTag(InvitationTags.ADD)
        assertTrue(textExists("Mira Okafor shared an event"))
        rule.onNodeWithTag(InvitationTags.ADD).performScrollTo().performClick()
        rule.waitUntil(10_000) { answers(f).isNotEmpty() }
        assertEquals(listOf("respond:$NEWEST:YES:10"), answers(f))
        rule.waitUntil(10_000) { textExists("Added to your calendar") }
    }

    @Test
    fun aCancellationIsRemovedOnlyWhenItCanBe() {
        val f = fixture(invite(kind = InvitationKind.CANCEL, canRespond = false, canRemove = true, storedEvent = 900))
        f.show(rule)
        rule.waitForTag(InvitationTags.REMOVE)
        assertTrue(textExists("Mira Okafor cancelled this event"))
        rule.onNodeWithTag(InvitationTags.REMOVE).performScrollTo().performClick()
        rule.waitUntil(10_000) { answers(f).isNotEmpty() }
        assertEquals(listOf("cancel:$NEWEST"), answers(f))
        rule.waitUntil(10_000) { textExists("Removed from your calendar") }
    }

    @Test
    fun aCancellationFromSomeoneElseSaysWhyNot() {
        val f = fixture(invite(kind = InvitationKind.CANCEL, organizerIsSender = false, canRespond = false, canRemove = false, storedEvent = 900))
        f.show(rule)
        rule.waitForTag(InvitationTags.CANNOT_REMOVE)
        assertTrue(!exists(InvitationTags.REMOVE))
        assertTrue(answers(f).isEmpty())
    }
}
