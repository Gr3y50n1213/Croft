// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.AccountId
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MailSession
import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.RemovalPlan
import com.example.croft.mail.accounts.SavedSignIn
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.mail.DeclineFolderButton
import com.example.croft.ui.mail.MoveToSheet
import kotlinx.coroutines.launch

/** The folders S45 maps, in the order it lists them: the role, the label and the key of its tag. */
private val MappedRoles = listOf(
    Triple(FolderRole.TRASH, R.string.account_folder_trash, "trash"),
    Triple(FolderRole.ARCHIVE, R.string.account_folder_archive, "archive"),
    Triple(FolderRole.SPAM, R.string.account_folder_junk, "junk"),
    Triple(FolderRole.SENT, R.string.account_folder_sent, "sent"),
    Triple(FolderRole.DRAFTS, R.string.account_folder_drafts, "drafts"),
)

/**
 * S45 Account: the name (shown in Croft only), the account's status, its colour, the folder mapping for Trash, Archive, Junk, Sent and
 * Drafts ([com.example.croft.mail.FolderMapping]: a folder the person chose where the server has none of its own), Remove (the same
 * confirmation as S44) and "Erase all Croft data on this device" (the same as S60).
 *
 * **Left out on purpose** (no disabled placeholders): identities and signatures and the sync window (the core has no storage for them
 * yet: compose, T18) and server settings (the core cannot read an account's server settings back, so S05 could not be pre-filled). They
 * are listed in docs/OPEN-ITEMS.md.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AccountScreen(
    session: MailSession,
    accountId: AccountId,
    prefs: AccountPrefs,
    onPrefsChange: (AccountPrefs) -> Unit,
    removeAccount: suspend (AccountId) -> Int?,
    savedSignIn: suspend (AccountId) -> SavedSignIn,
    onRemoved: (left: Int) -> Unit,
    eraseAll: suspend () -> Boolean,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val removeFailed = stringResource(R.string.accounts_remove_failed)
    val eraseFailed = stringResource(R.string.about_erase_failed)
    val accounts by rememberAccountList(session)
    val account = accounts?.firstOrNull { it.id == accountId }
    var folders by remember { mutableStateOf<List<FolderInfo>>(emptyList()) }
    // The folder chosen for each role (kept in the screen's state so the rows follow it; `FolderMapping` is what the app remembers).
    var chosenIds by remember(session, accountId) { mutableStateOf(MappedRoles.associate { it.first to session.folderMapping.mapped(accountId, it.first) }) }
    LaunchedEffect(session, accountId, account?.status) {
        folders = try { session.repository.listFolders(accountId).filter { !it.isVirtual } } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (e: Exception) { folders }
    }
    // A removed account (here or elsewhere) leaves nothing to show: back to the list.
    if (accounts != null && account == null) { LaunchedEffect(Unit) { onBack() }; return }
    if (account == null) return

    val latestPrefs by rememberUpdatedState(prefs)
    val latestChange by rememberUpdatedState(onPrefsChange)
    var name by rememberSaveable(accountId) { mutableStateOf(if (account.isDemo) "" else prefs.nameOf(account)) }
    // Set once the account is removed: nothing may be written under its id after that.
    val gone = remember { java.util.concurrent.atomic.AtomicBoolean(false) }
    val commitName = {
        if (!account.isDemo && !gone.get() && name.trim() != latestPrefs.nameOf(account)) latestChange(latestPrefs.withName(accountId, name))
    }
    val latestCommit by rememberUpdatedState(commitName)
    DisposableEffect(accountId) { onDispose { latestCommit() } }

    var removing by remember { mutableStateOf<RemovalPlan?>(null) }
    var erasing by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val shownName = account.shownName(prefs)

    SettingsFrame(
        title = shownName, onBack = { commitName(); onBack() }, snackbarHostState = snackbar,
        modifier = modifier.testTag(AccountsTags.ACCOUNT),
    ) {
        Text(
            if (account.isDemo) stringResource(R.string.accounts_sample_hint) else account.email,
            style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // Tapping a status that is not OK tries a sync now (there is no other fix to offer here).
        AccountStatusLabel(shownName, account.status, onClick = { scope.launch { try { session.repository.syncNow(accountId) } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (e: Exception) { /* the status chip shows the result */ } } })
        if (!account.isDemo) {
            CroftTextField(
                value = name, onValueChange = { name = it.take(AccountPrefs.MAX_NAME) }, label = stringResource(R.string.account_name),
                supportingText = stringResource(R.string.account_name_hint),
                imeAction = ImeAction.Done, onImeAction = { commitName() },
                onFocusChange = { focused -> if (!focused) commitName() },
                modifier = Modifier.testTag(AccountsTags.NAME),
            )
        }

        SectionLabel(stringResource(R.string.account_colour))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            val current = colourOf(prefs, accountId)
            AccountColour.entries.forEach { colour ->
                ColourChoice(colour, selected = colour == current) { onPrefsChange(prefs.withColour(accountId, colour)) }
            }
        }

        SectionLabel(stringResource(R.string.account_folders))
        Text(stringResource(R.string.account_folders_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        var choosing by remember { mutableStateOf<FolderRole?>(null) }
        MappedRoles.forEach { (role, label, key) ->
            FolderRow(role, stringResource(label), folders, chosenIds[role], Modifier.testTag(AccountsTags.folder(key))) { choosing = role }
        }
        choosing?.let { role ->
            val label = stringResource(MappedRoles.first { it.first == role }.second)
            MoveToSheet(
                title = stringResource(R.string.account_folder_choose, label), folders = folders, recent = emptyList(), excluding = emptySet(),
                onPick = { f -> session.folderMapping.remember(accountId, role, f.id); chosenIds = chosenIds + (role to f.id); choosing = null },
                onDismiss = { choosing = null },
                footer = if (chosenIds[role] != null) ({
                    DeclineFolderButton(stringResource(R.string.account_folder_clear), { session.folderMapping.forget(accountId, role); chosenIds = chosenIds + (role to null); choosing = null })
                }) else null,
            )
        }

        CroftButton(
            stringResource(R.string.account_remove), onClick = {
                scope.launch { removing = removalPlanOf(session, account, savedSignIn) }
            },
            Modifier.fillMaxWidth().testTag(AccountsTags.REMOVE), kind = ButtonKind.OUTLINED, enabled = !busy,
        )
        Text(stringResource(R.string.account_remove_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        CroftButton(
            stringResource(R.string.about_erase), onClick = { erasing = true },
            Modifier.fillMaxWidth().testTag(AccountsTags.ERASE), kind = ButtonKind.TEXT, enabled = !busy,
        )
    }

    removing?.let { plan ->
        RemoveAccountDialog(
            name = shownName, plan = plan, onDismiss = { removing = null },
            onConfirm = {
                removing = null; busy = true
                // `gone` is set before the removal starts: the screen may be left while it runs, and leaving must not write the name back.
                gone.set(true)
                scope.launch {
                    val left = removeAccount(accountId)
                    busy = false
                    if (left == null) { gone.set(false); snackbar.showSnackbar(removeFailed) } else onRemoved(left)
                }
            },
        )
    }
    if (erasing) {
        EraseAllDialog(
            onDismiss = { erasing = false },
            onConfirm = {
                erasing = false; busy = true
                gone.set(true)
                scope.launch { val ok = eraseAll(); busy = false; if (!ok) { gone.set(false); snackbar.showSnackbar(eraseFailed) } }
            },
        )
    }
}

@Composable
private fun ColourChoice(colour: AccountColour, selected: Boolean, onClick: () -> Unit) {
    val label = stringResource(
        when (colour) {
            AccountColour.PRIMARY -> R.string.account_colour_primary
            AccountColour.SECONDARY -> R.string.account_colour_secondary
            AccountColour.TERTIARY -> R.string.account_colour_tertiary
            AccountColour.AMBER -> R.string.account_colour_amber
            AccountColour.ERROR -> R.string.account_colour_error
            AccountColour.NEUTRAL -> R.string.account_colour_neutral
        },
    )
    val described = if (selected) stringResource(R.string.account_colour_selected, label) else label
    Box(
        Modifier
            .size(48.dp)
            .testTag(AccountsTags.colour(colour))
            .semantics { contentDescription = described; role = Role.RadioButton; this.selected = selected }
            .clip(CircleShape)
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(36.dp).clip(CircleShape).background(colour.toColor()), contentAlignment = Alignment.Center) {
            // The tick, not the colour alone, says which is chosen.
            if (selected) Icon(Icons.Filled.Check, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.surface)
        }
    }
}

/**
 * One special folder: what Croft uses for [role]. A folder the server marks for that use is used and shown (nothing to choose); with
 * none, the person's choice is shown, or "Not set", and a tap opens the folder chooser.
 */
@Composable
private fun FolderRow(
    role: FolderRole,
    label: String,
    folders: List<FolderInfo>,
    chosenId: Long?,
    modifier: Modifier,
    onChoose: () -> Unit,
) {
    val server = folders.firstOrNull { it.role == role }
    val chosen = chosenId?.let { id -> folders.firstOrNull { it.id == id } }
    val clickable = server == null
    val supporting = when {
        server != null -> stringResource(R.string.account_folder_from_server, server.path)
        chosen != null -> chosen.path
        else -> stringResource(R.string.account_folder_not_set)
    }
    Surface(modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerLow) {
        ListItem(
            modifier = if (clickable) Modifier.clickable(role = Role.Button, onClick = onChoose) else Modifier,
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            headlineContent = { Text(label) },
            supportingContent = { Text(supporting) },
        )
    }
}
