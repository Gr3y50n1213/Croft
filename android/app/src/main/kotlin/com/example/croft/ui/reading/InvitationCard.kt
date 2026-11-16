// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.LocationOn
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.EventPerson
import com.example.croft.mail.calendar.RsvpChoice
import com.example.croft.mail.reading.CalendarChoice
import com.example.croft.mail.reading.InvitationData
import com.example.croft.mail.reading.InvitationKind
import com.example.croft.theme.statusColors
import com.example.croft.ui.calendar.RsvpButtons
import com.example.croft.ui.calendar.whenText
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftFilterChip
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

/** Test tags of S22's card. */
object InvitationTags {
    const val CARD = "S22"
    const val HEADLINE = "S22-headline"
    const val TITLE = "S22-title"
    const val WHEN = "S22-when"
    const val PEOPLE = "S22-people"
    const val CLASH = "S22-clash"
    const val NOT_SENDER = "S22-not-sender"
    const val OUTDATED = "S22-outdated"
    const val ORGANIZER_DIFFERS = "S22-organizer-differs"
    const val NOT_LISTED = "S22-not-listed"
    const val ADD = "S22-add"
    const val REMOVE = "S22-remove"
    const val CANNOT_REMOVE = "S22-cannot-remove"
    const val FOOTNOTE = "S22-footnote"
    fun calendar(id: Long) = "S22-calendar-$id"
}

/**
 * T40, S22: the invitation a message carries, above the conversation. What it is and when, where and with whom; the first clash with the
 * person's own events; notes when something is off (the organiser is not the sender, a newer version is stored, another organiser, not on
 * the guest list); then Yes, Maybe and No (a request), Add to calendar (a shared event) or Remove from calendar (a cancellation). Nothing
 * happens until the person taps.
 */
@Composable
fun InvitationCard(
    invitation: InvitationData,
    calendars: List<CalendarChoice>,
    chosenCalendar: Long?,
    answering: RsvpChoice?,
    removing: Boolean,
    onCalendar: (Long) -> Unit,
    onAnswer: (RsvpChoice) -> Unit,
    onRemove: () -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
    senderAddress: String? = null,
) {
    val i = invitation
    val organizerName = i.organizer?.let { it.name ?: it.address } ?: stringResource(R.string.invite_someone)
    Surface(modifier.fillMaxWidth().testTag(InvitationTags.CARD), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            val headline = when (i.kind) {
                InvitationKind.REQUEST -> stringResource(R.string.invite_invited_you, organizerName)
                InvitationKind.PUBLISH -> stringResource(R.string.invite_shared, organizerName)
                InvitationKind.CANCEL -> stringResource(R.string.invite_cancelled, organizerName)
                InvitationKind.REPLY -> {
                    val who = i.repliedBy?.let { it.name ?: it.address } ?: stringResource(R.string.invite_someone)
                    when (i.repliedBy?.status) {
                        "ACCEPTED" -> stringResource(R.string.invite_replied_yes, who)
                        "TENTATIVE" -> stringResource(R.string.invite_replied_maybe, who)
                        else -> stringResource(R.string.invite_replied_no, who)
                    }
                }
            }
            Text(headline, Modifier.testTag(InvitationTags.HEADLINE).semantics { heading() }, style = MaterialTheme.typography.titleSmall)
            Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (i.title.isNotBlank() || i.kind != InvitationKind.REPLY) {
                        Text(i.title.ifBlank { stringResource(R.string.calendar_untitled) }, Modifier.testTag(InvitationTags.TITLE), style = MaterialTheme.typography.titleMedium)
                    }
                    if (i.startMs != null && i.endMs != null) {
                        val time = whenText(i.startMs, i.endMs, i.allDay, zone)
                        val line = if (i.repeats) time + " · " + stringResource(R.string.invite_repeats) else time
                        Line(Icons.Filled.DateRange, line, Modifier.testTag(InvitationTags.WHEN))
                    }
                    if (i.location.isNotBlank()) Line(Icons.Filled.LocationOn, i.location)
                    if (i.kind == InvitationKind.REQUEST && i.attendees.isNotEmpty()) {
                        Line(Icons.Filled.Person, peopleText(i), Modifier.testTag(InvitationTags.PEOPLE))
                    }
                    i.clashes.firstOrNull()?.let { c ->
                        val locale = LocalConfiguration.current.locales[0]
                        val at = Instant.ofEpochMilli(c.startMs).atZone(zone).format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale))
                        val title = c.title.ifBlank { stringResource(R.string.calendar_untitled) }
                        val text = if (i.clashes.size > 1) stringResource(R.string.invite_clash_more, title, at, i.clashes.size - 1) else stringResource(R.string.invite_clash, title, at)
                        Surface(Modifier.fillMaxWidth().testTag(InvitationTags.CLASH), shape = MaterialTheme.shapes.small, color = statusColors().amberContainer, contentColor = statusColors().onAmberContainer) {
                            Row(Modifier.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Icon(Icons.Filled.Warning, contentDescription = null, Modifier.size(18.dp))
                                Text(text, style = MaterialTheme.typography.labelLarge)
                            }
                        }
                    }
                    Notes(i, senderAddress, organizerName)
                    Actions(i, calendars, chosenCalendar, answering, removing, onCalendar, onAnswer, onRemove)
                }
            }
            if (i.kind == InvitationKind.REQUEST && i.canRespond) {
                Text(
                    stringResource(R.string.invite_footnote),
                    Modifier.testTag(InvitationTags.FOOTNOTE),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun Notes(i: InvitationData, senderAddress: String?, organizerName: String) {
    if (!i.organizerIsSender && senderAddress != null && i.kind != InvitationKind.REPLY) {
        Note(stringResource(R.string.invite_not_sender, senderAddress, i.organizer?.address ?: organizerName), InvitationTags.NOT_SENDER)
    }
    if (i.outdated) Note(stringResource(R.string.invite_outdated), InvitationTags.OUTDATED)
    if (i.organizerDiffers) Note(stringResource(R.string.invite_organizer_differs), InvitationTags.ORGANIZER_DIFFERS)
    if (i.kind == InvitationKind.REQUEST && i.canRespond && i.invitedAs == null) Note(stringResource(R.string.invite_not_listed), InvitationTags.NOT_LISTED)
    if (i.ownEvent && i.kind == InvitationKind.REQUEST) Note(stringResource(R.string.invite_own_event), "S22-own")
}

@Composable
private fun Actions(
    i: InvitationData,
    calendars: List<CalendarChoice>,
    chosenCalendar: Long?,
    answering: RsvpChoice?,
    removing: Boolean,
    onCalendar: (Long) -> Unit,
    onAnswer: (RsvpChoice) -> Unit,
    onRemove: () -> Unit,
) {
    val adding = i.canRespond && i.storedEvent == null && (i.kind == InvitationKind.REQUEST || i.kind == InvitationKind.PUBLISH)
    if (adding && calendars.size > 1) {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.invite_calendar), style = MaterialTheme.typography.labelLarge)
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (c in calendars) CroftFilterChip(c.name, chosenCalendar == c.id, { onCalendar(c.id) }, Modifier.testTag(InvitationTags.calendar(c.id)))
            }
        }
    }
    when (i.kind) {
        InvitationKind.REQUEST -> if (i.canRespond) RsvpButtons(i.myAnswer, answering, onAnswer)
        InvitationKind.PUBLISH -> if (i.canRespond) {
            CroftButton(
                stringResource(R.string.invite_add), { onAnswer(RsvpChoice.YES) }, Modifier.testTag(InvitationTags.ADD),
                kind = ButtonKind.TONAL, enabled = answering == null,
                progress = if (answering != null) ButtonProgress.LOADING else ButtonProgress.IDLE,
            )
        }
        InvitationKind.CANCEL -> when {
            i.canRemove -> CroftButton(
                stringResource(R.string.invite_remove), onRemove, Modifier.testTag(InvitationTags.REMOVE),
                kind = ButtonKind.TONAL, enabled = !removing, progress = if (removing) ButtonProgress.LOADING else ButtonProgress.IDLE,
            )
            i.storedEvent == null -> Text(stringResource(R.string.invite_not_in_calendar), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            else -> Text(stringResource(R.string.invite_cannot_remove), Modifier.testTag(InvitationTags.CANNOT_REMOVE), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        InvitationKind.REPLY -> {}
    }
}

@Composable
private fun Note(text: String, tag: String) {
    Text(text, Modifier.testTag(tag), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** "Sam, Mira and you" (the person by "you"), and "and 12 more" past three names. */
@Composable
private fun peopleText(i: InvitationData): String {
    val you = stringResource(R.string.invite_you)
    val names = i.attendees.take(3).map { p: EventPerson ->
        if (i.invitedAs != null && p.address.equals(i.invitedAs, ignoreCase = true)) you else p.name ?: p.address
    }
    val shown = names.joinToString(", ")
    val more = i.attendeeCount - names.size
    return if (more > 0) stringResource(R.string.invite_people_more, shown, more) else shown
}

@Composable
private fun Line(icon: ImageVector, text: String, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, contentDescription = null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(text, style = MaterialTheme.typography.bodyMedium)
    }
}
