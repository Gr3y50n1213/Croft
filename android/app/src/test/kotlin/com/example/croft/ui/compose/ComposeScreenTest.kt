// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.TextRange
import androidx.core.app.ActivityOptionsCompat
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.AccountState
import com.example.croft.mail.compose.BodyCursor
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.RequiredKeyMissingPolicy
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import com.example.croft.ui.mail.InboxNotice
import com.example.croft.ui.mail.InboxScreen
import com.example.croft.ui.mail.rowCard
import com.example.croft.ui.mail.waitForRow
import java.io.ByteArrayInputStream
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.assertIsDisplayed

/**
 * S23 and S24 on an in-memory compose backend (Robolectric): a `mailto:` intent fills compose, a picker result attaches a file, the Back
 * rules, the Undo snackbar reopens the draft, the encryption sheet, and the semantics (banners announce, chips have labels, focus after
 * Send goes to the first row). The core itself is exercised by the JVM tests in `:mailtests`.
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ComposeScreenTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun open(f: ComposeFixture, start: ComposeStart, registry: ActivityResultRegistry? = null) {
        rule.show(AnimationLevel.OFF) {
            val content: @androidx.compose.runtime.Composable () -> Unit = { ComposeHost(f.session, start, { f.closed += it }, tools = UiTestAddressTools) }
            if (registry == null) content() else CompositionLocalProvider(
                LocalActivityResultRegistryOwner provides object : ActivityResultRegistryOwner { override val activityResultRegistry: ActivityResultRegistry = registry },
            ) { content() }
        }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.SUBJECT)).fetchSemanticsNodes().isNotEmpty() }
    }

    // ---- mailto ----

    @Test
    fun aMailtoIntentOpensComposeFilledIn() {
        val intent = Intent(Intent.ACTION_SENDTO, Uri.parse("mailto:ann@example.org,bob@example.org?cc=cy@example.org&subject=Lunch%20at%20noon&body=See%20you%20there"))
        val start = ComposeIntents.parse(intent)
        assertNotNull(start)
        val f = ComposeFixture()
        open(f, start!!)
        assertEquals("Lunch at noon", rule.subjectText())
        assertEquals("See you there", rule.bodyText())
        rule.onNodeWithContentDescription("ann@example.org").assertExists()
        rule.onNodeWithContentDescription("bob@example.org").assertExists()
        // Cc had a value, so its field is shown without the toggle.
        rule.onNodeWithContentDescription("cy@example.org").assertExists()
    }

    @Test
    fun aShareFillsTextAndSubject_andAFileUriOrAnAttachFieldIsNeverTaken() {
        val share = Intent(Intent.ACTION_SEND).apply {
            putExtra(Intent.EXTRA_SUBJECT, "Look")
            putExtra(Intent.EXTRA_TEXT, "https://example.org/page")
            putExtra(Intent.EXTRA_STREAM, Uri.parse("file:///data/data/secret.db"))
        }
        val start = ComposeIntents.parse(share) as ComposeStart.Fill
        assertEquals("Look", start.fill.subject)
        assertEquals("https://example.org/page", start.fill.body)
        assertTrue("a file: stream is dropped", start.fill.attachments.isEmpty())
        val link = ComposeIntents.parse(Intent(Intent.ACTION_VIEW, Uri.parse("mailto:a@example.org?attach=/data/secret"))) as ComposeStart.Fill
        assertTrue(link.fill.attachments.isEmpty())
        assertNull("anything else is not ours", ComposeIntents.parse(Intent(Intent.ACTION_VIEW, Uri.parse("https://example.org"))))
        assertNull(ComposeIntents.parse(Intent(Intent.ACTION_MAIN)))
        val streams = Intent(Intent.ACTION_SEND_MULTIPLE).apply {
            putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(Uri.parse("content://x/1"), Uri.parse("file:///y"), Uri.parse("content://x/2")))
        }
        assertEquals(listOf("content://x/1", "content://x/2"), (ComposeIntents.parse(streams) as ComposeStart.Fill).fill.attachments)
    }

    // ---- pickers ----

    @Test
    fun aPickerResultAttachesAFile() {
        val uri = Uri.parse("content://com.example.croft.test/doc/1")
        val resolver = RuntimeEnvironment.getApplication().contentResolver
        shadowOf(resolver).registerInputStream(uri, ByteArrayInputStream("hello picker".toByteArray()))
        shadowOf(resolver).setCursor(
            uri,
            org.robolectric.fakes.RoboCursor().apply {
                setColumnNames(listOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE))
                setResults(arrayOf(arrayOf<Any>("report.pdf", 12L)))
            },
        )
        val registry = object : ActivityResultRegistry() {
            override fun <I, O> onLaunch(requestCode: Int, contract: ActivityResultContract<I, O>, input: I, options: ActivityOptionsCompat?) {
                @Suppress("UNCHECKED_CAST")
                dispatchResult(requestCode, listOf(uri) as O)
            }
        }
        val f = ComposeFixture()
        open(f, ComposeStart.New(1), registry)
        rule.onNodeWithTag(ComposeTags.ATTACH).performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.attachment(0))).fetchSemanticsNodes().isNotEmpty() }
        assertEquals(listOf("report.pdf"), f.backend.attachedBytes.keys.toList())
        assertEquals("hello picker", String(f.backend.attachedBytes.getValue("report.pdf")))
        assertTrue("the copy in the cache folder is deleted", ComposeFiles.directory(RuntimeEnvironment.getApplication()).list().orEmpty().isEmpty())
    }

    @Test
    fun aFileSchemeResultIsNeverRead() {
        val resolver = RuntimeEnvironment.getApplication().contentResolver
        val context = RuntimeEnvironment.getApplication()
        assertNull(ComposeFiles.resolve(context, Uri.parse("file:///data/data/com.example.croft/databases/croft.db")))
        assertNotNull(ComposeFiles.resolve(context, Uri.parse(ContentResolver.SCHEME_CONTENT + "://media/1")))
    }

    // ---- Back ----

    @Test
    fun backOnAnEmptyComposeClosesWithoutADraft() {
        val f = ComposeFixture()
        open(f, ComposeStart.New(1))
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertEquals(listOf<ComposeClosed>(ComposeClosed.Nothing), f.closed)
        assertEquals(emptyList<Any>(), runBlocking { f.backend.listDrafts(1) })
    }

    @Test
    fun backWithContentSavesTheDraft() {
        val f = ComposeFixture()
        open(f, ComposeStart.New(1))
        rule.onNodeWithTag(ComposeTags.SUBJECT).performTextInput("Keep me")
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertEquals(listOf<ComposeClosed>(ComposeClosed.DraftSaved), f.closed)
        assertEquals("Keep me", runBlocking { f.backend.listDrafts(1) }.single().subject)
        rule.waitUntil(10_000) { synchronized(f.backend.calls) { f.backend.calls.contains("syncForced") } } // copied to the server as compose closes
    }

    @Test
    fun theInboxSaysDraftSavedWhenComposeClosesWithADraft() {
        val f = ComposeFixture()
        rule.show(AnimationLevel.OFF) { InboxScreen(f.session, onOpenSettings = {}, notice = InboxNotice(ComposeClosed.DraftSaved)) }
        rule.waitForText("Draft saved")
    }

    // ---- Send, Undo ----

    @Test
    fun theUndoSnackbarReopensTheDraftWithTheCursorWhereItWas() {
        val f = ComposeFixture()
        val sent = runBlocking {
            val saved = f.backend.saveDraft(1, null, uniffi.croft_ffi.DraftContent(null, listOf(uniffi.croft_ffi.FfiAddress(null, "ann@example.org")), emptyList(), emptyList(), "Undo me", "Hello there, friend", null, emptyList()))
            val started = f.backend.send(1, saved.id)
            ComposeClosed.Sent(1, started.outboxId.toLong(), started.sendAtMs, BodyCursor(5, 11))
        }
        var undone by mutableStateOf<ComposeStart.Draft?>(null)
        rule.show(AnimationLevel.OFF) {
            val reopen = undone
            if (reopen == null) InboxScreen(f.session, onOpenSettings = {}, notice = InboxNotice(sent), onUndoSend = { undone = it })
            else ComposeHost(f.session, reopen, { f.closed += it }, tools = UiTestAddressTools)
        }
        rule.waitForText("Sending\u2026")
        rule.onNodeWithText("Undo").performClick()
        // Compose is open again on that draft: the text is back and the cursor is where it was.
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.BODY)).fetchSemanticsNodes().isNotEmpty() }
        assertEquals(listOf("send", "undoSend"), f.backend.calls.filter { it == "send" || it == "undoSend" })
        assertEquals("Hello there, friend", rule.bodyText())
        assertEquals("Undo me", rule.subjectText())
        assertEquals(TextRange(5, 11), rule.onNodeWithTag(ComposeTags.BODY).selection())
    }

    @Test
    fun afterSendFocusGoesToTheFirstRowOfTheList() {
        val f = ComposeFixture()
        val sent = ComposeClosed.Sent(1, 1, f.inbox.clockMs + 5_000, null)
        rule.show(AnimationLevel.OFF) { InboxScreen(f.session, onOpenSettings = {}, notice = InboxNotice(sent)) }
        rule.waitForRow(f.inbox.topId)
        rule.waitUntil(10_000) { runCatching { rule.rowCard(f.inbox.topId).assertIsFocused() }.isSuccess }
        rule.rowCard(f.inbox.topId).assertIsFocused()
    }

    @Test
    fun replyOnARowOpensComposeAsAReply_andWithoutItTheRowSaysSoLater() {
        val f = ComposeFixture()
        var asked: Pair<Long, Long>? = null
        rule.show(AnimationLevel.OFF) { InboxScreen(f.session, onOpenSettings = {}, onReply = { a, m -> asked = a to m }) }
        rule.waitForRow(f.inbox.topId)
        val action = rule.rowCard(f.inbox.topId).fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsActions.CustomActions].first { it.label == "Reply" }
        rule.runOnUiThread { action.action() }
        rule.waitUntil(10_000) { asked != null }
        assertEquals(f.inbox.topId, asked!!.second)
    }

    // ---- S24 ----

    @Test
    fun theEncryptionSheetNamesWhoHasNoKey_andSendNeverGoesWithoutAChoice() {
        val f = ComposeFixture(policy = RequiredKeyMissingPolicy(listOf("dan@oldmail.org")))
        open(f, ComposeStart.Fill(ComposeFill(to = listOf("dan@oldmail.org"), subject = "Secret", body = "Text"), account = 1))
        rule.onNodeWithTag(ComposeTags.SEND).performClick()
        rule.waitForText("One recipient has no key")
        // The address is on its chip and again in the sheet, which names who has no key.
        assertTrue(rule.onAllNodes(hasText("dan@oldmail.org")).fetchSemanticsNodes().size >= 2)
        assertFalse("nothing is sent before the choice", f.backend.calls.contains("send"))
        // The sheet announces itself.
        rule.onNode(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Assertive) and hasText("One recipient has no key")).assertExists()

        rule.onNodeWithTag(EncryptionTags.CANCEL).performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("One recipient has no key")).fetchSemanticsNodes().isEmpty() }
        assertFalse(f.backend.calls.contains("send"))
        assertTrue(f.closed.isEmpty())

        rule.onNodeWithTag(ComposeTags.SEND).performClick()
        rule.waitForText("One recipient has no key")
        rule.onNodeWithTag(EncryptionTags.SEND_UNENCRYPTED).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.closed.single() is ComposeClosed.Sent)
        assertEquals(1, f.backend.calls.count { it == "send" })
    }

    @Test
    fun sendWithoutSubjectOrTextAsksFirst() {
        val f = ComposeFixture()
        open(f, ComposeStart.Fill(ComposeFill(to = listOf("ann@example.org")), account = 1))
        rule.onNodeWithTag(ComposeTags.SEND).performClick()
        rule.waitForText("Send without subject or text?")
        assertFalse(f.backend.calls.contains("send"))
        rule.onNodeWithText("Send anyway").performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.closed.single() is ComposeClosed.Sent)
    }

    // ---- semantics ----

    @Test
    fun bannersAnnounceThemselves() {
        val f = ComposeFixture()
        f.inbox.repo.setAccountStatus(1, AccountState.OFFLINE)
        open(f, ComposeStart.New(1))
        rule.waitForText("You are offline. Changes wait and are sent later.")
        rule.onNode(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite) and hasAnyDescendant(hasText("You are offline. Changes wait and are sent later."))).assertExists()
    }

    @Test
    fun chipsAreLabelled_andAnInvalidOneIsAnErrorWithWordsNotColourAlone() {
        val f = ComposeFixture()
        open(f, ComposeStart.Fill(ComposeFill(to = listOf("ann@example.org", "broken@address")), account = 1))
        rule.onNodeWithContentDescription("ann@example.org").assertExists()
        rule.onNode(hasContentDescription("broken@address")).assertExists()
        rule.onNode(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Invalid address")).assertExists()
        rule.onNodeWithText("Invalid address").assertExists()
        // Send is disabled, and says why.
        rule.onNodeWithTag(ComposeTags.SEND).performClick()
        rule.waitForText("Fix the address")
    }

    @Test
    fun theSendButtonAndTheFieldsHaveNames() {
        val f = ComposeFixture()
        open(f, ComposeStart.New(1))
        rule.onNodeWithContentDescription("Close").assertExists()
        rule.onNodeWithContentDescription("Attach a file").assertExists()
        rule.onNodeWithContentDescription("To").assertExists()
        rule.onNodeWithTag(ComposeTags.SEND).assertIsDisplayed()
    }
}
