// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FolderMappingTest {
    @Test
    fun aChosenFolderAndTheRecentOnesSurviveARestart() {
        val dir = Files.createTempDirectory("croft-folders").toFile()
        try {
            val file = File(dir, "croft-folders.txt")
            val first = FileFolderMapping(file)
            first.remember(1, FolderRole.ARCHIVE, 11)
            first.remember(2, FolderRole.TRASH, 22)
            for (f in listOf(5L, 6L, 7L, 5L, 8L, 9L, 10L)) first.recordRecent(1, f)

            val second = FileFolderMapping(file)
            assertEquals(11L, second.mapped(1, FolderRole.ARCHIVE))
            assertEquals(22L, second.mapped(2, FolderRole.TRASH))
            assertNull(second.mapped(1, FolderRole.TRASH))
            // Newest first, no repeats, five at most.
            assertEquals(listOf(10L, 9L, 8L, 5L, 7L), second.recents(1))
            second.forget(1, FolderRole.ARCHIVE)
            assertNull(FileFolderMapping(file).mapped(1, FolderRole.ARCHIVE))
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun aDamagedFileLoadsWhatItCanAndNeverThrows() {
        val dir = Files.createTempDirectory("croft-folders").toFile()
        try {
            val file = File(dir, "croft-folders.txt")
            file.writeText("m 1 ARCHIVE 11\nnonsense\nm x ARCHIVE 1\nm 1 NO_SUCH_ROLE 3\nr 1 4,5\nr 2 a,b\n")
            val m = FileFolderMapping(file)
            assertEquals(11L, m.mapped(1, FolderRole.ARCHIVE))
            assertEquals(listOf(4L, 5L), m.recents(1))
            assertEquals(emptyList<Long>(), m.recents(2))
        } finally {
            dir.deleteRecursively()
        }
    }
}
