// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.AccountId
import com.example.croft.mail.MessageId
import com.example.croft.mail.calendar.EventPerson
import com.example.croft.mail.calendar.RsvpChoice

/** T40: what an invitation message is. */
enum class InvitationKind { REQUEST, PUBLISH, CANCEL, REPLY }

/** T40: one of the person's own events at the same time as an invitation. */
data class InvitationClashData(val eventId: Long, val title: String, val startMs: Long, val endMs: Long) {
    override fun toString(): String = "InvitationClashData($eventId)"
}

/** A calendar an invitation can be added to. */
data class CalendarChoice(val id: Long, val name: String, val color: String?) {
    override fun toString(): String = "CalendarChoice($id)"
}

/**
 * T40, S22's card: what a message's invitation is (the core reads it; nothing is sent or stored by reading it). See `InvitationInfo` in
 * the core for each field.
 */
data class InvitationData(
    val messageId: MessageId,
    val account: AccountId,
    val kind: InvitationKind,
    val title: String,
    val location: String,
    val startMs: Long?,
    val endMs: Long?,
    val allDay: Boolean,
    val repeats: Boolean,
    val organizer: EventPerson?,
    val organizerIsSender: Boolean,
    val ownEvent: Boolean,
    val attendees: List<EventPerson>,
    val attendeeCount: Int,
    val invitedAs: String?,
    val myAnswer: RsvpChoice?,
    val clashes: List<InvitationClashData>,
    val storedEvent: Long?,
    val storedCalendar: Long?,
    val outdated: Boolean,
    val organizerDiffers: Boolean,
    val canRespond: Boolean,
    val canRemove: Boolean,
    val cancelsAll: Boolean,
    val repliedBy: EventPerson?,
) {
    // Titles, places and addresses are shown on screen but never logged.
    override fun toString(): String = "InvitationData($messageId, $kind, canRespond=$canRespond)"
}
