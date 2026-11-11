// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSensitivity
import com.example.croft.swipe.SwipeSettings
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** The swipe settings survive a restart (DataStore on a file, like the theme) and a damaged file reads as the defaults. */
class SwipeSettingsStoreTest {
    @get:Rule
    val folder = TemporaryFolder()

    private fun store(file: File, scope: CoroutineScope) = SwipeSettingsStore.onFile(file, scope)

    @Test
    fun whatWasChosenIsThereAfterARestart() = runBlocking {
        val file = File(folder.root, "datastore/croft-swipe.json")
        val first = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        store(file, first).set(SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM, SwipeSensitivity.LOW))
        first.cancel()
        val second = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        assertEquals(SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM, SwipeSensitivity.LOW), store(file, second).settings.first())
        second.cancel()
    }

    @Test
    fun aNewInstallReadsTheDefaults() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        assertEquals(SwipeSettings.Default, store(File(folder.root, "none/croft-swipe.json"), scope).settings.first())
        scope.cancel()
    }

    @Test
    fun aDamagedFileReadsAsTheDefaultsAndTheStoreStillWorks() = runBlocking {
        val file = File(folder.root, "croft-swipe.json").also { it.writeText("{ this is not json") }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val s = store(file, scope)
        assertEquals(SwipeSettings.Default, s.settings.first())
        s.set(SwipeSettings(SwipeAction.TOGGLE_STAR, SwipeAction.NONE))
        assertTrue(file.readText().contains("\"startToEnd\": \"star\""))
        scope.cancel()
    }
}
