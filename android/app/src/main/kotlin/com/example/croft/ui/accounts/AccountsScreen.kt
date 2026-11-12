// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.MailSession
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.accounts.RemovalPlan
import com.example.croft.mail.accounts.SavedSignIn
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import kotlinx.coroutines.launch

/**
 * S44 Accounts: one row per account with its name, address and status chip as the core reports them (the sample mailbox says it is
 * sample mail), Add account (S02, and back here when it is done), Move up and Move down (the order is the app's: [AccountPrefs]), and
 * Remove, which first lists what will be deleted on this device and only then calls [removeAccount].
 *
 * [removeAccount] removes the account (the core's rows and its saved password) and returns how many accounts are left, or null when it
 * failed (a line says so and nothing else changes). When the last account is gone the app returns to S01 ([onNoAccounts]).
 */
@Composable
fun AccountsScreen(
    session: MailSession,
    prefs: AccountPrefs,
    onPrefsChange: (AccountPrefs) -> Unit,
    onAdd: () -> Unit,
    onOpenAccount: (AccountId) -> Unit,
    removeAccount: suspend (AccountId) -> Int?,
    savedSignIn: suspend (AccountId) -> SavedSignIn,
    onNoAccounts: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val failed = stringResource(R.string.accounts_remove_failed)
    var reload by remember { mutableStateOf(0) }
    val loaded by rememberAccountList(session, reload)
    val shown = loaded?.let { prefs.ordered(it) }
    var removing by remember { mutableStateOf<AccountInfo?>(null) }
    var plan by remember { mutableStateOf<RemovalPlan?>(null) }
    var busy by remember { mutableStateOf(false) }
    val removalName = removing?.shownName(prefs)

    SettingsFrame(
        title = stringResource(R.string.accounts_title), onBack = onBack, snackbarHostState = snackbar,
        modifier = modifier.testTag(AccountsTags.ACCOUNTS),
    ) {
        shown?.forEachIndexed { index, account ->
            AccountRow(
                account = account, prefs = prefs, canMoveUp = index > 0, canMoveDown = index < shown.lastIndex, enabled = !busy,
                onOpen = { onOpenAccount(account.id) },
                onUp = { onPrefsChange(prefs.moved(shown.map { it.id }, account.id, -1)) },
                onDown = { onPrefsChange(prefs.moved(shown.map { it.id }, account.id, 1)) },
                onRemove = {
                    scope.launch {
                        plan = removalPlanOf(session, account, savedSignIn)
                        removing = account
                    }
                },
            )
        }
        CroftButton(
            stringResource(R.string.accounts_add), onAdd, Modifier.fillMaxWidth().testTag(AccountsTags.ADD),
            kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.Add, enabled = !busy,
        )
    }

    val target = removing
    val targetPlan = plan
    if (target != null && targetPlan != null && removalName != null) {
        RemoveAccountDialog(
            name = removalName, plan = targetPlan,
            onDismiss = { removing = null; plan = null },
            onConfirm = {
                removing = null; plan = null; busy = true
                scope.launch {
                    val left = removeAccount(target.id)
                    busy = false
                    when {
                        left == null -> snackbar.showSnackbar(failed)
                        left == 0 -> onNoAccounts()
                        else -> reload++ // (also when the count could not be read: the list reads the core again)
                    }
                }
            },
        )
    }
}

@Composable
private fun AccountRow(
    account: AccountInfo,
    prefs: AccountPrefs,
    canMoveUp: Boolean,
    canMoveDown: Boolean,
    enabled: Boolean,
    onOpen: () -> Unit,
    onUp: () -> Unit,
    onDown: () -> Unit,
    onRemove: () -> Unit,
) {
    val name = account.shownName(prefs)
    val open = stringResource(R.string.accounts_row_open, name)
    Surface(
        modifier = Modifier.fillMaxWidth().testTag(AccountsTags.row(account.id)),
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column {
            ListItem(
                modifier = Modifier.clickable(onClickLabel = open, role = Role.Button, onClick = onOpen),
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                leadingContent = { AccountDot(colourOf(prefs, account.id)) },
                headlineContent = { Text(name) },
                supportingContent = {
                    Text(if (account.isDemo) stringResource(R.string.accounts_sample_hint) else account.email)
                },
            )
            Row(
                Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                AccountStatusLabel(name, account.status, onOpen, Modifier.testTag(AccountsTags.status(account.id)))
                androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
                IconButton(onClick = onUp, enabled = enabled && canMoveUp, modifier = Modifier.testTag(AccountsTags.up(account.id))) {
                    Icon(Icons.Filled.KeyboardArrowUp, stringResource(R.string.accounts_move_up, name))
                }
                IconButton(onClick = onDown, enabled = enabled && canMoveDown, modifier = Modifier.testTag(AccountsTags.down(account.id))) {
                    Icon(Icons.Filled.KeyboardArrowDown, stringResource(R.string.accounts_move_down, name))
                }
                IconButton(onClick = onRemove, enabled = enabled, modifier = Modifier.testTag(AccountsTags.remove(account.id))) {
                    Icon(Icons.Filled.Delete, stringResource(R.string.accounts_remove_row, name))
                }
            }
        }
    }
}
