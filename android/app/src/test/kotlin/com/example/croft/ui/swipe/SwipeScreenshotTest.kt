// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.center
import androidx.compose.ui.test.down
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.width
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.ScreenshotKit.Device
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.InboxScreen
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.rowCard
import com.example.croft.ui.mail.waitForRow
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The screens of the swipe engine whose pictures are taken. IDs are the screen IDs of design/screens.csv. */
enum class SwipeScreenId(val label: String) {
    /** The card follows the finger toward Mark read, below the arm point: the tonal pill. */
    S14("S14"),

    /** The card dragged toward Delete past the arm point: the full red pill, the icon grown. */
    S15("S15"),

    /** After the action: the row has left, the slot has closed, the snackbar has Undo. */
    S16("S16"),

    /** Swipe actions: the preview, the pickers, sensitivity, presets. */
    S47("S47"),

    /** Choose swipe action: the radio list. */
    S48("S48"),
}

/**
 * Screenshot tests (Roborazzi, JVM) of S14, S15, S16, S47 and S48: compact, medium and expanded widths, light and dark, font scale
 * 1.0 and 2.0, left-to-right and right-to-left. 5 pictures x 3 x 2 x 2 x 2 = 120. S14 to S16 are taken with real touch input on the
 * Inbox; the drag follows the stored direction, so in right-to-left the card goes the other way on screen. Names carry the screen ID:
 * `S15-compact-dark-font2.0-rtl`. Pictures go to `app/build/outputs/screenshots/`; none is committed as a golden.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class SwipeScreenshotTest(
    private val screen: SwipeScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in SwipeScreenId.entries) for (device in Device.entries) for (dark in listOf(false, true))
                for (font in listOf(1.0f, 2.0f)) for (rtl in listOf(false, true)) add(arrayOf<Any>(screen, device, dark, font, rtl))
        }

        fun nameOf(screen: SwipeScreenId, device: Device, dark: Boolean, font: Float, rtl: Boolean) =
            "${screen.label}-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(font)}-${if (rtl) "rtl" else "ltr"}"
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        when (screen) {
            SwipeScreenId.S47, SwipeScreenId.S48 -> {
                rule.setContent {
                    CroftTheme(settings, systemDark = dark) {
                        CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                            SwipeSettingsFlow(
                                SwipeSettings.Default, {}, onBack = {},
                                startAt = if (screen == SwipeScreenId.S48) SwipeRoute.S48_START_TO_END else SwipeRoute.S47,
                                demo = false,
                            )
                        }
                    }
                }
                rule.waitForIdle()
                ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
            }
            else -> inboxPicture(settings)
        }
    }

    private fun inboxPicture(settings: ThemeSettings) {
        val f = InboxFixture(80)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics, LocalBackGestureInset provides 24.dp) {
                    InboxScreen(f.session, onOpenSettings = {})
                }
            }
        }
        rule.waitForRow(f.topId, 20_000)
        rule.waitForIdle()
        val id = f.topId
        val rowWidth = rule.onNodeWithTag(InboxTags.row(id)).getUnclippedBoundsInRoot().width.value
        // The stored direction decides which way the card goes on screen: Mark read is START_TO_END, Delete END_TO_START.
        val direction = if (screen == SwipeScreenId.S14) SwipeDirection.START_TO_END else SwipeDirection.END_TO_START
        val sign = direction.physicalSign(rtl)
        val fraction = if (screen == SwipeScreenId.S14) 0.28f else 0.5f // Normal arms at 0.35 of the width
        val travel = sign * rowWidth * fraction
        rule.rowCard(id).performTouchInput {
            down(center)
            moveBy(Offset(sign * 10f, 0f))
            moveBy(Offset(travel, 0f))
        }
        if (screen == SwipeScreenId.S16) {
            // Let go: the card leaves, the slot closes and the snackbar has Undo.
            rule.rowCard(id).performTouchInput { up() }
            rule.waitUntil(20_000) { rule.onAllNodes(hasText("1 conversation deleted")).fetchSemanticsNodes().isNotEmpty() }
            rule.waitForIdle()
            ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
        } else {
            rule.waitForIdle()
            ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
            // The finger is lifted before the window is torn down (see the notes of T14).
            rule.rowCard(id).performTouchInput { up() }
            rule.waitForIdle()
        }
    }
}
