// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.lifecycle.Lifecycle
import com.example.croft.mail.MailSettings
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.reading.ReadProblem
import com.example.croft.mail.reading.RemoteImages
import com.example.croft.ui.components.show
import java.io.File
import java.nio.file.Files
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** S21: the message body: plain text and links, the blocked-content bar, the plain-text note, attachment chips, and the dialog of a link. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class MessageBodyTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val dirs = mutableListOf<File>()

    private fun tempDir(): File = Files.createTempDirectory("croft-shared-attachments").toFile().also { dirs += it }

    @After
    fun tearDown() = dirs.forEach { it.deleteRecursively() }

    // ---- Plain text and links ----

    private fun linksOf(text: AnnotatedString) = text.getLinkAnnotations(0, text.length)

    @Test
    fun plainText_isShownAsText_andItsLinksGoThroughTheSameConfirmFlowAsHtml() {
        val url = "https://example.org/path?a=1"
        val f = ReadingFixture(newestBody = textBody(NEWEST, "Please read $url. Thanks!"))
        f.show(rule)
        rule.waitForTag(BodyTags.text(NEWEST))
        val shown = rule.onNodeWithTag(BodyTags.text(NEWEST)).fetchSemanticsNode().config.getOrNull(SemanticsProperties.Text)!!.first()
        val links = linksOf(shown)
        assertEquals("one link, without the full stop that ends the sentence", 1, links.size)
        assertEquals(url, shown.text.substring(links[0].start, links[0].end))
        assertTrue("no WebView for plain text", rule.activity.webView() == null)

        // The tap does not open anything: it asks.
        val clickable = links[0].item as LinkAnnotation.Clickable
        rule.runOnUiThread { clickable.linkInteractionListener!!.onClick(clickable) }
        rule.waitForTag(BodyTags.LINK_DIALOG)
        rule.onNodeWithTag(BodyTags.LINK_URL).assertTextEquals(url)
        assertTrue(f.links.isEmpty())
        rule.onNodeWithTag(BodyTags.LINK_CANCEL).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(BodyTags.LINK_DIALOG).assertDoesNotExist()
        assertTrue("cancel opens nothing", f.links.isEmpty())

        rule.runOnUiThread { clickable.linkInteractionListener!!.onClick(clickable) }
        rule.waitForTag(BodyTags.LINK_DIALOG)
        rule.onNodeWithTag(BodyTags.LINK_OPEN).performClick()
        rule.waitForIdle()
        assertEquals("only on confirm, with the real address", listOf(url), f.links.toList())
    }

    @Test
    fun plainTextWithAMailtoLink_opensCroftsOwnCompose_onConfirm() {
        val f = ReadingFixture(newestBody = textBody(NEWEST, "Write to mailto:team@example.org?subject=Hi now"))
        f.show(rule)
        rule.waitForTag(BodyTags.text(NEWEST))
        val shown = rule.onNodeWithTag(BodyTags.text(NEWEST)).fetchSemanticsNode().config.getOrNull(SemanticsProperties.Text)!!.first()
        val clickable = linksOf(shown).single().item as LinkAnnotation.Clickable
        rule.runOnUiThread { clickable.linkInteractionListener!!.onClick(clickable) }
        rule.waitForTag(BodyTags.LINK_DIALOG)
        rule.onNodeWithTag(BodyTags.LINK_URL).assertTextEquals("mailto:team@example.org?subject=Hi")
        rule.onNodeWithTag(BodyTags.LINK_OPEN).performClick()
        rule.waitForIdle()
        val start = f.composed.single() as ComposeStart.Fill
        assertEquals(listOf("team@example.org"), start.fill.to)
        assertEquals("Hi", start.fill.subject)
        assertEquals(1L, start.account)
        assertTrue("no browser for a mailto: link", f.links.isEmpty())
    }

    @Test
    fun aLinkInHtmlShowsItsRealDestination_neverLoadsInThePage_andOpensOnlyOnConfirm() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.waitForIdle()
        val view = rule.activity.webView()!!
        val client = view.tag as MailWebViewClient
        val real = "https://phish.example.net/login?next=%2Fbank"
        rule.runOnUiThread { assertTrue(client.shouldOverrideUrlLoading(view, FakeRequest(real))) }
        rule.waitForTag(BodyTags.LINK_DIALOG)
        rule.onNodeWithTag(BodyTags.LINK_URL).assertTextEquals(real)
        rule.onNodeWithText("This link goes to phish.example.net").assertExists()
        assertNull("the page itself never loaded the link", shadowOf(view).lastLoadedUrl)
        assertTrue(f.links.isEmpty())
        rule.onNodeWithTag(BodyTags.LINK_OPEN).performClick()
        rule.waitForIdle()
        assertEquals(listOf(real), f.links.toList())
        assertNull(shadowOf(view).lastLoadedUrl)
    }

    @Test
    fun aLinkThatCouldBeDisguisedIsShownWithItsHiddenCharactersWrittenOut() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.waitForIdle()
        val view = rule.activity.webView()!!
        val client = view.tag as MailWebViewClient
        rule.runOnUiThread { client.shouldOverrideUrlLoading(view, FakeRequest("https://example.org/‮fdp.exe")) }
        rule.waitForTag(BodyTags.LINK_DIALOG)
        rule.onNodeWithTag(BodyTags.LINK_URL).assertTextEquals("https://example.org/\\u202Efdp.exe")
    }

    @Test
    fun aLinkOfAnotherKindDoesNothing() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.waitForIdle()
        val view = rule.activity.webView()!!
        val client = view.tag as MailWebViewClient
        rule.runOnUiThread {
            for (url in listOf("javascript:alert(1)", "file:///etc/passwd", "intent://x#Intent;end", "content://com.example.croft.attachments/x")) client.shouldOverrideUrlLoading(view, FakeRequest(url))
        }
        rule.waitForIdle()
        rule.onNodeWithTag(BodyTags.LINK_DIALOG).assertDoesNotExist()
        assertTrue(f.links.isEmpty() && f.composed.isEmpty())
    }

    // ---- HTML ----

    @Test
    fun htmlGoesToTheWebViewAsTheSanitisedStringOnly_withTheNetworkBlockedUntilAllowed() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.waitForIdle()
        val view = rule.activity.webView()!!
        val load = shadowOf(view).lastLoadDataWithBaseURL
        assertNull(load.baseUrl)
        assertEquals(htmlBody(NEWEST).html, load.data)
        assertEquals("text/html", load.mimeType)
        assertTrue(view.settings.blockNetworkLoads && view.settings.blockNetworkImage)
        assertFalse(view.settings.javaScriptEnabled)
    }

    // ---- The "Remote images blocked" bar ----

    @Test
    fun theBlockedBarShowsTheCount_andShowImagesAllowsTheMessageAndReloadsIt() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).assertIsDisplayed()
        rule.onNodeWithText("Remote images blocked: 2 pictures").assertExists()
        rule.onNodeWithTag(BodyTags.remoteAlways(NEWEST)).assertDoesNotExist()
        val before = shadowOf(rule.activity.webView()!!).lastLoadDataWithBaseURL

        rule.onNodeWithTag(BodyTags.remoteShow(NEWEST)).performClick()
        rule.waitUntil(10_000) { f.reading.calls.count { it.startsWith("loadBody") } >= 2 }
        rule.waitForIdle()
        assertEquals(listOf("allowRemoteContent:$NEWEST"), f.reading.calls.filter { it.startsWith("allow") })
        rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).assertDoesNotExist()
        val view = rule.activity.webView()!!
        // The network opens for pictures, and the page is loaded again with the allowed HTML (https pictures only).
        assertFalse(view.settings.blockNetworkLoads)
        assertFalse(view.settings.blockNetworkImage)
        val after = shadowOf(view).lastLoadDataWithBaseURL
        assertTrue(after.data != before.data)
        assertTrue(after.data.contains("src=\"https://"))
        assertNull(after.baseUrl)
        assertTrue((view.tag as MailWebViewClient).remoteAllowed)
    }

    @Test
    fun theBarAnnouncesItself_andItsButtonsAreNamed() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        val bar = rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).fetchSemanticsNode()
        assertEquals(androidx.compose.ui.semantics.LiveRegionMode.Polite, bar.config.getOrNull(SemanticsProperties.LiveRegion))
        rule.onNodeWithText("Show images").assertExists()
        rule.onNodeWithTag(BodyTags.remoteDismiss(NEWEST)).assertExists()
    }

    @Test
    fun underKnownSendersTheBarAlsoOffersAlwaysForThisSender_andItTrustsThem() {
        val f = ReadingFixture()
        f.show(rule, settings = { MailSettings(remoteImages = RemoteImages.KNOWN_SENDERS) })
        rule.waitForTag(BodyTags.remoteAlways(NEWEST))
        rule.onNodeWithTag(BodyTags.remoteAlways(NEWEST)).assertTextContains("Always for this sender")
        rule.onNodeWithTag(BodyTags.remoteAlways(NEWEST)).performClick()
        rule.waitUntil(10_000) { f.reading.calls.any { it.startsWith("trustSender") } }
        rule.waitUntil(10_000) { f.reading.calls.count { it.startsWith("loadBody") } >= 2 }
        rule.waitForIdle()
        assertEquals(listOf("mira@example.net"), runBlockingTrusted(f))
        // The setting is passed on every load, and the core now knows the sender: the images show.
        assertTrue(f.reading.calls.contains("loadBody:$NEWEST:known"))
        rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).assertDoesNotExist()
    }

    private fun runBlockingTrusted(f: ReadingFixture) = kotlinx.coroutines.runBlocking { f.reading.trustedSenders(1) }

    @Test
    fun underNeverTheBarSaysSoAndOffersNothingThatLoads() {
        val f = ReadingFixture()
        f.show(rule, settings = { MailSettings(remoteImages = RemoteImages.NEVER) })
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        rule.onNodeWithText("Remote images are turned off in your settings, so none are loaded.").assertExists()
        rule.onNodeWithTag(BodyTags.remoteShow(NEWEST)).assertDoesNotExist()
        rule.onNodeWithTag(BodyTags.remoteAlways(NEWEST)).assertDoesNotExist()
        assertTrue(f.reading.calls.none { it.startsWith("allow") })
    }

    @Test
    fun noBarWhenNothingWasBlocked_andTheBarCanBeDismissed() {
        val clean = ReadingFixture(newestBody = htmlBody(NEWEST, blocked = 0))
        clean.show(rule)
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).assertDoesNotExist()
    }

    @Test
    fun dismissingTheBarHidesItWithoutLoadingAnything() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        rule.onNodeWithTag(BodyTags.remoteDismiss(NEWEST)).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).assertDoesNotExist()
        assertTrue(f.reading.calls.none { it.startsWith("allow") })
        assertTrue(rule.activity.webView()!!.settings.blockNetworkLoads)
    }

    // ---- Too complex, truncated, empty ----

    @Test
    fun aMessageThatCouldNotBeShownAsHtmlIsShownAsTextWithAQuietNote() {
        val body = textBody(NEWEST, "The text of a very complex message.").copy(truncated = true)
        val f = ReadingFixture(newestBody = body)
        f.show(rule)
        rule.waitForTag(BodyTags.plainNote(NEWEST))
        rule.onNodeWithTag(BodyTags.plainNote(NEWEST)).assertTextEquals("This message is shown as plain text")
        rule.onNodeWithTag(BodyTags.text(NEWEST)).assertTextContains("The text of a very complex message.")
        assertNull(rule.activity.webView())
    }

    @Test
    fun aPlainTextMessageHasNoNote() {
        val f = ReadingFixture(newestBody = textBody(NEWEST, "Just text."))
        f.show(rule)
        rule.waitForTag(BodyTags.text(NEWEST))
        rule.onNodeWithTag(BodyTags.plainNote(NEWEST)).assertDoesNotExist()
    }

    @Test
    fun everyReadErrorShowsItsOwnPlainText_inPlaceOfTheBody() {
        var phase by mutableStateOf<BodyPhase>(BodyPhase.Failed(ReadProblem.NO_SUCH_MESSAGE))
        rule.show {
            MessageBodyView(
                id = 5, phase = phase, remoteSetting = RemoteImages.ASK, barDismissed = false, chipState = { ChipPhase.NOT_DOWNLOADED },
                onRetry = {}, onShowImages = {}, onAlwaysForSender = null, onDismissBar = {}, onChip = {}, onLink = {},
            )
        }
        val res = rule.activity.resources
        for (problem in ReadProblem.entries) {
            rule.runOnUiThread { phase = BodyPhase.Failed(problem) }
            rule.waitForIdle()
            val tag = if (problem == ReadProblem.OFFLINE) BodyTags.notDownloaded(5) else BodyTags.failure(5)
            val expected = res.getString(if (problem == ReadProblem.OFFLINE) com.example.croft.R.string.reading_not_downloaded else ReadCopy.resFor(problem))
            rule.onNodeWithTag(tag).assertTextEquals(expected)
            // Retry only where trying again can work.
            assertEquals("$problem", problem.canRetry, rule.onAllNodes(androidx.compose.ui.test.hasTestTag(BodyTags.retry(5))).fetchSemanticsNodes().isNotEmpty())
        }
    }

    @Test
    fun loadingShowsAStatus_andAnEmptyTextBodySaysThereIsNoText() {
        var phase by mutableStateOf<BodyPhase?>(BodyPhase.Loading)
        rule.show {
            MessageBodyView(
                id = 6, phase = phase, remoteSetting = RemoteImages.ASK, barDismissed = false, chipState = { ChipPhase.NOT_DOWNLOADED },
                onRetry = {}, onShowImages = {}, onAlwaysForSender = null, onDismissBar = {}, onChip = {}, onLink = {},
            )
        }
        rule.onNodeWithText("Loading the message").assertExists()
        rule.runOnUiThread { phase = BodyPhase.Ready(textBody(6, "")) }
        rule.waitForIdle()
        rule.onNodeWithText("This message has no text.").assertExists()
    }

    // ---- Attachments ----

    private fun chip(id: Long) = rule.onNodeWithTag(BodyTags.chip(NEWEST, id))

    private fun chipWord(id: Long): String = chip(id).fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription).orEmpty()

    private fun attachmentFixture(): ReadingFixture {
        val body = htmlBody(NEWEST, attachments = listOf(PDF, PROGRAM, INLINE))
        return ReadingFixture(newestBody = body, newestAllowed = body).also { it.attachmentDir = tempDir() }
    }

    @Test
    fun chipsAreLabelledWithNameSizeAndState_theInlinePictureHasNoChip() {
        val f = attachmentFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        val description = chip(PDF.id).fetchSemanticsNode().config.getOrNull(SemanticsProperties.ContentDescription)!!.joinToString()
        assertTrue(description, description.contains("plan.pdf") && description.contains("Not downloaded"))
        assertEquals("Not downloaded", chipWord(PDF.id))
        chip(PROGRAM.id).assertExists()
        assertEquals("Blocked file type", chipWord(PROGRAM.id))
        rule.onNodeWithTag(BodyTags.blockedLabel(NEWEST, PROGRAM.id)).assertTextEquals("Blocked for your safety")
        rule.onNodeWithTag(BodyTags.chip(NEWEST, INLINE.id)).assertDoesNotExist()
    }

    @Test
    fun aTapSavesTheAttachmentIntoTheSharedFolderAndOpensIt_theChipIsDownloadedThenResetsWhenTheViewerReturns() {
        val f = attachmentFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        chip(PDF.id).performClick()
        rule.waitUntil(10_000) { f.opened.isNotEmpty() }
        val (file, mime) = f.opened.single()
        assertEquals(f.attachmentDir!!.canonicalFile, file.canonicalFile.parentFile)
        assertEquals("application/pdf", mime)
        assertTrue(file.exists())
        assertEquals(listOf("saveAttachment:$NEWEST:${PDF.id}"), f.reading.calls.filter { it.startsWith("save") })
        assertEquals("Downloaded", chipWord(PDF.id))

        // The viewer returns (the app was paused and resumed): the file is deleted and the chip is "not downloaded" again.
        rule.activityRule.scenario.moveToState(Lifecycle.State.STARTED)
        rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        rule.waitUntil(10_000) { !file.exists() }
        rule.waitForIdle()
        assertEquals("Not downloaded", chipWord(PDF.id))
        assertEquals(0, f.attachmentDir!!.listFiles()!!.size)
    }

    @Test
    fun aBlockedExecutableIsNeverSavedOrOpened_theTapOnlyExplains() {
        val f = attachmentFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PROGRAM.id))
        chip(PROGRAM.id).performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Croft never opens programs or scripts from email.").assertExists()
        assertTrue(f.reading.calls.none { it.startsWith("save") })
        assertTrue(f.opened.isEmpty())
        assertEquals(0, f.attachmentDir!!.listFiles()!!.size)
    }

    @Test
    fun aFailedDownloadShowsTheFailedChip_withTheReasonInPlainWords_andATapTriesAgain() {
        val f = attachmentFixture()
        f.reading.saveFails = ReadProblem.WRITE_FAILED
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        chip(PDF.id).performClick()
        rule.waitUntil(10_000) { chipWord(PDF.id) == "Download failed" }
        rule.onNodeWithText("Croft couldn't save the file on this phone", substring = true).assertExists()
        assertTrue(f.opened.isEmpty())
        f.reading.saveFails = null
        chip(PDF.id).performClick()
        rule.waitUntil(10_000) { f.opened.isNotEmpty() }
    }

    @Test
    fun withNoViewerAppTheScreenSaysSo() {
        val f = attachmentFixture()
        f.viewerExists = false
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        chip(PDF.id).performClick()
        rule.waitUntil(10_000) { f.opened.isNotEmpty() }
        rule.waitForIdle()
        rule.onNodeWithText("No app on this phone can open this file.").assertExists()
    }

    @Test
    fun theSharedFolderIsEmptiedWhenTheScreenIsLeft() {
        val f = attachmentFixture()
        var visible by mutableStateOf(true)
        rule.show { if (visible) f.Screen(settings = MailSettings(markRead = com.example.croft.mail.reading.MarkReadDelay.NEVER)) }
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        chip(PDF.id).performClick()
        rule.waitUntil(10_000) { f.opened.isNotEmpty() }
        val file = f.opened.single().first
        assertTrue(file.exists())
        rule.runOnUiThread { visible = false }
        rule.waitUntil(10_000) { !file.exists() }
    }
}
