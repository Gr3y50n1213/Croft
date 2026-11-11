// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.haptics

/**
 * The recording fake (docs/ux/haptics.md, Implementation): stores every event it is given, in order. Tests put it where the
 * platform haptics would be, either directly (`LocalHaptics provides recording`) or behind a [LevelFilteredHaptics].
 */
class RecordingHaptics : Haptics {
    private val log = ArrayList<HapticEvent>()

    /** The events so far, in the order they arrived. */
    val events: List<HapticEvent> get() = synchronized(log) { log.toList() }

    override fun perform(event: HapticEvent) {
        synchronized(log) { log.add(event) }
    }

    /** Forget everything recorded so far. */
    fun clear() = synchronized(log) { log.clear() }
}
