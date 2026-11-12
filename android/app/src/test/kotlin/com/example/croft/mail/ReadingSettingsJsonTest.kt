// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.RemoteImages
import org.junit.Assert.assertEquals
import org.junit.Test

/** The two settings of reading (S49) in the mail settings file: defaults from the docs, a round trip, and a file that cannot be read. */
class ReadingSettingsJsonTest {
    @Test
    fun theDefaultsAreTheDocsOnes_askAndAfterTwoSeconds() {
        assertEquals(RemoteImages.ASK, MailSettings().remoteImages)
        assertEquals(MarkReadDelay.AFTER_2_SECONDS, MailSettings().markRead)
        assertEquals(2_000L, MarkReadDelay.AFTER_2_SECONDS.delayMs)
        assertEquals(0L, MarkReadDelay.IMMEDIATELY.delayMs)
        assertEquals(null, MarkReadDelay.NEVER.delayMs)
    }

    @Test
    fun everyChoiceRoundTripsThroughTheFile() {
        for (r in RemoteImages.entries) for (m in MarkReadDelay.entries) for (u in MailSettings.UndoChoices) {
            val s = MailSettings(u, r, m)
            assertEquals(s, MailSettingsJson.decodeOrDefault(MailSettingsJson.encode(s)))
        }
    }

    @Test
    fun anOlderFileWithoutTheFieldsReadsAsTheDefaults_andAnUnknownValueIsTheDefaultForThatFieldOnly() {
        assertEquals(MailSettings(3), MailSettingsJson.decodeOrDefault("""{"version":1,"undoSeconds":3}"""))
        assertEquals(
            MailSettings(10, RemoteImages.NEVER, MarkReadDelay.AFTER_2_SECONDS),
            MailSettingsJson.decodeOrDefault("""{"version":1,"undoSeconds":10,"remoteImages":"never","markRead":"sometimes"}"""),
        )
        assertEquals(MailSettings(5, RemoteImages.ASK, MarkReadDelay.NEVER), MailSettingsJson.decodeOrDefault("""{"version":1,"remoteImages":42,"markRead":"never"}"""))
        assertEquals(MailSettings(), MailSettingsJson.decodeOrDefault("""{"version":99,"remoteImages":"never"}"""))
    }
}
