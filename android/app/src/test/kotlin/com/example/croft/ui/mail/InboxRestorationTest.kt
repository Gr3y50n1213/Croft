// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.FileAnchorStore
import com.example.croft.mail.MessageScope
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.settingsAt
import com.example.croft.ui.components.show
import java.io.File
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private val inboxScope = MessageScope.UnifiedInbox

/** The position of the loaded row [index] among the items of the list, as the screen builds them. */
private fun itemIndexOf(f: InboxFixture, index: Int, loaded: Int): Int {
    val rows = f.repo.snapshot(inboxScope).take(loaded)
    return buildListItems(rows, NOW, ZoneId.of("UTC")).indexOfFirst { it is ListItem.Row && it.row.id == rows[index].id }
}

/**
 * The scroll place of the Inbox survives rotation, returning from another screen, process death and a cold start: the anchor
 * is a message id and a dp offset (docs/ux/message-list.md), kept in `rememberSaveable` and in a persisted per-folder store.
 * Each test scrolls to a row, nudges the list a few pixels so the offset is not zero, and compares the row's top edge before
 * and after.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class InboxRestorationTest {
    @get:Rule
    val rule = createComposeRule()

    private fun nudge() {
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipe(Offset(centerX, centerY), Offset(centerX, centerY - 33f), durationMillis = 1_000) }
        rule.waitForIdle()
    }

    /** Scroll row [index] of the first page to the top, nudge, and return its id and top edge. */
    private fun placeAt(f: InboxFixture, index: Int, loaded: Int = 50): Pair<Long, Float> {
        val id = f.repo.snapshot(inboxScope)[index].id
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(itemIndexOf(f, index, loaded))
        rule.waitForIdle()
        nudge()
        return id to rule.rowTop(id).value
    }

    private fun assertRestored(id: Long, before: Float) {
        rule.waitForRow(id)
        rule.waitForIdle()
        assertEquals("the same row at the same offset", before, rule.rowTop(id).value, 1f)
    }

    // ---- Rotation: StateRestorationTester ----

    @Test
    fun rotation_keepsTheAnchorRowAndItsOffset() {
        val f = InboxFixture()
        val tester = StateRestorationTester(rule)
        tester.setContent { CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) { InboxScreen(f.session, onOpenSettings = {}) } }
        rule.waitForRow(f.topId)
        val (id, before) = placeAt(f, 25)
        tester.emulateSavedInstanceStateRestore()
        assertRestored(id, before)
    }

    @Test
    fun rotation_keepsTheSelectionToo() {
        val f = InboxFixture()
        val tester = StateRestorationTester(rule)
        tester.setContent { CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) { InboxScreen(f.session, onOpenSettings = {}) } }
        rule.waitForRow(f.topId)
        rule.rowCard(f.topId).performTouchInput { longClick() }
        rule.waitForIdle()
        rule.onNodeWithText("1 selected").assertIsDisplayed()
        tester.emulateSavedInstanceStateRestore()
        rule.waitForRow(f.topId)
        rule.onNodeWithText("1 selected").assertIsDisplayed()
        rule.rowCard(f.topId).assertIsSelectedCompat()
    }

    @Test
    fun rotation_atTheTopStaysAtTheTop_evenIfAnOldAnchorIsStored() {
        val f = InboxFixture()
        // An old anchor is in the store (from an earlier visit); the person is at the top when the screen rotates.
        f.anchors.save(inboxScope.key, f.repo.snapshot(inboxScope)[30].let { com.example.croft.mail.ListAnchor(it.id, it.dateMs, 0f) })
        val tester = StateRestorationTester(rule)
        tester.setContent { CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) { InboxScreen(f.session, onOpenSettings = {}) } }
        // Not an assertion of the first start (it restores the stored place); the rotation below is what is checked.
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(0)
        rule.waitForIdle()
        tester.emulateSavedInstanceStateRestore()
        rule.waitForIdle()
        rule.waitForRow(f.inboxIds().first())
        assertTrue("a rotation at the top does not fall back to the stored place", rule.rowTop(f.inboxIds().first()).value < 400f)
    }

    // ---- Process death with the state restorer: a new session, so only the saved state can know the place ----

    @Test
    fun processDeath_aNewProcessGetsThePlaceFromTheSavedState_notFromTheMemoryOfTheOldOne() {
        var current = InboxFixture()
        val tester = StateRestorationTester(rule)
        tester.setContent { CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) { InboxScreen(current.session, onOpenSettings = {}) } }
        rule.waitForRow(current.topId)
        val (id, before) = placeAt(current, 20)
        val fresh = InboxFixture() // a new repository, an empty cache and an empty anchor store
        assertNull(fresh.session.cached(inboxScope.key))
        assertNull(fresh.anchors.load(inboxScope.key))
        current = fresh
        tester.emulateSavedInstanceStateRestore()
        assertRestored(id, before)
    }

    @Test
    fun processDeath_aPlaceBeyondTheFirstPageIsFoundByPagingForward() {
        var current = InboxFixture(500)
        val tester = StateRestorationTester(rule)
        tester.setContent { CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) { InboxScreen(current.session, onOpenSettings = {}) } }
        rule.waitForRow(current.topId)
        // Page twice so that row 100 exists in the list.
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(itemIndexOf(current, 40, 50))
        rule.waitUntil(10_000) { current.repo.requests.size >= 2 }
        rule.waitForIdle()
        val loaded = current.session.cached(inboxScope.key)!!.rows.size
        assertTrue(loaded >= 150)
        val (id, before) = placeAt(current, 100, loaded)
        val fresh = InboxFixture(500)
        current = fresh
        tester.emulateSavedInstanceStateRestore()
        // Row 100 is past the first 50 and inside the next 100: two requests find it.
        rule.waitUntil(20_000) { fresh.repo.requests.size >= 2 }
        assertRestored(id, before)
        assertEquals("the fresh repository was paged forward until the anchor was loaded", 50, fresh.repo.requests.first().limit)
    }

    // ---- Returning from another screen: the persisted anchor, no saved state ----

    @Test
    fun returningFromAnotherScreen_restoresThePlaceFromTheAnchorStore() {
        val f = InboxFixture()
        var inInbox by mutableStateOf(true)
        rule.show(AnimationLevel.OFF, f.haptics) {
            if (inInbox) InboxScreen(f.session, onOpenSettings = {}) else androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.testTag("other"))
        }
        rule.waitForRow(f.topId)
        val (id, before) = placeAt(f, 18)
        rule.runOnUiThread { inInbox = false }
        rule.waitForIdle()
        rule.onNodeWithTag("other").assertIsDisplayed()
        assertNotNull("the place was written to the store when the screen left", f.anchors.load(inboxScope.key))
        rule.runOnUiThread { inInbox = true }
        assertRestored(id, before)
    }

    @Test
    fun theStoredAnchorIsAMessageIdAndADpOffset_neverAPixelOrAnIndex() {
        val f = InboxFixture()
        rule.showInbox(f, AnimationLevel.OFF)
        rule.waitForRow(f.topId)
        val (id, _) = placeAt(f, 12)
        rule.waitForIdle()
        val anchor = f.anchors.load(inboxScope.key)
        assertNotNull(anchor)
        assertEquals(id, anchor!!.messageId)
        assertEquals(f.repo.snapshot(inboxScope).first { it.id == id }.dateMs, anchor.dateMs)
        assertTrue("an offset of a few dp, not a scroll position in pixels", kotlin.math.abs(anchor.offsetDp) < 72f)
    }

    // ---- Cold start: a new store on the same file ----

    @Test
    fun coldStart_theAnchorFileBringsTheListBackToTheSamePlace() {
        val file = File.createTempFile("croft-anchors", ".json")
        try {
            val store1 = FileAnchorStore(file)
            val f1 = InboxFixture(anchors = store1)
            var current by mutableStateOf(f1)
            rule.show(AnimationLevel.OFF, f1.haptics) { key(current) { InboxScreen(current.session, onOpenSettings = {}) } }
            rule.waitForRow(f1.topId)
            val (id, before) = placeAt(f1, 22)
            rule.waitForIdle()
            store1.flush()
            // The process ends; a new one starts with the same file, an empty repository cache and no saved state.
            val f2 = InboxFixture(anchors = FileAnchorStore(file))
            rule.runOnUiThread { current = f2 }
            assertRestored(id, before)
        } finally {
            file.delete()
        }
    }

    @Test
    fun coldStart_aPlaceInAnotherFolderIsNotUsed() {
        val f = InboxFixture()
        f.anchors.save(MessageScope.Folder(1, 107).key, com.example.croft.mail.ListAnchor(f.inboxIds()[30], 1L, 0f))
        rule.showInbox(f, AnimationLevel.OFF)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        assertTrue("the unified Inbox starts at the top", rule.rowTop(f.topId).value < 400f)
        rule.onNodeWithTag(InboxTags.row(f.topId)).assertIsDisplayed()
    }
}

/** The Robolectric activity of the process-death test below: it builds its own content, so it can be recreated. */
class RecreatableInboxActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val f = fixture ?: error("no fixture")
        setContent {
            CroftTheme(settingsAt(AnimationLevel.OFF), systemDark = false) {
                CompositionLocalProvider(LocalHaptics provides f.haptics) { InboxScreen(f.session, onOpenSettings = {}) }
            }
        }
    }

    companion object {
        /** What the next (re)creation of the activity shows. Set by the test; a real process death resets it too. */
        @Volatile var fixture: InboxFixture? = null
    }
}

/**
 * A Robolectric activity recreate that simulates process death: the activity is destroyed with its saved instance state, the
 * "process" is replaced (a new repository, an empty cache, an empty anchor store), and the activity is created again with the
 * saved state. Only the Bundle can tell the new screen where the person was.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class InboxProcessDeathTest {
    @get:Rule
    val rule = createEmptyComposeRule()

    @Test
    fun anActivityRecreateWithANewProcessKeepsTheAnchorRowAndOffset() {
        val old = InboxFixture()
        RecreatableInboxActivity.fixture = old
        val controller = Robolectric.buildActivity(RecreatableInboxActivity::class.java).setup()
        rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(InboxTags.row(old.topId))).fetchSemanticsNodes().isNotEmpty() }
        val id = old.repo.snapshot(inboxScope)[24].id
        rule.onNodeWithTag(InboxTags.LIST).performScrollToIndex(itemIndexOf(old, 24, 50))
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { swipe(Offset(centerX, centerY), Offset(centerX, centerY - 33f), durationMillis = 1_000) }
        rule.waitForIdle()
        val before = rule.onNodeWithTag(InboxTags.row(id)).fetchSemanticsNode().boundsInRoot.top

        RecreatableInboxActivity.fixture = InboxFixture() // the new process
        controller.recreate()
        rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(InboxTags.row(id))).fetchSemanticsNodes().isNotEmpty() }
        rule.waitForIdle()
        val after = rule.onNodeWithTag(InboxTags.row(id)).fetchSemanticsNode().boundsInRoot.top
        assertEquals("the same row at the same offset after the recreate", before, after, 1f)
        controller.pause().stop().destroy()
    }
}

private fun androidx.compose.ui.test.SemanticsNodeInteraction.assertIsSelectedCompat() = assertIsSelected()
