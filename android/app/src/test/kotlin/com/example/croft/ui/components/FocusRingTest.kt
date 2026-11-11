// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import com.example.croft.theme.AnimationLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The focus ring of docs/ux/controls.md: 3 dp, primary colour, 2 dp gap, outside the control and never clipped. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class FocusRingTest {
    @get:Rule
    val rule = createComposeRule()

    private fun ringVisible(tag: String) = rule.onNodeWithTag(tag).fetchSemanticsNode().config[FocusRingVisible]

    @Test
    fun theRingFollowsFocusOnEveryControlWithAnInteractionSource() {
        rule.show(AnimationLevel.OFF) {
            androidx.compose.foundation.layout.Column {
                CroftButton("A", {}, Modifier.testTag("plain"))
                CroftButton("B", {}, Modifier.testTag("focused"), interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
                CroftFilterChip("C", false, {}, Modifier.testTag("chip"), interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
                CroftSwitchRow("D", false, {}, Modifier.testTag("switch"), interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
                MessageRow("E", "s", "p", "t", MessageRowState(), {}, Modifier.testTag("row"), interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
            }
        }
        rule.waitForIdle()
        assertEquals(false, ringVisible("plain"))
        assertEquals(true, ringVisible("focused"))
        assertEquals(true, ringVisible("chip"))
        assertEquals(true, ringVisible("switch"))
    }

    @Test
    fun theRingIsDrawnInThePrimaryColourOutsideTheControlsBounds() {
        rule.show(AnimationLevel.OFF) {
            var primary = MaterialTheme.colorScheme.primary
            Box(Modifier.testTag("frame").padding(12.dp)) {
                CroftButton("B", {}, interactionSource = rememberForcedInteractionSource(ForcedState.FOCUSED))
            }
        }
        rule.waitForIdle()
        val bmp = rule.onNodeWithTag("frame").captureToImage().asAndroidBitmap()
        // 12 dp of padding at mdpi: the ring (2 dp gap + 3 dp stroke) sits in the band between 4 and 10 px from the control.
        val c = bmp.getPixel(bmp.width / 2, 12 - 4)
        assertTrue("a ring pixel above the button is not the page background: ${Integer.toHexString(c)}", c != bmp.getPixel(2, 2))
    }
}
