// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.calendar.Occurrence
import com.example.croft.mail.calendar.WeekLayout
import com.example.croft.ui.components.CroftIconButton
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import kotlin.math.roundToInt

/** Test tags of S35. */
object WeekTags {
    const val GRID = "week-grid"
    const val PREVIOUS = "week-previous"
    const val NEXT = "week-next"
    const val NOW = "week-now"
    fun day(date: LocalDate) = "week-day-$date"
    fun block(eventId: Long, keyMs: Long) = "week-block-$eventId-$keyMs"
}

/**
 * S35 Week: [days] days from [start] as a time grid. All-day events in a strip at the top; timed ones as blocks in their calendar's
 * colour, side by side where they overlap; a line at the current time on today. A tap on an empty slot calls [onNewAt] (the slot's
 * quarter hour); a long press lifts a block (Drag start haptic) and dragging moves it in 15-minute steps and across days (a Tick
 * at each step); letting go calls [onMove] with the new start. The block jumps from step to step, with no animation at any level.
 * Screen readers get a "Move" action on each block, which calls [onMoveRequest] (the editor's times) instead of dragging.
 */
@Composable
fun WeekGrid(
    occurrences: List<Occurrence>,
    start: LocalDate,
    days: Int,
    today: LocalDate,
    nowMs: Long,
    zone: ZoneId,
    onOpen: (Occurrence) -> Unit,
    onNewAt: (LocalDateTime) -> Unit,
    onMove: (Occurrence, Long) -> Unit,
    onMoveRequest: (Occurrence) -> Unit,
    onShift: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val dates = remember(start, days) { (0 until days).map { start.plusDays(it.toLong()) } }
    val scroll = rememberScrollState()
    val density = LocalDensity.current
    val hourPx = with(density) { HOUR.toPx() }
    // Open at 08:00 (or the current hour when it is later), once.
    LaunchedEffect(Unit) {
        val hour = if (today in dates) Instant.ofEpochMilli(nowMs).atZone(zone).hour.coerceAtLeast(1) - 1 else 8
        scroll.scrollTo((hour.coerceAtMost(8) * hourPx).roundToInt())
    }
    Column(modifier.fillMaxSize().testTag(WeekTags.GRID)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CroftIconButton(Icons.AutoMirrored.Filled.KeyboardArrowLeft, stringResource(R.string.week_previous), { onShift(-1) }, Modifier.testTag(WeekTags.PREVIOUS))
            Row(Modifier.weight(1f)) {
                for (d in dates) {
                    Box(Modifier.weight(1f), contentAlignment = Alignment.Center) { DayHeader(d, d == today) }
                }
            }
            CroftIconButton(Icons.AutoMirrored.Filled.KeyboardArrowRight, stringResource(R.string.week_next), { onShift(1) }, Modifier.testTag(WeekTags.NEXT))
        }
        // All-day strip.
        Row(Modifier.fillMaxWidth().padding(start = GUTTER)) {
            for (d in dates) {
                Column(Modifier.weight(1f).padding(horizontal = 1.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    for (o in WeekLayout.allDay(occurrences, d, zone)) {
                        Text(
                            o.title.ifBlank { stringResource(R.string.calendar_untitled) },
                            Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small)
                                .background(calendarColor(o.color, MaterialTheme.colorScheme.primary).copy(alpha = BLOCK_ALPHA))
                                .clickable { onOpen(o) }.padding(horizontal = 4.dp, vertical = 2.dp)
                                .testTag(WeekTags.block(o.eventId, o.keyMs)),
                            style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(Modifier.fillMaxSize().verticalScroll(scroll)) {
            Column(Modifier.width(GUTTER)) {
                for (h in 0 until 24) {
                    Box(Modifier.height(HOUR).fillMaxWidth(), contentAlignment = Alignment.TopEnd) {
                        if (h > 0) Text("%02d".format(h), Modifier.padding(end = 6.dp).clearAndSetSemantics { }, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            BoxWithConstraints(Modifier.weight(1f).height(HOUR * 24)) {
                val dayWidth = maxWidth / days
                val dayWidthPx = with(density) { dayWidth.toPx() }
                // Hour lines.
                for (h in 1 until 24) {
                    HorizontalDivider(Modifier.offset(y = HOUR * h), color = MaterialTheme.colorScheme.outlineVariant)
                }
                dates.forEachIndexed { i, d ->
                    Box(
                        Modifier.offset(x = dayWidth * i).width(dayWidth).fillMaxHeight()
                            .testTag(WeekTags.day(d))
                            .pointerInput(d) {
                                detectTapGestures { p ->
                                    val minute = WeekLayout.snapDown((p.y / hourPx * 60f).toInt())
                                    onNewAt(d.atStartOfDay().plusMinutes(minute.toLong()))
                                }
                            },
                    )
                    for (p in WeekLayout.place(occurrences, d, zone)) {
                        Block(p.occurrence, p.startMinute, p.endMinute, p.column, p.columns, dayWidth, i, hourPx, dayWidthPx, days, zone, onOpen, onMove, onMoveRequest)
                    }
                    if (d == today) {
                        val now = Instant.ofEpochMilli(nowMs).atZone(zone)
                        val y = HOUR * ((now.hour * 60 + now.minute) / 60f)
                        Box(Modifier.offset(x = dayWidth * i, y = y).width(dayWidth).height(2.dp).background(MaterialTheme.colorScheme.error).testTag(WeekTags.NOW).clearAndSetSemantics { })
                    }
                }
            }
        }
    }
}

@Composable
private fun Block(
    o: Occurrence,
    startMinute: Int,
    endMinute: Int,
    column: Int,
    columns: Int,
    dayWidth: Dp,
    dayIndex: Int,
    hourPx: Float,
    dayWidthPx: Float,
    days: Int,
    zone: ZoneId,
    onOpen: (Occurrence) -> Unit,
    onMove: (Occurrence, Long) -> Unit,
    onMoveRequest: (Occurrence) -> Unit,
) {
    val haptics = LocalHaptics.current
    var dragging by remember(o) { mutableStateOf(false) }
    var dx by remember(o) { mutableFloatStateOf(0f) }
    var dy by remember(o) { mutableFloatStateOf(0f) }
    var shownSteps by remember(o) { mutableIntStateOf(0) }
    var shownDays by remember(o) { mutableIntStateOf(0) }
    val width = dayWidth / columns
    val moveLabel = stringResource(R.string.week_move)
    val described = o.title.ifBlank { stringResource(R.string.calendar_untitled) } + ", " + timeText(o, zone)
    val accent = calendarColor(o.color, MaterialTheme.colorScheme.primary)
    Box(
        Modifier
            .offset { IntOffset(((dayWidth * dayIndex + width * column).toPx() + shownDays * dayWidthPx).roundToInt(), ((startMinute + shownSteps * WeekLayout.STEP_MINUTES) / 60f * hourPx).roundToInt()) }
            .width(width).height(HOUR * ((endMinute - startMinute) / 60f))
            .padding(1.dp)
            .clip(MaterialTheme.shapes.small)
            .background(accent.copy(alpha = if (dragging) DRAG_ALPHA else BLOCK_ALPHA))
            .semantics {
                contentDescription = described
                customActions = listOf(CustomAccessibilityAction(moveLabel) { onMoveRequest(o); true })
            }
            .testTag(WeekTags.block(o.eventId, o.keyMs))
            .clickable { onOpen(o) }
            .pointerInput(o) {
                detectDragGesturesAfterLongPress(
                    onDragStart = { dragging = true; dx = 0f; dy = 0f; haptics.perform(HapticEvent.DragStart) },
                    onDrag = { change, amount ->
                        change.consume()
                        dx += amount.x
                        dy += amount.y
                        val steps = WeekLayout.steps(dy / hourPx * 60f)
                        val dayShift = (dx / dayWidthPx).roundToInt().coerceIn(-dayIndex, days - 1 - dayIndex)
                        if (steps != shownSteps || dayShift != shownDays) {
                            shownSteps = steps
                            shownDays = dayShift
                            haptics.perform(HapticEvent.Tick)
                        }
                    },
                    onDragEnd = {
                        dragging = false
                        val steps = shownSteps
                        val dayShift = shownDays
                        shownSteps = 0
                        shownDays = 0
                        if (steps != 0 || dayShift != 0) onMove(o, WeekLayout.movedStart(o, steps, dayShift, zone))
                    },
                    onDragCancel = { dragging = false; shownSteps = 0; shownDays = 0 },
                )
            },
    ) {
        Text(
            o.title.ifBlank { stringResource(R.string.calendar_untitled) },
            Modifier.padding(horizontal = 4.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelMedium, maxLines = 3, overflow = TextOverflow.Ellipsis,
        )
    }
}

private val HOUR = 56.dp
private val GUTTER = 40.dp
private const val BLOCK_ALPHA = 0.3f
private const val DRAG_ALPHA = 0.55f
