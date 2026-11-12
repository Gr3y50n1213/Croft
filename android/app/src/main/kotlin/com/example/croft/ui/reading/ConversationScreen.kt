// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.example.croft.R
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.mail.MessageId
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.reading.ConversationMessageData
import com.example.croft.mail.reading.LinkAction
import com.example.croft.mail.reading.LinkRules
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSpinner
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.mail.DeclineFolderButton
import com.example.croft.ui.mail.InboxTexts
import com.example.croft.ui.mail.MoveToSheet
import com.example.croft.ui.mail.RowTimes
import com.example.croft.ui.mail.displayName
import java.io.File

/** Test tags of S20. */
object ReadingTags {
    const val SCREEN = "S20"
    const val BACK = "S20-back"
    const val ARCHIVE = "S20-archive"
    const val DELETE = "S20-delete"
    const val MARK_UNREAD = "S20-mark-unread"
    const val MORE = "S20-more"
    const val SUBJECT = "S20-subject"
    const val STAR = "S20-star"
    const val FOLDER = "S20-folder"
    const val NOT_SAVED = "S20-not-saved"
    const val REPLY_BAR = "S20-reply-bar"
    const val REPLY = "S20-reply"
    const val REPLY_ALL = "S20-reply-all"
    const val FORWARD = "S20-forward"
    const val FAILED = "S20-failed"
    const val LOADING = "S20-loading"
    const val DELETE_FOREVER_DIALOG = "S20-delete-forever"
    fun message(id: MessageId) = "S20-message-$id"
    fun header(id: MessageId) = "S20-header-$id"
    fun reply(id: MessageId) = "S20-reply-$id"
}

/**
 * S20 Conversation over a [session]: opens the conversation of [messageId], shows it, and does what the person asks. [onBack] returns to
 * the list; [onClosed] returns to the list after an action, with the notice the list shows (Archived, Undo). [onCompose] opens compose
 * (reply, reply all, forward, a `mailto:` link). [settings] are the S49 settings in force (remote images, mark-read delay).
 *
 * [opener], [browser] and [attachmentDir] are what the real screen builds from the context (the chooser, `ACTION_VIEW`,
 * `cacheDir/shared-attachments`); tests give their own.
 */
@Composable
fun ConversationHost(
    session: MailSession,
    messageId: MessageId,
    settings: MailSettings,
    onBack: () -> Unit,
    onClosed: (ConversationNotice?) -> Unit,
    onCompose: (ComposeStart) -> Unit,
    modifier: Modifier = Modifier,
    opener: AttachmentOpener? = null,
    browser: ((String) -> Boolean)? = null,
    attachmentDir: File? = null,
) {
    val context = LocalContext.current
    val res = context.resources
    val scope = rememberCoroutineScope()
    val chooserTitle = stringResource(R.string.reading_attachment_open_with)
    val linkChooserTitle = stringResource(R.string.reading_link_chooser)
    val controller = remember(session, messageId) {
        ConversationController(session, scope, messageId, InboxTexts(res), ReadingTexts(res), attachmentDir ?: SharedAttachments.directory(context))
    }
    controller.settings = settings
    controller.onClose = onClosed
    controller.onCompose = onCompose
    controller.opener = opener ?: ChooserAttachmentOpener(context, chooserTitle)
    controller.browser = browser ?: { url -> context.openInChooser(url, linkChooserTitle) }
    LaunchedEffect(controller) { controller.run() }

    // The viewer returned (the app was paused and is back): what was saved for it is deleted. The folder is also emptied when the screen is left.
    DisposableEffect(controller, context) {
        val owner = context.lifecycleOwner()
        var paused = false
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_PAUSE -> paused = true
                Lifecycle.Event.ON_RESUME -> if (paused) { paused = false; controller.clearSharedFiles() }
                else -> Unit
            }
        }
        owner?.lifecycle?.addObserver(observer)
        onDispose { owner?.lifecycle?.removeObserver(observer); controller.leave() }
    }
    BackHandler { onBack() }
    ConversationScreen(controller, session, onBack, modifier)
}

private tailrec fun Context.lifecycleOwner(): LifecycleOwner? = when (this) {
    is LifecycleOwner -> this
    is ContextWrapper -> baseContext.lifecycleOwner()
    else -> null
}

/** Opens [url] with the system chooser (`ACTION_VIEW`, browsable): the person picks the browser. Returns false when no app can open it. */
fun Context.openInChooser(url: String, title: String): Boolean {
    val view = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addCategory(Intent.CATEGORY_BROWSABLE)
    val chooser = Intent.createChooser(view, title)
    if (generateSequence(this) { (it as? ContextWrapper)?.baseContext }.none { it is androidx.activity.ComponentActivity }) chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    return try {
        startActivity(chooser)
        true
    } catch (e: android.content.ActivityNotFoundException) {
        false
    } catch (e: SecurityException) {
        false
    }
}

/** S20 over a [controller] that is (being) run. See [ConversationHost]. */
@Composable
fun ConversationScreen(controller: ConversationController, session: MailSession, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val phase = controller.phase
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(ReadingTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars)) {
            ActionBar(controller, onBack)
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                when (phase) {
                    ConversationPhase.Loading -> Row(
                        Modifier.padding(24.dp).testTag(ReadingTags.LOADING),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        CroftSpinner(Modifier.size(20.dp))
                        Text(stringResource(R.string.reading_loading), style = MaterialTheme.typography.bodyMedium)
                    }
                    is ConversationPhase.Failed -> Column(Modifier.padding(24.dp).widthIn(max = CONTENT_MAX_WIDTH).testTag(ReadingTags.FAILED).semantics(mergeDescendants = true) { }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.reading_cannot_open_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        Text(stringResource(ReadCopy.resFor(phase.problem)), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    ConversationPhase.Ready -> Messages(controller, session)
                }
            }
            if (phase == ConversationPhase.Ready) ReplyBar(controller)
        }
        CroftSnackbarHost(controller.snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = SNACKBAR_ABOVE_REPLY_BAR))
    }
    Sheets(controller)
}

private val CONTENT_MAX_WIDTH = 720.dp
private val SNACKBAR_ABOVE_REPLY_BAR = 72.dp

// ---- The action bar and the overflow menu ----

@Composable
private fun ActionBar(controller: ConversationController, onBack: () -> Unit) {
    val ready = controller.phase == ConversationPhase.Ready
    var menuOpen by remember { mutableStateOf(false) }
    val role = controller.openedRole
    val inSpam = role == FolderRole.SPAM
    val foreverWord = role == FolderRole.TRASH || role == FolderRole.SPAM
    Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.reading_back), onBack, Modifier.testTag(ReadingTags.BACK))
        Box(Modifier.weight(1f))
        if (ready) {
            if (role != FolderRole.ARCHIVE) CroftIconButton(CroftIcons.Archive, stringResource(R.string.reading_action_archive), controller::archive, Modifier.testTag(ReadingTags.ARCHIVE))
            CroftIconButton(
                Icons.Filled.Delete,
                stringResource(if (foreverWord) R.string.reading_action_delete_forever else R.string.reading_action_delete),
                controller::delete,
                Modifier.testTag(ReadingTags.DELETE),
            )
            CroftIconButton(CroftIcons.EnvelopeOpen, stringResource(R.string.reading_action_mark_unread), controller::markUnread, Modifier.testTag(ReadingTags.MARK_UNREAD))
            Box {
                CroftIconButton(ReadingIcons.More, stringResource(R.string.reading_action_more), { menuOpen = true }, Modifier.testTag(ReadingTags.MORE))
                // Mark as unread, Move to, Report spam. Show original (the core has no call for it), unsubscribe, Snooze and Mute thread (later
                // phases) are not offered at all: no disabled placeholders.
                CroftMenu(
                    expanded = menuOpen,
                    onDismiss = { menuOpen = false },
                    items = listOf(
                        MenuItemSpec(stringResource(R.string.reading_action_mark_unread), controller::markUnread),
                        MenuItemSpec(stringResource(R.string.reading_menu_move), controller::openMove),
                        MenuItemSpec(stringResource(if (inSpam) R.string.reading_menu_not_spam else R.string.reading_menu_spam), controller::reportSpam, destructive = !inSpam),
                    ),
                )
            }
        }
    }
}

// ---- The messages ----

@Composable
private fun Messages(controller: ConversationController, session: MailSession) {
    val conversation = controller.conversation ?: return
    val opened = controller.opened
    Column(
        Modifier.widthIn(max = CONTENT_MAX_WIDTH).fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (opened != null) Header(controller, opened, conversation.localOnly)
        conversation.messages.forEach { m ->
            if (m.id in controller.expanded) ExpandedMessage(controller, session, m) else CollapsedMessage(controller, session, m)
        }
    }
}

@Composable
private fun Header(controller: ConversationController, opened: ConversationMessageData, localOnly: Boolean) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                opened.subject.ifBlank { stringResource(R.string.reading_no_subject) },
                Modifier.weight(1f).testTag(ReadingTags.SUBJECT).semantics { heading() },
                style = MaterialTheme.typography.headlineSmall,
            )
            CroftIconButton(
                if (opened.flagged) Icons.Filled.Star else CroftIcons.StarOutline,
                stringResource(if (opened.flagged) R.string.reading_unstar else R.string.reading_star),
                controller::toggleStar,
                Modifier.testTag(ReadingTags.STAR),
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            controller.openedFolder?.let { folder ->
                Surface(Modifier.testTag(ReadingTags.FOLDER).semantics(mergeDescendants = true) { }, shape = MaterialTheme.shapes.small, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
                    Text(folder.displayName(), Modifier.padding(horizontal = 12.dp, vertical = 4.dp), style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            if (localOnly) {
                // The sample mailbox: every change stays on this device (docs/ux/server-contract.md), as the list rows say.
                Text(stringResource(R.string.row_not_saved), Modifier.testTag(ReadingTags.NOT_SAVED), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun Avatar(name: String) {
    val letter = name.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "?"
    Box(
        Modifier.size(AVATAR).clip(CircleShape).background(MaterialTheme.colorScheme.secondaryContainer).clearAndSetSemantics { },
        contentAlignment = Alignment.Center,
    ) { Text(letter, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSecondaryContainer, maxLines = 1) }
}

private val AVATAR = 40.dp

@Composable
private fun shownName(m: ConversationMessageData): String = if (m.isOwn) stringResource(R.string.reading_you) else m.displayName

@Composable
private fun timeOf(session: MailSession, m: ConversationMessageData): String =
    RowTimes.shown(m.dateMs, session.clock(), session.zone, LocalConfiguration.current.locales[0])

/** An older message: sender, date and snippet in one row. A tap, or the TalkBack action Expand, opens it. */
@Composable
private fun CollapsedMessage(controller: ConversationController, session: MailSession, m: ConversationMessageData) {
    val name = shownName(m)
    val time = timeOf(session, m)
    val expandLabel = stringResource(R.string.reading_expand)
    val collapsedWord = stringResource(R.string.reading_message_collapsed)
    val open = { controller.toggle(m.id) }
    Surface(
        Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainer,
    ) {
        Row(
            Modifier
                .testTag(ReadingTags.message(m.id))
                .clickable(role = Role.Button, onClickLabel = expandLabel, onClick = open)
                .semantics(mergeDescendants = true) {
                    stateDescription = collapsedWord
                    customActions = listOf(CustomAccessibilityAction(expandLabel) { open(); true })
                }
                .heightIn(min = 64.dp)
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Avatar(name)
            Column(Modifier.weight(1f)) {
                Row(horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
                    Text(name, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.titleMedium, fontWeight = if (m.seen) FontWeight.Normal else FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(time, Modifier.padding(start = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                }
                if (m.snippet.isNotBlank()) {
                    Text(m.snippet, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = if (LocalDensity.current.fontScale > BIG_TEXT) 3 else 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

private const val BIG_TEXT = 1.3f

/** A message shown in full: header (a tap collapses it), the body (S21) and a reply button of its own. */
@Composable
private fun ExpandedMessage(controller: ConversationController, session: MailSession, m: ConversationMessageData) {
    val name = shownName(m)
    val time = timeOf(session, m)
    val me = stringResource(R.string.reading_to_me)
    val recipients = m.to.map { if (it.address != null && it.address.equals(controller.myAddress, ignoreCase = true)) me else it.display }.filter { it.isNotBlank() }.joinToString(", ")
    val line = if (recipients.isBlank()) time else stringResource(R.string.reading_to_line, recipients, time)
    val collapseLabel = stringResource(R.string.reading_collapse)
    val expandedWord = stringResource(R.string.reading_message_expanded)
    Surface(
        Modifier.fillMaxWidth().testTag(ReadingTags.message(m.id)),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(
                    Modifier
                        .weight(1f)
                        .testTag(ReadingTags.header(m.id))
                        .clickable(role = Role.Button, onClickLabel = collapseLabel) { controller.toggle(m.id) }
                        .semantics(mergeDescendants = true) { stateDescription = expandedWord },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Avatar(name)
                    Column(Modifier.weight(1f)) {
                        Text(name, style = MaterialTheme.typography.titleMedium, fontWeight = if (m.seen) FontWeight.Normal else FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = if (LocalDensity.current.fontScale > BIG_TEXT) 3 else 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                CroftIconButton(ReadingIcons.Reply, stringResource(R.string.reading_reply_to, name), { controller.reply(m.id) }, Modifier.testTag(ReadingTags.reply(m.id)))
            }
            MessageBodyView(
                id = m.id,
                phase = controller.bodies[m.id],
                remoteSetting = controller.settings.remoteImages,
                barDismissed = m.id in controller.dismissedBars,
                chipState = { a -> controller.chipPhase(ChipKey(m.id, a.id)) },
                onRetry = { controller.retry(m.id) },
                onShowImages = { controller.showImages(m.id) },
                onAlwaysForSender = if (controller.offersTrustSender && m.senderAddress != null) ({ controller.alwaysForSender(m.id) }) else null,
                onDismissBar = { controller.dismissBar(m.id) },
                onChip = { a -> controller.onChip(m.id, a) },
                onLink = controller::onLink,
            )
        }
    }
}

// ---- The reply bar ----

/** Reply, Reply all (only when more than one other person is on the message) and Forward, below the messages. */
@Composable
private fun ReplyBar(controller: ConversationController) {
    val reply = stringResource(R.string.reading_reply)
    val replyAll = stringResource(R.string.reading_reply_all)
    val forward = stringResource(R.string.reading_forward)
    val buttons: List<Triple<String, Pair<androidx.compose.ui.graphics.vector.ImageVector, String>, () -> Unit>> = buildList {
        add(Triple(reply, ReadingIcons.Reply to ReadingTags.REPLY, { controller.reply() }))
        if (controller.showReplyAll) add(Triple(replyAll, ReadingIcons.ReplyAll to ReadingTags.REPLY_ALL, { controller.replyAll() }))
        add(Triple(forward, ReadingIcons.Forward to ReadingTags.FORWARD, { controller.forward() }))
    }
    val big = LocalDensity.current.fontScale > BIG_TEXT
    Surface(Modifier.fillMaxWidth().testTag(ReadingTags.REPLY_BAR), color = MaterialTheme.colorScheme.surfaceContainer) {
        Box(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), contentAlignment = Alignment.Center) {
            if (big) {
                Column(Modifier.widthIn(max = CONTENT_MAX_WIDTH), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    buttons.forEach { (label, icon, click) ->
                        CroftButton(label, click, Modifier.fillMaxWidth().testTag(icon.second), kind = ButtonKind.OUTLINED, leadingIcon = icon.first)
                    }
                }
            } else {
                Row(Modifier.widthIn(max = CONTENT_MAX_WIDTH), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    buttons.forEach { (label, icon, click) ->
                        Box(Modifier.weight(1f)) {
                            CroftButton(label, click, Modifier.fillMaxWidth().testTag(icon.second), kind = ButtonKind.OUTLINED, leadingIcon = icon.first)
                        }
                    }
                }
            }
        }
    }
}

// ---- Sheets and dialogs ----

@Composable
private fun Sheets(controller: ConversationController) {
    // S30 Move to.
    if (controller.moveRequest != null) {
        MoveToSheet(
            title = stringResource(R.string.move_title),
            folders = controller.moveFolders,
            recent = controller.moveRecents,
            excluding = controller.moveRequest?.excluding.orEmpty(),
            onPick = controller::pickMove,
            onDismiss = controller::dismissMove,
        )
    }
    // The first Archive or Delete of an account with no such folder: which folder is yours? (remembered, asked once)
    controller.folderRequest?.let { request ->
        MoveToSheet(
            title = stringResource(if (request.role == FolderRole.TRASH) R.string.choose_trash_title else R.string.choose_archive_title),
            folders = controller.folders.filter { it.account == request.account },
            recent = emptyList(),
            excluding = emptySet(),
            onPick = controller::answerFolderRequest,
            onDismiss = controller::dismissFolderRequest,
            footer = if (request.role == FolderRole.ARCHIVE) ({ DeclineFolderButton(stringResource(R.string.choose_decline_archive), controller::declineFolderRequest) }) else null,
        )
    }
    if (controller.confirmDeleteForever) {
        com.example.croft.ui.components.DestructiveDialog(
            title = stringResource(R.string.reading_delete_forever_title),
            message = stringResource(R.string.reading_delete_forever_message),
            confirmLabel = stringResource(R.string.reading_action_delete_forever),
            onConfirm = controller::deleteForever,
            onDismiss = controller::dismissDeleteForever,
            modifier = Modifier.testTag(ReadingTags.DELETE_FOREVER_DIALOG),
        )
    }
    controller.pendingLink?.let { link -> LinkDialog(link, onOpen = controller::confirmLink, onCancel = controller::cancelLink) }
}

/**
 * The question after a tap on a link: where it really goes, written out in full, and Cancel or Open. The destination is the URL the
 * message's HTML holds, never the words shown for the link; characters that could disguise it are written out. A web link opens in the
 * browser chooser; a `mailto:` link opens Croft's own compose.
 */
@Composable
fun LinkDialog(link: LinkAction, onOpen: () -> Unit, onCancel: () -> Unit) {
    val url = when (link) {
        is LinkAction.Web -> link.url
        is LinkAction.Mail -> link.url
        LinkAction.Ignore -> return
    }
    val web = link is LinkAction.Web
    val shown = LinkRules.displayable(url)
    val host = if (web) LinkRules.hostOf(url) else null
    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(BodyTags.LINK_DIALOG),
        title = { Text(stringResource(if (web) R.string.reading_link_title else R.string.reading_mailto_title)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (host != null) Text(stringResource(R.string.reading_link_goes_to, host), style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Bold)
                Text(stringResource(R.string.reading_link_address_label), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                SelectionContainer { Text(shown, Modifier.testTag(BodyTags.LINK_URL), style = MaterialTheme.typography.bodyMedium) }
            }
        },
        dismissButton = { TextButton(onClick = onCancel, modifier = Modifier.testTag(BodyTags.LINK_CANCEL)) { Text(stringResource(R.string.control_cancel)) } },
        confirmButton = {
            TextButton(onClick = onOpen, modifier = Modifier.testTag(BodyTags.LINK_OPEN)) {
                Text(stringResource(if (web) R.string.reading_link_open else R.string.reading_mailto_open))
            }
        },
    )
}
