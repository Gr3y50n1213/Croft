// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.Context
import android.content.ContextWrapper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.ComposeStart
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * What compose keeps while the activity is rebuilt (rotation, a size or theme change): the [ComposeStart] it was opened with and its
 * controller, with the text, the chips and the files as they are. The autosave timers run on [scope], which outlives the window, so a
 * draft being typed is saved while the screen turns. Only a configuration change keeps it; when the process is killed the draft is what
 * autosave stored (it is in the Drafts folder).
 */
class ComposeRetained : ViewModel() {
    /** The start compose is open for, or null when it is not open. */
    var start: ComposeStart? = null
        private set

    /** The controller of [start]. */
    var controller: ComposeController? = null

    /** Whether the controller was opened on [start] already (opening is done once, not at every rebuilt window). */
    var opened = false

    /** The previous compose was saved as a draft because a link or a share replaced it: the new screen says "Draft saved". */
    var savedNotice by mutableStateOf(false)

    /** The scope the controller's timers run on. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var endedStart: ComposeStart? = null

    /** Whether compose for [start] has ended: a screen still on its way out must not open it again. */
    fun hasEnded(start: ComposeStart): Boolean = endedStart == start

    /** Compose opens for [start]: whatever was open before is dropped (its draft was saved by whoever dropped it). */
    fun begin(start: ComposeStart) {
        controller?.stop()
        endedStart = null
        this.start = start
        controller = null
        opened = false
    }

    /** Compose is over. */
    fun release() {
        controller?.stop()
        controller = null
        endedStart = start
        start = null
        opened = false
    }

    /** A new compose replaces the one that is open: its content is saved (as Back does), then the new one opens. */
    fun replaceWith(next: ComposeStart) {
        val old = controller
        begin(next)
        if (old != null) scope.launch {
            old.back()
            if (old.state.value.closed == com.example.croft.mail.compose.ComposeClosed.DraftSaved) savedNotice = true
        }
    }

    override fun onCleared() {
        // The activity is going for good: what was typed is saved.
        controller?.flushAsync()
        controller?.stop()
        scope.cancel()
    }
}

private tailrec fun Context.viewModelStoreOwner(): ViewModelStoreOwner? = when (this) {
    is ViewModelStoreOwner -> this
    is ContextWrapper -> baseContext.viewModelStoreOwner()
    else -> null
}

/** The [ComposeRetained] of the activity (or, with none, one that lives as long as this composition). */
@Composable
fun rememberComposeRetained(): ComposeRetained {
    val owner = LocalContext.current.viewModelStoreOwner()
    val fallback = remember { if (owner == null) ComposeRetained() else null }
    return if (owner != null) ViewModelProvider(owner)[ComposeRetained::class.java] else fallback!!
}
