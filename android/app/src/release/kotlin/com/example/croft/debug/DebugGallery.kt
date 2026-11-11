// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug

import androidx.compose.runtime.Composable

/**
 * The release build has no component gallery: the real one is in `src/debug`, which is not part of a release build, so
 * there is no screen, no route and no gesture that could reach it. Same API as the debug file.
 */
object DebugGallery {
    /** Always false in a release build. */
    const val available: Boolean = false

    /** Never shown. */
    @Composable
    fun Screen(onBack: () -> Unit) = Unit
}
