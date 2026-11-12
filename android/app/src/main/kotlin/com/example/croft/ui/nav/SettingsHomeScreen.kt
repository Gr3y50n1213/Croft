// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.example.croft.R

/** Test tags of the settings list. */
object SettingsTags {
    const val NOTIFICATIONS = "settings-notifications"
    const val SCREEN = "settings"
    const val APPEARANCE = "settings-appearance"
    const val SWIPE_ACTIONS = "settings-swipe-actions"
    const val INBOX = "settings-inbox"
    const val GALLERY = "settings-gallery"
    const val ABOUT = "settings-about"
    const val ACCOUNTS = "settings-accounts"
}

/**
 * The settings list as far as it exists: Accounts (S44), Appearance (S50), Swipe actions, Inbox, About and diagnostics (S60) and, in
 * debug builds only, the component gallery. The other settings screens (S43 onwards) will fill it in.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsHomeScreen(
    info: com.example.croft.RustInfo?,
    onOpenAppearance: () -> Unit,
    onOpenGallery: (() -> Unit)?,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    /** S47 Swipe actions; the entry is hidden when null. */
    onOpenSwipeActions: (() -> Unit)? = null,
    /** S49 Inbox and categories; the entry is hidden when null. */
    onOpenInbox: (() -> Unit)? = null,
    /** S44 Accounts; the entry is hidden when null. */
    onOpenAccounts: (() -> Unit)? = null,
    /** S60 About and diagnostics; the About line is plain text when null. */
    onOpenAbout: (() -> Unit)? = null,
    /** S46 Notifications (T20). */
    onOpenNotifications: (() -> Unit)? = null,
) {
    Scaffold(
        modifier = modifier.testTag(SettingsTags.SCREEN),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.nav_back)) }
                },
            )
        },
    ) { inner ->
        Column(Modifier.padding(inner).verticalScroll(rememberScrollState())) {
            if (onOpenAccounts != null) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.settings_accounts)) },
                    supportingContent = { Text(stringResource(R.string.settings_accounts_hint)) },
                    modifier = Modifier.testTag(SettingsTags.ACCOUNTS).clickable(role = Role.Button, onClick = onOpenAccounts),
                )
            }
            ListItem(
                headlineContent = { Text(stringResource(R.string.settings_appearance)) },
                supportingContent = { Text(stringResource(R.string.settings_appearance_hint)) },
                modifier = Modifier.testTag(SettingsTags.APPEARANCE).clickable(role = Role.Button, onClick = onOpenAppearance),
            )
            if (onOpenSwipeActions != null) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.settings_swipe_actions)) },
                    supportingContent = { Text(stringResource(R.string.settings_swipe_actions_hint)) },
                    modifier = Modifier.testTag(SettingsTags.SWIPE_ACTIONS).clickable(role = Role.Button, onClick = onOpenSwipeActions),
                )
            }
            if (onOpenNotifications != null) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.settings_notifications)) },
                    supportingContent = { Text(stringResource(R.string.settings_notifications_hint)) },
                    modifier = Modifier.testTag(SettingsTags.NOTIFICATIONS).clickable(role = Role.Button, onClick = onOpenNotifications),
                )
            }
            if (onOpenInbox != null) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.settings_inbox)) },
                    supportingContent = { Text(stringResource(R.string.settings_inbox_hint)) },
                    modifier = Modifier.testTag(SettingsTags.INBOX).clickable(role = Role.Button, onClick = onOpenInbox),
                )
            }
            ListItem(
                headlineContent = { Text(stringResource(R.string.settings_about)) },
                supportingContent = {
                    Text(
                        if (info != null) stringResource(R.string.core_version, info.croftVersion) + ". " + stringResource(R.string.ffi_api_version, info.ffiApiVersion.toString())
                        else stringResource(R.string.core_unavailable),
                    )
                },
                modifier = Modifier.testTag(SettingsTags.ABOUT).then(if (onOpenAbout != null) Modifier.clickable(role = Role.Button, onClick = onOpenAbout) else Modifier),
            )
            if (onOpenGallery != null) {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.settings_component_gallery)) },
                    supportingContent = { Text(stringResource(R.string.settings_component_gallery_hint)) },
                    modifier = Modifier.testTag(SettingsTags.GALLERY).clickable(role = Role.Button, onClick = onOpenGallery),
                )
            }
        }
    }
}
