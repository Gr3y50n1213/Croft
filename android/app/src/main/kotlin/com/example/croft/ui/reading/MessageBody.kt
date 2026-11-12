// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.text.format.Formatter
import android.webkit.WebView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.example.croft.R
import com.example.croft.mail.MessageId
import com.example.croft.mail.reading.AttachmentData
import com.example.croft.mail.reading.BodyData
import com.example.croft.mail.reading.LinkAction
import com.example.croft.mail.reading.LinkRules
import com.example.croft.mail.reading.ReadProblem
import com.example.croft.mail.reading.RemoteImages
import com.example.croft.design.CroftMode
import com.example.croft.theme.LocalCroftMode
import com.example.croft.theme.LocalReadingFontFamily
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.colorScheme
import com.example.croft.theme.statusColors
import com.example.croft.ui.components.AttachmentChip
import com.example.croft.ui.components.AttachmentState
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftSpinner

/** Test tags of S21 (and the dialog of a tapped link). */
object BodyTags {
    fun body(id: MessageId) = "S21-body-$id"
    fun html(id: MessageId) = "S21-html-$id"
    fun text(id: MessageId) = "S21-text-$id"
    fun remoteBar(id: MessageId) = "S21-remote-$id"
    fun remoteShow(id: MessageId) = "S21-remote-show-$id"
    fun remoteAlways(id: MessageId) = "S21-remote-always-$id"
    fun remoteDismiss(id: MessageId) = "S21-remote-dismiss-$id"
    fun plainNote(id: MessageId) = "S21-plain-note-$id"
    fun notDownloaded(id: MessageId) = "S21-not-downloaded-$id"
    fun failure(id: MessageId) = "S21-failure-$id"
    fun retry(id: MessageId) = "S21-retry-$id"
    fun chip(message: MessageId, attachment: Long) = "S21-chip-$message-$attachment"
    fun blockedLabel(message: MessageId, attachment: Long) = "S21-blocked-$message-$attachment"
    const val LINK_DIALOG = "S21-link-dialog"
    const val LINK_URL = "S21-link-url"
    const val LINK_OPEN = "S21-link-open"
    const val LINK_CANCEL = "S21-link-cancel"
}

/**
 * S21, the body of one expanded message: loading, the text of a failure (with Retry), or the message itself: the "Remote images blocked"
 * bar when there is one, the text or the HTML, the quiet note when only the text version is shown, and the attachment chips.
 * Everything that changes something goes up through the lambdas; nothing here reaches the core.
 */
@Composable
fun MessageBodyView(
    id: MessageId,
    phase: BodyPhase?,
    remoteSetting: RemoteImages,
    barDismissed: Boolean,
    chipState: (AttachmentData) -> ChipPhase,
    onRetry: () -> Unit,
    onShowImages: () -> Unit,
    onAlwaysForSender: (() -> Unit)?,
    onDismissBar: () -> Unit,
    onChip: (AttachmentData) -> Unit,
    onLink: (LinkAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.fillMaxWidth().testTag(BodyTags.body(id)), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        when (phase) {
            null, BodyPhase.Loading -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CroftSpinner(Modifier.size(20.dp))
                Text(stringResource(R.string.reading_body_loading), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            is BodyPhase.Failed -> BodyFailure(id, phase.problem, onRetry)
            is BodyPhase.Ready -> {
                val body = phase.body
                if (body.blockedRemoteCount > 0 && !body.remoteAllowed && !barDismissed) {
                    RemoteContentBar(id, body.blockedRemoteCount, remoteSetting, onShowImages, onAlwaysForSender, onDismissBar)
                }
                val html = body.html
                if (html != null) {
                    HtmlBody(id, html, body.remoteAllowed, onLink)
                } else {
                    PlainTextBody(id, body.text.orEmpty(), onLink)
                    if (body.shownAsPlainText) {
                        Text(
                            stringResource(R.string.reading_plain_note),
                            Modifier.testTag(BodyTags.plainNote(id)),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                AttachmentChips(id, body.chips, chipState, onChip)
            }
        }
    }
}

/** A body that did not load: the plain-language reason, and Retry where trying again can work. Offline says it will load by itself. */
@Composable
private fun BodyFailure(id: MessageId, problem: ReadProblem, onRetry: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val offline = problem == ReadProblem.OFFLINE
        Text(
            stringResource(if (offline) R.string.reading_not_downloaded else ReadCopy.resFor(problem)),
            Modifier.testTag(if (offline) BodyTags.notDownloaded(id) else BodyTags.failure(id)),
            style = MaterialTheme.typography.bodyMedium,
        )
        if (problem.canRetry) {
            CroftButton(stringResource(R.string.reading_retry), onRetry, Modifier.testTag(BodyTags.retry(id)), kind = ButtonKind.OUTLINED)
        }
    }
}

/**
 * The "Remote images blocked" bar (docs/ux/controls.md, banners): the count, and what can be done about it. It is a polite live region,
 * so a screen reader announces it. Under Ask: Show images. Under Known senders: Show images and Always for this sender. Under Never it
 * says so and offers nothing that loads. A dismiss button hides it for this message.
 */
@Composable
fun RemoteContentBar(
    id: MessageId,
    count: Int,
    setting: RemoteImages,
    onShow: () -> Unit,
    onAlways: (() -> Unit)?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val message = if (setting == RemoteImages.NEVER) stringResource(R.string.reading_remote_never) else pluralStringResource(R.plurals.reading_remote_blocked, count, count)
    val bigText = LocalDensity.current.fontScale > STACK_FROM_FONT_SCALE
    val buttons: @Composable () -> Unit = {
        if (setting != RemoteImages.NEVER) {
            TextButton(onClick = onShow, modifier = Modifier.testTag(BodyTags.remoteShow(id))) { Text(stringResource(R.string.reading_remote_show)) }
            if (setting == RemoteImages.KNOWN_SENDERS && onAlways != null) {
                TextButton(onClick = onAlways, modifier = Modifier.testTag(BodyTags.remoteAlways(id))) { Text(stringResource(R.string.reading_remote_always)) }
            }
        }
        IconButton(onClick = onDismiss, modifier = Modifier.testTag(BodyTags.remoteDismiss(id))) {
            Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.reading_remote_dismiss))
        }
    }
    Surface(
        modifier = modifier.fillMaxWidth().testTag(BodyTags.remoteBar(id)).semantics { liveRegion = LiveRegionMode.Polite },
        shape = MaterialTheme.shapes.medium,
        color = statusColors().amberContainer,
        contentColor = statusColors().onAmberContainer,
    ) {
        Column(Modifier.padding(start = 16.dp, top = 8.dp, bottom = 4.dp, end = 4.dp)) {
            Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(Icons.Filled.Info, contentDescription = null, Modifier.padding(top = 2.dp).size(20.dp))
                Text(message, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                if (!bigText) Row(verticalAlignment = Alignment.CenterVertically) { buttons() }
            }
            if (bigText) Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) { buttons() }
        }
    }
}

private const val STACK_FROM_FONT_SCALE = 1.3f

/**
 * A plain-text message: selectable text in the reading font. A link written out in the text is tappable only through the same confirm
 * flow as a link in HTML ([onLink] opens the dialog that shows where it really goes).
 */
@Composable
fun PlainTextBody(id: MessageId, text: String, onLink: (LinkAction) -> Unit, modifier: Modifier = Modifier) {
    val onLinkNow by rememberUpdatedState(onLink)
    val linkColour = MaterialTheme.colorScheme.primary
    val annotated: AnnotatedString = remember(text, linkColour) {
        buildAnnotatedString {
            append(text)
            LinkRules.linkSpans(text).forEach { span ->
                addLink(
                    LinkAnnotation.Clickable(
                        tag = "link",
                        styles = TextLinkStyles(SpanStyle(color = linkColour, textDecoration = TextDecoration.Underline)),
                        linkInteractionListener = { onLinkNow(LinkRules.classify(span.url)) },
                    ),
                    span.start, span.end,
                )
            }
        }
    }
    if (text.isBlank()) {
        Text(stringResource(R.string.reading_body_empty), modifier.testTag(BodyTags.text(id)), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    } else {
        SelectionContainer(modifier.fillMaxWidth()) {
            Text(annotated, Modifier.fillMaxWidth().testTag(BodyTags.text(id)), style = MaterialTheme.typography.bodyLarge.copy(fontFamily = LocalReadingFontFamily.current))
        }
    }
}

/** The light card HTML mail is drawn on: in dark mode too, so that the email's own colours are never recoloured (docs/UX.md). */
@Composable
private fun htmlCardColour(): Color {
    val settings = LocalThemeSettings.current
    val mode = LocalCroftMode.current
    val light = MaterialTheme.colorScheme
    return if (mode == CroftMode.LIGHT) light.surfaceContainerLowest else remember(settings) { settings.colorScheme(CroftMode.LIGHT).surfaceContainerLowest }
}

/**
 * An HTML message in the one hardened WebView ([MailWebViewFactory]). It is given [html] (the core's sanitised HTML) and nothing else;
 * [remoteAllowed] decides whether the page may use the network (for `https` pictures only). The page is as high as its content, inside
 * the scrolling conversation. If the device has no WebView the text of the message is not available here, so a short note says so.
 */
@Composable
fun HtmlBody(id: MessageId, html: String, remoteAllowed: Boolean, onLink: (LinkAction) -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val fontScale = LocalDensity.current.fontScale
    val card = htmlCardColour()
    val onLinkNow by rememberUpdatedState(onLink)
    val view: WebView? = remember(context) { try { MailWebViewFactory.create(context) } catch (e: RuntimeException) { null } }
    if (view == null) {
        Text(stringResource(R.string.reading_body_failed), modifier.testTag(BodyTags.html(id)), style = MaterialTheme.typography.bodyMedium)
        return
    }
    Box(modifier.fillMaxWidth().clip(MaterialTheme.shapes.medium).background(card)) {
        AndroidView(
            factory = { view },
            modifier = Modifier.fillMaxWidth().testTag(BodyTags.html(id)),
            update = { v ->
                val client = v.tag as? MailWebViewClient ?: return@AndroidView
                client.onLink = { onLinkNow(it) }
                client.remoteAllowed = remoteAllowed
                // The network is switched on or off before the page is (re)loaded, never after.
                MailWebViewFactory.apply(v.settings, remoteAllowed, fontScale)
                v.setBackgroundColor(card.toArgb())
                if (client.shown != html to remoteAllowed) {
                    client.shown = html to remoteAllowed
                    v.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
                }
            },
            onRelease = { it.destroy() },
        )
    }
}

/** The attachment chips of a message (not the pictures drawn inside its HTML), with the state of each. A blocked one says why under the chip. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AttachmentChips(
    messageId: MessageId,
    attachments: List<AttachmentData>,
    chipState: (AttachmentData) -> ChipPhase,
    onChip: (AttachmentData) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (attachments.isEmpty()) return
    val context = LocalContext.current
    FlowRow(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        attachments.forEach { a ->
            val state = when {
                a.blocked -> AttachmentState.BLOCKED
                else -> when (chipState(a)) {
                    ChipPhase.NOT_DOWNLOADED -> AttachmentState.NOT_DOWNLOADED
                    ChipPhase.DOWNLOADING -> AttachmentState.DOWNLOADING
                    ChipPhase.DOWNLOADED -> AttachmentState.DOWNLOADED
                    ChipPhase.FAILED -> AttachmentState.FAILED
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                AttachmentChip(
                    name = a.fileName,
                    size = Formatter.formatShortFileSize(context, a.size),
                    state = state,
                    onClick = { onChip(a) },
                    modifier = Modifier.testTag(BodyTags.chip(messageId, a.id)),
                    tapLabelOverride = if (a.blocked) stringResource(R.string.reading_attachment_blocked_tap) else null,
                )
                if (a.blocked) {
                    Text(
                        stringResource(R.string.reading_attachment_blocked_label),
                        Modifier.padding(horizontal = 12.dp).testTag(BodyTags.blockedLabel(messageId, a.id)),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
