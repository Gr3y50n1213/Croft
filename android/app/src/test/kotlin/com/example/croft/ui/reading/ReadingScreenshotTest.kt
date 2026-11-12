// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.MailSettings
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.RemoteImages
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

/** The two pictures of T18b: S20 (a conversation: two collapsed messages and the newest open) and S21 (a message with the blocked bar and chips). */
enum class ReadingScreenId(val label: String) { S20("S20"), S21("S21") }

/**
 * Screenshot tests (Roborazzi, JVM) of S20 and S21, with the blocked-images bar and attachments (a PDF, a text file and a blocked
 * program): compact and expanded width, light and dark, font scale 1.0 and 2.0 (all left-to-right), plus one right-to-left picture of each
 * at compact width, light, font 1.0. 2 screens x 2 x 2 x 2 = 16, and 2 for right-to-left. Names carry the screen ID:
 * `S21-compact-dark-font2.0-ltr`. Pictures go to `app/build/outputs/screenshots/`; none is committed as a golden. The WebView draws nothing
 * under Robolectric (there is no browser engine), so the HTML card of the message is an empty light card here: the real rendering of HTML
 * mail is checked on a device.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class ReadingScreenshotTest(
    private val screen: ReadingScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in ReadingScreenId.entries) {
                for (device in listOf(Device.COMPACT, Device.EXPANDED)) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) add(arrayOf<Any>(screen, device, dark, font, false))
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
        // Both screens show the blocked bar with its two buttons (the Known senders setting) and the chips of a PDF, a text file and a program.
        val body = htmlBody(NEWEST, blocked = 3, attachments = listOf(PDF, NOTES, PROGRAM))
        // S20 is a conversation of three messages in the sample mailbox (the "Not saved to server" mark); S21 is one message on its own.
        val f = ReadingFixture(newestBody = body, newestAllowed = body, localOnly = screen == ReadingScreenId.S20, onlyNewest = screen == ReadingScreenId.S21)
        val mail = MailSettings(undoSeconds = 5, remoteImages = RemoteImages.KNOWN_SENDERS, markRead = MarkReadDelay.NEVER)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) { f.Screen(NEWEST, mail) }
            }
        }
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        if (screen == ReadingScreenId.S20) rule.waitForTag(ReadingTags.message(OLD_ONE))
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), name())
    }
}
