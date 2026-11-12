// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.RustInfo
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.mail.accounts.AccountColour
import com.example.croft.mail.accounts.AccountPrefs
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

/** The screens whose pictures are taken. IDs are the screen IDs of design/screens.csv. */
enum class AccountScreenId { S44, S45, S60 }

private val shotPrefs = AccountPrefs(order = listOf(1L, 2L, 3L), names = mapOf(2L to "Work"), colours = mapOf(1L to AccountColour.PRIMARY, 2L to AccountColour.AMBER))

/** One screen on made-up data: S44 with an OK, a Needs sign-in and the sample account; S45 of the second account; S60 as it opens. */
@Composable
internal fun AccountScreenFor(id: AccountScreenId) {
    val (session, _) = androidx.compose.runtime.remember { accountsSession() }
    when (id) {
        AccountScreenId.S44 -> AccountsScreen(
            session, shotPrefs, onPrefsChange = {}, onAdd = {}, onOpenAccount = {}, removeAccount = { null }, savedSignIn = { com.example.croft.mail.accounts.SavedSignIn.PASSWORD }, onNoAccounts = {}, onBack = {},
        )
        AccountScreenId.S45 -> AccountScreen(
            session, accountId = 2, prefs = shotPrefs, onPrefsChange = {}, removeAccount = { null }, savedSignIn = { com.example.croft.mail.accounts.SavedSignIn.PASSWORD }, onRemoved = {}, eraseAll = { false }, onBack = {},
        )
        AccountScreenId.S60 -> AboutScreen(
            info = RustInfo("0.1.0", 6u), session = session, onOpenLicences = {}, eraseAll = { false }, onBack = {},
            openUrl = { true }, copyText = { _, _ -> },
        )
    }
}

/**
 * Screenshot tests (Roborazzi, JVM) of S44, S45 and S60: compact and expanded widths, light and dark, font scale 1.0 and 2.0 in
 * left-to-right, plus one right-to-left picture of each screen (compact, light, 1.0): 3 x (2 x 2 x 2 + 1) = 27 pictures. Names carry the
 * screen ID, for example `S44-compact-dark-font2.0-ltr`. Pictures go to `app/build/outputs/screenshots/`; none is committed as a golden.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class AccountsScreenshotTest(
    private val screen: AccountScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in AccountScreenId.entries) {
                for (device in listOf(Device.COMPACT, Device.EXPANDED)) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) {
                    add(arrayOf<Any>(screen, device, dark, font, false))
                }
                add(arrayOf<Any>(screen, Device.COMPACT, false, 1.0f, true))
            }
        }

        fun nameOf(screen: AccountScreenId, device: Device, dark: Boolean, font: Float, rtl: Boolean) =
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
                CompositionLocalProvider(LocalHaptics provides NoHaptics) { AccountScreenFor(screen) }
            }
        }
        // The accounts are read once the screen is on (S44 and S45 wait for the first read).
        val ready = when (screen) {
            AccountScreenId.S44 -> AccountsTags.row(1)
            AccountScreenId.S45 -> AccountsTags.ACCOUNT
            AccountScreenId.S60 -> AccountsTags.ABOUT
        }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(ready)).fetchSemanticsNodes().isNotEmpty() }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
    }
}
