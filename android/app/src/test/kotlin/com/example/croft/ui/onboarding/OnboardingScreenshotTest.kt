// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.onboarding.Attempt
import com.example.croft.mail.onboarding.ConnectionTest
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.mail.onboarding.Lookup
import com.example.croft.mail.onboarding.OnboardingFailure
import com.example.croft.mail.onboarding.OnboardingState
import com.example.croft.mail.onboarding.ProbeRow
import com.example.croft.mail.onboarding.RowState
import com.example.croft.mail.onboarding.ServerForm
import com.example.croft.mail.onboarding.Step
import com.example.croft.mail.onboarding.TestStep
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.ScreenshotKit.Device
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.ProbeFailureReason
import uniffi.croft_ffi.VerifyFailure

/** The sign-in screens whose pictures are taken. IDs are the screen IDs of design/screens.csv. */
enum class OnboardingScreenId { S01, S02, S03, S04, S05, S09 }

/** One screen on made-up data, in the state that shows the most: S03 with every row state, S04 with the wrong-password message, S05 with a failed test. */
@Composable
internal fun OnboardingScreenFor(id: OnboardingScreenId) {
    val email = "sam@example.org"
    when (id) {
        OnboardingScreenId.S01 -> WelcomeScreen(onAddAccount = {}, onTrySampleMail = {})
        OnboardingScreenId.S02 -> AddAccountScreen(email, rejected = false, onEmailChange = {}, onShortcut = {}, onContinue = {}, onBack = {}, autofocus = false)
        OnboardingScreenId.S03 -> FindingScreen(
            email = email,
            probes = mapOf(
                ProbeRow.BUILT_IN_TABLE to RowState.NoMatch(ProbeFailureReason.NOT_PUBLISHED),
                ProbeRow.AUTOCONFIG to RowState.Found,
                ProbeRow.SRV to RowState.Running,
                ProbeRow.JMAP_WELL_KNOWN to RowState.Waiting,
            ),
            lookup = Lookup.Running, onRetry = {}, onEnterManually = {}, onBack = {},
        )
        OnboardingScreenId.S04 -> SignInScreen(
            state = OnboardingState(
                stack = listOf(Step.WELCOME, Step.ADD_ACCOUNT, Step.SIGN_IN), email = email,
                discovered = fakeFound(email, KnownProvider.GMAIL),
                attempt = Attempt.Failed(OnboardingFailure.Verify(VerifyFailure.WrongPassword(), "mail.example.org")),
            ),
            onSignIn = { _, _ -> }, onServerSettings = {}, onTrust = {}, onCancelTrust = {}, onPasswordEdited = {}, onBack = {},
        )
        OnboardingScreenId.S05 -> ServerSettingsScreen(
            state = OnboardingState(
                stack = listOf(Step.WELCOME, Step.ADD_ACCOUNT, Step.SERVER_SETTINGS), email = email,
                form = ServerForm.guessFor(email), formGuessed = true,
                attempt = Attempt.Failed(OnboardingFailure.Verify(VerifyFailure.CertificateInvalid(), "mail.example.org")),
                test = ConnectionTest.stoppedAt(TestStep.CERTIFICATE),
                discovered = fakeFound(email),
            ),
            onFormChange = {}, onTest = {}, onAdd = {}, onTrust = {}, onCancelTrust = {}, onBack = {},
        )
        OnboardingScreenId.S09 -> FirstSyncScreen("sam", FirstSyncStatus(inboxMessages = 12, done = false, failed = false), onContinue = {})
    }
}

/**
 * Screenshot tests (Roborazzi, JVM) of S01 to S05 and S09: compact and expanded widths, light and dark, font scale 1.0 and 2.0 in
 * left-to-right, plus one right-to-left picture of each screen (compact, light, 1.0): 6 x (2 x 2 x 2 + 1) = 54 pictures. Names carry the
 * screen ID, for example `S04-compact-dark-font2.0-ltr`. Pictures go to `app/build/outputs/screenshots/`; none is committed as a golden.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class OnboardingScreenshotTest(
    private val screen: OnboardingScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in OnboardingScreenId.entries) {
                for (device in listOf(Device.COMPACT, Device.EXPANDED)) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) {
                    add(arrayOf<Any>(screen, device, dark, font, false))
                }
                add(arrayOf<Any>(screen, Device.COMPACT, false, 1.0f, true))
            }
        }

        fun nameOf(screen: OnboardingScreenId, device: Device, dark: Boolean, font: Float, rtl: Boolean) =
            "$screen-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(font)}-${if (rtl) "rtl" else "ltr"}"
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) { OnboardingScreenFor(screen) }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
    }
}
