// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.ClipData
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.ProviderInfo
import android.net.Uri
import android.os.BadParcelableException
import android.os.Bundle
import android.os.Parcelable
import android.provider.MediaStore
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.ViewModelProvider
import androidx.test.core.app.ActivityScenario
import com.example.croft.CroftRobolectricRunner
import com.example.croft.CroftTheme
import com.example.croft.mail.compose.ComposeClosed
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.Mailto
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.settingsAt
import com.example.croft.ui.components.show
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
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.FfiAddress

/** The intents another app can send, the provider rule, the chip's address and the window being rebuilt (the opus review of T18d). */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ComposeIntentsReviewTest {
    private fun fill(intent: Intent, context: android.content.Context? = null) = (ComposeIntents.parse(intent, context) as ComposeStart.Fill).fill

    // ---- H1: streams ----

    @Test
    fun aNullOrAnIntentInTheStreamListDoesNotCrash_andOnlyUrisCount() {
        val withNull = Intent(Intent.ACTION_SEND_MULTIPLE).putExtra(Intent.EXTRA_STREAM, arrayListOf<Parcelable?>(null) as java.io.Serializable)
        assertEquals(emptyList<String>(), fill(withNull).attachments)

        val withIntent = Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(Intent("not.a.uri")))
        assertEquals(emptyList<String>(), fill(withIntent).attachments)

        val mixed = Intent(Intent.ACTION_SEND_MULTIPLE).putExtra(
            Intent.EXTRA_STREAM, arrayListOf<Parcelable?>(null, Uri.parse("content://other.app/1"), Intent("x"), Uri.parse("file:///y")) as java.io.Serializable,
        )
        assertEquals(listOf("content://other.app/1"), fill(mixed).attachments)

        // A single stream that is not a Uri.
        val single = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, Intent("not.a.uri") as Parcelable)
        assertEquals(emptyList<String>(), fill(single).attachments)
    }

    // ---- H2: recipient arrays ----

    @Test
    fun aNullEntryInTheRecipientArraysDoesNotCrash() {
        val intent = Intent(Intent.ACTION_SEND)
            .putExtra(Intent.EXTRA_EMAIL, arrayOf<String?>(null, "a@example.org", null))
            .putExtra(Intent.EXTRA_CC, arrayOf<String?>(null))
            .putExtra(Intent.EXTRA_BCC, "single@example.org")
        val f = fill(intent)
        assertEquals(listOf("a@example.org"), f.to)
        assertEquals(emptyList<String>(), f.cc)
        assertEquals(listOf("single@example.org"), f.bcc)
    }

    @Test
    fun theRecipientArraysAreCapped() {
        val many = Array<String?>(5_000) { "u$it@example.org" }
        assertEquals(Mailto.MAX_RECIPIENTS, fill(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_EMAIL, many)).to.size)
    }

    // ---- H3: a class this app does not have ----

    @Test
    fun anIntentWhoseExtrasCannotBeUnparcelledOpensAnEmptyCompose() {
        val bad = object : Intent(Intent.ACTION_SEND) {
            override fun getExtras(): Bundle? = throw BadParcelableException("ClassNotFoundException when unmarshalling")
            override fun getStringExtra(name: String?): String? = throw BadParcelableException("x")
            override fun getClipData(): ClipData? = throw BadParcelableException("x")
        }
        assertEquals(ComposeStart.Fill(ComposeFill()), ComposeIntents.parse(bad))
        val badMultiple = object : Intent(Intent.ACTION_SEND_MULTIPLE) {
            override fun getExtras(): Bundle? = throw ClassCastException("x")
        }
        assertEquals(ComposeStart.Fill(ComposeFill()), ComposeIntents.parse(badMultiple))
        val badLink = object : Intent(Intent.ACTION_SENDTO, Uri.parse("mailto:a@example.org")) {
            override fun getExtras(): Bundle? = throw BadParcelableException("x")
        }
        val start = ComposeIntents.parse(badLink) as ComposeStart.Fill
        assertEquals(listOf("a@example.org"), start.fill.to)
        // What is not ours is still not ours.
        assertNull(ComposeIntents.parse(Intent(Intent.ACTION_MAIN)))
    }

    @Test
    fun aTextThatWasCutIsFlaggedForTheScreen() {
        val f = fill(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_TEXT, "y".repeat(Mailto.MAX_BODY + 1)))
        assertEquals(Mailto.MAX_BODY, f.body.length)
        assertTrue(f.truncated)
    }

    // ---- M1: only other apps' providers ----

    private fun provider(authority: String, pkg: String, uid: Int) = ProviderInfo().apply {
        this.authority = authority
        name = "Provider"
        packageName = pkg
        applicationInfo = ApplicationInfo().apply { packageName = pkg; this.uid = uid }
    }

    @Test
    fun contentOfCroftsOwnProviderIsRefused_andAnotherAppsIsAccepted() {
        val context = RuntimeEnvironment.getApplication()
        val packages = shadowOf(context.packageManager)
        packages.addOrUpdateProvider(provider("com.example.croft.files", context.packageName, android.os.Process.myUid()))
        // The other app is installed first: a provider needs its package.
        packages.installPackage(
            android.content.pm.PackageInfo().apply {
                packageName = "com.other.app"
                applicationInfo = ApplicationInfo().apply { packageName = "com.other.app"; uid = 12_345 }
            },
        )
        packages.addOrUpdateProvider(provider("com.other.app.files", "com.other.app", 12_345))

        val own = Uri.parse("content://com.example.croft.files/shared/secret.db")
        assertFalse(ComposeFiles.isForeign(context, own))
        assertNull(ComposeFiles.resolve(context, own))
        assertNull(ComposeFiles.resolve(context, own.toString()))
        // A user id in front of the authority changes nothing.
        assertFalse(ComposeFiles.isForeign(context, Uri.parse("content://0@com.example.croft.files/shared/secret.db")))
        assertNull(ComposeFiles.resolve(context, "content://10@com.example.croft.files/x"))

        val other = Uri.parse("content://com.other.app.files/doc/1")
        assertTrue(ComposeFiles.isForeign(context, other))
        assertNotNull(ComposeFiles.resolve(context, other))

        // The intent drops the stream of the own provider when it is given a context, and keeps the other one.
        val share = Intent(Intent.ACTION_SEND_MULTIPLE).putExtra(Intent.EXTRA_STREAM, arrayListOf<Parcelable?>(own, other) as java.io.Serializable)
        assertEquals(listOf(other.toString()), fill(share, context).attachments)
    }

    @Test
    fun aMediaStoreItemCroftSavedItselfIsRefused() {
        val context = RuntimeEnvironment.getApplication()
        val mine = Uri.parse("content://media/external/images/media/1")
        val theirs = Uri.parse("content://media/external/images/media/2")
        val unknown = Uri.parse("content://media/external/images/media/3")
        // What MediaStore says about who owns the item (`OWNER_PACKAGE_NAME`).
        val owner: (android.content.Context, Uri) -> String? = { _, uri ->
            when (uri) { mine -> context.packageName; theirs -> "com.other.app"; else -> null }
        }
        assertFalse(ComposeFiles.isForeign(context, mine, owner))
        assertTrue(ComposeFiles.isForeign(context, theirs, owner))
        assertTrue("an owner that cannot be told is accepted (documented gap)", ComposeFiles.isForeign(context, unknown, owner))
        // A user id in front of the authority changes nothing.
        assertFalse(ComposeFiles.isForeign(context, Uri.parse("content://0@media/external/images/media/1"), { _, _ -> context.packageName }))
        // Other authorities never ask MediaStore.
        assertTrue(ComposeFiles.isForeign(context, Uri.parse("content://com.other.app.files/x"), { _, _ -> error("not asked") }))
    }

    @Test
    fun aFileUriAndANonsenseUriAreNeverForeignContent() {
        val context = RuntimeEnvironment.getApplication()
        assertFalse(ComposeFiles.isForeign(context, Uri.parse("file:///data/data/x")))
        assertFalse(ComposeFiles.isForeign(context, Uri.parse("content:///no-authority")))
        assertFalse(ComposeFiles.isForeign(context, Uri.parse("https://example.org/x")))
    }
}

/** The compose screen: the chip's address, the notices, Undo on removing a file, and a rebuilt window. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ComposeReviewUiTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun open(f: ComposeFixture, start: ComposeStart) {
        rule.show(AnimationLevel.OFF) { ComposeHost(f.session, start, { f.closed += it }, tools = UiTestAddressTools) }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.SUBJECT)).fetchSemanticsNodes().isNotEmpty() }
    }

    // ---- M2 ----

    @Test
    fun aChipShowsTheAddressBehindAName() {
        val f = ComposeFixture()
        // The link of the review: a name that reads as another address.
        val link = Mailto.parse("mailto:%22boss%40corp.com%22%20%3Cattacker%40evil.example%3E?subject=Hi")!!
        open(f, ComposeStart.Fill(link, 1))
        rule.onNode(hasContentDescription("attacker@evil.example", substring = true)).assertExists()
        rule.onNodeWithText("boss@corp.com <attacker@evil.example>").assertExists()
    }

    // ---- M6 ----

    @Test
    fun anAddressTypedInToAndNotYetAChipIsSentWithTheMessage() {
        val f = ComposeFixture()
        open(f, ComposeStart.New(1))
        rule.onNodeWithContentDescription("To").performTextInput("ann@example.org")
        rule.onNodeWithTag(ComposeTags.SUBJECT).performTextInput("Typed")
        // The button counts the typed address as a recipient.
        rule.waitUntil(10_000) { runCatching { rule.onNodeWithTag(ComposeTags.SEND).assertIsEnabled() }.isSuccess }
        rule.onNodeWithTag(ComposeTags.SEND).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.closed.single() is ComposeClosed.Sent)
        assertEquals(listOf("ann@example.org"), runBlocking { f.backend.loadDraft(1, 1) }.content.to.map { it.address })
    }

    // ---- L3 ----

    @Test
    fun aCutTextIsSaidOnce() {
        val f = ComposeFixture()
        open(f, ComposeStart.Fill(ComposeFill(body = "z".repeat(10), truncated = true), 1))
        rule.waitForText("The shared text was too long, so it was cut.")
    }

    // ---- L8 ----

    @Test
    fun removingAnAttachmentCanBeUndone() {
        val f = ComposeFixture()
        val file = java.io.File.createTempFile("croft-ui", ".bin").also { it.deleteOnExit(); it.writeBytes(ByteArray(10)) }
        val start = runBlocking {
            val saved = f.backend.saveDraft(1, null, DraftContent(null, listOf(FfiAddress(null, "ann@example.org")), emptyList(), emptyList(), "S", "B", null, emptyList()))
            f.backend.addAttachmentFile(1, saved.id, file.path, "plan.pdf", "application/pdf")
            ComposeStart.Draft(1, saved.id)
        }
        open(f, start)
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.attachment(0))).fetchSemanticsNodes().isNotEmpty() }
        // The chip is below the body in a scrolling column: scroll to it, or the tap lands outside the window.
        rule.onNodeWithTag(ComposeTags.attachment(0)).performScrollTo().performClick()
        rule.waitForText("Attachment removed")
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.attachment(0))).fetchSemanticsNodes().isEmpty() }
        assertFalse("not removed for good yet", f.backend.calls.contains("removeAttachment"))
        rule.onNodeWithText("Undo").performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.attachment(0))).fetchSemanticsNodes().isNotEmpty() }
        assertFalse(f.backend.calls.contains("removeAttachment"))
    }

    // ---- L5 ----

    @Test
    fun aLinkWhileComposeIsOpenSavesWhatIsWrittenThenOpensTheNewOne() {
        val f = ComposeFixture()
        var start by mutableStateOf<ComposeStart>(ComposeStart.New(1))
        rule.show(AnimationLevel.OFF) { ComposeHost(f.session, start, { f.closed += it }, tools = UiTestAddressTools) }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.SUBJECT)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(ComposeTags.SUBJECT).performTextInput("Half written")

        // What the navigation does for a link or a share that arrives while compose is open.
        val next = ComposeStart.Fill(ComposeFill(to = listOf("new@example.org"), subject = "From a link"), 1)
        rule.runOnUiThread {
            ViewModelProvider(rule.activity)[ComposeRetained::class.java].replaceWith(next)
            start = next
        }
        rule.waitForText("Draft saved")
        rule.waitUntil(10_000) { rule.subjectText() == "From a link" }
        assertEquals("the old draft was saved", listOf("Half written"), runBlocking { f.backend.listDrafts(1) }.map { it.subject })
        assertTrue("the old screen did not end compose", f.closed.isEmpty())
    }
}

/** M4: a rotation (the activity is rebuilt) keeps compose open with what was typed. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class ComposeRotationTest {
    @get:Rule
    val rule = createEmptyComposeRule()

    @Test
    fun aRebuiltWindowKeepsComposeOpenWithItsContent() {
        val f = ComposeFixture()
        val start: ComposeStart = ComposeStart.New(1)
        val content: @androidx.compose.runtime.Composable () -> Unit = {
            CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) {
                ComposeHost(f.session, start, { f.closed += it }, tools = UiTestAddressTools)
            }
        }
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent(content = content) }
            rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.SUBJECT)).fetchSemanticsNodes().isNotEmpty() }
            rule.onNodeWithTag(ComposeTags.SUBJECT).performTextInput("Typed before the turn")
            rule.onNodeWithTag(ComposeTags.BODY).performTextInput("Body before the turn")
            val identitiesBefore = synchronized(f.backend.calls) { f.backend.calls.count { it == "identities" } }

            scenario.recreate()
            scenario.onActivity { it.setContent(content = content) }
            rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ComposeTags.BODY)).fetchSemanticsNodes().isNotEmpty() }
            assertEquals("Typed before the turn", rule.subjectText())
            assertEquals("Body before the turn", rule.bodyText())
            assertTrue("compose was not closed by the rebuild", f.closed.isEmpty())
            assertEquals("and it was not opened a second time", identitiesBefore, synchronized(f.backend.calls) { f.backend.calls.count { it == "identities" } })
        }
    }
}
