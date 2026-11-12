// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
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
import androidx.compose.runtime.State
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import com.example.croft.mail.MailSession
import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.RemovalPlan
import com.example.croft.mail.accounts.SavedSignIn
import com.example.croft.theme.statusColors
import com.example.croft.ui.components.AccountStatus
import com.example.croft.ui.components.AccountStatusChip
import com.example.croft.ui.components.DestructiveDialog
import kotlinx.coroutines.delay

/** Test tags of S44, S45 and S60. */
object AccountsTags {
    const val SIGN_IN_AGAIN = "S45-sign-in-again"
    const val ACCOUNTS = "S44"
    const val ADD = "S44-add"
    fun row(id: AccountId) = "S44-row-$id"
    fun up(id: AccountId) = "S44-up-$id"
    fun down(id: AccountId) = "S44-down-$id"
    fun remove(id: AccountId) = "S44-remove-$id"
    fun status(id: AccountId) = "S44-status-$id"
    const val REMOVE_DIALOG = "account-remove-dialog"

    const val ACCOUNT = "S45"
    const val NAME = "S45-name"
    fun colour(c: AccountColour) = "S45-colour-${c.name.lowercase()}"
    fun folder(role: String) = "S45-folder-$role"
    const val REMOVE = "S45-remove"
    const val ERASE = "S45-erase"

    const val ABOUT = "S60"
    const val VERSION = "S60-version"
    const val PRIVACY = "S60-privacy"
    const val LICENCES = "S60-licences"
    const val SOURCE = "S60-source"
    const val COPY = "S60-copy-diagnostics"
    const val ERASE_ALL = "S60-erase"
    const val NOT_AFFILIATED = "S60-not-affiliated"
    const val ERASE_DIALOG = "erase-dialog"
    const val LICENCES_SCREEN = "S60-licences-screen"
    const val LICENCE_LIST = "S60-licence-list"
}

private const val RETRY_AFTER_ERROR_MS = 1_000L

/**
 * The accounts of [session] as the core reports them, kept current: read once, then again after every change notice (a status turning
 * from Syncing to OK, an account added or removed). [reload] restarts the loop (after the person changed something). Null until the first read.
 */
@Composable
internal fun rememberAccountList(session: MailSession, reload: Int = 0): State<List<AccountInfo>?> =
    produceState<List<AccountInfo>?>(null, session, reload) {
        var after = 0L
        while (true) {
            try {
                value = session.repository.listAccounts()
                after = session.repository.nextChange(after).seq
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                delay(RETRY_AFTER_ERROR_MS)
            }
        }
    }

/** The colour an account shows: the one the person chose, else one by its id (so each account differs without a choice). */
internal fun colourOf(prefs: AccountPrefs, id: AccountId): AccountColour =
    prefs.colours[id] ?: AccountColour.entries[((id - 1).mod(AccountColour.entries.size.toLong())).toInt()]

/** An [AccountColour] as a colour of the theme (never a typed colour). */
@Composable
internal fun AccountColour.toColor(): Color = when (this) {
    AccountColour.PRIMARY -> MaterialTheme.colorScheme.primary
    AccountColour.SECONDARY -> MaterialTheme.colorScheme.secondary
    AccountColour.TERTIARY -> MaterialTheme.colorScheme.tertiary
    AccountColour.AMBER -> statusColors().amber
    AccountColour.ERROR -> MaterialTheme.colorScheme.error
    AccountColour.NEUTRAL -> MaterialTheme.colorScheme.outline
}

/** The small coloured dot before an account's name. It is decoration: the name and the status carry the meaning. */
@Composable
internal fun AccountDot(colour: AccountColour, modifier: Modifier = Modifier) {
    androidx.compose.foundation.layout.Box(modifier.size(14.dp).clip(CircleShape).background(colour.toColor()))
}

/** The status chip of S44 and S45: "OK" when all is well (the Inbox's chip says nothing then), else the shared chip with its words and icon. */
@Composable
internal fun AccountStatusLabel(name: String, state: AccountState, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val chip = when (state) {
        AccountState.OK -> AccountStatus.CONNECTED
        AccountState.SYNCING -> AccountStatus.SYNCING
        AccountState.OFFLINE -> AccountStatus.OFFLINE
        AccountState.NEEDS_SIGN_IN -> AccountStatus.NEEDS_SIGN_IN
        AccountState.ERROR -> AccountStatus.SERVER_PROBLEM
    }
    if (chip != AccountStatus.CONNECTED) {
        AccountStatusChip(name, chip, onClick, modifier)
        return
    }
    val description = stringResource(R.string.accounts_status_ok_description, name)
    Surface(
        modifier = modifier.semantics { contentDescription = description },
        shape = MaterialTheme.shapes.small,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Row(Modifier.padding(horizontal = 10.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Check, null, Modifier.size(16.dp))
            Text(stringResource(R.string.accounts_status_ok), style = MaterialTheme.typography.labelLarge, maxLines = 1)
        }
    }
}

/** The plan the removal dialog lists for [account]: the counts from its folders and the sign-in the device has saved for it. A read that fails lists what it knows. */
internal suspend fun removalPlanOf(session: MailSession, account: AccountInfo, savedSignIn: suspend (AccountId) -> SavedSignIn): RemovalPlan {
    val folders = try { session.repository.listFolders(account.id) } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (e: Exception) { emptyList() }
    val saved = try { savedSignIn(account.id) } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (e: Exception) { if (account.isDemo) SavedSignIn.NONE else SavedSignIn.PASSWORD }
    return RemovalPlan.of(account, folders, saved)
}

/** What an account is called on screen: the sample mailbox says so; any other account has the person's name or the core's label. */
@Composable
internal fun AccountInfo.shownName(prefs: AccountPrefs): String = if (isDemo) stringResource(R.string.accounts_sample_name) else prefs.nameOf(this)

/**
 * "Remove [name]?" with the list of what is deleted on this device, as T17 asks: the messages and folders (with counts), the queued
 * changes, the drafts and outbox, and the saved password (not for the sample mailbox, which has none). The server is not touched.
 */
@Composable
internal fun RemoveAccountDialog(name: String, plan: RemovalPlan, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    DestructiveDialog(
        title = stringResource(R.string.accounts_remove_title, name),
        message = stringResource(if (plan.sample) R.string.accounts_remove_sample_intro else R.string.accounts_remove_intro),
        confirmLabel = stringResource(R.string.accounts_remove_confirm),
        onConfirm = onConfirm,
        onDismiss = onDismiss,
        modifier = Modifier.testTag(AccountsTags.REMOVE_DIALOG),
        body = {
            val stored = stringResource(
                R.string.accounts_remove_stored,
                pluralStringResource(R.plurals.accounts_remove_messages, plan.messages, plan.messages),
                pluralStringResource(R.plurals.accounts_remove_folders, plan.folders, plan.folders),
            )
            val lines = buildList {
                add(stored)
                add(stringResource(R.string.accounts_remove_queue))
                add(stringResource(R.string.accounts_remove_drafts))
                when (plan.savedSignIn) {
                    SavedSignIn.PASSWORD -> add(stringResource(R.string.accounts_remove_password))
                    SavedSignIn.TOKEN -> add(stringResource(R.string.accounts_remove_token))
                    SavedSignIn.NONE -> Unit
                }
            }
            lines.forEach { Text("• $it", style = MaterialTheme.typography.bodyMedium) }
        },
    )
}

/** "Erase all Croft data on this device?" with the list of what goes (S60 and S45 share it). */
@Composable
internal fun EraseAllDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    DestructiveDialog(
        title = stringResource(R.string.about_erase_title),
        message = stringResource(R.string.about_erase_intro),
        confirmLabel = stringResource(R.string.about_erase_confirm),
        onConfirm = onConfirm,
        onDismiss = onDismiss,
        modifier = Modifier.testTag(AccountsTags.ERASE_DIALOG),
        body = {
            listOf(R.string.about_erase_accounts, R.string.about_erase_passwords, R.string.about_erase_settings).forEach {
                Text("• " + stringResource(it), style = MaterialTheme.typography.bodyMedium)
            }
        },
    )
}
