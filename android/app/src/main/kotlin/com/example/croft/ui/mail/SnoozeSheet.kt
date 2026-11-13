// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.ui.components.CroftBottomSheet
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.temporal.ChronoUnit
import java.time.temporal.TemporalAdjusters
import java.util.Locale

/** Test tags of S29 Snooze. */
object SnoozeTags {
    const val SHEET = "S29"
    const val PICK = "S29-pick"

    /** The tag of the preset [kind]. */
    fun preset(kind: SnoozePresets.Kind) = "S29-${kind.name.lowercase()}"
}

/** S29's presets as pure functions of the time and the time zone, so each case can be tested without a screen (T36). */
object SnoozePresets {
    /** The presets, in the sheet's order. */
    enum class Kind { LATER_TODAY, TOMORROW, THIS_WEEKEND, NEXT_WEEK }

    /** A preset and when it wakes (Unix ms). */
    data class Preset(val kind: Kind, val atMs: Long)

    /** After this hour "Later today" is three hours from now instead of 18:00. */
    const val LATER_TODAY_CUTOFF_HOUR = 15

    /**
     * The presets at [nowMs] in [zone]: Later today (18:00, or in three hours from 15:00 on), Tomorrow 08:00, This weekend (Saturday
     * 09:00; left out on Saturday and Sunday, when it would mean next week's), Next week (Monday 08:00).
     */
    fun presets(nowMs: Long, zone: ZoneId): List<Preset> {
        val now = ZonedDateTime.ofInstant(Instant.ofEpochMilli(nowMs), zone)
        val today = now.toLocalDate()
        val laterToday = if (now.hour < LATER_TODAY_CUTOFF_HOUR) today.atTime(18, 0).atZone(zone) else now.plusHours(3).truncatedTo(ChronoUnit.MINUTES)
        val tomorrow = today.plusDays(1).atTime(8, 0).atZone(zone)
        val saturday = today.with(TemporalAdjusters.next(DayOfWeek.SATURDAY)).atTime(9, 0).atZone(zone)
        val monday = today.with(TemporalAdjusters.next(DayOfWeek.MONDAY)).atTime(8, 0).atZone(zone)
        val weekend = now.dayOfWeek == DayOfWeek.SATURDAY || now.dayOfWeek == DayOfWeek.SUNDAY
        return listOfNotNull(
            Preset(Kind.LATER_TODAY, laterToday.toInstant().toEpochMilli()),
            Preset(Kind.TOMORROW, tomorrow.toInstant().toEpochMilli()),
            if (weekend) null else Preset(Kind.THIS_WEEKEND, saturday.toInstant().toEpochMilli()),
            Preset(Kind.NEXT_WEEK, monday.toInstant().toEpochMilli()),
        )
    }

    /** A picked day (the date picker's UTC midnight) and time, in [zone], as Unix ms. */
    fun picked(dayUtcMs: Long, hour: Int, minute: Int, zone: ZoneId): Long {
        val day = Instant.ofEpochMilli(dayUtcMs).atZone(ZoneOffset.UTC).toLocalDate()
        return day.atTime(LocalTime.of(hour, minute)).atZone(zone).toInstant().toEpochMilli()
    }

    /** "Sat 09:00" (or with the date when it is more than a week away), in [zone] and [locale]. */
    fun label(atMs: Long, nowMs: Long, zone: ZoneId, locale: Locale): String {
        val at = ZonedDateTime.ofInstant(Instant.ofEpochMilli(atMs), zone)
        val now = ZonedDateTime.ofInstant(Instant.ofEpochMilli(nowMs), zone)
        val time = at.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale))
        val farAway = ChronoUnit.DAYS.between(now.toLocalDate(), at.toLocalDate()) > 6
        val day = if (farAway) at.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale)) else at.format(DateTimeFormatter.ofPattern("EEE", locale))
        return "$day $time"
    }

    /** Today, for the date picker (it works in UTC days). */
    fun todayUtcMs(nowMs: Long, zone: ZoneId): Long =
        LocalDate.ofInstant(Instant.ofEpochMilli(nowMs), zone).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
}

/**
 * S29 Snooze: the presets with their times, and Pick date and time. [onPick] gets the wake time; the caller snoozes (T36). The
 * message comes back unread at that time, on this phone (without a Companion other devices keep showing it).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SnoozeSheet(
    nowMs: Long,
    zone: ZoneId,
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val locale = Locale.getDefault()
    val presets = remember(nowMs, zone) { SnoozePresets.presets(nowMs, zone) }
    var pickingDay by remember { mutableStateOf(false) }
    var pickedDay by remember { mutableStateOf<Long?>(null) }
    CroftBottomSheet(onDismiss = onDismiss, modifier = modifier.testTag(SnoozeTags.SHEET), hasUnsavedInput = false) {
        Text(stringResource(R.string.snooze_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(bottom = 8.dp).semantics { heading() })
        for (p in presets) {
            ListItem(
                headlineContent = { Text(stringResource(presetName(p.kind))) },
                trailingContent = { Text(SnoozePresets.label(p.atMs, nowMs, zone, locale), color = MaterialTheme.colorScheme.onSurfaceVariant) },
                modifier = Modifier.fillMaxWidth().clickable { onPick(p.atMs) }.testTag(SnoozeTags.preset(p.kind)),
            )
        }
        ListItem(
            headlineContent = { Text(stringResource(R.string.snooze_pick)) },
            modifier = Modifier.fillMaxWidth().clickable { pickingDay = true }.testTag(SnoozeTags.PICK),
        )
        Text(stringResource(R.string.snooze_this_phone), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
    }
    if (pickingDay) {
        val today = SnoozePresets.todayUtcMs(nowMs, zone)
        val state = rememberDatePickerState(initialSelectedDateMillis = today + 24L * 60 * 60 * 1000)
        DatePickerDialog(
            onDismissRequest = { pickingDay = false },
            confirmButton = {
                TextButton(onClick = { pickingDay = false; pickedDay = state.selectedDateMillis }, enabled = state.selectedDateMillis?.let { it >= today } == true) {
                    Text(stringResource(R.string.snooze_next))
                }
            },
            dismissButton = { TextButton(onClick = { pickingDay = false }) { Text(stringResource(R.string.snooze_cancel)) } },
        ) { DatePicker(state = state) }
    }
    pickedDay?.let { day ->
        val time = rememberTimePickerState(initialHour = 8, initialMinute = 0)
        AlertDialog(
            onDismissRequest = { pickedDay = null },
            confirmButton = {
                TextButton(onClick = {
                    val at = SnoozePresets.picked(day, time.hour, time.minute, zone)
                    pickedDay = null
                    if (at > nowMs) onPick(at)
                }) { Text(stringResource(R.string.snooze_confirm)) }
            },
            dismissButton = { TextButton(onClick = { pickedDay = null }) { Text(stringResource(R.string.snooze_cancel)) } },
            text = { TimePicker(state = time) },
        )
    }
}

private fun presetName(kind: SnoozePresets.Kind): Int = when (kind) {
    SnoozePresets.Kind.LATER_TODAY -> R.string.snooze_later_today
    SnoozePresets.Kind.TOMORROW -> R.string.snooze_tomorrow
    SnoozePresets.Kind.THIS_WEEKEND -> R.string.snooze_this_weekend
    SnoozePresets.Kind.NEXT_WEEK -> R.string.snooze_next_week
}
