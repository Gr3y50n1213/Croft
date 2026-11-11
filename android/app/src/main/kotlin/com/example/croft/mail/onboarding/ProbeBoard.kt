// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.DiscoveryProbe
import uniffi.croft_ffi.ProbeFailureReason
import uniffi.croft_ffi.ProbeProgress

/** The four rows S03 shows. The core has five probes: the two autoconfig places share one row. */
enum class ProbeRow { BUILT_IN_TABLE, AUTOCONFIG, SRV, JMAP_WELL_KNOWN;
    companion object {
        /** The row a core probe is shown in. */
        fun of(probe: DiscoveryProbe): ProbeRow = when (probe) {
            DiscoveryProbe.PROVIDER_TABLE -> BUILT_IN_TABLE
            DiscoveryProbe.AUTOCONFIG_SUBDOMAIN, DiscoveryProbe.AUTOCONFIG_WELL_KNOWN -> AUTOCONFIG
            DiscoveryProbe.SRV -> SRV
            DiscoveryProbe.JMAP_WELL_KNOWN -> JMAP_WELL_KNOWN
        }
    }
}

/** Where one row stands. [NoMatch] is a probe that looked and found nothing usable; [reason] is shown behind Details. */
sealed interface RowState {
    data object Waiting : RowState
    data object Running : RowState
    data object Found : RowState
    data object Skipped : RowState
    data class NoMatch(val reason: ProbeFailureReason) : RowState
}

/** Every row, in the order S03 shows them. */
typealias ProbeRows = Map<ProbeRow, RowState>

/** The start of a lookup: every row waiting. */
fun waitingRows(): ProbeRows = ProbeRow.entries.associateWith { RowState.Waiting }

/**
 * Collects the core's progress callbacks ([DiscoveryObserver], called one at a time from a core thread) into the four rows, and
 * reports each change to [onChange]. The autoconfig row combines its two probes: Found if either found, else Running if either
 * runs, else Waiting if either waits, else No match if either failed, else Skipped.
 */
class ProbeBoard(private val onChange: (ProbeRows) -> Unit) : DiscoveryObserver {
    private val probes = HashMap<DiscoveryProbe, ProbeProgress>()

    @Synchronized
    override fun onProbe(probe: DiscoveryProbe, state: ProbeProgress) {
        probes[probe] = state
        onChange(rows())
    }

    /** The rows as they stand now. */
    @Synchronized
    fun rows(): ProbeRows = ProbeRow.entries.associateWith { row -> combine(DiscoveryProbe.entries.filter { ProbeRow.of(it) == row }.map { probes[it] ?: ProbeProgress.Waiting }) }

    companion object {
        /** One row from the states of the probes that share it. */
        fun combine(states: List<ProbeProgress>): RowState = when {
            states.any { it is ProbeProgress.Found } -> RowState.Found
            states.any { it is ProbeProgress.Running } -> RowState.Running
            states.any { it is ProbeProgress.Waiting } -> RowState.Waiting
            else -> states.filterIsInstance<ProbeProgress.Failed>().firstOrNull()?.let { RowState.NoMatch(it.reason) } ?: RowState.Skipped
        }
    }
}
