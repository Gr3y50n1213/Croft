// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.MessageScope
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.ui.components.AccountStatus
import com.example.croft.ui.components.AccountStatusChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.minTouchTarget

/** The account chip state of the app's controls for the core's account status. */
fun AccountState.toChip(): AccountStatus = when (this) {
    AccountState.OK -> AccountStatus.CONNECTED
    AccountState.SYNCING -> AccountStatus.SYNCING
    AccountState.OFFLINE -> AccountStatus.OFFLINE
    AccountState.NEEDS_SIGN_IN -> AccountStatus.NEEDS_SIGN_IN
    AccountState.ERROR -> AccountStatus.SERVER_PROBLEM
}

/**
 * S13: the account switcher, a card over a scrim (it is drawn inside the screen, not in a separate window, so Back, focus and
 * tests treat it like the rest of the screen). "All accounts" is the unified Inbox; every account has its status chip that
 * says what to do (Syncing, Offline, Needs sign-in, Server problem) or "Up to date". Back, the close button and a tap
 * on the scrim close it. The card scales in from 90 percent with a fade at Full, fades at Reduced, and is there at once at Off.
 */
@Composable
fun AccountSwitcher(
    visible: Boolean,
    accounts: List<AccountInfo>,
    scope: MessageScope,
    unreadAll: Int,
    formatSynced: (Long) -> String,
    onSelectAll: () -> Unit,
    onSelectAccount: (AccountInfo) -> Unit,
    onFixStatus: (AccountInfo) -> Unit,
    onAddAccount: () -> Unit,
    onManageAccounts: () -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val level = LocalMotionLevel.current
    val scheme = MaterialTheme.motionScheme
    val scrimEnter: EnterTransition = if (level == MotionLevel.OFF) EnterTransition.None else fadeIn(scheme.defaultEffectsSpec())
    val scrimExit: ExitTransition = if (level == MotionLevel.OFF) ExitTransition.None else fadeOut(scheme.defaultEffectsSpec())
    val cardEnter: EnterTransition = when (level) {
        MotionLevel.FULL -> scaleIn(scheme.defaultSpatialSpec(), initialScale = CARD_START_SCALE) + fadeIn(scheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeIn(scheme.defaultEffectsSpec())
        MotionLevel.OFF -> EnterTransition.None
    }
    val cardExit: ExitTransition = when (level) {
        MotionLevel.FULL -> scaleOut(scheme.defaultSpatialSpec(), targetScale = CARD_START_SCALE) + fadeOut(scheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeOut(scheme.defaultEffectsSpec())
        MotionLevel.OFF -> ExitTransition.None
    }
    BackHandler(enabled = visible, onBack = onClose)
    Box(modifier.fillMaxSize()) {
        AnimatedVisibility(visible, enter = scrimEnter, exit = scrimExit) {
            // The scrim does not take part in semantics: a tap on it closes, and Back and the close button do the same for everyone else.
            Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = null, indication = null, onClick = onClose).clearAndSetSemantics { })
        }
        AnimatedVisibility(visible, enter = cardEnter, exit = cardExit, modifier = Modifier.align(Alignment.TopCenter)) {
            val title = stringResource(R.string.mail_switcher_title)
            Surface(
                Modifier.padding(16.dp).fillMaxWidth().widthIn(max = 480.dp).testTag(InboxTags.SWITCHER).semantics { paneTitle = title },
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                shadowElevation = 6.dp,
            ) {
                Column(Modifier.heightIn(max = 640.dp).verticalScroll(rememberScrollState()).padding(vertical = 8.dp)) {
                    Row(Modifier.padding(start = 24.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(title, Modifier.weight(1f).semantics { heading() }, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        CroftIconButton(Icons.Filled.Close, stringResource(R.string.mail_switcher_close), onClose)
                    }
                    SwitcherRow(
                        tag = InboxTags.switcherAccount(0),
                        label = stringResource(R.string.mail_all_accounts),
                        sub = null,
                        initial = "U",
                        selected = scope == MessageScope.UnifiedInbox,
                        containerColour = MaterialTheme.colorScheme.primary,
                        contentColour = MaterialTheme.colorScheme.onPrimary,
                        onClick = onSelectAll,
                    ) { Text(stringResource(R.string.mail_drawer_unread, unreadAll), style = MaterialTheme.typography.labelLarge) }
                    accounts.forEachIndexed { i, a ->
                        val (container, content) = accountColours(i)
                        SwitcherRow(
                            tag = InboxTags.switcherAccount(a.id),
                            label = a.label,
                            sub = a.email,
                            initial = a.label,
                            selected = scope is MessageScope.Folder && scope.account == a.id,
                            containerColour = container,
                            contentColour = content,
                            onClick = { onSelectAccount(a) },
                        ) {
                            if (a.status == AccountState.OK) {
                                Text(
                                    a.lastSyncedMs?.let { stringResource(R.string.mail_switcher_synced_at, formatSynced(it)) } ?: stringResource(R.string.mail_switcher_up_to_date),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            } else AccountStatusChip(a.label, a.status.toChip(), onClick = { onFixStatus(a) })
                        }
                    }
                    HorizontalDivider(Modifier.padding(vertical = 8.dp, horizontal = 16.dp))
                    ActionRow(Icons.Filled.Add, stringResource(R.string.mail_switcher_add), onAddAccount)
                    ActionRow(Icons.Filled.Settings, stringResource(R.string.mail_switcher_manage), onManageAccounts)
                }
            }
        }
    }
}

private const val CARD_START_SCALE = 0.9f
private const val SCRIM_ALPHA = 0.32f

@Composable
private fun accountColours(index: Int): Pair<Color, Color> {
    val c = MaterialTheme.colorScheme
    return when (index % 3) {
        0 -> c.primary to c.onPrimary
        1 -> c.tertiary to c.onTertiary
        else -> c.secondary to c.onSecondary
    }
}

@Composable
private fun SwitcherRow(
    tag: String,
    label: String,
    sub: String?,
    initial: String,
    selected: Boolean,
    containerColour: Color,
    contentColour: Color,
    onClick: () -> Unit,
    trailing: @Composable () -> Unit,
) {
    Surface(
        Modifier.padding(horizontal = 12.dp).fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent,
        contentColor = if (selected) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurface,
    ) {
        Row(
            Modifier.testTag(tag).clickable(role = Role.Button, onClick = onClick).heightIn(min = 64.dp).padding(horizontal = 12.dp, vertical = 8.dp).semantics { this.selected = selected },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            InitialCircle(initial, 40.dp, containerColour, contentColour)
            Column(Modifier.weight(1f)) {
                Text(label, style = MaterialTheme.typography.titleMedium)
                if (sub != null) Text(sub, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            trailing()
        }
    }
}

@Composable
private fun ActionRow(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp).minTouchTarget().clickable(role = Role.Button, onClick = onClick).heightIn(min = 56.dp).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Text(label, style = MaterialTheme.typography.titleMedium)
    }
}
