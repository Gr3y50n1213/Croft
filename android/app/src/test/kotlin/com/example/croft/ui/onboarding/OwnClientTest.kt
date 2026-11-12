// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import android.content.ClipboardManager
import android.content.Intent
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.Modifier
import androidx.compose.foundation.verticalScroll
import com.example.croft.CroftRobolectricRunner
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.onboarding.OAuthBackend
import com.example.croft.mail.onboarding.OAuthPhase
import com.example.croft.mail.onboarding.OAuthProviderChoice
import com.example.croft.mail.onboarding.OAuthSignIn
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import uniffi.croft_ffi.OAuthFailure

/** The core's OAuth calls, scripted. Every value is made up. */
class FakeOAuth : OAuthBackend {
    val begun = CopyOnWriteArrayList<String>()
    val cancelled = CopyOnWriteArrayList<String>()
    var secretSeen: String? = null
    var finish = CompletableDeferred<ByteArray>()
    var failWith: OAuthFailure? = null

    override suspend fun begin(provider: OAuthProviderChoice, clientId: String, clientSecret: ByteArray?, email: String, donePage: String): Pair<String, String> {
        begun += "${provider.key}:$clientId:$email"
        secretSeen = clientSecret?.let { String(it) }
        return "flow-${begun.size}" to "https://accounts.google.com/o/oauth2/v2/auth?client_id=$clientId"
    }

    override suspend fun finish(flowId: String): ByteArray {
        failWith?.let { throw it }
        return finish.await()
    }

    override fun cancel(flowId: String) { cancelled += flowId; finish.completeExceptionally(OAuthFailure.Cancelled()) }

    override suspend fun rememberedClientId(provider: OAuthProviderChoice): String? = "remembered-id"
}

@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class OwnClientTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(oauth: OAuthSignIn, provider: OAuthProviderChoice, remembered: String? = null) {
        rule.setContent {
            CroftTheme {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    val phase by oauth.phase.collectAsState()
                    // On S04 the section sits in the screen's scrolling column.
                    androidx.compose.foundation.layout.Column(Modifier.verticalScroll(androidx.compose.foundation.rememberScrollState())) {
                    OwnClientSection(
                        provider = provider, phase = phase, rememberedClientId = remembered,
                        onStart = { id, secret -> oauth.start(provider, id, secret, "sam@example.org", "Done") },
                        onCancel = oauth::cancel, onEdited = oauth::clearFailure,
                    )
                    }
                }
            }
        }
    }

    private fun controller(fake: FakeOAuth, opened: MutableList<String>, signedIn: MutableList<String>) =
        OAuthSignIn(fake, CoroutineScope(SupervisorJob() + Dispatchers.Main), openTab = { opened += it }, onSignedIn = { signedIn += String(it); it.fill(0) })

    @Test
    fun theGoogleSectionTakesTheClientIdAndSecretOpensThePageAndHandsTheSignInToTheFlow() {
        val fake = FakeOAuth()
        val opened = mutableListOf<String>()
        val signedIn = mutableListOf<String>()
        show(controller(fake, opened, signedIn), OAuthProviderChoice.GOOGLE)
        rule.onNodeWithTag(OwnClientTags.SIGN_IN).performScrollTo().assertIsNotEnabled()
        rule.onNodeWithTag(OwnClientTags.CLIENT_ID).performTextInput("1234-made-up.apps.example")
        rule.onNodeWithTag(OwnClientTags.CLIENT_SECRET).performTextInput("made-up-secret")
        rule.onNodeWithTag(OwnClientTags.SIGN_IN).performScrollTo().assertIsEnabled().performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); opened.isNotEmpty() }
        assertEquals(listOf("google:1234-made-up.apps.example:sam@example.org"), fake.begun)
        assertEquals("made-up-secret", fake.secretSeen)
        assertTrue(opened.single().startsWith("https://accounts.google.com/"))
        rule.onNodeWithTag(OwnClientTags.WAITING).assertExists()
        fake.finish.complete("made-up-sign-in-secret".toByteArray())
        rule.waitUntil(5_000) { rule.waitForIdle(); signedIn.isNotEmpty() }
        assertEquals(listOf("made-up-sign-in-secret"), signedIn)
        rule.onNodeWithTag(OwnClientTags.WAITING).assertDoesNotExist()
    }

    @Test
    fun outlookHasNoSecretFieldAndItsHelpCopiesTheRedirect() {
        val fake = FakeOAuth()
        show(controller(fake, mutableListOf(), mutableListOf()), OAuthProviderChoice.MICROSOFT, remembered = "remembered-id")
        rule.onNodeWithTag(OwnClientTags.CLIENT_SECRET).assertDoesNotExist()
        rule.onNodeWithText("remembered-id").assertExists()
        rule.onNodeWithTag(OwnClientTags.HELP).performScrollTo().performClick()
        rule.onNodeWithTag(OwnClientTags.HELP_TEXT).assertExists()
        rule.onNodeWithTag(OwnClientTags.COPY).performScrollTo().performClick()
        val clip = rule.activity.getSystemService(ClipboardManager::class.java).primaryClip!!
        assertEquals("http://localhost", clip.getItemAt(0).text.toString())
    }

    @Test
    fun theGoogleHelpWarnsAboutTheSevenDayTestingLimit() {
        show(controller(FakeOAuth(), mutableListOf(), mutableListOf()), OAuthProviderChoice.GOOGLE)
        rule.onNodeWithTag(OwnClientTags.HELP).performScrollTo().performClick()
        rule.onNodeWithText("stop working after 7 days", substring = true).assertExists()
        rule.onNodeWithText("Desktop app", substring = true).assertExists()
    }

    @Test
    fun cancellingShowsNoMessage() {
        val fake = FakeOAuth()
        val oauth = controller(fake, mutableListOf(), mutableListOf())
        show(oauth, OAuthProviderChoice.MICROSOFT, remembered = "made-up-id")
        rule.waitForIdle()
        rule.onNodeWithTag(OwnClientTags.SIGN_IN).performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); rule.onAllNodes(hasTestTag(OwnClientTags.CANCEL)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(OwnClientTags.CANCEL).performScrollTo().performClick()
        rule.waitForIdle()
        assertEquals(listOf("flow-1"), fake.cancelled)
        assertEquals(OAuthPhase.Idle, oauth.phase.value)
        rule.onNodeWithTag(OwnClientTags.FAILURE).assertDoesNotExist()
    }

    @Test
    fun aStaleOrRefusedSignInShowsAPlainMessage() {
        val fake = FakeOAuth().apply { failWith = OAuthFailure.StateMismatch() }
        show(controller(fake, mutableListOf(), mutableListOf()), OAuthProviderChoice.MICROSOFT, remembered = "made-up-id")
        rule.waitForIdle()
        rule.onNodeWithTag(OwnClientTags.SIGN_IN).performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); rule.onAllNodes(hasTestTag(OwnClientTags.FAILURE)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("That sign-in page was from an earlier attempt. Start again.").assertExists()
    }

    @Test
    fun aSecondStartEndsTheFirst() {
        val fake = FakeOAuth()
        val oauth = controller(fake, mutableListOf(), mutableListOf())
        oauth.start(OAuthProviderChoice.GOOGLE, "a", null, "x@example.org", "Done")
        rule.waitUntil(5_000) { rule.waitForIdle(); fake.begun.size == 1 }
        fake.finish = CompletableDeferred()
        oauth.start(OAuthProviderChoice.GOOGLE, "b", null, "x@example.org", "Done")
        rule.waitUntil(5_000) { rule.waitForIdle(); fake.begun.size == 2 }
        assertEquals(listOf("flow-1"), fake.cancelled)
    }

    @Test
    fun theClientSecretArrayIsWipedOnceHandedOver() {
        val fake = FakeOAuth()
        val oauth = controller(fake, mutableListOf(), mutableListOf())
        val secret = "made-up-secret".toByteArray()
        oauth.start(OAuthProviderChoice.GOOGLE, "a", secret, "x@example.org", "Done")
        rule.waitUntil(5_000) { rule.waitForIdle(); fake.begun.size == 1 }
        assertArrayEquals(ByteArray(secret.size), secret)
    }

    @Test
    fun theSignInPageOpensInABrowserTabWithTheBuiltUrl() {
        val url = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=made-up"
        openInBrowserTab(rule.activity, url)
        val started: Intent = shadowOf(rule.activity).nextStartedActivity
        assertEquals(Intent.ACTION_VIEW, started.action)
        assertEquals(url, started.dataString)
        // Only an https page is ever opened.
        val refused = runCatching { openInBrowserTab(rule.activity, "http://evil.example/") }
        assertTrue(refused.isFailure)
    }

    @Test
    fun noWebViewIsUsedInTheOAuthCode() {
        // docs/ux/screens.md S04 and the store rule: the provider's page opens in the browser, never in an embedded view.
        val files = listOf(
            "src/main/kotlin/com/example/croft/mail/onboarding/OAuthSignIn.kt",
            "src/main/kotlin/com/example/croft/ui/onboarding/OwnClientSection.kt",
            "src/main/kotlin/com/example/croft/ui/onboarding/OnboardingHost.kt",
            "src/main/kotlin/com/example/croft/ui/onboarding/SignInScreen.kt",
            "src/main/kotlin/com/example/croft/ui/onboarding/OnboardingWiring.kt",
        )
        for (f in files) {
            val text = File(f).readText()
            assertTrue("$f must exist", text.isNotEmpty())
            // The code, not the comments (they say "never a WebView").
            val code = text.lines().filterNot { it.trimStart().startsWith("*") || it.trimStart().startsWith("//") || it.trimStart().startsWith("/**") }
                .joinToString("\n") { it.substringBefore("//") }
            assertTrue("$f uses a WebView", !Regex("""\bWebView\b|android\.webkit""").containsMatchIn(code))
        }
    }
}
