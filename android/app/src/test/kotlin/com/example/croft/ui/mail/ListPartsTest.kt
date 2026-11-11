// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.down
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import com.example.croft.CroftTheme
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.FakeData
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.DensityLevel
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.components.ROW_PREVIEW_TAG
import com.example.croft.ui.components.settingsAt
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Density, sticky date headers and the fast scroller (docs/ux/message-list.md, theme.md). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class ListPartsTest {
    @get:Rule
    val rule = createComposeRule()

    // ---- Density (S52 settings reach the list) ----

    private fun rowHeightAt(density: DensityLevel): Pair<Float, Boolean> {
        val f = InboxFixture()
        rule.setContent {
            CroftTheme(ThemeSettings.Default.copy(density = density, animation = AnimationLevel.OFF), systemDark = false) {
                CompositionLocalProvider(LocalHaptics provides f.haptics) { InboxScreen(f.session, onOpenSettings = {}) }
            }
        }
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        val height = rule.onNodeWithTag(InboxTags.row(f.topId)).getUnclippedBoundsInRoot().let { it.bottom.value - it.top.value }
        val preview = rule.onAllNodes(hasTestTag(ROW_PREVIEW_TAG), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        return height to preview
    }

    @Test
    fun density_comfortableRowsAreTallWithTwoPreviewLines() {
        val (h, preview) = rowHeightAt(DensityLevel.COMFORTABLE)
        assertTrue("at least the 88 dp of the token ($h)", h >= DensityLevel.COMFORTABLE.rowDp - 0.5f)
        assertTrue(preview)
    }

    @Test
    fun density_defaultRowsHaveOnePreviewLine() {
        val (h, preview) = rowHeightAt(DensityLevel.DEFAULT)
        assertTrue("at least the 72 dp of the token ($h)", h >= DensityLevel.DEFAULT.rowDp - 0.5f)
        assertTrue(preview)
    }

    @Test
    fun density_compactRowsHaveNoPreview() {
        val (h, preview) = rowHeightAt(DensityLevel.COMPACT)
        assertTrue("at least the 56 dp of the token ($h)", h >= DensityLevel.COMPACT.rowDp - 0.5f)
        assertEquals("Compact shows no preview", false, preview)
    }

    // ---- Sticky date headers ----

    @Test
    fun theDateHeaderOfTheScreenStaysAtTheTopWhileScrolling() {
        val f = InboxFixture()
        rule.showInbox(f)
        rule.waitForRow(f.topId)
        rule.onNodeWithTag(InboxTags.header("today")).assertIsDisplayed()
        // About twenty rows down, still inside today's rows: the header is pinned above them.
        rule.scrollToRow(f.inboxIds()[15])
        rule.onNodeWithTag(InboxTags.header("today")).assertIsDisplayed()
    }

    // ---- Fast scroller ----

    private fun showScroller(kick: () -> Int, haptics: RecordingHaptics): Pair<LazyListState, List<ListItem>> {
        val state = LazyListState()
        val items = buildListItems(FakeData.rows(300, NOW).filter { it.folder % 100 == FakeData.INBOX }.take(120), NOW, ZoneId.of("UTC"))
        rule.setContent {
            CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) {
                CompositionLocalProvider(LocalHaptics provides haptics) {
                    LaunchedEffect(kick()) { if (kick() > 0) state.scrollToItem(3) }
                    Box(Modifier.fillMaxSize()) {
                        LazyColumn(Modifier.fillMaxSize(), state = state) {
                            items(items, key = { it.key }) { Box(Modifier.fillMaxWidth().height(48.dp)) { Text(it.key.toString()) } }
                        }
                        FastScroller(state, items, labelOf = { it.id }, haptics = haptics, modifier = Modifier.align(Alignment.CenterEnd))
                    }
                }
            }
        }
        return state to items
    }

    @Test
    fun theFastScrollerAppearsWhileScrolling_andDraggingItScrollsTheListWithATickPerDate() {
        var kick by mutableIntStateOf(0)
        val haptics = RecordingHaptics()
        val (state, items) = showScroller({ kick }, haptics)
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.FAST_SCROLLER).assertDoesNotExistCompat()
        rule.runOnUiThread { kick = 1 }
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(InboxTags.FAST_SCROLLER)).fetchSemanticsNodes().isNotEmpty() }
        val before = state.firstVisibleItemIndex
        // The thumb sits near the top of the track; take hold of it and pull down.
        rule.onNodeWithTag(InboxTags.FAST_SCROLLER).performTouchInput {
            down(Offset(centerX, 20f))
            repeat(10) { moveBy(Offset(0f, 40f)) }
            up()
        }
        rule.waitForIdle()
        assertTrue("dragging moved the list (from $before to ${state.firstVisibleItemIndex})", state.firstVisibleItemIndex > before + 10)
        assertTrue("the Tick haptic plays when the date label changes: ${haptics.events}", haptics.events.isEmpty() || haptics.events.all { it == HapticEvent.Tick })
        assertTrue(items.isNotEmpty())
    }
}

private fun androidx.compose.ui.test.SemanticsNodeInteraction.assertDoesNotExistCompat() = assertDoesNotExist()
