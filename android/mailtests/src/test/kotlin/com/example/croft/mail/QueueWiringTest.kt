// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * T16 "Done when", one test each: the app's mail layer ([CoreMessageRepository]) on the real Rust core and a real in-process
 * IMAP server, with real timers (an Undo window of 1 s unless a test says otherwise; read and star wait 1.5 s in the core).
 * No emulator, no Android. The fake clock of the engine's own back-off test cannot reach the core through the FFI; the schedule
 * (5 s, 30 s, 2 min, 10 min, 1 h, 8 attempts) is tested with a fake clock in `core/engine/tests/fake_backend.rs`, and here the
 * first retry is seen to happen on the real clock.
 */
class QueueWiringTest {
    private val rigs = mutableListOf<Rig>()

    private fun rig(messages: Int = 3, window: Long = 1_000): Rig = Rig(messages, window).also { rigs += it }

    @After
    fun tearDown() = rigs.forEach { it.close() }

    @Test(timeout = 60_000)
    fun archiveThenUndoInsideTheWindow_theServerSeesNoCommand() {
        val r = rig()
        r.awaitInbox(3)
        r.settle()
        val id = r.inbox().first().id

        val ops = r.run { r.repo.archive(listOf(id)) }
        assertEquals("the row leaves the list at once", 2, r.inbox().size)
        assertTrue("undo inside the window", r.run { r.repo.undo(ops.single()) })
        assertEquals("and comes back", 3, r.inbox().size)

        // Past the window, and a sync on top: the server heard nothing that changes anything.
        Thread.sleep(2_200)
        r.run { r.repo.syncNow(r.account) }
        Thread.sleep(800)
        assertEquals(emptyList<String>(), r.changingCommands())
        assertEquals(emptyList<UInt>(), r.server.uids("Archive"))
        assertEquals(3, r.server.uids("INBOX").size)
    }

    @Test(timeout = 60_000)
    fun archiveAndLetTheWindowPass_theMessageIsInArchive_andTheRowStaysGone() {
        val r = rig()
        r.awaitInbox(3)
        r.settle()
        val row = r.inbox().first()

        r.run { r.repo.archive(listOf(row.id)) }
        assertFalse("gone at once", r.inbox().any { it.id == row.id })
        await("the message in the server's Archive folder") { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }
        await("the Inbox on the server shrank") { r.server.uids("INBOX").takeIf { it.size == 2 } }
        // It never comes back to the list while the server catches up, and it shows in Archive once synced.
        repeat(10) { assertFalse(r.inbox().any { it.id == row.id }); Thread.sleep(100) }
        await("the row in the Archive list") { r.rows("Archive").firstOrNull { it.subject == row.subject } }
        assertEquals(1, r.server.commands().count { it.contains("MOVE") || it.contains("COPY") })
    }

    @Test(timeout = 60_000)
    fun aReadChangeShowsTheClockWhilePending_andItGoesWhenTheServerConfirms() {
        val r = rig()
        val row = r.awaitInbox(3).first()
        assertFalse(row.seen)
        r.settle()

        r.run { r.repo.markSeen(listOf(row.id), true) }
        val pending = r.inbox().first { it.id == row.id }
        assertTrue("applied at once", pending.seen)
        assertTrue("the clock marker is shown", pending.pending)

        await("\\Seen on the server") { r.server.flagsOfSubject("INBOX", row.subject).takeIf { "\\Seen" in it } }
        val confirmed = await("the clock marker gone") { r.inbox().first { it.id == row.id }.takeIf { !it.pending } }
        assertTrue(confirmed.seen)
    }

    @Test(timeout = 60_000)
    fun killAndReopenTheCoreWithAnOpPending_itIsSentExactlyOnce() {
        val r = rig(window = 4_000)
        r.awaitInbox(3)
        r.settle()
        val row = r.inbox().first()

        r.run { r.repo.archive(listOf(row.id)) }
        assertEquals("nothing sent yet", emptyList<String>(), r.changingCommands())
        r.killAndReopen()
        assertEquals("the queue was stored with the change", 2, r.inbox().size)

        await("the message in Archive after the reopen") { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }
        Thread.sleep(1_500)
        assertEquals("sent exactly once", 1, r.server.commands().count { it.contains("MOVE") || it.contains("COPY") })
        assertEquals(1, r.server.uids("Archive").size)
        assertEquals(2, r.server.uids("INBOX").size)
    }

    @Test(timeout = 60_000)
    fun aPermanentRefusalPutsTheRowBackWithTheFailure_andIsNotRetried() {
        val r = rig()
        r.awaitInbox(3)
        r.settle()
        val row = r.inbox().first()
        // The server refuses to move anything out of the Inbox (NO [READ-ONLY]): a permanent answer.
        r.server.setReadOnly("INBOX", true)

        val ops = r.run { r.repo.archive(listOf(row.id)) }
        assertEquals(2, r.inbox().size)
        val failure = await("the failure notice") { r.failures().firstOrNull() }
        assertEquals(ops.single(), failure.opId)
        assertEquals(FailureReason.REJECTED, failure.reason)
        assertEquals("the notice names the message, for the red edge", listOf(row.id), failure.messages)
        assertEquals("the row is back", 3, r.inbox().size)
        assertTrue(r.inbox().any { it.id == row.id })

        // Failed at once: past the first back-off step (5 s) there is no second try, no second notice, and the account is fine.
        Thread.sleep(6_000)
        assertEquals(1, r.failures().size)
        assertEquals(3, r.inbox().size)
        assertTrue("nothing changed on the server", r.changingCommands().isEmpty())
        assertTrue(r.server.uids("Archive").isEmpty())
        assertNotEquals(AccountState.NEEDS_SIGN_IN, r.status())

        // Retry after the server allows it: the same action runs again on the same message.
        r.server.setReadOnly("INBOX", false)
        r.run { r.repo.retryFailed(ops.single()) }
        await("the retried archive on the server") { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }
    }

    @Test(timeout = 90_000)
    fun aTransientAnswerKeepsTheRowGone_andTheChangeGoesOutWhenTheServerIsBack() {
        val r = rig()
        r.awaitInbox(3)
        r.settle()
        val row = r.inbox().first()
        // The server says "try later" to every sign-in (NO [UNAVAILABLE]) and the open connections are cut.
        r.server.setLoginUnavailable(true)
        r.server.dropConnections()

        r.run { r.repo.archive(listOf(row.id)) }
        Thread.sleep(3_500)
        assertTrue("nothing could be sent", r.changingCommands().isEmpty())
        assertFalse("the row stays as the person left it", r.inbox().any { it.id == row.id })
        assertEquals("a transient answer is not a failure", emptyList<OpFailure>(), r.failures())
        assertNotEquals("and it is not a sign-in problem", AccountState.NEEDS_SIGN_IN, r.status())
        assertTrue("the provider was not asked to sign in", r.provider.signInCalls.isEmpty())

        // The server comes back: the retry (first back-off step, 5 s) delivers it.
        r.server.setLoginUnavailable(false)
        await("the retried archive on the server", 40_000) { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }
        assertEquals(emptyList<OpFailure>(), r.failures())
        assertEquals(1, r.server.commands().count { it.contains("MOVE") || it.contains("COPY") })
    }

    @Test(timeout = 90_000)
    fun aRefusedLoginShowsNeedsSignIn_pausesTheQueue_andSigningInSendsItOnce() {
        val r = rig()
        r.awaitInbox(3)
        r.settle()
        val row = r.inbox().first()
        // The password was changed on the server: every sign-in is refused, and the connections are cut.
        r.server.setRejectLogins(true)
        r.server.dropConnections()

        r.run { r.repo.archive(listOf(row.id)) }
        await("Needs sign-in") { r.status().takeIf { it == AccountState.NEEDS_SIGN_IN } }
        await("the app was told to ask for a sign-in") { r.provider.signInCalls.takeIf { it.isNotEmpty() } }
        Thread.sleep(2_000)
        assertTrue("the queue is paused: nothing was sent", r.changingCommands().isEmpty())
        assertFalse("the change is still applied locally", r.inbox().any { it.id == row.id })
        assertEquals("no failure: the change waits", emptyList<OpFailure>(), r.failures())
        assertEquals("asked once, however often the engine reported it", listOf(r.account), r.provider.signInCalls.toList())

        // The person signs in again (the server accepts): the queue resumes and the move goes out once.
        r.server.setRejectLogins(false)
        r.run { r.repo.signedIn(r.account) }
        await("the archive on the server") { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }
        Thread.sleep(1_000)
        assertEquals(1, r.server.commands().count { it.contains("MOVE") || it.contains("COPY") })
        assertNotEquals(AccountState.NEEDS_SIGN_IN, r.status())
    }

    @Test(timeout = 90_000)
    fun airplaneMode_everyActionWorksLocally_nothingIsSent_andReconnectingSendsInOrder() {
        val r = rig(window = 600)
        r.awaitInbox(3)
        r.settle()
        val rows = r.inbox()
        val (a, b) = rows[0] to rows[1]

        r.run { r.repo.setNetworkAvailable(false) }
        assertEquals(AccountState.OFFLINE, r.status())
        // Read it, then archive it; star another. All of it works on this device.
        r.run { r.repo.markSeen(listOf(a.id), true) }
        r.run { r.repo.archive(listOf(a.id)) }
        r.run { r.repo.setFlagged(listOf(b.id), true) }
        val shown = r.inbox().first { it.id == b.id }
        assertTrue(shown.flagged)
        assertTrue("the clock marker while it waits", shown.pending)
        assertFalse(r.inbox().any { it.id == a.id })

        // Well past every window: still nothing sent.
        Thread.sleep(3_000)
        assertEquals(emptyList<String>(), r.changingCommands())

        r.run { r.repo.setNetworkAvailable(true) }
        await("the archive on the server") { r.server.subjects("Archive").takeIf { it == listOf(a.subject) } }
        await("the star on the server") { r.server.flagsOfSubject("INBOX", b.subject).takeIf { "\\Flagged" in it } }
        await("back online") { r.status().takeIf { it != AccountState.OFFLINE } }
        // In order: the read flag of the first message goes out before its move.
        val commands = r.server.commands()
        val store = commands.indexOfFirst { it.contains("STORE") && it.contains("Seen") }
        val move = commands.indexOfFirst { it.contains("MOVE") || it.contains("COPY") }
        assertTrue("both were sent: $commands", store >= 0 && move >= 0)
        assertTrue("read before archive: $commands", store < move)
    }

    @Test(timeout = 60_000)
    fun whileAReadFlagIsPending_aServerSideFlagChangeDoesNotChangeTheRow_andAfterConfirmTheServerValueShows() {
        val r = rig()
        r.awaitInbox(3)
        val row = r.inbox().first()
        val uid = r.server.uids("INBOX").last()
        // Another client read it: the row shows read.
        r.server.changeFlags("INBOX", uid, listOf("\\Seen"), emptyList())
        r.run { r.repo.syncNow(r.account) }
        await("the row read") { r.inbox().first { it.id == row.id }.takeIf { it.seen } }
        r.settle()

        // The person marks it unread here (pending for 1.5 s); meanwhile another client stars it.
        r.run { r.repo.markSeen(listOf(row.id), false) }
        r.server.changeFlags("INBOX", uid, listOf("\\Flagged"), emptyList())
        r.run { r.repo.syncNow(r.account) }
        Thread.sleep(500)
        val during = r.inbox().first { it.id == row.id }
        assertTrue(during.pending)
        assertFalse("this device's value wins", during.seen)
        assertFalse("and the row does not change under the person's finger", during.flagged)

        // Confirmed: the server's value shows.
        val after = await("confirmed and synced") {
            r.run { r.repo.syncNow(r.account) }
            r.inbox().first { it.id == row.id }.takeIf { !it.pending && it.flagged }
        }
        assertFalse("the person's change went out", after.seen)
        assertFalse("\\Seen" in r.server.flags("INBOX", uid))
        assertTrue("\\Flagged" in r.server.flags("INBOX", uid))
    }

    @Test(timeout = 90_000)
    fun readArchiveStarQuickly_theStarLandsOnTheMovedCopy() = readArchiveStar(uidplus = true)

    @Test(timeout = 90_000)
    fun readArchiveStarQuicklyWithoutUidplus_theNewUidComesLate_theStarStillLandsOnTheMovedCopy() = readArchiveStar(uidplus = false)

    private fun readArchiveStar(uidplus: Boolean) {
        val r = rig(window = 600)
        r.awaitInbox(3)
        if (!uidplus) {
            // No COPYUID: the new UID of the moved message is learned later, by its Message-ID.
            r.server.setUidplus(false)
            r.server.dropConnections()
        }
        r.settle()
        val row = r.inbox().first()

        r.run { r.repo.markSeen(listOf(row.id), true) }
        r.run { r.repo.archive(listOf(row.id)) }
        r.run { r.repo.setFlagged(listOf(row.id), true) }

        await("the message in Archive with both flags") {
            r.server.flagsOfSubject("Archive", row.subject).takeIf { "\\Seen" in it && "\\Flagged" in it }
        }
        assertEquals(listOf(row.subject), r.server.subjects("Archive"))
        assertEquals("nothing left behind in the Inbox with a flag", 2, r.server.uids("INBOX").size)
        assertTrue(r.server.uids("INBOX").all { uid -> r.server.flags("INBOX", uid).none { it == "\\Flagged" } })
        assertEquals(emptyList<OpFailure>(), r.failures())
    }

    @Test(timeout = 90_000)
    fun aBulkOfFiftyAndOneGoesThroughTheQueueAsOneCall() {
        // The "asks first" rule is the screen's (SelectionRules.needsConfirmation, tested in ListRulesTest and the Inbox screen
        // tests); the repository takes any number. 51 messages are trashed in one call, and all of them reach the server.
        val r = rig(messages = 51, window = 300)
        r.awaitInbox(51)
        r.settle()
        val ids = r.inbox().map { it.id }
        val ops = r.run { r.repo.trash(ids) }
        assertEquals(51, ops.size)
        assertEquals(0, r.inbox().size)
        await("51 messages in the server's Trash", 60_000) { r.server.uids("Trash").takeIf { it.size == 51 } }
        assertEquals(emptyList<OpFailure>(), r.failures())
    }
}
