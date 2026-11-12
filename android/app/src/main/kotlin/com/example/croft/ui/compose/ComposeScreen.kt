// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.Context
import android.content.ContextWrapper
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.AccountId
import com.example.croft.mail.MailEvent
import com.example.croft.mail.MailSession
import com.example.croft.mail.compose.AddressTools
import com.example.croft.mail.compose.AttachmentError
import com.example.croft.mail.compose.AttachmentIntake
import com.example.croft.mail.compose.BodyCursor
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.CoreAddressTools
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.ComposeProblem
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.ComposeState
import com.example.croft.mail.compose.RecipientField
import com.example.croft.mail.compose.SaveStatus
import com.example.croft.mail.compose.SendPrompt
import com.example.croft.mail.compose.SendState
import com.example.croft.mail.AccountState
import com.example.croft.ui.components.AttachmentChip
import com.example.croft.ui.components.AttachmentState
import com.example.croft.ui.components.BannerHost
import com.example.croft.ui.components.BannerKind
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.components.SendActions
import com.example.croft.ui.components.SendButton
import com.example.croft.ui.components.SendCondition
import kotlinx.coroutines.launch

/** Test tags of S23. */
object ComposeTags {
    const val SCREEN = "compose-screen"
    const val SEND = "compose-send"
    const val SUBJECT = "compose-subject"
    const val BODY = "compose-body"
    const val FROM = "compose-from"
    const val SAVE_STATUS = "compose-save-status"
    const val ATTACH = "compose-attach"
    const val PHOTO = "compose-photo"
    fun attachment(index: Int) = "compose-attachment-$index"
    fun pending(id: Long) = "compose-pending-$id"
}

/**
 * S23 Compose, opened as [start] says: the entry point for the Inbox's FAB, the `mailto:` and share intents, and the reading screens'
 * Reply, Reply all and Forward (`ComposeStart.Reply(account, messageId)`, `ReplyAll`, `Forward`). It builds the [ComposeController] over
 * the session's compose backend and calls [onClosed] when compose is over (Back, Send, Discard); the screen that opened it shows the
 * snackbar ([ComposeClosed.DraftSaved] "Draft saved", [ComposeClosed.Sent] the Undo snackbar).
 */
@Composable
fun ComposeHost(
    session: MailSession,
    start: ComposeStart,
    onClosed: (ComposeClosed) -> Unit,
    modifier: Modifier = Modifier,
    /** Reading and checking addresses: the core's own functions; tests that must not load the native library give a stand-in. */
    tools: AddressTools = CoreAddressTools,
) {
    val backend = session.compose
    if (backend == null) {
        LaunchedEffect(Unit) { onClosed(ComposeClosed.Nothing) }
        return
    }
    val context = LocalContext.current.applicationContext
    // The controller lives in the activity's retained holder: a rebuilt window (rotation) finds the same text, chips and files.
    val holder = rememberComposeRetained()
    if (holder.hasEnded(start)) return
    if (holder.start != start) holder.begin(start)
    val controller = holder.controller ?: ComposeController(
        backend = backend,
        scope = holder.scope,
        accounts = { session.repository.listAccounts() },
        intake = AttachmentIntake(backend, ComposeFiles.directory(context)),
        tools = tools,
        policy = session.encryptionPolicy,
        cursors = session.cursors,
        files = { ComposeFiles.resolve(context, it) },
    ).also { holder.controller = it }
    LaunchedEffect(controller) {
        if (!holder.opened) { holder.opened = true; controller.open(start) }
    }
    // The Offline and Sign in banners follow the core's own notices.
    LaunchedEffect(controller) {
        var after = 0L
        while (true) {
            val batch = session.repository.nextChange(after)
            after = batch.seq
            controller.refreshState()
        }
    }
    // A new screen for a new controller: nothing of the old one's fields is kept.
    androidx.compose.runtime.key(controller) {
        ComposeScreen(
            controller,
            // Only the controller that is current may end compose: one that was replaced by a link or a share is ignored.
            onClosed = { closed -> if (holder.controller === controller) { holder.release(); onClosed(closed) } },
            modifier = modifier,
            onSignIn = { session.raise(MailEvent.SignInRequested(it)) },
            savedNotice = holder.savedNotice,
            onSavedNoticeShown = { holder.savedNotice = false },
        )
    }
}

private tailrec fun Context.lifecycleOwner(): LifecycleOwner? = when (this) {
    is LifecycleOwner -> this
    is ContextWrapper -> baseContext.lifecycleOwner()
    else -> null
}

/** S23 over a [controller] that is (being) opened. See [ComposeHost]. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ComposeScreen(
    controller: ComposeController,
    onClosed: (ComposeClosed) -> Unit,
    modifier: Modifier = Modifier,
    onSignIn: (AccountId) -> Unit = {},
    /** A link or a share replaced a compose that had content: its draft was saved, and this screen says so. */
    savedNotice: Boolean = false,
    onSavedNoticeShown: () -> Unit = {},
) {
    val state by controller.state.collectAsState()
    val scope = rememberCoroutineScope()
    val snackbar = remember { CroftSnackbarState() }
    val haptics = LocalHaptics.current
    val res = LocalContext.current.resources
    val context = LocalContext.current

    // Back with content autosaves ("Draft saved" is shown by whoever opened compose); an empty compose just closes.
    BackHandler(enabled = state.phase != ComposeState.Phase.CLOSED) { scope.launch { controller.back() } }
    LaunchedEffect(state.closed) { state.closed?.let(onClosed) }

    // The app pauses: save on the device and copy to the server now.
    DisposableEffect(controller, context) {
        val owner = context.lifecycleOwner()
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_PAUSE) controller.flushAsync() }
        owner?.lifecycle?.addObserver(observer)
        // Not stopped here: the controller outlives a rebuilt window (rotation); its timers go on and the draft is saved.
        onDispose { owner?.lifecycle?.removeObserver(observer) }
    }

    // The pickers: the system document picker and the photo picker. No storage or media permission is asked for.
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        controller.addFiles(uris.mapNotNull { ComposeFiles.resolve(context, it) })
    }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        controller.addFiles(uris.mapNotNull { ComposeFiles.resolve(context, it) })
    }
    val attachFile: () -> Unit = { documents.launch(arrayOf("*/*")) }
    val attachPhoto: () -> Unit = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) }

    // What the snackbar says about a refused file or send.
    val limitMb = state.sizeLimitMb
    state.attachmentError?.let { error ->
        val text = when (error) {
            AttachmentError.TOO_LARGE -> stringResource(R.string.attachment_error_too_large, MAX_ATTACHMENT_MB)
            AttachmentError.TOO_MANY -> stringResource(R.string.attachment_error_too_many)
            AttachmentError.TOTAL_TOO_LARGE -> stringResource(R.string.attachment_error_total)
            AttachmentError.UNREADABLE -> stringResource(R.string.attachment_error_unreadable)
            AttachmentError.TIMED_OUT -> stringResource(R.string.attachment_error_timed_out)
            AttachmentError.FAILED -> stringResource(R.string.attachment_error_failed)
        }
        LaunchedEffect(error) { haptics.perform(HapticEvent.Rejected); snackbar.show(text); controller.attachmentErrorShown() }
    }
    if (state.truncatedNotice) {
        val text = stringResource(R.string.compose_truncated)
        LaunchedEffect(Unit) { snackbar.show(text); controller.truncatedNoticeShown() }
    }
    if (savedNotice) {
        val text = stringResource(R.string.compose_snack_draft_saved)
        LaunchedEffect(Unit) { snackbar.show(text); onSavedNoticeShown() }
    }
    // Removing an attachment can be taken back for a few seconds: the core deletes it for good, so the removal is held until then.
    state.removedAttachment?.let { removed ->
        val text = stringResource(R.string.attachment_removed)
        val undo = stringResource(R.string.mail_undo)
        LaunchedEffect(removed.id) { snackbar.show(text, undo, stayMs = REMOVE_UNDO_MS) { scope.launch { controller.undoRemoveAttachment() } } }
    }
    state.sendProblem?.let { problem ->
        val text = stringResource(
            when (problem) {
                ComposeProblem.CANNOT_SEND -> R.string.compose_error_cannot_send
                ComposeProblem.IN_OUTBOX -> R.string.compose_error_in_outbox
                else -> R.string.compose_error_generic
            },
        )
        LaunchedEffect(problem) { haptics.perform(HapticEvent.Rejected); snackbar.show(text) }
    }
    val prompt = state.prompt
    if (prompt is SendPrompt.Explain) {
        val reason = when (prompt.state) {
            SendState.NO_RECIPIENT -> stringResource(R.string.send_reason_no_recipient)
            SendState.INVALID_RECIPIENT -> stringResource(R.string.send_reason_invalid_recipient)
            else -> stringResource(R.string.send_reason_over_limit, limitMb)
        }
        LaunchedEffect(prompt) { snackbar.show(reason); controller.cancelPrompt() }
    }

    Box(
        modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .testTag(ComposeTags.SCREEN)
            .onPreviewKeyEvent { e ->
                // Ctrl+Enter sends (docs/ux/accessibility.md).
                if (e.type == KeyEventType.KeyDown && e.isCtrlPressed && (e.key == Key.Enter || e.key == Key.NumPadEnter)) { scope.launch { controller.requestSend() }; true } else false
            },
    ) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
            when (state.phase) {
                ComposeState.Phase.LOADING -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.semantics { contentDescription = res.getString(R.string.compose_title) })
                }
                ComposeState.Phase.FAILED_TO_OPEN, ComposeState.Phase.LOCKED -> {
                    TopBar(state, controller, scope, snackbar, onAttachFile = {}, onAttachPhoto = {}, sendVisible = false)
                    Text(
                        stringResource(if (state.phase == ComposeState.Phase.LOCKED) R.string.compose_locked else R.string.compose_failed_to_open),
                        Modifier.padding(24.dp).semantics { liveRegion = LiveRegionMode.Assertive },
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
                else -> {
                    TopBar(state, controller, scope, snackbar, attachFile, attachPhoto, sendVisible = true)
                    val banner = when (state.accountStatus) {
                        AccountState.NEEDS_SIGN_IN -> BannerKind.NEEDS_SIGN_IN
                        AccountState.OFFLINE -> BannerKind.OFFLINE
                        else -> null
                    }
                    BannerHost(
                        active = listOfNotNull(banner), onAction = { if (it == BannerKind.NEEDS_SIGN_IN) onSignIn(state.account) }, onDismiss = {},
                        accountName = state.senders.firstOrNull { it.account == state.account }?.identity?.address.orEmpty(),
                    )
                    Fields(state, controller, scope, Modifier.weight(1f))
                    ComposeToolbar(attachFile, attachPhoto)
                }
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars).imePadding().padding(bottom = 56.dp))
    }

    // Questions before sending.
    when (prompt) {
        SendPrompt.EmptyConfirm -> AlertDialog(
            onDismissRequest = { controller.cancelPrompt() },
            title = { Text(stringResource(R.string.compose_empty_title)) },
            text = { Text(stringResource(R.string.compose_empty_message)) },
            dismissButton = { TextButton({ controller.cancelPrompt() }) { Text(stringResource(R.string.control_cancel)) } },
            confirmButton = { TextButton({ scope.launch { controller.confirmEmpty() } }) { Text(stringResource(R.string.compose_empty_confirm)) } },
        )
        is SendPrompt.EncryptionCheck -> EncryptionCheckSheet(
            missing = prompt.missing,
            onSendUnencrypted = { scope.launch { controller.confirmUnencrypted() } },
            onCancel = { controller.cancelPrompt() },
        )
        else -> Unit
    }
}

/** How long a removed attachment can be taken back: the controller's default ([com.example.croft.mail.compose.AutosaveTiming.removeUndoMs]). */
private const val REMOVE_UNDO_MS = 5_000L

/** Largest single attachment the core accepts, for the message when a file is over it (the core's number: 40 MB). */
private const val MAX_ATTACHMENT_MB = 40

@Composable
private fun TopBar(
    state: ComposeState,
    controller: ComposeController,
    scope: kotlinx.coroutines.CoroutineScope,
    snackbar: CroftSnackbarState,
    onAttachFile: () -> Unit,
    onAttachPhoto: () -> Unit,
    sendVisible: Boolean,
) {
    var menu by remember { mutableStateOf(false) }
    var confirmDiscard by remember { mutableStateOf(false) }
    val scheduleLater = stringResource(R.string.compose_schedule_later)
    Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        CroftIconButton(Icons.Filled.Close, stringResource(R.string.compose_close), { scope.launch { controller.back() } })
        Box(Modifier.weight(1f))
        if (sendVisible) {
            SendButton(
                condition = SendCondition.valueOf(state.sendState.name),
                actions = SendActions(
                    onSend = { scope.launch { controller.requestSend() } },
                    onAskEmpty = { scope.launch { controller.requestSend() } },
                    onEncryptionCheck = { scope.launch { controller.requestSend() } },
                    onSchedule = { snackbar.show(scheduleLater) },
                    onExplain = { snackbar.show(it) },
                ),
                sizeLimitMb = state.sizeLimitMb,
                modifier = Modifier.testTag(ComposeTags.SEND),
            )
            Box {
                CroftIconButton(Icons.Filled.MoreVert, stringResource(R.string.compose_more), { menu = true })
                CroftMenu(
                    expanded = menu,
                    onDismiss = { menu = false },
                    items = listOf(
                        MenuItemSpec(stringResource(R.string.compose_show_cc_bcc), { controller.showCcBcc() }, visible = !state.showCcBcc),
                        MenuItemSpec(stringResource(R.string.compose_attach), onAttachFile, icon = CroftIcons.Attachment),
                        MenuItemSpec(stringResource(R.string.compose_attach_photo), onAttachPhoto, icon = CroftIcons.Photo),
                        MenuItemSpec(stringResource(R.string.compose_discard), { confirmDiscard = true }, icon = Icons.Filled.Delete, destructive = true),
                    ),
                )
            }
        }
    }
    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(stringResource(R.string.compose_discard_title)) },
            text = { Text(stringResource(R.string.compose_discard_message)) },
            dismissButton = { TextButton({ confirmDiscard = false }) { Text(stringResource(R.string.control_cancel)) } },
            confirmButton = {
                TextButton({ confirmDiscard = false; scope.launch { controller.discard() } }) {
                    Text(stringResource(R.string.compose_discard_confirm), color = MaterialTheme.colorScheme.error)
                }
            },
        )
    }
}

@Composable
private fun ComposeToolbar(onAttachFile: () -> Unit, onAttachPhoto: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).windowInsetsPadding(WindowInsets.navigationBars).imePadding().padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CroftIconButton(CroftIcons.Attachment, stringResource(R.string.compose_attach), onAttachFile, Modifier.testTag(ComposeTags.ATTACH))
        CroftIconButton(CroftIcons.Photo, stringResource(R.string.compose_attach_photo), onAttachPhoto, Modifier.testTag(ComposeTags.PHOTO))
    }
}

@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
private fun Fields(state: ComposeState, controller: ComposeController, scope: kotlinx.coroutines.CoroutineScope, modifier: Modifier) {
    val bodyFocus = remember { FocusRequester() }
    val toFocus = remember { FocusRequester() }
    var body by remember {
        val at = state.initialCursor?.let { TextRange(it.start.coerceIn(0, state.body.length), it.end.coerceIn(0, state.body.length)) } ?: TextRange(state.body.length)
        mutableStateOf(TextFieldValue(state.body, at))
    }
    // A new message starts in To; a reply, a reopened draft and an Undo start in the body, at the cursor.
    LaunchedEffect(Unit) {
        try { if (state.to.isEmpty()) toFocus.requestFocus() else bodyFocus.requestFocus() } catch (e: IllegalStateException) { /* not on screen yet */ }
    }
    var subject by remember { mutableStateOf(state.subject) }
    val limitMb = state.sizeLimitMb

    Column(modifier.fillMaxWidth().verticalScroll(rememberScrollState()).imePadding(), horizontalAlignment = Alignment.CenterHorizontally) {
        Column(Modifier.widthIn(max = 640.dp).fillMaxWidth().padding(horizontal = 16.dp)) {
            FromRow(state, controller, scope)
            RecipientRow(
                stringResource(R.string.compose_to), RecipientField.TO, state.to, controller, focusRequester = toFocus, clearTick = state.recipientTextTick,
                trailing = if (!state.showCcBcc) ({ CcBccToggle { controller.showCcBcc() } }) else null,
            )
            if (state.showCcBcc) {
                RecipientRow(stringResource(R.string.compose_cc), RecipientField.CC, state.cc, controller, clearTick = state.recipientTextTick)
                RecipientRow(stringResource(R.string.compose_bcc), RecipientField.BCC, state.bcc, controller, clearTick = state.recipientTextTick)
            }
            OutlinedTextField(
                value = subject,
                onValueChange = { subject = it; controller.onSubject(it) },
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp).testTag(ComposeTags.SUBJECT),
                label = { Text(stringResource(R.string.compose_subject)) },
                singleLine = true,
            )
            if (state.quoteMissing) {
                Text(
                    stringResource(R.string.compose_quote_missing),
                    Modifier.padding(top = 8.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            OutlinedTextField(
                value = body,
                onValueChange = { next ->
                    val textChanged = next.text != body.text
                    body = next
                    val sel = BodyCursor(next.selection.start, next.selection.end)
                    if (textChanged) controller.onBody(next.text, sel) else controller.onSelection(sel)
                },
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp).heightIn(min = 160.dp).focusRequester(bodyFocus).testTag(ComposeTags.BODY),
                label = { Text(stringResource(R.string.compose_body)) },
                minLines = 6,
            )
            if (state.attachments.isNotEmpty() || state.attachmentsBusy > 0) {
                FlowRow(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    state.attachments.forEachIndexed { i, a ->
                        val over = state.sendState == SendState.OVER_SIZE_LIMIT
                        AttachmentChip(
                            name = a.name,
                            size = formatSize(LocalContext.current, a.sizeBytes.toLong()),
                            state = if (over) AttachmentState.OVER_LIMIT else AttachmentState.DOWNLOADED,
                            onClick = { scope.launch { controller.removeAttachment(a.id) } },
                            modifier = Modifier.testTag(ComposeTags.attachment(i)),
                            limitText = stringResource(R.string.attachment_over_limit_text, limitMb),
                            tapLabelOverride = stringResource(R.string.attachment_remove, a.name),
                        )
                    }
                    state.pending.forEach { p ->
                        Row(Modifier.heightIn(min = 48.dp).testTag(ComposeTags.pending(p.id)), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            CircularProgressIndicator(Modifier.padding(4.dp).size(20.dp), strokeWidth = 2.dp)
                            Text(stringResource(R.string.attachment_adding), style = MaterialTheme.typography.bodySmall)
                            val cancel = stringResource(R.string.attachment_cancel_adding)
                            TextButton({ controller.cancelPending(p.id) }, Modifier.semantics { contentDescription = cancel }) { Text(stringResource(R.string.control_cancel)) }
                        }
                    }
                }
            }
            SaveLine(state.saveStatus)
        }
    }
}

@Composable
private fun SaveLine(status: SaveStatus) {
    val text = when (status) {
        SaveStatus.NONE -> return
        SaveStatus.SAVED -> stringResource(R.string.compose_status_saved)
        SaveStatus.SAVED_ON_DEVICE -> stringResource(R.string.compose_status_device)
        SaveStatus.WAITING_FOR_CONNECTION -> stringResource(R.string.compose_status_waiting)
        SaveStatus.COULD_NOT_SAVE -> stringResource(R.string.compose_status_failed)
    }
    // A quiet line: not a live region, so a screen reader is not interrupted at every save.
    Row(Modifier.padding(vertical = 12.dp).testTag(ComposeTags.SAVE_STATUS), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        if (status == SaveStatus.COULD_NOT_SAVE) Icon(Icons.Filled.Warning, null, Modifier.padding(end = 2.dp), tint = MaterialTheme.colorScheme.error)
        Text(text, style = MaterialTheme.typography.labelMedium, color = if (status == SaveStatus.COULD_NOT_SAVE) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** From: the picked sender. With several identities or accounts it is a menu; with one it is plain text. */
@Composable
private fun FromRow(state: ComposeState, controller: ComposeController, scope: kotlinx.coroutines.CoroutineScope) {
    val current = state.senders.firstOrNull { it.account == state.account && it.identity.id == state.identity }
        ?: state.senders.firstOrNull { it.account == state.account }
    val shown = current?.identity?.address ?: state.identities.firstOrNull { it.id == state.identity }?.address.orEmpty()
    var open by remember { mutableStateOf(false) }
    val choices = state.senders
    val description = stringResource(R.string.compose_from_description, shown)
    Column(Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth()) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp)
                    .then(if (choices.size > 1) Modifier.clickable { open = true } else Modifier)
                    .semantics(mergeDescendants = true) { contentDescription = if (choices.size > 1) description else shown }
                    .testTag(ComposeTags.FROM),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(stringResource(R.string.compose_from), Modifier.padding(end = 12.dp).widthIn(min = 40.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(shown, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, maxLines = 2)
                if (choices.size > 1) Icon(Icons.Filled.KeyboardArrowDown, contentDescription = null)
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                choices.forEach { c ->
                    val other = c.account != state.account
                    val locked = other && !state.canSwitchAccount
                    DropdownMenuItem(
                        text = {
                            Column {
                                Text(c.identity.address)
                                if (locked) Text(stringResource(R.string.compose_account_locked), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        },
                        enabled = !locked,
                        onClick = { open = false; scope.launch { controller.setSender(c.account, c.identity.id) } },
                    )
                }
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

/** A size as the chip shows it, in the person's language and units. */
internal fun formatSize(context: Context, bytes: Long): String = android.text.format.Formatter.formatShortFileSize(context, bytes)
