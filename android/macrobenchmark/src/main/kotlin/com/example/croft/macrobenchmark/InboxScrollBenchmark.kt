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
import org.junit.Rule
import org.junit.Test

/**
 * T14, "On a device": scrolling the Inbox with the fake repository's 5,000 rows should hold the display's frame rate (120 or
 * 60 fps) on a mid-range phone. This measures frame timing (the 50th, 90th, 95th and 99th percentile of frame duration and the
 * number of frames over budget) while the list is flung up and down. **The result is unverified: it has not been run on a
 * device**; nothing in the JVM tests claims anything about frame rates.
 *
 * Run it with a device attached: `./gradlew :macrobenchmark:connectedBenchmarkAndroidTest` (add
 * `-Pandroid.testInstrumentationRunnerArguments.androidx.benchmark.suppressErrors=EMULATOR` on an emulator, whose numbers mean little).
 * The list is found by the test tag of the Inbox list, which the app exposes as a resource id (`testTagsAsResourceId`).
 */
class InboxScrollBenchmark {
    @get:Rule
    val rule = MacrobenchmarkRule()

    @Test
    fun scrollTheFiveThousandRowInbox() = rule.measureRepeated(
        packageName = targetPackage,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(),
        startupMode = StartupMode.WARM,
        iterations = 5,
        setupBlock = {
            pressHome()
            startActivityAndWait()
            device.wait(Until.hasObject(By.res(INBOX_LIST_TAG)), LIST_TIMEOUT_MS)
        },
    ) {
        val list = device.findObject(By.res(INBOX_LIST_TAG)) ?: error("the Inbox list is not on screen")
        // Keep inside the gesture margins so the system does not take the swipe for navigation.
        list.setGestureMargin(device.displayWidth / 5)
        repeat(FLINGS) {
            list.fling(Direction.DOWN)
            device.waitForIdle()
        }
        repeat(FLINGS) {
            list.fling(Direction.UP)
            device.waitForIdle()
        }
    }

    /** The application id of the app under test, passed by the build (`croft.targetPackage`, from gradle.properties). */
    private val targetPackage: String =
        InstrumentationRegistry.getArguments().getString("croft.targetPackage") ?: error("croft.targetPackage is not set")

    private companion object {
        /** `InboxTags.LIST`. */
        const val INBOX_LIST_TAG = "inbox-list"
        const val LIST_TIMEOUT_MS = 10_000L
        const val FLINGS = 6
    }
}
