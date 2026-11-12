// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.notify

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.notify.CheckAccount
import com.example.croft.notify.NotifySettings
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

/** S07 (asking, and refused with the banner) and S46. */
enum class NotifyScreenId(val label: String) { S07("S07"), S07_DENIED("S07-denied"), S46("S46") }

@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class NotifyScreenshotTest(
    private val screen: NotifyScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in NotifyScreenId.entries) {
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
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    when (screen) {
                        NotifyScreenId.S07, NotifyScreenId.S07_DENIED -> NotifyIntroScreen(
                            state = if (screen == NotifyScreenId.S07) IntroState.ASK else IntroState.DENIED,
                            onTurnOn = {}, onSendTest = {}, onSkip = {}, onDone = {}, onOpenSettings = {},
                        )
                        NotifyScreenId.S46 -> NotificationSettingsScreen(
                            settings = NotifySettings(mutedAccounts = setOf(2)),
                            accounts = listOf(CheckAccount(1, "Work"), CheckAccount(2, "Home")),
                            canPost = true,
                            onChange = {}, onSendTest = {}, onOpenSettings = {}, onOpenBatterySettings = {}, onBack = {},
                        )
                    }
                }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), name())
    }
}
