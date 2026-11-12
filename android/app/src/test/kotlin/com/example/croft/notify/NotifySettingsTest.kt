// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NotifySettingsTest {
    @Test
    fun theDefaultsAreEvery30MinutesArchiveAndMarkReadAndAPrivateLockScreen() {
        val d = NotifySettings()
        assertTrue(d.enabled)
        assertEquals(CheckFrequency.MIN_30, d.frequency)
        assertEquals(listOf(NotifyAction.ARCHIVE, NotifyAction.MARK_READ), d.actions)
        assertTrue(d.hideOnLockScreen)
        assertFalse(d.introDone)
    }

    @Test
    fun aRoundTripKeepsEverything() {
        val s = NotifySettings(
            enabled = false, mutedAccounts = setOf(3, 1), frequency = CheckFrequency.HOUR_2, batteryNotLow = true,
            actions = listOf(NotifyAction.DELETE, NotifyAction.REPLY), hideOnLockScreen = false, introDone = true, signInNotified = setOf(7),
        )
        assertEquals(s, NotifySettingsJson.decodeOrDefault(NotifySettingsJson.encode(s)))
    }

    @Test
    fun aDamagedOrNewerFileReadsAsDefaultsFieldByField() {
        assertEquals(NotifySettings(), NotifySettingsJson.decodeOrDefault("not json"))
        assertEquals(NotifySettings(), NotifySettingsJson.decodeOrDefault("""{"version": 99, "enabled": false}"""))
        val partial = NotifySettingsJson.decodeOrDefault("""{"frequency": "weekly", "actions": ["archive","read","delete","reply"], "enabled": false}""")
        assertEquals(CheckFrequency.MIN_30, partial.frequency)
        assertEquals("more than three buttons is not a choice S46 offers", NotifySettings().actions, partial.actions)
        assertFalse(partial.enabled)
    }

    @Test
    fun neverMoreThanThreeButtonsAndAlwaysInTheSameOrder() {
        var s = NotifySettings(actions = emptyList())
        s = s.withAction(NotifyAction.REPLY, true).withAction(NotifyAction.ARCHIVE, true).withAction(NotifyAction.DELETE, true)
        assertEquals(listOf(NotifyAction.ARCHIVE, NotifyAction.DELETE, NotifyAction.REPLY), s.actions)
        assertEquals(s, s.withAction(NotifyAction.MARK_READ, true))
        assertEquals(listOf(NotifyAction.ARCHIVE, NotifyAction.REPLY), s.withAction(NotifyAction.DELETE, false).actions)
    }

    @Test
    fun forgettingAnAccountDropsItsSwitchAndItsSignInFlag() {
        val s = NotifySettings(mutedAccounts = setOf(1, 2), signInNotified = setOf(1)).forget(1)
        assertEquals(setOf(2L), s.mutedAccounts)
        assertTrue(s.signInNotified.isEmpty())
        assertFalse(NotifySettings(mutedAccounts = setOf(2)).wants(2))
        assertFalse(NotifySettings(enabled = false).wants(1))
    }
}
