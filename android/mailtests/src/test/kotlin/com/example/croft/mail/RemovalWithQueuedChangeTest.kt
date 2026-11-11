// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.accounts.CoreAccountOps
import com.example.croft.mail.accounts.SavedSignIn
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Removing an account with a change still waiting in its queue (a move inside its undo window) on the real core and a real in-process
 * IMAP server: the removal stops the account first, so nothing is sent to the server afterwards, and the queued row is gone with the
 * account (M2 of the T17c review).
 *
 * **What is not covered:** a queued *Outbox message*. The core's FFI has no compose or send call yet (another task wires real-account
 * sending), so no message can be queued from here. The Outbox rows are deleted with the account by the same `ON DELETE CASCADE`
 * that deletes the queue rows (the core test `removing_the_sample_mailbox_deletes_all_its_rows` counts `outbox` and `op_queue`), and
 * the engine's own tests cover what a removed account's task does with its outbox. Add a message here when sending is wired.
 */
class RemovalWithQueuedChangeTest {
    private val rigs = mutableListOf<Rig>()

    private fun rig(window: Long): Rig = Rig(messages = 3, undoWindowMs = window).also { rigs += it }

    @After
    fun tearDown() = rigs.forEach { it.close() }

    @Test(timeout = 60_000)
    fun removingTheAccountWithAChangeInTheQueueSendsNothingAfterwards() {
        val r = rig(window = 1_500)
        r.awaitInbox(3)
        r.settle()
        val id = r.inbox().first().id
        // A move that waits out its undo window (1.5 s).
        val ops = r.run { r.repo.archive(listOf(id)) }
        assertEquals(1, ops.size)
        assertEquals("the row leaves the list at once", 2, r.inbox().size)

        // Through the app's removal path (`AccountOps.remove`, whose real implementation calls `AccountSecrets.removeAccount`: the core
        // first, then the secret). The secret store is the app's and not in this module, so the lambda stands for its second step.
        val accountOps = CoreAccountOps(
            r.core,
            removeAccountAndSecrets = { id -> r.core.removeAccount(id) },
            savedSignIn = { SavedSignIn.PASSWORD },
            eraseQuarantine = {}, closeCore = {}, eraseStorage = {}, clearLocalFiles = {},
        )
        r.run { accountOps.remove(r.account) }

        // Well past the window, and a sync request on top: the server hears nothing that changes anything.
        Thread.sleep(3_000)
        assertEquals(emptyList<String>(), r.changingCommands())
        assertEquals("nothing was moved on the server", emptyList<UInt>(), r.server.uids("Archive"))
        assertEquals(3, r.server.uids("INBOX").size)
        // The account, its folders and its queued change are gone from the device.
        assertTrue(r.run { r.repo.listAccounts() }.none { it.id == r.account })
        assertTrue(r.run { r.repo.listFolders(r.account) }.none { !it.isVirtual })
        assertEquals(false, r.run { r.repo.undo(ops.single()) })
    }
}
