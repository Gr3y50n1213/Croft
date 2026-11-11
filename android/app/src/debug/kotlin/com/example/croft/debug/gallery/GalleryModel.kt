// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.compose.runtime.Composable
import com.example.croft.ui.components.CroftSnackbarState

/** What a gallery entry needs from the screen around it. */
class GalleryEnv(
    /** The one snackbar host of the gallery (one at a time). */
    val snackbar: CroftSnackbarState,
)

/**
 * One control in one state (or one small group of states of the same control). [id] is `<section>.<name>` and is the test tag
 * `gallery:<id>`. [docRows] are the first-column cells of the tables in docs/ux/controls.md that this entry shows.
 */
class GalleryEntry(
    val id: String,
    val label: String,
    val docRows: List<String> = emptyList(),
    val content: @Composable GalleryEnv.() -> Unit,
)

/** A group of entries for one kind of control. */
class GallerySection(val id: String, val title: String, val entries: List<GalleryEntry>)

/** The tag a test uses to find an entry. */
fun galleryTag(entryId: String) = "gallery:$entryId"
