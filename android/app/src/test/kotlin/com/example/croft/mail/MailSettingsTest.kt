// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

/** The mail settings file (S49, the Undo time): the choices, the default, a file that cannot be read, and a restart. */
class MailSettingsTest {
    @Test
    fun theChoicesAreThreeFiveAndTenSeconds_andFiveIsTheDefault() {
        assertEquals(listOf(3, 5, 10), MailSettings.UndoChoices)
        assertEquals(5, MailSettings().undoSeconds)
        assertEquals(5_000L, MailSettings().undoWindowMs)
        assertEquals(10_000L, MailSettings(10).undoWindowMs)
    }

    @Test
    fun theFileRoundTripsAndNeverFails() {
        for (s in MailSettings.UndoChoices) assertEquals(MailSettings(s), MailSettingsJson.decodeOrDefault(MailSettingsJson.encode(MailSettings(s))))
        assertEquals(MailSettings(), MailSettingsJson.decodeOrDefault(""))
        assertEquals(MailSettings(), MailSettingsJson.decodeOrDefault("not json"))
        assertEquals("a time S49 does not offer", MailSettings(), MailSettingsJson.decodeOrDefault("""{"version":1,"undoSeconds":7}"""))
        assertEquals("a newer file is not read", MailSettings(), MailSettingsJson.decodeOrDefault("""{"version":99,"undoSeconds":3}"""))
    }

    @Test
    fun aChoiceSurvivesARestart() {
        val dir = kotlin.io.path.createTempDirectory("croft-mail-settings").toFile()
        try {
            val file = File(dir, "croft-mail.json")
            val first = CoroutineScope(SupervisorJob() + Dispatchers.IO)
            runBlocking { MailSettingsStore.onFile(file, first).set(MailSettings(10)) }
            first.cancel()
            val second = CoroutineScope(SupervisorJob() + Dispatchers.IO)
            assertEquals(MailSettings(10), runBlocking { MailSettingsStore.onFile(file, second).settings.first() })
            second.cancel()
        } finally {
            dir.deleteRecursively()
        }
    }
}
