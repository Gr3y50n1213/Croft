// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.search

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.performTextInput
import com.example.croft.CroftTheme
import com.example.croft.mail.search.FilterChip
import com.example.croft.mail.search.InMemorySearchHistory
import com.example.croft.mail.search.SearchController
import com.example.croft.mail.search.ServerSearchResult
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ThemeSettings
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** S27 and S28 on screen: history with Undo and Clear, chips, the Back sequence, notes and highlights. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SearchScreenTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val backs = AtomicInteger()

    private fun show(backend: FakeSearchBackend = FakeSearchBackend(), history: List<String> = emptyList()): SearchController {
        val controller = SearchController(backend, scope, InMemorySearchHistory(history), debounceMs = 0)
        rule.setContent {
            CroftTheme(ThemeSettings.Default.copy(animation = AnimationLevel.OFF), systemDark = false) {
                SearchScreen(controller, searchAccounts, onOpenMessage = {}, onBack = { backs.incrementAndGet() })
            }
        }
        rule.waitForIdle()
        return controller
    }

    private fun waitFor(tag: String) = rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() }

    /** Waits for [cond]; on a timeout the failure says what the controller and the backend saw. */
    private fun waitOrExplain(c: SearchController, backend: FakeSearchBackend, cond: () -> Boolean) {
        try {
            rule.waitUntil(20_000, cond)
        } catch (e: Throwable) {
            throw AssertionError("state=${c.state.value.copy(hits = emptyList())} hits=${c.state.value.hits.size} calls=${backend.calls}", e)
        }
    }

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    @Test
    fun aHistoryItemSearchesAgain_removeHasUndo_andClearEmptiesIt() {
        val c = show(history = listOf("lunch", "budget"))
        rule.onNodeWithTag(SearchTags.historyItem("lunch")).performClick()
        waitFor(SearchTags.result(1))
        c.clear()
        rule.waitForIdle()
        rule.onNodeWithTag(SearchTags.historyRemove("budget")).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(SearchTags.historyItem("budget")).assertDoesNotExist()
        rule.onNodeWithText("Undo").performClick()
        waitFor(SearchTags.historyItem("budget"))
        rule.onNodeWithTag(SearchTags.MENU).performClick()
        rule.onNodeWithText("Clear history").performClick()
        rule.waitForIdle()
        assertTrue(c.state.value.history.isEmpty())
        rule.onNodeWithText("Your recent searches show here. They stay on this device.").assertExists()
    }

    @Test
    fun chipsChangeTheQuery() {
        val backend = FakeSearchBackend()
        val c = show(backend)
        rule.onNodeWithTag(SearchTags.chip(FilterChip.UNREAD)).performClick()
        rule.waitForIdle()
        assertEquals("is:unread", c.state.value.query)
        rule.onNodeWithTag(SearchTags.chip(FilterChip.UNREAD)).assertIsSelected()
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput("lunch ")
        rule.onNodeWithTag(SearchTags.chip(FilterChip.ATTACHMENT)).performClick()
        rule.waitForIdle()
        assertTrue(c.state.value.query, FilterChip.ATTACHMENT.isOn(c.state.value.query) && FilterChip.UNREAD.isOn(c.state.value.query))
        // The account chips come after the filter chips, past the edge of a narrow screen: their click action, not a tap at a position.
        rule.onNodeWithTag(SearchTags.scope(2)).performSemanticsAction(SemanticsActions.OnClick)
        waitOrExplain(c, backend) { backend.calls.any { it.startsWith("local:2:") } }
    }

    @Test
    fun backGoesFromResultsToRecentSearchesThenToTheInbox_withTheQueryCleared() {
        val c = show(history = listOf("lunch"))
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput("lunch")
        waitFor(SearchTags.result(1))
        back()
        assertEquals("", c.state.value.query)
        waitFor(SearchTags.HISTORY)
        assertEquals(0, backs.get())
        back()
        assertEquals(1, backs.get())
    }

    @Test
    fun offlineShowsTheLocalResultsAndTheLine_andNoResultsOffersSearchServer() {
        val backend = FakeSearchBackend(server = ServerSearchResult(emptyList(), offline = true))
        val c = show(backend)
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput("lunch")
        waitFor(SearchTags.result(1))
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Can't reach the server. Showing results from this device.")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput(" nothing")
        waitFor(SearchTags.EMPTY)
        rule.onNodeWithText("No results for “lunch nothing”").assertExists()
        rule.waitForIdle()
        val before = backend.calls.count { it == "server:null:lunch nothing" }
        rule.onNodeWithTag(SearchTags.SERVER_BUTTON, useUnmergedTree = true).assertExists()
        rule.onNode(hasClickAction() and hasAnyDescendant(hasText("Search server"))).performSemanticsAction(SemanticsActions.OnClick)
        waitOrExplain(c, backend) { backend.calls.count { it == "server:null:lunch nothing" } > before }
    }

    @Test
    fun aBadDateSaysHowToWriteOne() {
        show()
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput("after:bad")
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Use a date like 2026-11-12 after before: or after:.")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun resultsShowTheBodySnippetAsThePreview() {
        show()
        rule.onNodeWithTag(SearchTags.FIELD).performTextInput("lunch")
        waitFor(SearchTags.result(2))
        rule.onNodeWithText("…we could do lunch after the review…").assertExists()
        rule.onNodeWithTag(SearchTags.FIELD).assertTextEquals("lunch")
    }
}
