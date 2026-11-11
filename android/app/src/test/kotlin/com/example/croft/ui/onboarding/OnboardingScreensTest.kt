// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import com.example.croft.CroftRobolectricRunner
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.mail.onboarding.ProbeRow
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ThemeSettings
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.DiscoveryProbe
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.ProbeFailureReason
import uniffi.croft_ffi.ProbeProgress
import uniffi.croft_ffi.VerifyFailure

/**
 * The sign-in screens S01 to S05 and S09 as a person uses them, over a scripted backend (no network): the core's answers are what
 * each test says they are. The words in the assertions are the strings of `strings_onboarding.xml`. The flow over the real core is
 * tested in `:mailtests` (`OnboardingFlowTest`).
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class OnboardingScreensTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val backend = FakeBackend()
    private val firstSync = FakeFirstSync()
    private val finished = CopyOnWriteArrayList<Long>()
    private var sampleTaps = 0

    private fun show(restoration: StateRestorationTester? = null) {
        val env = OnboardingEnvironment(backend, firstSync) { fakeSession() }
        val content: @androidx.compose.runtime.Composable () -> Unit = {
            CroftTheme(ThemeSettings.Default.copy(animation = AnimationLevel.OFF), systemDark = false) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    OnboardingHost(env, onTrySampleMail = { sampleTaps++ }, onFinished = { finished += it })
                }
            }
        }
        if (restoration != null) restoration.setContent(content) else rule.setContent(content)
    }

    private fun tag(t: String) = rule.onNodeWithTag(t)
    private fun exists(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun hasWords(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun waitTag(t: String) = rule.waitUntil(10_000) { exists(t) }
    private fun waitWords(text: String) = rule.waitUntil(10_000) { hasWords(text) }

    /** A tap on something in a screen's scrolling column: scrolled into view first, as a person does in a small window. */
    private fun SemanticsNodeInteraction.tap() = performScrollTo().performClick()

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    private fun toAddAccount() { tag(OnboardingTags.ADD_BUTTON).tap(); waitTag(OnboardingTags.ADD_ACCOUNT) }

    /** S01 to S02, the address, Continue. The lookup may already be over when this returns (a scripted answer is instant), so the caller waits for the screen it expects. */
    private fun toLookup(email: String = "sam@example.org") {
        toAddAccount()
        tag(OnboardingTags.EMAIL).performTextInput(email)
        tag(OnboardingTags.CONTINUE).tap()
    }

    private fun toSignIn(provider: KnownProvider? = null) {
        backend.onDiscover = { e, _ -> fakeFound(e, provider) }
        toLookup()
        waitTag(OnboardingTags.SIGN_IN)
    }

    private fun typePassword(tagName: String = OnboardingTags.PASSWORD, text: String = "made-up-pw-1234") {
        tag(tagName).performScrollTo().performTextInput(text)
    }

    private fun row(r: ProbeRow) = tag(OnboardingTags.probeRow(r))

    // ---- S01 ----

    @Test
    fun theWelcomeScreenOffersAddingAnAccountAndTheSampleMail_andNoCompanionButton() {
        show()
        tag(OnboardingTags.WELCOME).assertIsDisplayed()
        tag(OnboardingTags.ADD_BUTTON).assertIsDisplayed()
        tag(OnboardingTags.SAMPLE_BUTTON).performScrollTo().assertIsDisplayed()
        // Hidden, not disabled: there is no such node at all (no store flag exists yet, so it is hidden in every build).
        assertFalse(exists(OnboardingTags.COMPANION_BUTTON))
        assertFalse(hasWords("Companion"))
        tag(OnboardingTags.SAMPLE_BUTTON).tap()
        assertEquals(1, sampleTaps)
    }

    @Test
    fun backOnTheFirstScreenLeavesTheApp() {
        show()
        tag(OnboardingTags.WELCOME).assertIsDisplayed()
        back()
        assertTrue(rule.activity.isFinishing)
    }

    // ---- S02 ----

    @Test
    fun continueIsDisabledUntilTheAddressIsValid_checkedOnLeavingTheFieldAndThenLive() {
        show()
        toAddAccount()
        tag(OnboardingTags.CONTINUE).assertIsNotEnabled()
        tag(OnboardingTags.EMAIL).performTextInput("sam")
        tag(OnboardingTags.CONTINUE).assertIsNotEnabled()
        tag(OnboardingTags.EMAIL).performTextReplacement("sam@example")
        tag(OnboardingTags.CONTINUE).assertIsNotEnabled()
        // No error while typing.
        assertFalse(hasWords("Enter your full email address"))
        // Leaving the field (the keyboard's Done) checks it.
        tag(OnboardingTags.EMAIL).performImeAction()
        waitWords("Enter your full email address")
        tag(OnboardingTags.CONTINUE).assertIsNotEnabled()
        // After the first error it updates live: the message goes as soon as the address is right, and Continue opens up.
        tag(OnboardingTags.EMAIL).performTextReplacement("sam@example.org")
        rule.waitForIdle()
        assertFalse(hasWords("Enter your full email address"))
        tag(OnboardingTags.CONTINUE).assertIsEnabled()
    }

    @Test
    fun theShortcutsFillInTheProvidersDomain() {
        show()
        toAddAccount()
        tag(OnboardingTags.EMAIL).performTextInput("sam")
        tag(OnboardingTags.shortcut("gmail")).tap()
        tag(OnboardingTags.EMAIL).assert(hasText("sam@gmail.com"))
        tag(OnboardingTags.CONTINUE).assertIsEnabled()
        tag(OnboardingTags.shortcut("outlook")).tap()
        tag(OnboardingTags.EMAIL).assert(hasText("sam@outlook.com"))
        tag(OnboardingTags.shortcut("icloud")).tap()
        tag(OnboardingTags.EMAIL).assert(hasText("sam@icloud.com"))
        tag(OnboardingTags.shortcut("other")).assertIsDisplayed()
    }

    // ---- Back through S02 to S05 ----

    @Test
    fun backGoesOneStepBackThroughTheScreens() {
        show()
        // S02 -> S03 (held): Back cancels the lookup and returns to S02.
        backend.discoverGate = CompletableDeferred()
        toLookup()
        waitTag(OnboardingTags.FINDING)
        back()
        waitTag(OnboardingTags.ADD_ACCOUNT)
        // S03 -> S04: Back from S04 is the address screen (the finished lookup is not shown again).
        backend.discoverGate = null
        tag(OnboardingTags.CONTINUE).tap()
        waitTag(OnboardingTags.SIGN_IN)
        back()
        waitTag(OnboardingTags.ADD_ACCOUNT)
        // Nothing found: S05, and Back from there is S02 as well.
        backend.onDiscover = { _, _ -> throw DiscoverException.NothingFound() }
        tag(OnboardingTags.CONTINUE).tap()
        waitTag(OnboardingTags.SETTINGS)
        back()
        waitTag(OnboardingTags.ADD_ACCOUNT)
        // S02 -> S01 -> out of the app.
        back()
        waitTag(OnboardingTags.WELCOME)
        assertFalse(rule.activity.isFinishing)
        back()
        assertTrue(rule.activity.isFinishing)
    }

    @Test
    fun backFromServerSettingsOpenedOnSignInReturnsToSignIn() {
        show()
        toSignIn()
        tag(OnboardingTags.SIGN_IN_SETTINGS).tap()
        waitTag(OnboardingTags.SETTINGS)
        back()
        waitTag(OnboardingTags.SIGN_IN)
    }

    // ---- S03 ----

    @Test
    fun theProbeRowsShowWaitingCheckingFoundSkippedAndNoMatch() {
        show()
        var observer: DiscoveryObserver? = null
        backend.onDiscover = { e, o -> observer = o; fakeFound(e) }
        backend.discoverGate = CompletableDeferred()
        toLookup()
        waitTag(OnboardingTags.FINDING)
        rule.waitUntil(10_000) { observer != null }
        // Before any word from the core every row waits.
        ProbeRow.entries.forEach { row(it).assertTextContains("waiting") }
        observer!!.onProbe(DiscoveryProbe.PROVIDER_TABLE, ProbeProgress.Failed(ProbeFailureReason.NOT_PUBLISHED))
        observer!!.onProbe(DiscoveryProbe.AUTOCONFIG_SUBDOMAIN, ProbeProgress.Running)
        observer!!.onProbe(DiscoveryProbe.SRV, ProbeProgress.Skipped)
        rule.waitForIdle()
        row(ProbeRow.BUILT_IN_TABLE).assertTextContains("no match")
        row(ProbeRow.AUTOCONFIG).assertTextContains("checking")
        row(ProbeRow.SRV).assertTextContains("skipped")
        row(ProbeRow.JMAP_WELL_KNOWN).assertTextContains("waiting")
        // The two autoconfig places share a row: found in one makes the row found.
        observer!!.onProbe(DiscoveryProbe.AUTOCONFIG_WELL_KNOWN, ProbeProgress.Found)
        rule.waitForIdle()
        row(ProbeRow.AUTOCONFIG).assertTextContains("found")
        // The first match wins: the screen goes on to the sign-in.
        backend.discoverGate!!.complete(Unit)
        waitTag(OnboardingTags.SIGN_IN)
    }

    @Test
    fun offlineShowsTheFailureWithWaysOutAndNeverHangs() {
        show()
        backend.onDiscover = { _, _ -> throw DiscoverException.Offline() }
        toLookup()
        waitTag(OnboardingTags.FINDING_FAILURE)
        waitWords("Check that you are online")
        tag(OnboardingTags.FINDING_RETRY).performScrollTo().assertIsDisplayed()
        tag(OnboardingTags.FINDING_MANUAL).performScrollTo().assertIsDisplayed()
        // No raw code on the surface.
        assertFalse(hasWords("discover.offline"))
        // Details opens the technical lines, behind a link.
        rule.onNode(hasText("Details")).tap()
        waitWords("No more detail")
        // Try again, now online: on to the sign-in.
        backend.onDiscover = { e, _ -> fakeFound(e) }
        tag(OnboardingTags.FINDING_RETRY).tap()
        waitTag(OnboardingTags.SIGN_IN)
    }

    @Test
    fun nothingFoundGoesToServerSettingsFilledInWithWhatIsKnown() {
        show()
        backend.onDiscover = { _, _ -> throw DiscoverException.NothingFound() }
        toLookup("sam@example.org")
        waitTag(OnboardingTags.SETTINGS)
        waitWords("These are a guess")
        tag(OnboardingTags.HOST).assert(hasText("mail.example.org"))
        tag(OnboardingTags.SETTINGS_USERNAME).assert(hasText("sam@example.org"))
        tag(OnboardingTags.PORT).assert(hasText("993"))
    }

    // ---- S04 ----

    @Test
    fun aWrongPasswordShowsInlineInPlainWords_withNoDialog() {
        show()
        toSignIn()
        backend.onVerify = { _, _, _ -> throw VerifyFailure.WrongPassword() }
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitWords("did not accept that address and password")
        tag(OnboardingTags.SIGN_IN).assertIsDisplayed()
        assertFalse("no dialog for a wrong password", exists(OnboardingTags.TRUST_DIALOG))
        assertTrue("nothing was added", backend.adds.isEmpty())
        // Editing the password clears the message.
        tag(OnboardingTags.PASSWORD).performTextInput("x")
        rule.waitForIdle()
        assertFalse(hasWords("did not accept that address and password"))
    }

    @Test
    fun gmailAndOutlookSayTheyNeedAnAppPassword_andThereIsNoOAuthButtonYet() {
        show()
        toSignIn(KnownProvider.GMAIL)
        tag(OnboardingTags.PROVIDER_HINT).assertTextContains("app password", substring = true)
        tag(OnboardingTags.PROVIDER_HINT).assertTextContains("OAuth client", substring = true)
        assertFalse(hasWords("Sign in with Google"))
        assertFalse(hasWords("Sign in with Microsoft"))
    }

    @Test
    fun anUntrustedCertificateAsksOnce_thenTrustingAddsTheAccountAndGoesToTheFirstSync() {
        show()
        toSignIn()
        backend.onVerify = { _, _, pin -> if (pin == null) throw VerifyFailure.UntrustedCertificate(FAKE_FINGERPRINT) }
        typePassword(text = "made-up-pw-1234")
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.TRUST_DIALOG)
        tag(OnboardingTags.TRUST_FINGERPRINT).assertTextContains(FAKE_FINGERPRINT)
        waitWords("mail.example.org")
        assertTrue("nothing is added before the person trusts it", backend.adds.isEmpty())
        tag(OnboardingTags.TRUST_CONFIRM).performClick()
        waitTag(OnboardingTags.FIRST_SYNC)
        // Verified without the pin, then again with it; the account was added with it, and with the same password.
        assertEquals(listOf<Pair<String?, String>>(null to "made-up-pw-1234", FAKE_FINGERPRINT to "made-up-pw-1234"), backend.verifies.toList())
        assertEquals(listOf<Pair<String?, String>>(FAKE_FINGERPRINT to "made-up-pw-1234"), backend.adds.toList())
        assertFalse(exists(OnboardingTags.TRUST_DIALOG))
    }

    @Test
    fun cancellingTheTrustPromptAddsNothing() {
        show()
        toSignIn()
        backend.onVerify = { _, _, _ -> throw VerifyFailure.UntrustedCertificate(FAKE_FINGERPRINT) }
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.TRUST_DIALOG)
        tag(OnboardingTags.TRUST_CANCEL).performClick()
        rule.waitForIdle()
        assertFalse(exists(OnboardingTags.TRUST_DIALOG))
        tag(OnboardingTags.SIGN_IN).assertIsDisplayed()
        assertTrue(backend.adds.isEmpty())
        assertEquals("only the first check ran", 1, backend.verifies.size)
    }

    @Test
    fun aChangedCertificateAsksAgainWithAStrongerWarning() {
        show()
        toSignIn()
        backend.onVerify = { _, _, _ -> throw VerifyFailure.CertificateChanged(FAKE_FINGERPRINT) }
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.TRUST_DIALOG)
        waitWords("certificate changed")
        waitWords("someone may be intercepting")
        tag(OnboardingTags.TRUST_FINGERPRINT).assertTextContains(FAKE_FINGERPRINT)
        tag(OnboardingTags.TRUST_CONFIRM).assertIsDisplayed()
    }

    @Test
    fun everyOtherSignInFailureShowsItsOwnPlainMessageWithDetailsBehindALink() {
        show()
        toSignIn()
        backend.onVerify = { _, _, _ -> throw VerifyFailure.CannotConnect() }
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.SIGN_IN_FAILURE)
        waitWords("Couldn't connect to mail.example.org. Check the address, or try again.")
        rule.onNode(hasText("Details")).tap()
        waitWords("port 993")
    }

    @Test
    fun theSignInSurvivesTheScreenBeingRecreated_withoutThePassword() {
        val restoration = StateRestorationTester(rule)
        show(restoration)
        toSignIn()
        typePassword()
        restoration.emulateSavedInstanceStateRestore()
        waitTag(OnboardingTags.SIGN_IN)
        // The path and what was found came back; the password did not.
        tag(OnboardingTags.SIGN_IN_BUTTON).assertIsNotEnabled()
        tag(OnboardingTags.USERNAME).assert(hasText("sam@example.org"))
    }

    // ---- S05 ----

    private fun toServerSettings() {
        backend.onDiscover = { _, _ -> throw DiscoverException.NothingFound() }
        toLookup()
        waitTag(OnboardingTags.SETTINGS)
    }

    @Test
    fun testConnectionShowsItsStepsInOrderAndStopsAtTheOneThatFailed() {
        show()
        toServerSettings()
        typePassword(OnboardingTags.SETTINGS_PASSWORD)
        backend.verifyGate = CompletableDeferred()
        backend.onVerify = { _, _, _ -> throw VerifyFailure.CertificateInvalid() }
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitTag(OnboardingTags.testStep("REACH_SERVER"))
        // While the call works: the first step runs, the others wait.
        tag(OnboardingTags.testStep("REACH_SERVER")).assertTextContains("checking")
        tag(OnboardingTags.testStep("CERTIFICATE")).assertTextContains("waiting")
        tag(OnboardingTags.testStep("SIGN_IN")).assertTextContains("waiting")
        backend.verifyGate!!.complete(Unit)
        // Failed at the certificate: reaching the server worked, signing in never ran.
        waitWords("certificate of mail.example.org is expired")
        tag(OnboardingTags.testStep("REACH_SERVER")).assertTextContains("done")
        tag(OnboardingTags.testStep("CERTIFICATE")).assertTextContains("failed")
        tag(OnboardingTags.testStep("SIGN_IN")).assertTextContains("waiting")
        // Fixed on the server: all three steps are done and nothing was added.
        backend.verifyGate = null
        backend.onVerify = { _, _, _ -> }
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitWords("The connection works.")
        listOf("REACH_SERVER", "CERTIFICATE", "SIGN_IN").forEach { tag(OnboardingTags.testStep(it)).assertTextContains("done") }
        assertTrue(backend.adds.isEmpty())
    }

    @Test
    fun advancedOptionsStayHiddenUntilAskedFor_andThereIsNoUnencryptedChoice() {
        show()
        toServerSettings()
        assertFalse("the outgoing server is behind Advanced", exists(OnboardingTags.OUT_HOST))
        tag(OnboardingTags.ADVANCED).tap()
        waitTag(OnboardingTags.OUT_HOST)
        assertTrue(hasWords("SSL/TLS"))
        assertTrue(hasWords("STARTTLS"))
        assertTrue(hasWords("only connects over an encrypted connection"))
        assertFalse(hasWords("None"))
        assertFalse(hasWords("Unencrypted"))
    }

    @Test
    fun aBadServerNameIsReportedBesideTheField() {
        show()
        toServerSettings()
        typePassword(OnboardingTags.SETTINGS_PASSWORD)
        tag(OnboardingTags.HOST).performTextReplacement("not a host")
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitWords("That is not a server name")
        assertTrue("nothing was checked", backend.verifies.isEmpty())
    }

    @Test
    fun theSecurityChoicesAreDisabledWhileATestRuns() {
        show()
        toServerSettings()
        typePassword(OnboardingTags.SETTINGS_PASSWORD)
        backend.verifyGate = CompletableDeferred()
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitTag(OnboardingTags.testStep("REACH_SERVER"))
        rule.onNode(hasText("STARTTLS")).assertIsNotEnabled()
        rule.onNode(hasText("SSL/TLS")).assertIsNotEnabled()
        backend.verifyGate!!.complete(Unit)
        waitWords("The connection works.")
        rule.onNode(hasText("STARTTLS")).assertIsEnabled()
    }

    // ---- H1: a big provider's server never takes a pin ----

    /** Mirrors the core's rule (`verify` in `core/protocols`): settings that carry a provider never offer a pin, they are intercepted. */
    private fun mirrorTheCoreRule() {
        backend.onVerify = { d, _, pin ->
            if (d.provider != null) throw VerifyFailure.ConnectionIntercepted()
            if (pin == null) throw VerifyFailure.UntrustedCertificate(FAKE_FINGERPRINT)
        }
    }

    @Test
    fun aGmailAccountSwitchedToStartTlsOnServerSettingsStillShowsInterceptedAndNeverATrustPrompt() {
        show()
        // Scripted backend that mirrors the core's rule (a provider's settings are never trusted): the core itself is tested in Rust.
        mirrorTheCoreRule()
        backend.onDiscover = { e, _ -> fakeFound(e, KnownProvider.GMAIL, host = "imap.gmail.com") }
        toLookup("sam@gmail.com")
        waitTag(OnboardingTags.SIGN_IN)
        // First sign-in on an intercepted network: no trust offered.
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitWords("talking to your email provider")
        assertFalse(exists(OnboardingTags.TRUST_DIALOG))
        // Then Server settings, STARTTLS (port 143): the provider must stay on the settings.
        tag(OnboardingTags.SIGN_IN_SETTINGS).tap()
        waitTag(OnboardingTags.SETTINGS)
        rule.onNode(hasText("STARTTLS")).performScrollTo().performClick()
        rule.waitForIdle()
        tag(OnboardingTags.PORT).assert(hasText("143"))
        typePassword(OnboardingTags.SETTINGS_PASSWORD)
        backend.verifies.clear()
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitWords("talking to your email provider")
        assertFalse("never a trust prompt for imap.gmail.com", exists(OnboardingTags.TRUST_DIALOG))
        assertEquals(1, backend.verifies.size)
        assertEquals(KnownProvider.GMAIL, backend.lastVerified!!.provider)
        assertEquals(143.toUShort(), backend.lastVerified!!.imap!!.port)
    }

    @Test
    fun aProviderHostTypedByHandAfterNothingWasFoundStillCarriesItsProvider() {
        show()
        mirrorTheCoreRule()
        toServerSettings()
        typePassword(OnboardingTags.SETTINGS_PASSWORD)
        tag(OnboardingTags.HOST).performTextReplacement("IMAP.Gmail.com")
        tag(OnboardingTags.TEST_BUTTON).tap()
        waitWords("talking to your email provider")
        assertFalse(exists(OnboardingTags.TRUST_DIALOG))
        assertEquals(KnownProvider.GMAIL, backend.lastVerified!!.provider)
    }

    // ---- S09 ----

    @Test
    fun theFirstSyncSaysFinishedOnlyOnce_whateverReachesItFirst() {
        show()
        toSignIn()
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.FIRST_SYNC)
        tag(OnboardingTags.SYNC_CONTINUE).tap()
        tag(OnboardingTags.SYNC_CONTINUE).tap()
        firstSync.status = FirstSyncStatus(inboxMessages = 50, done = true, failed = false)
        back()
        rule.waitUntil(10_000) { finished.isNotEmpty() }
        rule.mainClock.advanceTimeBy(5_000)
        rule.waitForIdle()
        assertEquals("the Inbox's session is built once", listOf(7L), finished.toList())
    }

    @Test
    fun theFirstSyncShowsSkeletonsAndContinueInBackgroundIsAlwaysThere() {
        show()
        toSignIn()
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.FIRST_SYNC)
        tag(OnboardingTags.SYNC_SKELETON).performScrollTo().assertIsDisplayed()
        tag(OnboardingTags.SYNC_CONTINUE).performScrollTo().assertIsDisplayed()
        assertTrue("not finished while the sync runs", finished.isEmpty())
        tag(OnboardingTags.SYNC_CONTINUE).tap()
        rule.waitUntil(10_000) { finished.isNotEmpty() }
        assertEquals(listOf(7L), finished.toList())
    }

    @Test
    fun theFirstSyncGoesOnToTheInboxByItselfWhenItIsDone() {
        firstSync.status = FirstSyncStatus(inboxMessages = 50, done = true, failed = false)
        show()
        toSignIn()
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        rule.waitUntil(15_000) { finished.isNotEmpty() }
        assertEquals(listOf(7L), finished.toList())
    }

    @Test
    fun backOnTheFirstSyncIsContinueInBackground() {
        show()
        toSignIn()
        typePassword()
        tag(OnboardingTags.SIGN_IN_BUTTON).tap()
        waitTag(OnboardingTags.FIRST_SYNC)
        back()
        rule.waitUntil(10_000) { finished.isNotEmpty() }
        assertFalse("it did not leave the app", rule.activity.isFinishing)
    }
}
