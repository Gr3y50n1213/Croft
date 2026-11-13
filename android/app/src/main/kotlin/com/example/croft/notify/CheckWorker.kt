// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequest
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.example.croft.mail.MailRuntime
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException

/**
 * The periodic check (T20): WorkManager runs it every [CheckFrequency] while the device has a network. It opens the process's core if the
 * process was cold (the same instance the screens use) and runs [CheckRunner]. No foreground service, no expedited work, no exact alarm.
 * When the core cannot be opened (secure storage not ready) the run just ends; the next one tries again.
 */
class CheckWorker internal constructor(
    context: Context,
    params: WorkerParameters,
    private val makeRunner: suspend (Context) -> CheckRunner?,
) : CoroutineWorker(context, params) {
    /** The constructor WorkManager's default factory calls. */
    constructor(context: Context, params: WorkerParameters) : this(context, params, ::appRunner)

    override suspend fun doWork(): Result {
        val runner = try { makeRunner(applicationContext) } catch (e: CancellationException) { throw e } catch (e: Exception) { null } ?: return Result.success()
        return when (runner.run()) {
            CheckRunner.Result.SUCCESS -> Result.success()
            CheckRunner.Result.RETRY -> Result.retry()
        }
    }

    companion object {
        private suspend fun appRunner(context: Context): CheckRunner? {
            // No database (a fresh install, or after "Erase all Croft data"): nothing to check, and opening the core here would create one.
            if (!MailRuntime.databaseFile(context).exists()) return null
            val core = try { MailRuntime.openedCore(context).core } catch (e: Exception) { return null } catch (e: UnsatisfiedLinkError) { return null }
            return CheckRunner(CoreBackgroundChecker(core), AndroidNotifier(context), NotifySettingsStore.shared(context))
        }
    }
}

/** Schedules [CheckWorker] as one unique periodic work. */
object CheckSchedule {
    /** Stops the periodic check ("Erase all Croft data", the last real account removed). */
    fun cancel(context: Context) {
        try {
            val wm = WorkManager.getInstance(context.applicationContext)
            wm.cancelUniqueWork(UNIQUE_NAME)
            wm.cancelUniqueWork(PUSH_NAME)
        } catch (e: IllegalStateException) { /* not initialised */ }
    }

    /** Schedules the check (KEEP) when there is an account that can notify, else stops it. */
    fun forAccounts(context: Context, settings: NotifySettings, hasRealAccount: Boolean) {
        if (hasRealAccount) {
            try { apply(context, settings, changed = false) } catch (e: IllegalStateException) { /* not initialised: the next start */ }
        } else {
            cancel(context)
        }
    }

    /** The unique work name. */
    const val UNIQUE_NAME = "croft-mail-check"

    /** The unique name of the check a push asks for (T34d). */
    const val PUSH_NAME = "croft-push-check"

    /**
     * A check now, because the Companion said mail changed (T34d): one-time work with a network, KEEP, so pings that come while one is
     * waiting or running add nothing (the Companion pings again on the next change, and the periodic check stays as a safety net). Not
     * expedited and no foreground service (docs/store/READINESS.md row 4).
     */
    fun now(context: Context) {
        val request = OneTimeWorkRequestBuilder<CheckWorker>()
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .build()
        try {
            WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(PUSH_NAME, ExistingWorkPolicy.KEEP, request)
        } catch (e: IllegalStateException) {
            // WorkManager not initialised: the periodic check still runs.
        }
    }

    /** The periodic request for [settings], or null when checks are manual only. */
    fun request(settings: NotifySettings): PeriodicWorkRequest? {
        val minutes = settings.frequency.minutes ?: return null
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .setRequiresBatteryNotLow(settings.batteryNotLow)
            .build()
        return PeriodicWorkRequestBuilder<CheckWorker>(minutes, TimeUnit.MINUTES)
            .setConstraints(constraints)
            .build()
    }

    /**
     * Makes the scheduled work match [settings]. At the app's start the policy is KEEP (a running schedule is left alone); after a change
     * in S46 it is UPDATE, so a new interval or constraint applies without restarting the period. Manual only cancels the work.
     */
    fun apply(context: Context, settings: NotifySettings, changed: Boolean) {
        val wm = WorkManager.getInstance(context.applicationContext)
        val request = request(settings)
        if (request == null) {
            wm.cancelUniqueWork(UNIQUE_NAME)
            return
        }
        val policy = if (changed) ExistingPeriodicWorkPolicy.UPDATE else ExistingPeriodicWorkPolicy.KEEP
        wm.enqueueUniquePeriodicWork(UNIQUE_NAME, policy, request)
    }
}
