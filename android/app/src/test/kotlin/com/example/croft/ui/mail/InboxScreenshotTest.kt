// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.click
import androidx.compose.ui.test.down
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import com.example.croft.CroftTheme
import com.example.croft.haptics.NoHaptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.AccountState
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.DensityLevel
import com.example.croft.theme.ListStyle
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

/** The mail screens whose pictures are taken. IDs are the screen IDs of design/screens.csv; S17 has two moments. */
enum class MailScreenId(val label: String) {
    S10("S10"), S11("S11"), S12("S12"), S13("S13"),
    /** New mail below the top: the "N new" pill. */
    S17("S17"),
    /** The same screen while the person is pulling: the loading indicator. */
    S17_PULL("S17pull"),
    S18("S18"), S19("S19"),
}

/** Puts the Inbox into the state a screen shows, through the fake repository and the screen's own gestures (never by poking its state). */
internal fun MailScreenId.prepare(f: InboxFixture) {
    when (this) {
        MailScreenId.S13 -> {
            f.repo.setAccountStatus(1, AccountState.SYNCING)
            f.repo.setAccountStatus(2, AccountState.NEEDS_SIGN_IN)
        }
        MailScreenId.S18 -> {
            f.repo.setOffline(true)
            val ids = f.inboxIds()
            kotlinx.coroutines.runBlocking {
                f.repo.setFlagged(listOf(ids[0]), true)
                f.repo.markSeen(listOf(ids[1]), true)
            }
        }
        else -> Unit
    }
}

internal fun fixtureFor(screen: MailScreenId): InboxFixture =
    if (screen == MailScreenId.S19) InboxFixture(0, data = emptyList()) else InboxFixture(80)

/**
 * Screenshot tests (Roborazzi, JVM) of S10, S11, S12, S13, S17, S18 and S19 on the fake repository: compact, medium and
 * expanded widths, light and dark, font scale 1.0 and 2.0, left-to-right and right-to-left. 8 pictures x 3 x 2 x 2 x 2 = 192.
 * Names carry the screen ID: `S11-compact-dark-font2.0-rtl`. Pictures go to `app/build/outputs/screenshots/`; none is
 * committed as a golden (they could not be recorded where this was written; record some on CI, see `ScreenshotKit.Golden`).
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class InboxScreenshotTest(
    private val screen: MailScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in MailScreenId.entries) for (device in Device.entries) for (dark in listOf(false, true))
                for (font in listOf(1.0f, 2.0f)) for (rtl in listOf(false, true)) add(arrayOf<Any>(screen, device, dark, font, rtl))
        }

        fun nameOf(screen: MailScreenId, device: Device, dark: Boolean, font: Float, rtl: Boolean) =
            "${screen.label}-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(font)}-${if (rtl) "rtl" else "ltr"}"
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        val f = fixtureFor(screen)
        screen.prepare(f)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                CompositionLocalProvider(LocalHaptics provides NoHaptics) {
                    InboxScreen(f.session, onOpenSettings = {}, startDrawerOpen = screen == MailScreenId.S12, startSwitcherOpen = screen == MailScreenId.S13)
                }
            }
        }
        if (screen == MailScreenId.S19) {
            rule.waitUntil(20_000) { rule.onAllNodes(hasText("You're all caught up")).fetchSemanticsNodes().isNotEmpty() }
        } else rule.waitForRow(f.topId, 20_000)
        rule.waitForIdle()
        when (screen) {
            MailScreenId.S11 -> {
                rule.rowCard(f.topId).performTouchInput { longClick() }
                rule.waitForIdle()
                rule.rowCard(f.topId - 2).performClick()
                rule.waitForIdle()
            }
            MailScreenId.S17 -> {
                rule.scrollToRow(f.inboxIds()[6])
                f.repo.injectNewMail(3)
                rule.waitUntil(20_000) { rule.onAllNodes(hasText("3 new")).fetchSemanticsNodes().isNotEmpty() }
            }
            MailScreenId.S17_PULL -> {
                rule.onNodeWithTag(InboxTags.LIST).performTouchInput {
                    down(Offset(centerX, 40f))
                    repeat(8) { moveBy(Offset(0f, 30f)) }
                }
            }
            MailScreenId.S18 -> rule.waitUntil(20_000) { rule.onAllNodes(hasText("offline", substring = true, ignoreCase = true)).fetchSemanticsNodes().isNotEmpty() }
            else -> Unit
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
    }
}

/** Each density of the list (Comfortable, Default, Compact) and each list style, on the Inbox: compact width, light, font 1.0. */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class InboxDensityScreenshotTest(private val density: DensityLevel, private val listStyle: ListStyle) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}")
        fun cases(): List<Array<Any>> = DensityLevel.entries.flatMap { d -> ListStyle.entries.map { arrayOf<Any>(d, it) } }
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(Device.COMPACT, rtl = false, fontScale = 1.0f)
        val f = InboxFixture(80)
        val settings = ThemeSettings.Default.copy(mode = ModePreference.LIGHT, animation = AnimationLevel.OFF, density = density, listStyle = listStyle)
        rule.setContent { CroftTheme(settings, systemDark = false) { InboxScreen(f.session, onOpenSettings = {}) } }
        rule.waitForRow(f.topId, 20_000)
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), "S10-density-${density.id}-${listStyle.id}")
    }
}
