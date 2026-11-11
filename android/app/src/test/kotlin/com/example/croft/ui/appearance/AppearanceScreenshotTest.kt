// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.example.croft.CroftTheme
import com.example.croft.design.CroftPreset
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
enum class AppearanceScreenId { S08, S50, S51, S52, S53 }

/**
 * Screenshot tests (Roborazzi, JVM) of S08, S50, S51, S52 and S53 on fake data: compact, medium and expanded widths,
 * light and dark, font scale 1.0 and 2.0, left-to-right and right-to-left. 5 x 3 x 2 x 2 x 2 = 120 pictures.
 * Test names carry the screen ID, for example `S50-compact-dark-font2.0-rtl`.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class AppearanceScreenshotTest(
    private val screen: AppearanceScreenId,
    private val device: Device,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}-{4}")
        fun cases(): List<Array<Any>> = buildList {
            for (screen in AppearanceScreenId.entries) for (device in Device.entries) for (dark in listOf(false, true))
                for (font in listOf(1.0f, 2.0f)) for (rtl in listOf(false, true)) add(arrayOf<Any>(screen, device, dark, font, rtl))
        }

        fun nameOf(screen: AppearanceScreenId, device: Device, dark: Boolean, font: Float, rtl: Boolean) =
            "$screen-${device.name.lowercase()}-${if (dark) "dark" else "light"}-font${"%.1f".format(font)}-${if (rtl) "rtl" else "ltr"}"
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        ScreenshotKit.configure(device, rtl, fontScale)
        rule.setContent {
            var settings by remember { mutableStateOf(ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT)) }
            CroftTheme(settings, systemDark = dark) { Screen(screen, settings) { settings = it } }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), nameOf(screen, device, dark, fontScale, rtl))
    }
}

/** One of the five screens on fake data, with every callback a no-op except the settings. */
@Composable
internal fun Screen(id: AppearanceScreenId, settings: ThemeSettings, onChange: (ThemeSettings) -> Unit) {
    when (id) {
        AppearanceScreenId.S08 -> PickYourLookScreen(settings, onChange, onBack = {}, onContinue = {}, wallpaperAvailable = true)
        AppearanceScreenId.S50 -> AppearanceScreen(settings, onChange, onBack = {}, onOpenCustomColour = {}, onOpenLayout = {}, onOpenMotion = {}, onReset = {}, onExportTheme = {}, onImportTheme = {}, wallpaperAvailable = true)
        AppearanceScreenId.S51 -> CustomColourScreen(settings, onApply = onChange, onBack = {}, photoPicker = PhotoSeedPicker { it(null) })
        AppearanceScreenId.S52 -> LayoutDensityScreen(settings, onChange, onBack = {})
        AppearanceScreenId.S53 -> MotionHapticsScreen(settings, onChange, onBack = {})
    }
}

/** The preview and the S10-style sample for every preset, in light and dark (8 x 2 x 2 pictures). */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class PaletteScreenshotTest(private val preset: CroftPreset, private val dark: Boolean) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}")
        fun cases(): List<Array<Any>> = CroftPreset.entries.flatMap { p -> listOf(false, true).map { arrayOf<Any>(p, it) } }
    }

    @get:Rule
    val rule = createComposeRule()

    private fun mode() = if (dark) "dark" else "light"

    @Test
    fun previewPreset() {
        ScreenshotKit.configure(Device.COMPACT, rtl = false, fontScale = 1.0f)
        rule.setContent {
            CroftTheme(ThemeSettings.Default.withPreset(preset).copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT), systemDark = dark) {
                androidx.compose.material3.Surface(color = androidx.compose.material3.MaterialTheme.colorScheme.surfaceContainer) {
                    androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.padding(16.dp)) { AppearancePreview() }
                }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), "preview-${preset.id}-${mode()}")
    }

    @Test
    fun inboxSamplePreset() {
        ScreenshotKit.configure(Device.COMPACT, rtl = false, fontScale = 1.0f)
        rule.setContent {
            CroftTheme(ThemeSettings.Default.withPreset(preset).copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT), systemDark = dark) { InboxSample() }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onRoot(), "S10-sample-${preset.id}-${mode()}")
    }
}
