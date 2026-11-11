// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.background
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
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton

/** How often S09 asks the core how the first sync is going. A polling interval, not an animation. */
const val FirstSyncPollMs: Long = 500L

/**
 * S09 First sync. Skeleton rows stand in for the newest messages while the core fetches them (the newest first, older mail keeps
 * coming in the background), with the number of Inbox messages so far. "Continue in background" is always there; when the first
 * sync is done the host goes to the Inbox by itself. If the account stopped, the screen says so and still lets the person continue.
 * There is no Back arrow: Back does the same as Continue.
 */
@Composable
fun FirstSyncScreen(
    accountName: String,
    status: FirstSyncStatus?,
    onContinue: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val description = stringResource(R.string.onb_sync_progress_description)
    OnboardingFrame(stringResource(R.string.onb_sync_title, accountName), onBack = null, modifier = modifier.testTag(OnboardingTags.FIRST_SYNC)) {
        ScreenHeading(stringResource(R.string.onb_sync_heading))
        Text(
            stringResource(R.string.onb_sync_inbox_count, status?.inboxMessages ?: 0), style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (LocalMotionLevel.current == MotionLevel.OFF) {
            LinearProgressIndicator(progress = { STATIC_PROGRESS }, Modifier.fillMaxWidth().semantics { contentDescription = description })
        } else {
            LinearProgressIndicator(Modifier.fillMaxWidth().semantics { contentDescription = description })
        }
        Surface(Modifier.fillMaxWidth().testTag(OnboardingTags.SYNC_SKELETON), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
                repeat(SKELETON_ROWS) { SkeletonRow() }
            }
        }
        if (status?.failed == true) {
            Text(stringResource(R.string.onb_sync_failed), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
        } else {
            Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.secondaryContainer) {
                Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
                    Icon(Icons.Filled.Info, contentDescription = null, tint = MaterialTheme.colorScheme.onSecondaryContainer, modifier = Modifier.size(20.dp))
                    Text(stringResource(R.string.onb_sync_background_note), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
                }
            }
        }
        CroftButton(
            stringResource(R.string.onb_sync_continue), onContinue,
            Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.SYNC_CONTINUE), kind = ButtonKind.TONAL,
        )
    }
}

private const val SKELETON_ROWS = 3
private const val STATIC_PROGRESS = 0.4f

/** A grey stand-in for one message row: an avatar disc and two lines. */
@Composable
private fun SkeletonRow() {
    val fill = MaterialTheme.colorScheme.surfaceContainerHighest
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(48.dp).background(fill, CircleShape))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.fillMaxWidth(0.7f).heightIn(min = 14.dp).background(fill, MaterialTheme.shapes.extraSmall))
            Box(Modifier.fillMaxWidth().heightIn(min = 12.dp).background(fill, MaterialTheme.shapes.extraSmall))
        }
    }
}
