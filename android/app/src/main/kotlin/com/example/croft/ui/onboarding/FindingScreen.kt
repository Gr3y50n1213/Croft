// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.Lookup
import com.example.croft.mail.onboarding.ProbeRow
import com.example.croft.mail.onboarding.ProbeRows
import com.example.croft.mail.onboarding.RowState
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftSpinner

/**
 * S03 Finding your server. One row per place the core looks (known providers, autoconfig, DNS service records, JMAP), each showing
 * waiting, checking, found, skipped or no match, driven by the core's progress callbacks. The first match goes on to S04 by itself
 * (the flow does that). If the lookup fails (offline, no name lookup), the failure is shown here with Try again and "Enter settings
 * manually": the screen never waits for ever, because every probe of the core has its own time limit. Back (the arrow, or the
 * system) cancels the lookup and goes back to S02.
 */
@Composable
fun FindingScreen(
    email: String,
    probes: ProbeRows,
    lookup: Lookup,
    onRetry: () -> Unit,
    onEnterManually: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    OnboardingFrame(stringResource(R.string.onb_add_account_title), onBack, modifier.testTag(OnboardingTags.FINDING)) {
        ScreenHeading(stringResource(R.string.onb_finding_heading))
        Text(email, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(stringResource(R.string.onb_finding_in_parallel), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow) {
            Column(Modifier.padding(vertical = 8.dp)) {
                ProbeRow.entries.forEach { row -> ProbeRowItem(row, probes[row] ?: RowState.Waiting) }
            }
        }
        if (lookup is Lookup.Failed) {
            FailureNotice(lookup.failure, probeDetails(probes), Modifier.testTag(OnboardingTags.FINDING_FAILURE))
            CroftButton(stringResource(R.string.onb_try_again), onRetry, Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.FINDING_RETRY))
            CroftButton(
                stringResource(R.string.onb_enter_manually), onEnterManually,
                Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(OnboardingTags.FINDING_MANUAL), kind = ButtonKind.TONAL,
            )
        }
        CroftButton(
            stringResource(R.string.control_cancel), onBack,
            Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(OnboardingTags.FINDING_CANCEL), kind = ButtonKind.TEXT,
        )
    }
}

/** One row of S03: an icon for the state, the name of the place, and the state in words. */
@Composable
private fun ProbeRowItem(row: ProbeRow, state: RowState) {
    val stateText = stringResource(
        when (state) {
            RowState.Waiting -> R.string.onb_probe_waiting
            RowState.Running -> R.string.onb_probe_running
            RowState.Found -> R.string.onb_probe_found
            RowState.Skipped -> R.string.onb_probe_skipped
            is RowState.NoMatch -> R.string.onb_probe_no_match
        },
    )
    val name = probeLabel(row)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp)
            .semantics(mergeDescendants = true) { stateDescription = stateText }.testTag(OnboardingTags.probeRow(row)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ProbeIcon(state)
        Text(name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Text(stateText, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** The state as a shape, so it does not rest on colour: a check in a filled disc (found), a spinner (checking), a dash (skipped, no match), an empty ring (waiting). */
@Composable
private fun ProbeIcon(state: RowState) {
    val primary = MaterialTheme.colorScheme.primary
    val outline = MaterialTheme.colorScheme.outline
    Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) {
        when (state) {
            RowState.Found -> Box(Modifier.size(28.dp).background(primary, CircleShape), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.Check, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size(18.dp))
            }
            RowState.Running -> CroftSpinner(Modifier.size(24.dp), color = primary)
            RowState.Skipped, is RowState.NoMatch -> Box(Modifier.size(28.dp).background(MaterialTheme.colorScheme.surfaceContainerHighest, CircleShape), contentAlignment = Alignment.Center) {
                Box(Modifier.size(width = 12.dp, height = 2.dp).background(MaterialTheme.colorScheme.onSurfaceVariant))
            }
            RowState.Waiting -> Box(Modifier.size(24.dp).border(2.dp, outline, CircleShape))
        }
    }
}
