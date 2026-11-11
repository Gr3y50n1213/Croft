// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui

import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.graphics.asAndroidBitmap
import com.github.takahirom.roborazzi.RoborazziOptions
import com.github.takahirom.roborazzi.RoborazziTaskType
import com.github.takahirom.roborazzi.captureRoboImage
import java.io.File
import org.junit.Assert.assertTrue
import org.robolectric.RuntimeEnvironment

/**
 * JVM screenshot tests (Roborazzi on Robolectric native graphics; recorded in docs/DECISIONS.md). T13 to T21 reuse this.
 *
 * Every screenshot is written to `app/build/outputs/screenshots/<name>.png` (not committed). A small set of golden images
 * ([Golden.names]) is committed in `app/src/test/screenshots/` and compared on every run, with a small tolerance; to
 * refresh them after a deliberate visual change run `./gradlew :app:testDebugUnitTest -Pcroft.screenshots=record-goldens`.
 * Gradle passes the mode in the `croft.screenshots` system property: `verify` (default), `record-goldens` or `off`
 * (draw and check the picture but write nothing).
 */
object ScreenshotKit {
    enum class Device(val qualifiers: String, val widthDp: Int, val heightDp: Int) {
        COMPACT("w360dp-h800dp-mdpi", 360, 800),
        MEDIUM("w700dp-h900dp-mdpi", 700, 900),
        EXPANDED("w1000dp-h700dp-mdpi", 1000, 700),
    }

    private val mode: String = System.getProperty("croft.screenshots") ?: "verify"
    private val outDir = File("build/outputs/screenshots")
    private val goldenDir = File("src/test/screenshots")

    /** Names (without extension) of the screenshots that are committed and compared. */
    object Golden {
        val names = setOf(
            "S50-compact-light-font1.0-ltr",
            "S50-compact-dark-font2.0-ltr",
            "S08-compact-light-font1.0-ltr",
            "S51-compact-dark-font1.0-ltr",
            "S52-medium-light-font1.0-rtl",
            "S53-expanded-light-font1.0-ltr",
            "preview-moss-light",
            "preview-wax-dark",
        )
    }

    /** Set the simulated device before `setContent`: size, density, dark flag of the resources, layout direction and font scale. */
    fun configure(device: Device, rtl: Boolean, fontScale: Float) {
        RuntimeEnvironment.setQualifiers("+" + (if (rtl) "ldrtl" else "ldltr") + "-" + device.qualifiers)
        RuntimeEnvironment.setFontScale(fontScale)
    }

    /**
     * Draw [node], check that it is a real picture (the right size, more than one colour, not mostly empty), and save it.
     * Returns the picture's distinct-colour count for further checks.
     */
    fun capture(node: SemanticsNodeInteraction, name: String): Int {
        val bitmap = node.captureToImage().asAndroidBitmap()
        assertTrue("$name: empty picture", bitmap.width > 0 && bitmap.height > 0)
        val colours = HashSet<Int>()
        var y = 0
        while (y < bitmap.height) {
            var x = 0
            while (x < bitmap.width) {
                colours.add(bitmap.getPixel(x, y))
                x += 3
            }
            y += 3
        }
        assertTrue("$name: picture has only ${colours.size} colour(s)", colours.size >= 4)
        if (mode != "off") {
            val golden = name in Golden.names
            val options = when {
                golden && mode == "record-goldens" -> RoborazziOptions(taskType = RoborazziTaskType.Record)
                golden -> RoborazziOptions(
                    taskType = RoborazziTaskType.Verify,
                    compareOptions = RoborazziOptions.CompareOptions(changeThreshold = 0.01f),
                )
                else -> RoborazziOptions(taskType = RoborazziTaskType.Record)
            }
            val file = if (golden) File(goldenDir, "$name.png") else File(outDir, "$name.png")
            file.parentFile?.mkdirs()
            node.captureRoboImage(file, options)
        }
        return colours.size
    }
}
