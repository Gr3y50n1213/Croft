// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** A core that answers from a table and remembers what was asked and marked. Not a real mailbox: made-up data. */
class FakeChecker(
    var accounts: List<CheckAccount>,
    val outcomes: MutableMap<Long, CheckOutcome> = mutableMapOf(),
) : BackgroundChecker {
    val synced = ArrayList<Long>()
    val deadlines = ArrayList<Long>()
    val marked = LinkedHashMap<Long, MutableList<UInt>>()

    override suspend fun accounts() = accounts

    override suspend fun syncOnce(account: Long, deadlineMs: Long): CheckOutcome {
        synced += account
        deadlines += deadlineMs
        val out = outcomes[account] ?: CheckOutcome()
        // Like the core: what was marked is never offered again.
        val done = marked[account].orEmpty().toSet()
        return out.copy(newMail = out.newMail.filter { it.uid !in done })
    }

    override suspend fun markNotified(account: Long, uids: List<UInt>) {
        marked.getOrPut(account) { mutableListOf() } += uids
    }
}

class FakeNotifier(var allowed: Boolean = true) : Notifier {
    val posted = ArrayList<NewMail>()
    val signIns = ArrayList<Long>()
    val cleared = ArrayList<Long>()

    override fun canPost() = allowed

    override fun postNewMail(account: CheckAccount, mail: List<NewMail>, settings: NotifySettings): List<UInt> {
        posted += mail
        return mail.map { it.uid }
    }

    override fun postSignIn(account: CheckAccount): Boolean { signIns += account.id; return true }

    override fun clearSignIn(account: Long) { cleared += account }
}

fun mail(account: Long, id: Long, uid: Int, subject: String = "Lunch tomorrow") =
    NewMail(account, id, uid.toUInt(), "Ann Example", "ann@example.org", subject, 1_700_000_000_000)

class CheckRunnerTest {
    @get:Rule val tmp = TemporaryFolder()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After fun stop() = scope.cancel()

    private fun store(): NotifySettingsStore = NotifySettingsStore.onFile(File(tmp.root, "notify.json"), scope)

    private val a1 = CheckAccount(1, "Work")
    private val a2 = CheckAccount(2, "Home")

    @Test
    fun syncsEveryAccountPostsNewMailAndMarksItSoARerunPostsNothingTwice() = runBlocking {
        val checker = FakeChecker(listOf(a1, a2), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5))), 2L to CheckOutcome(listOf(mail(2, 20, 7), mail(2, 21, 8)))))
        val notifier = FakeNotifier()
        val runner = CheckRunner(checker, notifier, store())
        assertEquals(CheckRunner.Result.SUCCESS, runner.run())
        assertEquals(listOf(1L, 2L), checker.synced)
        assertEquals(listOf(10L, 20L, 21L), notifier.posted.map { it.messageId })
        assertEquals(mapOf(1L to listOf(5u), 2L to listOf(7u, 8u)), checker.marked)

        assertEquals(CheckRunner.Result.SUCCESS, runner.run())
        assertEquals("nothing is posted twice", 3, notifier.posted.size)
    }

    @Test
    fun theSampleMailboxAndAnAccountThatCannotRunAreSkipped() = runBlocking {
        val checker = FakeChecker(listOf(CheckAccount(3, "Sample", isDemo = true), CheckAccount(4, "Broken", runnable = false), a1))
        CheckRunner(checker, FakeNotifier(), store()).run()
        assertEquals(listOf(1L), checker.synced)
    }

    @Test
    fun aPausedAccountIsSkippedAndGetsOneSignInNotificationNotOnePerRun() = runBlocking {
        val checker = FakeChecker(listOf(CheckAccount(1, "Work", needsSignIn = true), a2))
        val notifier = FakeNotifier()
        val s = store()
        val runner = CheckRunner(checker, notifier, s)
        runner.run(); runner.run(); runner.run()
        assertEquals(listOf(2L, 2L, 2L), checker.synced)
        assertEquals(listOf(1L), notifier.signIns)

        // Signed in again: the notification goes, and a later problem notifies once more.
        checker.accounts = listOf(a1, a2)
        runner.run()
        assertEquals(listOf(1L), notifier.cleared)
        assertTrue(1L !in s.current().signInNotified)
    }

    @Test
    fun aSignInProblemFoundDuringTheSyncAlsoNotifiesOnce() = runBlocking {
        val checker = FakeChecker(listOf(a1), mutableMapOf(1L to CheckOutcome(needsSignIn = true)))
        val notifier = FakeNotifier()
        val runner = CheckRunner(checker, notifier, store())
        runner.run(); runner.run()
        assertEquals(listOf(1L), notifier.signIns)
    }

    @Test
    fun aTransientFailureAsksForARetryButTheOtherAccountsStillRun() = runBlocking {
        val checker = FakeChecker(listOf(a1, a2), mutableMapOf(1L to CheckOutcome(transientFailure = true), 2L to CheckOutcome(listOf(mail(2, 20, 7)))))
        val notifier = FakeNotifier()
        assertEquals(CheckRunner.Result.RETRY, CheckRunner(checker, notifier, store()).run())
        assertEquals(listOf(20L), notifier.posted.map { it.messageId })
    }

    @Test
    fun aTimeoutIsNotARetry() = runBlocking {
        val checker = FakeChecker(listOf(a1), mutableMapOf(1L to CheckOutcome(timedOut = true)))
        assertEquals(CheckRunner.Result.SUCCESS, CheckRunner(checker, FakeNotifier(), store()).run())
    }

    @Test
    fun theWholeRunStopsBeforeItsDeadline() = runBlocking {
        var now = 0L
        val checker = object : BackgroundChecker by FakeChecker(listOf(a1, a2)) {
            val synced = ArrayList<Long>()
            override suspend fun syncOnce(account: Long, deadlineMs: Long): CheckOutcome {
                synced += account
                now += CheckRunner.DEADLINE_MS - 5_000 // the first account takes almost all of it
                return CheckOutcome()
            }
        }
        assertEquals(CheckRunner.Result.SUCCESS, CheckRunner(checker, FakeNotifier(), store(), clock = { now }).run())
        assertEquals("the second account waits for the next run", listOf(1L), checker.synced)
    }

    @Test
    fun theDeadlineGivenToAnAccountIsWhatIsLeftOfTheRun() = runBlocking {
        var now = 0L
        val fake = FakeChecker(listOf(a1, a2))
        val checker = object : BackgroundChecker by fake {
            override suspend fun syncOnce(account: Long, deadlineMs: Long): CheckOutcome { now += 60_000; return fake.syncOnce(account, deadlineMs) }
        }
        CheckRunner(checker, FakeNotifier(), store(), clock = { now }).run()
        assertEquals(listOf(CheckRunner.DEADLINE_MS, CheckRunner.DEADLINE_MS - 60_000), fake.deadlines)
    }

    @Test
    fun withoutPermissionNothingIsPostedAndNothingPilesUp() = runBlocking {
        val checker = FakeChecker(listOf(a1), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5)))))
        val notifier = FakeNotifier(allowed = false)
        CheckRunner(checker, notifier, store()).run()
        assertTrue(notifier.posted.isEmpty())
        assertEquals(mapOf(1L to listOf(5u)), checker.marked)
    }

    @Test
    fun aMutedAccountAndTheMasterSwitchPostNothing() = runBlocking {
        val checker = FakeChecker(listOf(a1, a2), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5))), 2L to CheckOutcome(listOf(mail(2, 20, 7)))))
        val notifier = FakeNotifier()
        val s = store()
        s.set(NotifySettings(mutedAccounts = setOf(1L)))
        CheckRunner(checker, notifier, s).run()
        assertEquals(listOf(20L), notifier.posted.map { it.messageId })

        val checker2 = FakeChecker(listOf(a1), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 11, 6)))))
        s.set(NotifySettings(enabled = false))
        CheckRunner(checker2, notifier, s).run()
        assertEquals(listOf(1L), checker2.synced)
        assertEquals("the master switch posts nothing", 1, notifier.posted.size)
    }

    @Test
    fun onlyWhatWasReallyPostedIsMarked() = runBlocking {
        val checker = FakeChecker(listOf(a1), mutableMapOf(1L to CheckOutcome(listOf(mail(1, 10, 5), mail(1, 11, 6)))))
        val notifier = object : Notifier by FakeNotifier() {
            override fun postNewMail(account: CheckAccount, mail: List<NewMail>, settings: NotifySettings) = listOf(mail.first().uid)
        }
        CheckRunner(checker, notifier, store()).run()
        assertEquals(mapOf(1L to listOf(5u)), checker.marked)
    }

    @Test
    fun theDebugTextOfNewMailHasNoSenderOrSubject() {
        val text = mail(1, 10, 5, subject = "Secret plans").toString()
        assertTrue(text, "Secret" !in text && "ann@" !in text && "Ann" !in text)
    }
}
