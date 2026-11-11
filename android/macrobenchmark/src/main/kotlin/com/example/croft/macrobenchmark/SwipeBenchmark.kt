// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.macrobenchmark

import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.Until
import java.util.regex.Pattern
import org.junit.Rule
import org.junit.Test

/**
 * T15, "On a device": the 95th-percentile frame time while a row is swiped (docs/ux/motion.md, "Performance budget": 60 fps held
 * on a mid-range phone, the display's full rate where available). Each iteration swipes the top row of the Inbox to the right
 * past the arm point (Mark read: the card springs back) and the next one to the left (Delete: the card slides off and the slot
 * collapses), slowly enough that the card is dragged for many frames. Frame timing is the metric (P50, P90, P95, P99 and the frames over
 * budget). **The result is unverified: it has not been run on a device**; nothing in the JVM tests claims anything about frame rates.
 *
 * Run it with a device attached: `./gradlew :macrobenchmark:connectedBenchmarkAndroidTest` (add
 * `-Pandroid.testInstrumentationRunnerArguments.androidx.benchmark.suppressErrors=EMULATOR` on an emulator, whose numbers mean little).
 * Rows are found by their test tag (`row-<id>`), which the app exposes as a resource id (`testTagsAsResourceId`).
 */
class SwipeBenchmark {
    @get:Rule
    val rule = MacrobenchmarkRule()

    @Test
    fun swipeRowsInTheFiveThousandRowInbox() = rule.measureRepeated(
        packageName = targetPackage,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(),
        startupMode = StartupMode.WARM,
        iterations = 5,
        setupBlock = {
            pressHome()
            startActivityAndWait()
            device.wait(Until.hasObject(By.res(ROW_TAG)), LIST_TIMEOUT_MS)
        },
    ) {
        repeat(SWIPES) {
            val right = device.findObject(By.res(ROW_TAG)) ?: error("no message row is on screen")
            // Stay clear of the system back-gesture strips: the row ignores a drag that starts in them.
            right.setGestureMargin(device.displayWidth / 8)
            right.swipe(Direction.RIGHT, SWIPE_FRACTION, SWIPE_SPEED_PX_PER_SEC)
            device.waitForIdle()
            val left = device.findObject(By.res(ROW_TAG)) ?: error("no message row is on screen")
            left.setGestureMargin(device.displayWidth / 8)
            left.swipe(Direction.LEFT, SWIPE_FRACTION, SWIPE_SPEED_PX_PER_SEC)
            device.waitForIdle()
        }
    }

    /** The application id of the app under test, passed by the build (`croft.targetPackage`, from gradle.properties). */
    private val targetPackage: String =
        InstrumentationRegistry.getArguments().getString("croft.targetPackage") ?: error("croft.targetPackage is not set")

    private companion object {
        /** `InboxTags.row(id)` is `row-<id>`. */
        val ROW_TAG: Pattern = Pattern.compile("row-\\d+")
        const val LIST_TIMEOUT_MS = 10_000L
        const val SWIPES = 4

        /** Past the 35 percent arm point at Normal sensitivity. */
        const val SWIPE_FRACTION = 0.6f

        /** Slow: the card is dragged for many frames, and under the 1,200 dp/s fling speed on any phone. */
        const val SWIPE_SPEED_PX_PER_SEC = 600
    }
}
