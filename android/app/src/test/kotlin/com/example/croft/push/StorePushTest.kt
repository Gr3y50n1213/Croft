// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import com.example.croft.BuildConfig
import org.junit.Assert.assertEquals
import org.junit.Test

/** Runs in every flavour's unit tests: each store build has the push its store allows. */
class StorePushTest {
    @Test
    fun eachStoreBuildHasItsOwnPush() {
        val expected = when (BuildConfig.STORE) {
            "play" -> PushKind.FCM
            "fdroid", "github" -> PushKind.UNIFIED_PUSH
            else -> error("unknown store ${BuildConfig.STORE}")
        }
        assertEquals(expected, StorePush.kind)
    }
}
