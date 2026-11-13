// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The checked-in licence list of S60 (`assets/licenses/open-source-licences.json`, made by `android/tools/gen_licences.py`): it parses, is
 * not empty, names the libraries the app is built on and gives each a licence. That it matches the current dependency files is checked
 * by `android/tools/test_gen_licences.py` in CI.
 */
class LicenceListTest {
    private val entries = Licences.parse(File("src/main/assets/${Licences.ASSET}").readText())

    @Test
    fun theListIsNotEmptyAndEveryEntryHasANameAndALicence() {
        assertTrue("only ${entries.size} entries", entries.size > 100)
        entries.forEach { assertTrue(it.name, it.name.isNotBlank() && it.licence.isNotBlank() && it.licence != "?") }
    }

    @Test
    fun itNamesTheLibrariesTheAppIsBuiltOn() {
        val names = entries.map { it.name }
        for (expected in listOf("net.java.dev.jna:jna", "androidx.datastore:datastore", "org.jetbrains.kotlin:kotlin-stdlib", "serde", "tokio", "uniffi")) {
            assertTrue("$expected is missing", names.any { it == expected || it.startsWith("$expected ") })
        }
        assertTrue(entries.any { it.origin == LicenceEntry.Origin.FONT })
    }

    @Test
    fun eachStoreBuildListsItsOwnPushLibrary() {
        // T34d: fdroid and github share src/unifiedpush.
        val folder = if (com.example.croft.BuildConfig.STORE == "play") "play" else "unifiedpush"
        val own = Licences.parse(File("src/$folder/assets/${Licences.STORE_ASSET}").readText()).map { it.name }
        val expected = if (folder == "play") "com.google.firebase:firebase-messaging" else "org.unifiedpush.android:connector"
        assertTrue("$expected is missing from the $folder list", own.contains(expected))
        val other = if (folder == "play") "org.unifiedpush" else "com.google.firebase"
        assertTrue(own.none { it.startsWith(other) } && entries.none { it.name.startsWith(other) })
    }

    @Test
    fun aDamagedFileGivesAnEmptyListInsteadOfACrash() {
        assertTrue(Licences.parse("not json").isEmpty())
        assertTrue(Licences.parse("""{"entries": 5}""").isEmpty())
        assertEquals(1, Licences.parse("""{"entries":[{"name":"x","licence":"MIT"},{"nope":1}]}""").size)
    }
}
