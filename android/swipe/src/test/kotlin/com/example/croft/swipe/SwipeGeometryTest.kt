// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Golden tests on the pill geometry (docs/ux/swipe.md, "The pill"): the gap (4 dp), the inset (8 dp), the width (drag minus
 * 12 dp), the height (the row's) and the radius (half the smaller side), for drag distances from 0 to 100 percent of the row
 * width, at all three roundness settings (which set the card's corner: 10, 17 and 24 dp).
 */
class SwipeGeometryTest {
    private val rowWidth = 400f
    private val rowHeight = 88f
    private val radii = mapOf("tight" to 10f, "standard" to 17f, "round" to 24f)

    @Test
    fun literalGoldenRows() {
        // offset, width, radius, icon visible (hand-computed from the rules, not by the code under test)
        val golden = listOf(
            listOf(0f, 0f, 0f, 0f),
            listOf(12f, 0f, 0f, 0f),
            listOf(20f, 8f, 4f, 0f),
            listOf(36f, 24f, 12f, 1f),
            listOf(100f, 88f, 44f, 1f),
            listOf(140f, 128f, 44f, 1f),
            listOf(400f, 388f, 44f, 1f),
        )
        for ((offset, width, radius, icon) in golden) {
            val p = SwipeGeometry.pill(TestGeometry, offset, rowHeight)
            assertEquals("width at $offset", width, p.width, 0.0001f)
            assertEquals("radius at $offset", radius, p.radius, 0.0001f)
            assertEquals("icon at $offset", icon == 1f, p.iconVisible)
            assertEquals(rowHeight, p.height, 0f)
            assertEquals(4f, p.gapToCard, 0f)
            assertEquals(8f, p.insetFromEdge, 0f)
        }
    }

    @Test
    fun everyDragDistanceFromZeroToTheFullWidthAtEveryRoundness() {
        for ((name, cardRadius) in radii) {
            for (percent in 0..100) {
                val offset = rowWidth * percent / 100f
                val p = SwipeGeometry.pill(TestGeometry, offset, rowHeight)
                val tag = "$name $percent%"
                assertEquals("$tag width", maxOf(0f, offset - 12f), p.width, 0.0001f)
                assertEquals("$tag height", rowHeight, p.height, 0f)
                assertEquals("$tag radius", minOf(p.width, rowHeight) / 2f, p.radius, 0.0001f)
                assertEquals("$tag gap", 4f, p.gapToCard, 0f)
                assertEquals("$tag inset", 8f, p.insetFromEdge, 0f)
                // The pill plus its gap and inset exactly fill the space the card left (or less, while the width is held at 0).
                assertTrue("$tag fits", p.width + p.gapToCard + p.insetFromEdge <= maxOf(offset, 12f) + 0.0001f)
                // A round capsule while narrow, a stadium once wider than tall.
                if (p.width <= rowHeight) assertEquals("$tag capsule", p.width / 2f, p.radius, 0.0001f)
                else assertEquals("$tag stadium", rowHeight / 2f, p.radius, 0.0001f)
                // The card's corner: the row's own at 0, the roundness radius from 16 dp on, and never past either.
                val corner = SwipeGeometry.cardRadius(TestGeometry, offset, restRadius = 0f, cardRadius = cardRadius)
                if (offset >= 16f) assertEquals("$tag corner", cardRadius, corner, 0.0001f)
                assertTrue("$tag corner range", corner in 0f..cardRadius)
            }
        }
    }

    @Test
    fun theCardsCornerEasesUpOverTheFirst16dp() {
        for ((name, r) in radii) {
            assertEquals(name, 0f, SwipeGeometry.cardRadius(TestGeometry, 0f, 0f, r), 0f)
            assertEquals(name, r / 2f, SwipeGeometry.cardRadius(TestGeometry, 8f, 0f, r), 0.0001f)
            assertEquals(name, r, SwipeGeometry.cardRadius(TestGeometry, 16f, 0f, r), 0.0001f)
            assertEquals(name, r, SwipeGeometry.cardRadius(TestGeometry, 300f, 0f, r), 0.0001f)
        }
        // A list whose own corners are already round starts from there.
        assertEquals(17f, SwipeGeometry.cardRadius(TestGeometry, 0f, 17f, 17f), 0f)
    }

    @Test
    fun theIconAppearsOnceThePillIsWideEnoughToHoldIt() {
        assertFalse(SwipeGeometry.pill(TestGeometry, 35.9f, rowHeight).iconVisible)
        assertTrue(SwipeGeometry.pill(TestGeometry, 36f, rowHeight).iconVisible)
    }

    @Test
    fun theCommittedPillFillsTheRowEightDpInFromEachEdge() {
        val p = SwipeGeometry.committedPill(TestGeometry, rowWidth, rowHeight)
        assertEquals(384f, p.width, 0f)
        assertEquals(44f, p.radius, 0f)
        assertTrue(p.iconVisible)
    }
}
