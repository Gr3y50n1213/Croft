// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.view.View
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.hasTestTag
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.FakeData
import com.example.croft.mail.FakeMessageRepository
import com.example.croft.mail.FolderRole
import com.example.croft.mail.InMemoryAnchorStore
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.RepositoryRowActions
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.reading.AttachmentData
import com.example.croft.mail.reading.BodyData
import com.example.croft.mail.reading.ConversationMessageData
import com.example.croft.mail.reading.FakeMail
import com.example.croft.mail.reading.FakeReadingRepository
import com.example.croft.mail.reading.MailAddressData
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import com.example.croft.ui.mail.NOW
import com.example.croft.ui.mail.RecordingRowActions
import java.io.File
import java.time.ZoneId
import java.util.concurrent.CopyOnWriteArrayList

/** The made-up address of the account of the fixtures (`FakeData.accounts`). */
const val ME = "sam@example.org"

/** Ids of the three messages of the fixture conversation: two older ones (collapsed) and the newest (expanded, HTML). */
const val OLD_ONE = 1L
const val OLD_TWO = 2L
const val NEWEST = 3L

/** A body of plain text only. */
fun textBody(id: Long, text: String = "Thanks, I will test the new plan on Thursday.", attachments: List<AttachmentData> = emptyList()) = BodyData(
    messageId = id, text = text, html = null, remoteAllowed = false, blockedRemoteCount = 0, trackersRemoved = 0, senderKnown = false,
    truncated = false, attachments = attachments,
)

/** An HTML body with [blocked] remote pictures left out (the core's answer while they are blocked). */
fun htmlBody(id: Long, blocked: Int = 2, attachments: List<AttachmentData> = emptyList(), allowed: Boolean = false) = BodyData(
    messageId = id,
    text = "Plan for the migration.",
    html = if (allowed) "<p>Plan for the migration.</p><img src=\"https://img.example.org/chart.png\">" else "<p>Plan for the migration.</p><img src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\">",
    remoteAllowed = allowed, blockedRemoteCount = if (allowed) 0 else blocked, trackersRemoved = 1, senderKnown = false, truncated = false, attachments = attachments,
)

val PDF = AttachmentData(11, "plan.pdf", "application/pdf", 245_000, shownInline = false, blocked = false)
val NOTES = AttachmentData(12, "dns.txt", "text/plain", 1_200, shownInline = false, blocked = false)
val PROGRAM = AttachmentData(13, "proof.exe", "application/octet-stream", 90_000, shownInline = false, blocked = true)
val INLINE = AttachmentData(14, "logo.png", "image/png", 900, shownInline = true, blocked = false)

/**
 * A small mailbox for the reading tests: account 1 (`sam@example.org`) with three made-up messages of one conversation in its Inbox
 * ([OLD_ONE], [OLD_TWO], [NEWEST], oldest first), the Inbox rows of them in a [FakeMessageRepository], and a [FakeReadingRepository].
 * Everything the screen does is recorded: row actions ([actions]), compose starts ([composed]), closes ([closed]), opened files, opened links.
 */
class ReadingFixture(
    private val newestBody: BodyData = htmlBody(NEWEST, attachments = listOf(PDF, NOTES)),
    private val newestAllowed: BodyData = htmlBody(NEWEST, attachments = listOf(PDF, NOTES), allowed = true),
    localOnly: Boolean = false,
    newestTo: List<MailAddressData> = listOf(MailAddressData("Sam Rivera", ME), MailAddressData("Dan", "dan@example.net")),
    private val newestCc: List<MailAddressData> = listOf(MailAddressData(null, "ops@example.org")),
    senderAddress: String = "mira@example.net",
    olderTexts: Boolean = true,
    olderOneSeen: Boolean = true,
    /** Only the newest message exists (S21: a message on its own). */
    onlyNewest: Boolean = false,
) {
    val account = 1L
    val inbox = FakeData.folderId(account, FakeData.INBOX)

    private fun row(id: Long, name: String, address: String, subject: String, ageMinutes: Long, seen: Boolean) = MessageRowData(
        id = id, account = account, folder = inbox, senderName = name, senderAddress = address, subject = subject,
        snippet = "Snippet of message $id", dateMs = NOW - ageMinutes * 60_000, seen = seen, flagged = false, hasAttachments = false,
    )

    val rows = listOf(
        row(OLD_ONE, "Sam Rivera", ME, "Re: Server migration notes", 3 * 24 * 60, olderOneSeen),
        row(OLD_TWO, "Mira Okafor", "mira@example.net", "Re: Server migration notes", 2 * 24 * 60, true),
        row(NEWEST, "Mira Okafor", senderAddress, "Re: Server migration notes", 30, false),
    )
    val repo = FakeMessageRepository(rows, { NOW }, FakeData.accounts(NOW))
    val actions = RecordingRowActions(RepositoryRowActions(repo))

    private fun message(r: MessageRowData, to: List<MailAddressData>, cc: List<MailAddressData>, stored: Boolean) = ConversationMessageData(
        id = r.id, account = r.account, folder = r.folder, folderRole = FolderRole.INBOX, senderName = r.senderName, senderAddress = r.senderAddress,
        to = to, cc = cc, dateMs = r.dateMs, subject = r.subject, snippet = r.snippet, seen = r.seen, flagged = r.flagged, answered = false,
        hasAttachments = r.hasAttachments, pending = false, bodyStored = stored, remoteAllowed = false,
    )

    val reading = FakeReadingRepository(
        listOf(
            FakeMail(message(rows[0], listOf(MailAddressData("Mira", "mira@example.net")), emptyList(), olderTexts), textBody(OLD_ONE, "The plan is attached.")),
            FakeMail(message(rows[1], listOf(MailAddressData("Sam", ME)), emptyList(), olderTexts), textBody(OLD_TWO)),
            FakeMail(message(rows[2], newestTo, newestCc, true), newestBody, newestAllowed),
        ).filter { !onlyNewest || it.message.id == NEWEST },
        localOnly = localOnly,
    )

    val session = MailSession(repo, InMemoryAnchorStore(), { NOW }, ZoneId.of("UTC"), actions, reading = reading)

    val composed = CopyOnWriteArrayList<ComposeStart>()
    val closed = CopyOnWriteArrayList<ConversationNotice?>()
    val backs = CopyOnWriteArrayList<Unit>()
    val opened = CopyOnWriteArrayList<Pair<File, String>>()
    val links = CopyOnWriteArrayList<String>()
    val haptics = RecordingHaptics()

    /** The folder the "viewer" files are saved to; set by the test (a temporary folder). */
    var attachmentDir: File? = null

    /** Whether a viewer app exists for opened files. */
    var viewerExists = true

    /**
     * Shows S20 for [id] under the app theme. [settings] can be changed by the test while the screen is up; by default the screen does not
     * mark anything read (the mark-read tests choose their own delay), so the calls a test sees are its own.
     */
    fun show(rule: ComposeContentTestRule, id: Long = NEWEST, settings: () -> MailSettings = { MailSettings(markRead = MarkReadDelay.NEVER) }, level: AnimationLevel = AnimationLevel.OFF) {
        rule.show(level, haptics) { Screen(id, settings()) }
    }

    @Composable
    fun Screen(id: Long = NEWEST, settings: MailSettings = MailSettings(markRead = MarkReadDelay.NEVER)) {
        ConversationHost(
            session = session, messageId = id, settings = settings,
            onBack = { backs += Unit }, onClosed = { closed += it }, onCompose = { composed += it },
            opener = AttachmentOpener { file, mime -> opened += file to mime; viewerExists },
            browser = { url -> links += url; true },
            attachmentDir = attachmentDir,
        )
    }
}

/** Wait until something with [tag] is on screen. */
fun ComposeContentTestRule.waitForTag(tag: String, timeoutMs: Long = 10_000) {
    waitUntil(timeoutMs) { onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() }
}

/** The first [WebView] under the activity's window, or null. */
fun ComponentActivity.webView(): WebView? = window.decorView.findWebView()

fun View.findWebView(): WebView? {
    if (this is WebView) return this
    if (this is ViewGroup) for (i in 0 until childCount) getChildAt(i).findWebView()?.let { return it }
    return null
}

/** A [WebResourceRequest] for [url], as the WebView hands one to its client. */
class FakeRequest(private val url: String, private val mainFrame: Boolean = false) : WebResourceRequest {
    override fun getUrl(): android.net.Uri = android.net.Uri.parse(url)
    override fun isForMainFrame(): Boolean = mainFrame
    override fun isRedirect(): Boolean = false
    override fun hasGesture(): Boolean = true
    override fun getMethod(): String = "GET"
    override fun getRequestHeaders(): Map<String, String> = emptyMap()
}

/** A state that a test can change while the screen is up. */
fun <T> stateOf(initial: T) = mutableStateOf(initial)
