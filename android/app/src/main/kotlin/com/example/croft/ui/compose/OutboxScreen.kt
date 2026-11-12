// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MailSession
import com.example.croft.mail.compose.ComposeProblem
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.OutboxController
import com.example.croft.mail.compose.OutboxPrompt
import com.example.croft.mail.compose.OutboxRow
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.CroftSpinner
import kotlinx.coroutines.launch
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.OutboxReason

/** Test tags of S26. */
object OutboxTags {
    const val SCREEN = "outbox-screen"
    const val LIST = "outbox-list"
    fun row(account: Long, id: Long) = "outbox-row-$account-$id"
    const val RETRY = "outbox-retry"
    const val EDIT = "outbox-edit"
    const val DISCARD = "outbox-discard"
}

/** The Outbox over the session: its lists come from the core and refresh from its change notices. [onEdit] opens compose on the draft Edit returned. */
@Composable
fun OutboxHost(session: MailSession, onBack: () -> Unit, onEdit: (ComposeStart.Draft) -> Unit, modifier: Modifier = Modifier) {
    val backend = session.compose
    if (backend == null) {
        LaunchedEffect(Unit) { onBack() }
        return
    }
    val scope = rememberCoroutineScope()
    val controller = remember(session) {
        OutboxController(backend, { session.repository.listAccounts().map { it.id } }, { session.repository.nextChange(it) }, scope)
    }
    LaunchedEffect(controller) { controller.run() }
    OutboxScreen(controller, onBack, onEdit, modifier)
}

/** S26: Queued, Sending and Failed messages with their reasons, and Retry, Edit and Discard. */
@Composable
fun OutboxScreen(controller: OutboxController, onBack: () -> Unit, onEdit: (ComposeStart.Draft) -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val scope = rememberCoroutineScope()
    val snackbar = remember { CroftSnackbarState() }
    BackHandler { onBack() }
    state.problem?.let { problem ->
        val text = stringResource(if (problem == ComposeProblem.WRONG_STATE) R.string.outbox_problem_wrong_state else R.string.outbox_problem_generic)
        LaunchedEffect(problem) { snackbar.show(text); controller.problemShown() }
    }
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(OutboxTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.outbox_back), onBack)
                Text(stringResource(R.string.outbox_title), Modifier.padding(start = 8.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge)
            }
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                when {
                    !state.loaded -> Text(stringResource(R.string.outbox_loading), Modifier.padding(24.dp), style = MaterialTheme.typography.bodyMedium)
                    state.rows.isEmpty() -> Column(Modifier.padding(24.dp).widthIn(max = 480.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.outbox_empty_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        Text(stringResource(R.string.outbox_empty_message), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    else -> LazyColumn(Modifier.widthIn(max = 640.dp).fillMaxWidth().padding(horizontal = 12.dp).testTag(OutboxTags.LIST)) {
                        items(state.rows, key = { "${it.account}-${it.id}" }) { row ->
                            OutboxRowView(
                                row = row, reason = row.item.reason?.let { reasonText(it) },
                                onRetry = { scope.launch { controller.retry(row) } },
                                onEdit = { scope.launch { controller.edit(row)?.let(onEdit) } },
                                onDiscard = { controller.discard(row) },
                            )
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                        }
                    }
                }
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars))
    }
    when (val prompt = state.prompt) {
        is OutboxPrompt.ConfirmRetry -> AlertDialog(
            onDismissRequest = { controller.cancelPrompt() },
            title = { Text(stringResource(R.string.outbox_maybe_title)) },
            text = { Text(stringResource(R.string.outbox_maybe_message)) },
            dismissButton = { TextButton({ controller.cancelPrompt() }) { Text(stringResource(R.string.control_cancel)) } },
            confirmButton = { TextButton({ scope.launch { controller.confirmRetry() } }, Modifier.testTag("outbox-retry-anyway")) { Text(stringResource(R.string.outbox_maybe_confirm)) } },
        )
        is OutboxPrompt.ConfirmDiscard -> AlertDialog(
            onDismissRequest = { controller.cancelPrompt() },
            title = { Text(stringResource(R.string.outbox_discard_title)) },
            text = { Text(stringResource(R.string.outbox_discard_message)) },
            dismissButton = { TextButton({ controller.cancelPrompt() }) { Text(stringResource(R.string.control_cancel)) } },
            confirmButton = {
                TextButton({ scope.launch { controller.confirmDiscard() } }, Modifier.testTag("outbox-discard-confirm")) {
                    Text(stringResource(R.string.outbox_discard), color = MaterialTheme.colorScheme.error)
                }
            },
        )
        null -> Unit
    }
}

@Composable
private fun OutboxRowView(row: OutboxRow, reason: String?, onRetry: () -> Unit, onEdit: () -> Unit, onDiscard: () -> Unit) {
    val item = row.item
    val failed = item.state == OutboxItemState.FAILED
    val stateWord = stringResource(
        when (item.state) {
            OutboxItemState.QUEUED -> R.string.outbox_state_queued
            OutboxItemState.SENDING -> R.string.outbox_state_sending
            OutboxItemState.FAILED -> R.string.outbox_state_failed
        },
    )
    val detail = reason ?: stringResource(if (item.state == OutboxItemState.SENDING) R.string.outbox_sending_now else R.string.outbox_waiting)
    val subject = item.subject.ifBlank { stringResource(R.string.outbox_no_subject) }
    val recipients = pluralStringResource(R.plurals.outbox_recipients, item.recipientCount.toInt(), item.recipientCount.toInt())
    val colour = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    val spoken = stringResource(R.string.outbox_row_description, subject, "$stateWord. $detail", recipients)
    Column(Modifier.fillMaxWidth().padding(vertical = 12.dp).testTag(OutboxTags.row(row.account, row.id))) {
        Row(
            Modifier.fillMaxWidth().semantics(mergeDescendants = true) {
                contentDescription = spoken
                stateDescription = stateWord
                if (failed) liveRegion = LiveRegionMode.Polite
            },
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) {
                when (item.state) {
                    OutboxItemState.SENDING -> CroftSpinner(Modifier.size(24.dp))
                    OutboxItemState.FAILED -> Icon(Icons.Filled.Warning, null, Modifier.size(24.dp), tint = colour)
                    OutboxItemState.QUEUED -> Icon(CroftIcons.Schedule, null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Column(Modifier.weight(1f)) {
                Text(subject, style = MaterialTheme.typography.titleMedium, maxLines = 2)
                Text(recipients, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stateWord, style = MaterialTheme.typography.labelLarge, color = if (failed) colour else MaterialTheme.colorScheme.primary)
                }
                Text(detail, style = MaterialTheme.typography.bodyMedium, color = if (failed) colour else MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        if (item.state != OutboxItemState.SENDING) {
            Row(Modifier.fillMaxWidth().padding(start = 40.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                if (failed) TextButton(onRetry, Modifier.heightIn(min = 48.dp).testTag(OutboxTags.RETRY)) { Text(stringResource(R.string.outbox_retry)) }
                TextButton(onEdit, Modifier.heightIn(min = 48.dp).testTag(OutboxTags.EDIT)) { Text(stringResource(R.string.outbox_edit)) }
                TextButton(onDiscard, Modifier.heightIn(min = 48.dp).testTag(OutboxTags.DISCARD)) { Text(stringResource(R.string.outbox_discard), color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}

/**
 * The text of an [OutboxReason]: the core's translation key (`outbox_reason_key`) mapped to a string of the app. A key without a string
 * (a newer core) falls back to the core's own English text. The server's words are never shown: the core never passes them.
 */
@Composable
fun reasonText(reason: OutboxReason): String {
    val id = reasonStringId(uniffi.croft_ffi.outboxReasonKey(reason))
    return if (id != null) stringResource(id) else uniffi.croft_ffi.outboxReasonDefaultMessage(reason)
}

/** The string resource of a reason key, or null when the app has none. Listed one by one so that resource shrinking keeps them all. */
internal fun reasonStringId(key: String): Int? = when (key) {
    "outbox.reason.may_not_have_been_sent" -> R.string.outbox_reason_may_not_have_been_sent
    "outbox.reason.sender_refused" -> R.string.outbox_reason_sender_refused
    "outbox.reason.recipient_refused" -> R.string.outbox_reason_recipient_refused
    "outbox.reason.message_refused" -> R.string.outbox_reason_message_refused
    "outbox.reason.too_large" -> R.string.outbox_reason_too_large
    "outbox.reason.server_limits" -> R.string.outbox_reason_server_limits
    "outbox.reason.trying_again" -> R.string.outbox_reason_trying_again
    "outbox.reason.gave_up" -> R.string.outbox_reason_gave_up
    "outbox.reason.needs_sign_in" -> R.string.outbox_reason_needs_sign_in
    "outbox.reason.internal" -> R.string.outbox_reason_internal
    "outbox.reason.other" -> R.string.outbox_reason_other
    else -> null
}
