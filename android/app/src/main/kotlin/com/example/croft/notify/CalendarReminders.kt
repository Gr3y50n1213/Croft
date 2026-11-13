// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.calendar.CalendarBackend
import com.example.croft.mail.calendar.CoreCalendarBackend
import com.example.croft.mail.calendar.Reminder
import java.time.ZoneId
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException

/**
 * T39b: posts event reminders. One unique WorkManager job, planned for the next reminder (or [CalendarReminderPlan.MAX_WAIT_MS] at the
 * latest): inexact (it may be a few minutes late while the phone sleeps, never early), with no exact alarm and no permission of its own
 * (docs/store/READINESS.md row 5). It posts the reminders that fell due since its last run and plans itself again.
 */
class CalendarReminderWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val ctx = applicationContext
        if (!MailRuntime.databaseFile(ctx).exists()) return Result.success()
        val core = try {
            MailRuntime.openedCore(ctx).core
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            return Result.retry()
        } catch (e: UnsatisfiedLinkError) {
            return Result.success()
        }
        return try {
            val store = ReminderClock(ctx)
            val now = System.currentTimeMillis()
            val plan = CalendarReminderPlan.run(CoreCalendarBackend(core), store.lastRunMs(now), now, ZoneId.systemDefault().id)
            val notifier = AndroidNotifier(ctx)
            plan.due.forEach { notifier.postReminder(it) }
            store.setLastRunMs(now)
            CalendarReminderSchedule.planAt(ctx, plan.nextRunMs, now)
            Result.success()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Result.retry()
        }
    }
}

/** What one run of the job does: which reminders to post now and when to run next. Pure apart from the backend. */
object CalendarReminderPlan {
    /** The longest wait between runs (so a reminder added on another device is picked up after its sync). */
    const val MAX_WAIT_MS = 6 * 60 * 60 * 1000L

    /** How far back a first run (or one after a long pause) posts what it missed. */
    const val CATCH_UP_MS = 15 * 60 * 1000L

    /** The reminders to post now and the time of the next run. */
    data class Plan(val due: List<Reminder>, val nextRunMs: Long)

    /** The reminders due in `(lastRunMs, nowMs]` (at most [CATCH_UP_MS] back), and the next run: the next reminder, else [MAX_WAIT_MS]. */
    suspend fun run(backend: CalendarBackend, lastRunMs: Long?, nowMs: Long, zone: String): Plan {
        val from = maxOf(lastRunMs?.plus(1) ?: (nowMs - CATCH_UP_MS), nowMs - CATCH_UP_MS)
        val all = backend.reminders(from, nowMs + MAX_WAIT_MS, zone)
        val due = all.filter { it.atMs <= nowMs }
        val next = all.firstOrNull { it.atMs > nowMs }?.atMs ?: (nowMs + MAX_WAIT_MS)
        return Plan(due, next)
    }
}

/** When the job last ran (a time only, nothing of an event), so a reminder is posted once. */
class ReminderClock(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun lastRunMs(nowMs: Long): Long? = prefs.getLong(KEY, -1L).takeIf { it in 0..nowMs }

    fun setLastRunMs(ms: Long) {
        prefs.edit().putLong(KEY, ms).apply()
    }

    private companion object {
        const val FILE = "croft-reminders"
        const val KEY = "last-run"
    }
}

/** Plans [CalendarReminderWorker]. */
object CalendarReminderSchedule {
    const val UNIQUE_NAME = "croft-event-reminders"

    /** Runs the job now (it plans itself again): at start, after a sync or a change. */
    fun planNow(context: Context) = planAt(context, 0L, 0L)

    /** Runs the job at [atMs] (now when past). */
    fun planAt(context: Context, atMs: Long, nowMs: Long = System.currentTimeMillis()) {
        val wm = try { WorkManager.getInstance(context.applicationContext) } catch (e: IllegalStateException) { return }
        val request = OneTimeWorkRequestBuilder<CalendarReminderWorker>()
            .setInitialDelay((atMs - nowMs).coerceAtLeast(0L), TimeUnit.MILLISECONDS)
            .build()
        wm.enqueueUniqueWork(UNIQUE_NAME, ExistingWorkPolicy.REPLACE, request)
    }

    /** Stops it ("Erase all Croft data"). */
    fun cancel(context: Context) {
        try { WorkManager.getInstance(context.applicationContext).cancelUniqueWork(UNIQUE_NAME) } catch (e: IllegalStateException) { /* not initialised */ }
    }
}
