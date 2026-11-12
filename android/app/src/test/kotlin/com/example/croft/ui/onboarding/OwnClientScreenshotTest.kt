// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.onboarding.OAuthPhase
import com.example.croft.mail.onboarding.OAuthProviderChoice
import com.example.croft.mail.onboarding.OnboardingState
import com.example.croft.mail.onboarding.Step
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
import uniffi.croft_ffi.OAuthFailure

/** S04's OAuth variants (T21): Gmail, Outlook, the help open (Outlook, with the redirect to copy), and a refused client. */
enum class OwnClientScreenId(val label: String) { GMAIL("S04-gmail"), OUTLOOK("S04-outlook"), HELP("S04-oauth-help"), ERROR("S04-oauth-error") }

/** Three widths, light and dark, font scale 1.0 and 2.0, plus right-to-left: 4 x (3 x 2 x 2 + 1) = 52 pictures. None is a golden. */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class OwnClientScreenshotTest(
    private val screen: OwnClientScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in OwnClientScreenId.entries) {
                for (device in Device.entries) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) add(arrayOf<Any>(screen, device, dark, font, false))
                add(arrayOf<Any>(screen, Device.COMPACT, false, 1.0f, true))
            }
        }
    }

    @get:Rule
    val rule = createComposeRule()

    private fun name() = "${screen.label}-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(fontScale)}-${if (rtl) "rtl" else "ltr"}"

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        val gmail = screen == OwnClientScreenId.GMAIL || screen == OwnClientScreenId.ERROR
        val email = if (gmail) "sam@gmail.com" else "sam@outlook.com"
        val provider = if (gmail) OAuthProviderChoice.GOOGLE else OAuthProviderChoice.MICROSOFT
        val phase = if (screen == OwnClientScreenId.ERROR) OAuthPhase.Failed(OAuthFailure.Refused("invalid_client")) else OAuthPhase.Idle
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    SignInScreen(
                        state = OnboardingState(
                            stack = listOf(Step.WELCOME, Step.ADD_ACCOUNT, Step.SIGN_IN), email = email,
                            discovered = fakeFound(email, if (gmail) KnownProvider.GMAIL else KnownProvider.OUTLOOK, host = if (gmail) "imap.gmail.com" else "outlook.office365.com"),
                        ),
                        onSignIn = { _, _ -> }, onServerSettings = {}, onTrust = {}, onCancelTrust = {}, onPasswordEdited = {}, onBack = {},
                        ownClient = {
                            OwnClientSection(
                                provider = provider, phase = phase, rememberedClientId = "1234-made-up.apps.example",
                                onStart = { _, _ -> }, onCancel = {}, onEdited = {}, helpOpen = screen == OwnClientScreenId.HELP,
                            )
                        },
                    )
                }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), name())
    }
}
