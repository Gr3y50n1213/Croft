// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug

import android.content.Context
import com.example.croft.secrets.AppSecrets
import java.io.File

/**
 * The release build has no test hooks: the real ones are in `src/debug`, which is not part of a release build. This stub has the same
 * API, always reads as null and keeps nothing, so no test can swap the app's secrets in a release build.
 */
object TestHooks {
    /** Always null in a release build; setting it does nothing. */
    var secretsFactory: ((Context, File) -> AppSecrets)?
        get() = null
        @Suppress("UNUSED_PARAMETER")
        set(value) = Unit
}
