// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.AccountSweep
import com.example.croft.mail.accounts.AppFiles
import java.io.File
import java.nio.file.Files
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The start-up sweep that finishes a removal a crash cut short (M2 and M1 of the T17c review): the saved secrets, the folder choices, the
 * list positions and the prefs of ids the core does not list. A fake secret store records what the sweep did; the folder mapping and the
 * anchors are the real file-backed ones.
 */
class AccountSweepTest {
    private val dir: File = Files.createTempDirectory("croft-sweep-test").toFile()

    @After
    fun tearDown() { dir.deleteRecursively() }

    private class FakeSecrets(val stored: MutableSet<Long>) : AccountSweep.Secrets {
        override fun storedIds() = stored.toSet()
        override fun deleteSecrets(id: Long) { stored -= id }
    }

    @Test
    fun aCrashBetweenTheStepsOfARemovalIsFinishedAtTheNextStart() {
        // The core lists account 1 only. Account 2 was removed in the core, then the app was killed: its secret, its folder choices, its
        // list positions and its prefs are left. Account 4 has a stray secret and a declined role.
        val secrets = FakeSecrets(stored = mutableSetOf(1, 2, 4))
        val mapping = FileFolderMapping(File(dir, "croft-folders.txt"))
        for (id in listOf(1L, 2L, 4L)) mapping.remember(id, FolderRole.ARCHIVE, id * 100 + 3)
        mapping.recordRecent(2, 202)
        mapping.decline(4, FolderRole.TRASH)
        val anchors = FileAnchorStore(File(dir, "croft-list-anchors-core.json"))
        anchors.save("folder:1:101", ListAnchor(5, 1000, 0f))
        anchors.save("folder:2:201", ListAnchor(6, 1000, 0f))
        anchors.save("unified", ListAnchor(7, 1000, 0f))
        anchors.flush()
        val prefs = AccountPrefs(order = listOf(2, 1, 4), names = mapOf(1L to "One", 2L to "Two", 4L to "Four"), colours = mapOf(2L to AccountColour.AMBER, 1L to AccountColour.NEUTRAL))

        val sweep = AccountSweep(known = setOf(1L), secretStore = secrets)
        sweep.secrets()
        sweep.folderMapping(mapping)
        sweep.anchors(anchors)
        anchors.flush()
        val swept = sweep.prefs(prefs)

        assertEquals("only the listed account keeps its secret", setOf(1L), secrets.stored)
        assertEquals(setOf(1L), mapping.accounts())
        assertNull(mapping.mapped(2, FolderRole.ARCHIVE))
        assertEquals("account 1's choice stays", 103L, mapping.mapped(1, FolderRole.ARCHIVE))
        assertEquals(setOf(1L), anchors.accounts())
        assertNull(anchors.load("folder:2:201"))
        assertEquals("the unified Inbox's place is not an account's", 7L, anchors.load("unified")?.messageId)
        assertEquals(listOf(1L), swept.order)
        assertEquals(mapOf(1L to "One"), swept.names)
        assertEquals(mapOf(1L to AccountColour.NEUTRAL), swept.colours)
        // The folder choices and the list positions are in their files too: a new process finds them already swept.
        assertEquals(setOf(1L), FileFolderMapping(File(dir, "croft-folders.txt")).accounts())
        assertEquals(setOf(1L), FileAnchorStore(File(dir, "croft-list-anchors-core.json")).accounts())
    }

    @Test
    fun forgettingAnAccountDropsItsChoicesItsDeclinedRolesAndItsRecentFolders() {
        val mapping = FileFolderMapping(File(dir, "croft-folders.txt"))
        mapping.remember(7, FolderRole.TRASH, 701)
        mapping.decline(7, FolderRole.ARCHIVE)
        mapping.recordRecent(7, 702)
        mapping.remember(8, FolderRole.TRASH, 801)
        mapping.forgetAccount(7)
        assertEquals(setOf(8L), mapping.accounts())
        assertEquals(emptyList<Long>(), mapping.recents(7))
        assertEquals(false, mapping.isDeclined(7, FolderRole.ARCHIVE))
        assertEquals(setOf(8L), FileFolderMapping(File(dir, "croft-folders.txt")).accounts())
    }

    @Test
    fun eraseDeletesEveryCroftFileOfTheNoBackupFolderAndNothingElse() {
        // Every file name Croft keeps in the no-backup folder, and one that is not Croft's.
        for (name in listOf("croft.db", "croft.db-wal", "croft.db-shm", "croft.db-journal", "croft-folders.txt", "croft-list-anchors-core.json", "croft-list-anchors.json")) {
            File(dir, name).writeText("x")
        }
        File(dir, "croft-secrets").mkdirs(); File(dir, "croft-secrets/db-key.v1").writeText("x"); File(dir, "croft-secrets/lock").writeText("")
        File(dir, "croft-quarantine/1700000000000/superseded/secrets").mkdirs(); File(dir, "croft-quarantine/1700000000000/superseded/secrets/acct-1.password.v1").writeText("x")
        File(dir, "other-library-cache.bin").writeText("not Croft's")
        assertEquals(emptyList<String>(), AppFiles.eraseMailFiles(dir))
        assertEquals("only a file that is not Croft's is left", listOf("other-library-cache.bin"), dir.list()!!.toList())
    }
}
