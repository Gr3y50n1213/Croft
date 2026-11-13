// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.Occurrence
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle

/** Test tags of the calendar screens. */
object CalendarTags {
    const val SCREEN = "calendar-screen"
    const val LIST = "calendar-agenda"
    const val MONTH = "calendar-month"
    const val NEW = "calendar-new"
    const val VIEW_AGENDA = "calendar-view-agenda"
    const val VIEW_MONTH = "calendar-view-month"
    const val CALENDARS = "calendar-calendars"
    const val PROBLEMS = "calendar-problems"
    const val EMPTY = "calendar-empty"
    const val PREVIOUS = "calendar-previous"
    const val NEXT = "calendar-next"
    const val MORE = "calendar-more"
    const val OVERLAP = "calendar-overlap"
    fun day(date: LocalDate) = "calendar-day-$date"
    fun cell(date: LocalDate) = "calendar-cell-$date"
    fun event(eventId: Long, keyMs: Long) = "calendar-event-$eventId-$keyMs"
}

/** A calendar's colour from `#RRGGBB`, else [fallback] (the theme's primary): a calendar's own colour is data, not a design value. */
fun calendarColor(hex: String?, fallback: Color): Color =
    hex?.let { runCatching { Color(android.graphics.Color.parseColor(it)) }.getOrNull() } ?: fallback

/** The times of [o] as S34 writes them: "All day", or "12:00 to 13:00" in [zone]. */
@Composable
fun timeText(o: Occurrence, zone: ZoneId): String {
    if (o.allDay) return stringResource(R.string.calendar_all_day)
    val f = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(LocalConfiguration.current.locales[0])
    val s = Instant.ofEpochMilli(o.startMs).atZone(zone).format(f)
    val e = Instant.ofEpochMilli(o.endMs).atZone(zone).format(f)
    return stringResource(R.string.calendar_time_range, s, e)
}

/** S34's day label: the weekday above the date; today's date in a filled circle. */
@Composable
fun DayHeader(date: LocalDate, today: Boolean, modifier: Modifier = Modifier) {
    val locale = LocalConfiguration.current.locales[0]
    val weekday = date.dayOfWeek.getDisplayName(TextStyle.SHORT, locale).uppercase(locale)
    val spoken = date.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.FULL).withLocale(locale)) +
        if (today) ", " + stringResource(R.string.calendar_today) else ""
    Column(
        modifier.widthIn(min = 48.dp).semantics { heading(); contentDescription = spoken },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(weekday, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
        Box(
            // At least 40 dp, larger with large text, so the date always fits.
            Modifier.sizeIn(minWidth = 40.dp, minHeight = 40.dp).clip(CircleShape)
                .then(if (today) Modifier.background(MaterialTheme.colorScheme.primary) else Modifier).padding(4.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                date.dayOfMonth.toString(),
                style = MaterialTheme.typography.titleLarge,
                color = if (today) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
            )
        }
    }
}

/**
 * One event in S34 and S36: a card with its calendar's colour as a bar on the leading edge (the text never relies on the colour),
 * the title, the times and the place; a warning icon when it overlaps another event that day; small notes for an unknown time zone
 * or a repeat Croft can't show.
 */
@Composable
fun EventCard(o: Occurrence, zone: ZoneId, overlaps: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val accent = calendarColor(o.color, MaterialTheme.colorScheme.primary)
    Surface(
        modifier.fillMaxWidth().heightIn(min = 56.dp).clip(MaterialTheme.shapes.large).clickable(role = Role.Button, onClick = onClick),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(4.dp).fillMaxHeight().background(accent).clearAndSetSemantics { })
            Column(Modifier.weight(1f).padding(horizontal = 12.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    o.title.ifBlank { stringResource(R.string.calendar_untitled) },
                    style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis,
                )
                Text(timeText(o, zone), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                o.location?.takeIf { it.isNotBlank() }?.let {
                    Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (o.zoneUnknown) Text(stringResource(R.string.calendar_zone_unknown), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (o.repeatsUnsupported) Text(stringResource(R.string.calendar_repeats_unsupported), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (overlaps) {
                val label = stringResource(R.string.calendar_overlaps)
                Icon(
                    Icons.Filled.Warning, contentDescription = label,
                    modifier = Modifier.padding(12.dp).size(20.dp).testTag(CalendarTags.OVERLAP),
                    tint = MaterialTheme.colorScheme.tertiary,
                )
            }
        }
    }
}
