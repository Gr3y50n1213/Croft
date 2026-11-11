// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug

import androidx.compose.runtime.Composable
import com.example.croft.debug.gallery.GalleryScreen

/**
 * The component gallery, debug builds only. It lives in `src/debug`, so a release build does not contain it (the file in
 * `src/release` has the same API and shows nothing). There is no hidden gesture to open it: Settings lists it in debug builds.
 */
object DebugGallery {
    /** True in a debug build. */
    const val available: Boolean = true

    /** The gallery screen. */
    @Composable
    fun Screen(onBack: () -> Unit) = GalleryScreen(onBack)
}
