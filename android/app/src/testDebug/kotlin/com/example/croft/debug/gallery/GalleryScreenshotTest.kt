// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import com.example.croft.CroftTheme
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ModePreference
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.CroftSnackbarState
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Screenshot tests (Roborazzi, JVM) of every section of the gallery, so every control is drawn in every state it has: light and
 * dark, font scale 1.0 and 2.0, left-to-right and right-to-left. 17 sections x 2 x 2 x 2 = 136 pictures in
 * `app/build/outputs/screenshots/`, named `gallery-<section>-<light|dark>-font<scale>-<ltr|rtl>`. Two are committed and compared
 * (see `ScreenshotKit.Golden`). Animations are off, so a picture does not depend on when it was taken.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class GalleryScreenshotTest(
    private val sectionId: String,
    private val dark: Boolean,
    private val fontScale: Float,
    private val rtl: Boolean,
) {
    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}-{1}-{2}-{3}")
        fun cases(): List<Array<Any>> = buildList {
            for (s in GalleryCatalog.sections) for (dark in listOf(false, true)) for (font in listOf(1.0f, 2.0f)) for (rtl in listOf(false, true)) {
                add(arrayOf<Any>(s.id, dark, font, rtl))
            }
        }

        fun nameOf(section: String, dark: Boolean, font: Float, rtl: Boolean) =
            "gallery-$section-${if (dark) "dark" else "light"}-font${"%.1f".format(font)}-${if (rtl) "rtl" else "ltr"}"
    }

    @get:Rule
    val rule = createComposeRule()

    @Test
    fun screenshot() {
        val section = GalleryCatalog.sections.first { it.id == sectionId }
        // One tall window: the whole section is on screen at once, and the picture is of the content only.
        ScreenshotKit.configure(widthDp = 360, heightDp = if (fontScale > 1.5f) 12_000 else 6_000, rtl = rtl, fontScale = fontScale)
        val settings = ThemeSettings.Default.copy(mode = if (dark) ModePreference.DARK else ModePreference.LIGHT, animation = AnimationLevel.OFF)
        rule.setContent {
            CroftTheme(settings, systemDark = dark) {
                Box(Modifier.testTag("gallery-capture").background(MaterialTheme.colorScheme.background)) {
                    GallerySectionColumn(section, GalleryEnv(CroftSnackbarState()))
                }
            }
        }
        rule.waitForIdle()
        ScreenshotKit.capture(rule.onNodeWithTag("gallery-capture"), nameOf(sectionId, dark, fontScale, rtl))
    }
}
