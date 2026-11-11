// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Offset
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
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.ScreenshotKit.Device
import com.example.croft.ui.swipe.LocalBackGestureInset
import com.example.croft.ui.swipe.LocalSwipeSettings
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.croft_ffi.AccountStatus

/** The states of S16 and S18 that come from the real repository over a scripted core (see [CoreFixture]). */
enum class CoreState(val label: String) {
    /** After a swipe: the row has gone, the snackbar has Undo (S16). */
    S16_UNDO("S16-core"),

    /** Offline banner, the pending clock on a row, the account chip (S18). */
    S18_OFFLINE("S18-core-offline"),

    /** Needs sign-in banner and chip (S18). */
    S18_SIGN_IN("S18-core-signin"),

    /** The server refused a change: the row is back with the red edge and a Retry snackbar (S18). */
    S18_FAILED("S18-core-failed"),
}

/**
 * Screenshot tests (Roborazzi, JVM) of S16 and S18 with the state the real repository produces: four fixed states, compact width, font
 * scale 1.0, light and dark, left-to-right (8 pictures). They are taken through the app's [com.example.croft.mail.CoreMessageRepository]
 * over a scripted core, so the rows (thread count, pending, starred) and the banners come through the real mapping. Names carry the
 * screen ID: `S16-core-compact-dark-font1.0-ltr`. Pictures go to `app/build/outputs/screenshots/`; none is committed as a golden.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class CoreScreenshotTest(private val state: CoreState, private val dark: Boolean) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}")
        fun cases(): List<Array<Any>> = buildList { for (s in CoreState.entries) for (dark in listOf(false, true)) add(arrayOf<Any>(s, dark)) }
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(Device.COMPACT, rtl = false, fontScale = 1.0f)
        val f = CoreFixture(statusOf = { if (state == CoreState.S18_SIGN_IN && it == 2L) AccountStatus.NEEDS_SIGN_IN else if (state == CoreState.S18_OFFLINE) AccountStatus.OFFLINE else AccountStatus.OK })
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics, LocalSwipeSettings provides SwipeSettings.Default, LocalBackGestureInset provides 24.dp) {
                    InboxScreen(f.session, onOpenSettings = {})
                }
            }
        }
        rule.waitForRow(10, 20_000)
        if (state == CoreState.S16_UNDO || state == CoreState.S18_FAILED) swipeDelete()
        if (state == CoreState.S18_FAILED) {
            f.core.reject(1uL, account = 1)
            rule.waitForRow(10, 20_000)
            rule.waitUntil(20_000) { rule.onAllNodes(hasText("Couldn't finish that change", substring = true)).fetchSemanticsNodes().isNotEmpty() }
        }
        when (state) {
            CoreState.S18_OFFLINE -> rule.waitUntil(20_000) { rule.onAllNodes(hasText("You are offline. Changes wait and are sent later.")).fetchSemanticsNodes().isNotEmpty() }
            CoreState.S18_SIGN_IN -> rule.waitUntil(20_000) { rule.onAllNodes(hasText("Sign in again to Work")).fetchSemanticsNodes().isNotEmpty() }
            else -> Unit
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), "${state.label}-compact-${if (dark) "dark" else "light"}-font1.0-ltr")
    }

    private fun swipeDelete() {
        val width = rule.onNodeWithTag(InboxTags.row(10)).getUnclippedBoundsInRoot().width.value
        rule.rowCard(10).performTouchInput {
            down(center)
            moveBy(Offset(-10f, 0f))
            moveBy(Offset(-width * 0.5f, 0f))
            up()
        }
        rule.waitUntil(20_000) { rule.onAllNodes(hasText("1 conversation deleted")).fetchSemanticsNodes().isNotEmpty() }
        rule.waitForIdle()
    }
}
