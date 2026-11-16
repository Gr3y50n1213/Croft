// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

/** T40: an answer to an invitation. */
enum class RsvpChoice { YES, MAYBE, NO }

/** T40: why an answer was not (wholly) carried out (the core decides; see `RsvpProblem` in the core). */
enum class RsvpProblem { NOT_ANSWERABLE, OUTDATED, ORGANIZER_DIFFERS, NO_ORGANIZER, CANNOT_SEND, NO_CALENDAR }

/** T40: what an answer did: the reply is in the Outbox ([sent]); the event as stored now ([eventId]); what went wrong. */
data class RsvpResult(val sent: Boolean, val eventId: Long?, val problem: RsvpProblem?)

/** T40: the reply's subject and text, written in the person's language by the app. */
data class ReplyWords(val subject: String, val body: String) {
    // The subject carries the event's title: never logged.
    override fun toString(): String = "ReplyWords"
}
