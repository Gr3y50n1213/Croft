// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerFactory
import androidx.work.WorkerParameters
import androidx.work.testing.TestListenableWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import com.example.croft.CroftRobolectricRunner
import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class CheckWorkerTest {
    @get:Rule val tmp = TemporaryFolder()
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After fun stop() = scope.cancel()

    private fun worker(checker: BackgroundChecker, notifier: Notifier, store: NotifySettingsStore): CheckWorker =
        TestListenableWorkerBuilder<CheckWorker>(context)
            .setWorkerFactory(object : WorkerFactory() {
                override fun createWorker(appContext: Context, workerClassName: String, workerParameters: WorkerParameters): ListenableWorker =
                    CheckWorker(appContext, workerParameters) { CheckRunner(checker, notifier, store) }
            })
            .build()

    private fun store() = NotifySettingsStore.onFile(File(tmp.root, "n.json"), scope)

    @Test
    fun syncsTwoAccountsSkipsAPausedOneAndSucceeds_aRerunPostsNothingTwice() = runBlocking {
        val checker = FakeChecker(
            listOf(CheckAccount(1, "Work"), CheckAccount(2, "Home"), CheckAccount(3, "Old", needsSignIn = true)),
            mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5))), 2L to CheckOutcome(listOf(mail(2, 20, 9)))),
        )
        val notifier = FakeNotifier()
        val s = store()
        assertEquals(ListenableWorker.Result.success(), worker(checker, notifier, s).doWork())
        assertEquals(listOf(1L, 2L), checker.synced)
        assertEquals(listOf(10L, 20L), notifier.posted.map { it.messageId })
        assertEquals(listOf(3L), notifier.signIns)

        assertEquals(ListenableWorker.Result.success(), worker(checker, notifier, s).doWork())
        assertEquals(2, notifier.posted.size)
        assertEquals(listOf(3L), notifier.signIns)
    }

    @Test
    fun aTransientFailureIsARetry() = runBlocking {
        val checker = FakeChecker(listOf(CheckAccount(1, "Work")), mutableMapOf(1L to CheckOutcome(transientFailure = true)))
        assertEquals(ListenableWorker.Result.retry(), worker(checker, FakeNotifier(), store()).doWork())
    }

    @Test
    fun aCoreThatCannotOpenEndsTheRunQuietly() = runBlocking {
        val w = TestListenableWorkerBuilder<CheckWorker>(context)
            .setWorkerFactory(object : WorkerFactory() {
                override fun createWorker(appContext: Context, workerClassName: String, workerParameters: WorkerParameters): ListenableWorker =
                    CheckWorker(appContext, workerParameters) { throw IllegalStateException("secure storage not ready") }
            })
            .build()
        assertEquals(ListenableWorker.Result.success(), w.doWork())
    }

    @Test
    fun thePeriodicRequestHasTheChosenIntervalAndConstraintsAndIsNotExpedited() {
        val r = CheckSchedule.request(NotifySettings(frequency = CheckFrequency.HOUR_1, batteryNotLow = true))!!
        assertEquals(TimeUnit.HOURS.toMillis(1), r.workSpec.intervalDuration)
        assertEquals(NetworkType.CONNECTED, r.workSpec.constraints.requiredNetworkType)
        assertTrue(r.workSpec.constraints.requiresBatteryNotLow())
        assertFalse(r.workSpec.expedited)
        assertEquals(CheckWorker::class.java.name, r.workSpec.workerClassName)

        val d = CheckSchedule.request(NotifySettings())!!
        assertEquals(TimeUnit.MINUTES.toMillis(30), d.workSpec.intervalDuration)
        assertFalse(d.workSpec.constraints.requiresBatteryNotLow())
        assertEquals(TimeUnit.MINUTES.toMillis(15), CheckSchedule.request(NotifySettings(frequency = CheckFrequency.MIN_15))!!.workSpec.intervalDuration)
        assertNull("manual schedules nothing", CheckSchedule.request(NotifySettings(frequency = CheckFrequency.MANUAL)))
    }

    @Test
    fun applyEnqueuesOneUniqueWorkUpdatesItAndCancelsItForManual() {
        WorkManagerTestInitHelper.initializeTestWorkManager(context)
        val wm = WorkManager.getInstance(context)
        CheckSchedule.apply(context, NotifySettings(), changed = false)
        CheckSchedule.apply(context, NotifySettings(), changed = false)
        var infos = wm.getWorkInfosForUniqueWork(CheckSchedule.UNIQUE_NAME).get()
        assertEquals(1, infos.size)
        assertEquals(WorkInfo.State.ENQUEUED, infos[0].state)
        assertEquals(TimeUnit.MINUTES.toMillis(30), infos[0].periodicityInfo!!.repeatIntervalMillis)

        CheckSchedule.apply(context, NotifySettings(frequency = CheckFrequency.HOUR_2), changed = true)
        infos = wm.getWorkInfosForUniqueWork(CheckSchedule.UNIQUE_NAME).get()
        assertEquals(1, infos.size)
        assertEquals(TimeUnit.HOURS.toMillis(2), infos[0].periodicityInfo!!.repeatIntervalMillis)

        CheckSchedule.apply(context, NotifySettings(frequency = CheckFrequency.MANUAL), changed = true)
        infos = wm.getWorkInfosForUniqueWork(CheckSchedule.UNIQUE_NAME).get()
        assertTrue(infos.all { it.state == WorkInfo.State.CANCELLED })
    }
}
