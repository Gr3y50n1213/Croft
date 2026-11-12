// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug

import android.content.Context
import com.example.croft.secrets.AppSecrets
import java.io.File

/**
 * Hooks for tests that start the real activity. They live in the `debug` source set, which is not part of a release build; the release
 * source set has a stub with the same API that never holds anything (`src/release/.../TestHooks.kt`).
 */
object TestHooks {
    /**
     * Makes the app's secrets in place of the Keystore ones: a simulated device has no usable AndroidKeyStore. Null in the app. The field
     * has a name that exists nowhere else, so `tools/check-release-outputs.sh` can look for it in the release build's mapping.
     */
    @Volatile var debugOnlySecretsFactory: ((Context, File) -> AppSecrets)? = null

    /** The factory a test set, or null. */
    var secretsFactory: ((Context, File) -> AppSecrets)?
        get() = debugOnlySecretsFactory
        set(value) { debugOnlySecretsFactory = value }
}
