// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageScope
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.components.longPressMenu

/** The mailboxes in the order of the drawer; custom folders and labels follow under their own heading. */
private val DrawerOrder = listOf(
    FolderRole.INBOX, FolderRole.STARRED, FolderRole.SNOOZED, FolderRole.FOLLOW_UP, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX,
    FolderRole.ARCHIVE, FolderRole.SPAM, FolderRole.TRASH,
)

/** The icon of a mailbox. */
fun FolderRole.icon(): ImageVector = when (this) {
    FolderRole.INBOX -> CroftIcons.Inbox
    FolderRole.STARRED -> Icons.Filled.Star
    FolderRole.SNOOZED -> CroftIcons.Schedule
    FolderRole.FOLLOW_UP -> CroftIcons.Flag
    FolderRole.SENT -> Icons.AutoMirrored.Filled.Send
    FolderRole.DRAFTS -> CroftIcons.Drafts
    FolderRole.OUTBOX -> CroftIcons.Outbox
    FolderRole.ARCHIVE -> CroftIcons.Archive
    FolderRole.SPAM -> Icons.Filled.Warning
    FolderRole.TRASH -> Icons.Filled.Delete
    FolderRole.OTHER -> CroftIcons.Label
}

/** The name of a mailbox: the translated word for a known role, the server's own name for a custom folder. */
@Composable
fun FolderInfo.displayName(): String = when (role) {
    FolderRole.INBOX -> stringResource(R.string.mail_folder_inbox)
    FolderRole.STARRED -> stringResource(R.string.mail_folder_starred)
    FolderRole.SNOOZED -> stringResource(R.string.mail_folder_snoozed)
    FolderRole.FOLLOW_UP -> stringResource(R.string.mail_folder_follow_up)
    FolderRole.SENT -> stringResource(R.string.mail_folder_sent)
    FolderRole.DRAFTS -> stringResource(R.string.mail_folder_drafts)
    FolderRole.OUTBOX -> stringResource(R.string.mail_folder_outbox)
    FolderRole.ARCHIVE -> stringResource(R.string.mail_folder_archive)
    FolderRole.SPAM -> stringResource(R.string.mail_folder_spam)
    FolderRole.TRASH -> stringResource(R.string.mail_folder_trash)
    FolderRole.OTHER -> name
}

/**
 * S12: the content of the navigation drawer. Mailboxes with unread counts, labels, and Settings at the bottom. In the unified
 * Inbox the Inbox row is the unified one and the other mailboxes are those of the first account (T10 has no unified scope
 * but the Inbox); with one account shown they are that account's. Closing the drawer on a choice is the caller's job.
 */
@Composable
fun MailDrawerContent(
    folders: List<FolderInfo>,
    scope: MessageScope,
    unifiedUnread: Int,
    accountLabel: String?,
    onSelect: (FolderInfo) -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
    /** The Outbox (S26) entry; null hides it. */
    onOutbox: (() -> Unit)? = null,
    /** The long-press menu of a mailbox (docs/ux/controls.md): Mark all as read, Sync now, Mailbox settings. */
    onMarkAllRead: (FolderInfo) -> Unit = {},
    onSyncNow: (FolderInfo) -> Unit = {},
    onMailboxSettings: (FolderInfo) -> Unit = {},
) {
    Column(modifier.verticalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 12.dp).testTag(InboxTags.DRAWER)) {
        Text(
            stringResource(R.string.mail_drawer_title),
            Modifier.padding(horizontal = 16.dp, vertical = 12.dp).semantics { heading() },
            style = MaterialTheme.typography.titleLarge,
            color = MaterialTheme.colorScheme.primary,
        )
        if (accountLabel != null) Text(accountLabel, Modifier.padding(horizontal = 16.dp).padding(bottom = 8.dp), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        DrawerOrder.forEach { role ->
            val folder = folders.firstOrNull { it.role == role } ?: return@forEach
            val unified = role == FolderRole.INBOX && scope == MessageScope.UnifiedInbox
            DrawerRow(folder, selected = unified || scope == MessageScope.Folder(folder.account, folder.id), count = if (unified) unifiedUnread else folder.unreadCount, onClick = { onSelect(folder) }, onMarkAllRead = { onMarkAllRead(folder) }, onSyncNow = { onSyncNow(folder) }, onMailboxSettings = { onMailboxSettings(folder) })
        }
        val labels = folders.filter { it.role == FolderRole.OTHER }
        if (labels.isNotEmpty()) {
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text(
                stringResource(R.string.mail_drawer_labels),
                Modifier.padding(horizontal = 16.dp, vertical = 8.dp).semantics { heading() },
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            labels.forEach { f -> DrawerRow(f, selected = scope == MessageScope.Folder(f.account, f.id), count = 0, onClick = { onSelect(f) }, onMarkAllRead = { onMarkAllRead(f) }, onSyncNow = { onSyncNow(f) }, onMailboxSettings = { onMailboxSettings(f) }) }
        }
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        if (onOutbox != null) NavigationDrawerItem(
            label = { Text(stringResource(R.string.mail_folder_outbox)) },
            selected = false,
            onClick = onOutbox,
            icon = { Icon(CroftIcons.Outbox, contentDescription = null) },
            modifier = Modifier.testTag("drawer-outbox"),
        )
        NavigationDrawerItem(
            label = { Text(stringResource(R.string.nav_settings)) },
            selected = false,
            onClick = onSettings,
            icon = { Icon(Icons.Filled.Settings, contentDescription = null) },
            modifier = Modifier.testTag("drawer-settings"),
        )
    }
}

/**
 * One mailbox of the drawer. A tap opens it; a long-press opens its menu (Mark all as read, hidden when nothing is unread, Sync
 * now, Mailbox settings) with the long-press haptic and the 1 dp lift. The menu header is the mailbox name and its unread count.
 */
@Composable
private fun DrawerRow(folder: FolderInfo, selected: Boolean, count: Int, onClick: () -> Unit, onMarkAllRead: () -> Unit, onSyncNow: () -> Unit, onMailboxSettings: () -> Unit) {
    val name = folder.displayName()
    var menuOpen by remember { mutableStateOf(false) }
    val items = listOf(
        MenuItemSpec(stringResource(R.string.mail_menu_mark_all_read), onMarkAllRead, icon = CroftIcons.EnvelopeOpen, visible = folder.unreadCount > 0),
        MenuItemSpec(stringResource(R.string.mail_menu_sync_now), onSyncNow, icon = Icons.Filled.Refresh),
        MenuItemSpec(stringResource(R.string.mail_menu_mailbox_settings), onMailboxSettings, icon = Icons.Filled.Settings),
    )
    Box {
        NavigationDrawerItem(
            label = { Text(name) },
            selected = selected,
            onClick = onClick,
            icon = { Icon(folder.role.icon(), contentDescription = null) },
            badge = if (count > 0) ({ Text(count.toString(), style = MaterialTheme.typography.labelLarge) }) else null,
            modifier = Modifier.testTag(InboxTags.drawerItem(folder.id)).fillMaxWidth().longPressMenu { menuOpen = true },
        )
        CroftMenu(
            expanded = menuOpen,
            onDismiss = { menuOpen = false },
            items = items,
            modifier = Modifier.testTag(InboxTags.drawerMenu(folder.id)),
            header = {
                Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                    Text(name, style = MaterialTheme.typography.titleSmall)
                    if (folder.unreadCount > 0) Text(pluralStringResource(R.plurals.mail_menu_unread_count, folder.unreadCount, folder.unreadCount), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            },
        )
    }
}
