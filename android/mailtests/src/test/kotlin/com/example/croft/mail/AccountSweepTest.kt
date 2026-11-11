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
 * The start-up sweep that finishes a removal a crash cut short (M2 and M1 of the T17c review): the saved secrets, the folder choices and
 * the prefs of ids the core does not list, and the removal marks. A fake secret store records what the sweep did; the folder mapping is the
 * real file-backed one.
 */
class AccountSweepTest {
    private val dir: File = Files.createTempDirectory("croft-sweep-test").toFile()

    @After
    fun tearDown() { dir.deleteRecursively() }

    private class FakeSecrets(
        val stored: MutableSet<Long>,
        val marks: MutableSet<Long>,
        val quarantined: MutableSet<Long>,
    ) : AccountSweep.Secrets {
        override fun storedIds() = stored.toSet()
        override fun unfinishedRemovals() = marks.toSet()
        override fun deleteSecrets(id: Long) { stored -= id }
        override fun finishRemoval(id: Long) { stored -= id; quarantined -= id; marks -= id }
        override fun clearMark(id: Long) { marks -= id }
    }

    @Test
    fun aCrashBetweenTheStepsOfARemovalIsFinishedAtTheNextStart() {
        // The core lists account 1 only. Account 2 was removed in the core, then the app was killed (its secret, its mark, its quarantined
        // copy and its app-side entries are left). Account 3's removal began but never reached the core (the mark is there, it is listed).
        // Account 4 has a stray secret and nothing else.
        val secrets = FakeSecrets(stored = mutableSetOf(1, 2, 3, 4), marks = mutableSetOf(2, 3), quarantined = mutableSetOf(2, 3, 5))
        val mapping = FileFolderMapping(File(dir, "croft-folders.txt"))
        for (id in listOf(1L, 2L, 3L, 4L)) mapping.remember(id, FolderRole.ARCHIVE, id * 100 + 3)
        mapping.recordRecent(2, 202)
        mapping.decline(4, FolderRole.TRASH)
        val prefs = AccountPrefs(order = listOf(2, 1, 3, 4), names = mapOf(1L to "One", 2L to "Two", 4L to "Four"), colours = mapOf(2L to AccountColour.AMBER, 1L to AccountColour.NEUTRAL))

        val sweep = AccountSweep(known = setOf(1L, 3L), secretStore = secrets)
        sweep.secrets()
        sweep.folderMapping(mapping)
        val swept = sweep.prefs(prefs)

        assertEquals("account 2's removal is finished, and the stray secret of 4 goes; 1 and 3 are listed and stay", setOf(1L, 3L), secrets.stored)
        assertTrue("the marks are gone: 2's removal was finished, 3's never reached the core", secrets.marks.isEmpty())
        assertEquals("quarantined copies go only for a removal that was begun (2); 3's removal did not happen; 5 is an older database's", setOf(3L, 5L), secrets.quarantined)
        assertEquals(setOf(1L, 3L), mapping.accounts().filter { it in setOf(1L, 3L) }.toSet())
        assertNull(mapping.mapped(2, FolderRole.ARCHIVE))
        assertNull(mapping.mapped(4, FolderRole.ARCHIVE))
        assertEquals("account 1's choice stays", 103L, mapping.mapped(1, FolderRole.ARCHIVE))
        assertEquals(listOf(1L, 3L), swept.order)
        assertEquals(mapOf(1L to "One"), swept.names)
        assertEquals(mapOf(1L to AccountColour.NEUTRAL), swept.colours)
        // The folder choices are in the file too: a new process finds them already swept.
        assertEquals(setOf(1L, 3L), FileFolderMapping(File(dir, "croft-folders.txt")).accounts())
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
