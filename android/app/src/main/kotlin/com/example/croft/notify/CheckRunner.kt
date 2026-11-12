// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import kotlinx.coroutines.CancellationException

/** An account as the background check sees it. */
data class CheckAccount(
    val id: Long,
    /** The name the notification channel and the summary use. */
    val label: String,
    val needsSignIn: Boolean = false,
    /** False when the account cannot run (unusable stored settings): skipped, like a paused account. */
    val runnable: Boolean = true,
    val isDemo: Boolean = false,
)

/** A new message to notify about: sender and subject only, never the body. */
data class NewMail(
    val account: Long,
    val messageId: Long,
    val uid: UInt,
    val senderName: String?,
    val senderAddress: String?,
    val subject: String,
    val dateMs: Long,
) {
    /** What the notification's title shows: the sender's name, else the address. */
    val sender: String get() = senderName?.takeIf { it.isNotBlank() } ?: senderAddress.orEmpty()

    // Sender and subject are personal: never printed.
    override fun toString(): String = "NewMail(account=$account, messageId=$messageId, uid=$uid)"
}

/** What one account's check gave. */
data class CheckOutcome(
    val newMail: List<NewMail> = emptyList(),
    val timedOut: Boolean = false,
    val transientFailure: Boolean = false,
    val needsSignIn: Boolean = false,
)

/** The core's side of the check ([CoreBackgroundChecker] over the real core; a fake in tests). */
interface BackgroundChecker {
    suspend fun accounts(): List<CheckAccount>

    /** Syncs [account] once within [deadlineMs] and returns what to notify about. */
    suspend fun syncOnce(account: Long, deadlineMs: Long): CheckOutcome

    /** The notifications for these UIDs were posted (or are not wanted): never offered again. */
    suspend fun markNotified(account: Long, uids: List<UInt>)
}

/** Posting notifications ([AndroidNotifier]; a fake in tests). */
interface Notifier {
    /** Whether a notification can show at all (the permission on Android 13 and later, and the app's switch in system settings). */
    fun canPost(): Boolean

    /** Posts [mail] of [account]; returns the UIDs that were really posted. */
    fun postNewMail(account: CheckAccount, mail: List<NewMail>, settings: NotifySettings): List<UInt>

    /** Posts the one "Needs sign-in" notification of [account]. Returns whether it was posted. */
    fun postSignIn(account: CheckAccount): Boolean

    /** Removes the "Needs sign-in" notification of [account] (it works again). */
    fun clearSignIn(account: Long)
}

/**
 * One run of the periodic check (T20): every account in turn, within [totalDeadlineMs] for the whole run. The sample mailbox and accounts
 * that cannot run are skipped; an account that needs a sign-in gets **one** "Needs sign-in" notification, not one per run. New mail is
 * posted only when the person wants it (S46) and the system lets the app post; mail that is not wanted counts as handled, so turning
 * notifications on later does not bring a burst of old messages. The marker moves only for what was posted or not wanted.
 *
 * The result is [Result.RETRY] only when a server or the network failed for now (WorkManager backs off; the run itself never loops).
 */
class CheckRunner(
    private val checker: BackgroundChecker,
    private val notifier: Notifier,
    private val settings: NotifySettingsStore,
    private val clock: () -> Long = System::currentTimeMillis,
    private val totalDeadlineMs: Long = DEADLINE_MS,
) {
    enum class Result { SUCCESS, RETRY }

    suspend fun run(): Result {
        val started = clock()
        var retry = false
        val accounts = try { checker.accounts() } catch (e: CancellationException) { throw e } catch (e: Exception) { return Result.RETRY }
        for (account in accounts) {
            if (account.isDemo || !account.runnable) continue
            if (account.needsSignIn) { signInOnce(account); continue }
            val left = totalDeadlineMs - (clock() - started)
            // Out of time: the next periodic run carries on (not a retry: nothing failed).
            if (left < MIN_SLICE_MS) break
            val out = try {
                checker.syncOnce(account.id, left)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // A storage problem or an account removed meanwhile: the next run tries again.
                continue
            }
            if (out.needsSignIn) { signInOnce(account); continue }
            healthy(account)
            if (out.transientFailure) retry = true
            if (out.newMail.isNotEmpty()) deliver(account, out.newMail)
        }
        return if (retry) Result.RETRY else Result.SUCCESS
    }

    private suspend fun deliver(account: CheckAccount, mail: List<NewMail>) {
        val s = settings.current()
        val handled = if (!s.wants(account.id)) {
            mail.map { it.uid }
        } else if (!notifier.canPost()) {
            // The permission was refused or the app's notifications are off in system settings: nothing posts, and nothing piles up.
            mail.map { it.uid }
        } else {
            notifier.postNewMail(account, mail, s)
        }
        if (handled.isNotEmpty()) {
            try { checker.markNotified(account.id, handled) } catch (e: CancellationException) { throw e } catch (e: Exception) { /* offered again next run */ }
        }
    }

    private suspend fun signInOnce(account: CheckAccount) {
        val s = settings.current()
        if (account.id in s.signInNotified || !s.enabled || !notifier.canPost()) return
        if (notifier.postSignIn(account)) settings.update { it.copy(signInNotified = it.signInNotified + account.id) }
    }

    private suspend fun healthy(account: CheckAccount) {
        if (account.id !in settings.current().signInNotified) return
        notifier.clearSignIn(account.id)
        settings.update { it.copy(signInNotified = it.signInNotified - account.id) }
    }

    companion object {
        /** The whole run stops before this (WorkManager stops a worker after 10 minutes). */
        const val DEADLINE_MS = 8 * 60 * 1000L

        /** An account is not started with less than this left; the next run carries on. */
        const val MIN_SLICE_MS = 10_000L
    }
}
