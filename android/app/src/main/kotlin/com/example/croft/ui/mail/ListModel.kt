// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import android.content.res.Resources
import com.example.croft.R
import com.example.croft.mail.MessageRowData
import java.time.Instant
import java.time.LocalDate
import java.time.Month
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.time.temporal.ChronoUnit
import java.util.Locale

/** The date headers of the list (docs/ux/message-list.md): Today, Yesterday, This week, then month names. */
sealed interface DateBucket {
    /** A stable text for the header's list key. */
    val id: String

    /** Today. */
    data object Today : DateBucket { override val id = "today" }

    /** Yesterday. */
    data object Yesterday : DateBucket { override val id = "yesterday" }

    /** The days before yesterday that are still within the last week. */
    data object ThisWeek : DateBucket { override val id = "week" }

    /** An older month. */
    data class Month(val year: Int, val month: Int) : DateBucket { override val id get() = "m$year-$month" }

    companion object {
        /** The bucket of [dateMs] as seen at [nowMs] in [zone]. Dates in the future count as today. */
        fun of(dateMs: Long, nowMs: Long, zone: ZoneId): DateBucket {
            val day = Instant.ofEpochMilli(dateMs).atZone(zone).toLocalDate()
            val today = Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
            val days = ChronoUnit.DAYS.between(day, today)
            return when {
                days <= 0 -> Today
                days == 1L -> Yesterday
                days < 7 -> ThisWeek
                else -> Month(day.year, day.monthValue)
            }
        }
    }
}

/** What the lazy list draws: a sticky date header or a row. Keys are strings for headers and the message id for rows. */
sealed interface ListItem {
    /** The list key; it keeps the scroll position stable while rows come and go. */
    val key: Any

    /** A date header. */
    data class Header(val bucket: DateBucket) : ListItem { override val key: Any get() = "h:${bucket.id}" }

    /** A message row. */
    data class Row(val row: MessageRowData) : ListItem { override val key: Any get() = row.id }
}

/** Rows with a header before each new date bucket. [rows] must be newest first. */
fun buildListItems(rows: List<MessageRowData>, nowMs: Long, zone: ZoneId): List<ListItem> {
    val out = ArrayList<ListItem>(rows.size + 16)
    var current: DateBucket? = null
    for (r in rows) {
        val b = DateBucket.of(r.dateMs, nowMs, zone)
        if (b != current) { out += ListItem.Header(b); current = b }
        out += ListItem.Row(r)
    }
    return out
}

/** The header's text. */
fun DateBucket.label(res: Resources, nowMs: Long, zone: ZoneId, locale: Locale): String = when (this) {
    DateBucket.Today -> res.getString(R.string.mail_date_today)
    DateBucket.Yesterday -> res.getString(R.string.mail_date_yesterday)
    DateBucket.ThisWeek -> res.getString(R.string.mail_date_this_week)
    is DateBucket.Month -> {
        val name = Month.of(month).getDisplayName(TextStyle.FULL_STANDALONE, locale)
        val thisYear = Instant.ofEpochMilli(nowMs).atZone(zone).year
        if (year == thisYear) name else "$name $year"
    }
}

/** The time shown on a row, and how it is read aloud. */
object RowTimes {
    /**
     * Today: the time. Within the last week: the weekday. This year: day and month. Earlier: day, month and year. The patterns
     * come from the locale (`DateFormat.getBestDateTimePattern`), so order and names follow the person's language.
     */
    fun shown(dateMs: Long, nowMs: Long, zone: ZoneId, locale: Locale): String {
        val at = Instant.ofEpochMilli(dateMs).atZone(zone)
        val day: LocalDate = at.toLocalDate()
        val today = Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
        val days = ChronoUnit.DAYS.between(day, today)
        return when {
            days <= 0 -> DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale).format(at)
            days < 7 -> DateTimeFormatter.ofPattern(android.text.format.DateFormat.getBestDateTimePattern(locale, "EEE"), locale).format(at)
            day.year == today.year -> DateTimeFormatter.ofPattern(android.text.format.DateFormat.getBestDateTimePattern(locale, "MMMd"), locale).format(at)
            else -> DateTimeFormatter.ofPattern(android.text.format.DateFormat.getBestDateTimePattern(locale, "yMMMd"), locale).format(at)
        }
    }

    /** "just now", "5 minutes ago", "2 hours ago" for the last day; the shown time (a date) for anything older. */
    fun spoken(res: Resources, dateMs: Long, nowMs: Long, shown: String): String {
        val minutes = ((nowMs - dateMs) / 60_000L).coerceAtLeast(0L)
        return when {
            minutes < 1 -> res.getString(R.string.mail_time_just_now)
            minutes < 60 -> res.getQuantityString(R.plurals.mail_time_minutes_ago, minutes.toInt(), minutes.toInt())
            minutes < 24 * 60 -> (minutes / 60).toInt().let { res.getQuantityString(R.plurals.mail_time_hours_ago, it, it) }
            else -> shown
        }
    }
}
