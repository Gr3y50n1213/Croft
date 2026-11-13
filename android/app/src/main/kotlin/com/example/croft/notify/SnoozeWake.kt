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
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException

/**
 * T36: wakes snoozed mail. One unique WorkManager job set for the soonest wake time: inexact (it may be a few minutes late in Doze,
 * never early), kept across restarts by WorkManager, with no exact alarm and no permission of its own (docs/ux/server-contract.md,
 * "Snooze logic"; docs/store/READINESS.md row 4). It opens the core like the periodic check, wakes what is due, says "Back in your
 * inbox" (only the count, never a sender or subject) and plans itself again.
 */
class SnoozeWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
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
            val woken = core.wakeDueSnoozes()
            if (woken.isNotEmpty()) AndroidNotifier(ctx).postBackFromSnooze(woken.size)
            SnoozeSchedule.plan(ctx, core.nextSnoozeWake())
            Result.success()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Result.retry()
        }
    }
}

/** Plans [SnoozeWorker] for the soonest wake time. */
object SnoozeSchedule {
    const val UNIQUE_NAME = "croft-snooze-wake"

    /** How long to wait for [nextMs] from [nowMs] (0 when it is due), or null when nothing is snoozed. */
    fun delayMs(nextMs: Long?, nowMs: Long): Long? = nextMs?.let { (it - nowMs).coerceAtLeast(0L) }

    /** Sets (REPLACE) or cancels the job for [nextMs]. */
    fun plan(context: Context, nextMs: Long?, nowMs: Long = System.currentTimeMillis()) {
        val wm = try { WorkManager.getInstance(context.applicationContext) } catch (e: IllegalStateException) { return }
        val delay = delayMs(nextMs, nowMs)
        if (delay == null) {
            wm.cancelUniqueWork(UNIQUE_NAME)
            return
        }
        val request = OneTimeWorkRequestBuilder<SnoozeWorker>().setInitialDelay(delay, TimeUnit.MILLISECONDS).build()
        wm.enqueueUniqueWork(UNIQUE_NAME, ExistingWorkPolicy.REPLACE, request)
    }

    /** Stops it ("Erase all Croft data"). */
    fun cancel(context: Context) {
        try { WorkManager.getInstance(context.applicationContext).cancelUniqueWork(UNIQUE_NAME) } catch (e: IllegalStateException) { /* not initialised */ }
    }
}
