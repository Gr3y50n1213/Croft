// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.search.BadSearchDate
import com.example.croft.mail.search.CoreSearchBackend
import com.example.croft.mail.search.FilterChip
import com.example.croft.mail.search.InMemorySearchHistory
import com.example.croft.mail.search.LocalSearchResult
import com.example.croft.mail.search.SearchBackend
import com.example.croft.mail.search.SearchController
import com.example.croft.mail.search.SearchHitData
import com.example.croft.mail.search.SearchPhase
import com.example.croft.mail.search.ServerNote
import com.example.croft.mail.search.ServerSearchResult
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** S27 and S28 logic: the debounce, local then server, cancelling, the server notes, history and chips; and the real core offline. */
class SearchFlowTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @After fun tearDown() = scope.cancel()

    private fun row(id: Long, subject: String) = MessageRowData(
        id = id, account = 1, folder = 1, senderName = "Ann", senderAddress = "ann@example.org", subject = subject, snippet = "",
        dateMs = id, seen = false, flagged = false, hasAttachments = false, pending = false, threadCount = 1, recipients = null,
    )

    /** Local answers at once; the server answers when [serverGate] completes; records cancelled server calls. */
    private class Fake(val local: List<SearchHitData>, val server: List<SearchHitData>, var result: ServerSearchResult? = null) : SearchBackend {
        val localCalls = CopyOnWriteArrayList<String>()
        val serverCalls = CopyOnWriteArrayList<String>()
        val cancelledServer = CopyOnWriteArrayList<String>()
        var serverGate = CompletableDeferred<Unit>()

        override suspend fun local(account: Long?, query: String, limit: Int): LocalSearchResult {
            localCalls += query
            if ("after:bad" in query) throw BadSearchDate()
            return LocalSearchResult(local, bodiesPartial = true)
        }

        override suspend fun server(account: Long?, query: String): ServerSearchResult {
            serverCalls += query
            try {
                serverGate.await()
            } catch (e: CancellationException) {
                cancelledServer += query
                throw e
            }
            return result ?: ServerSearchResult(server)
        }
    }

    @Test
    fun typingLunchShowsLocalResultsAfterThePause_thenMergesServerHits() {
        val a = SearchHitData(row(1, "Lunch"), listOf(0..4))
        val b = SearchHitData(row(2, "Lunch again"))
        val fake = Fake(local = listOf(a), server = listOf(b, SearchHitData(row(1, "Lunch"))))
        val c = SearchController(fake, scope, InMemorySearchHistory())
        val t0 = System.nanoTime()
        c.onQuery("lunch")
        assertEquals(SearchPhase.TYPING, c.state.value.phase)
        val local = await("local results") { c.state.value.takeIf { it.phase == SearchPhase.SEARCHING_SERVER } }
        val ms = (System.nanoTime() - t0) / 1_000_000
        assertTrue("local results within the pause and a frame ($ms ms)", ms < SearchController.DEBOUNCE_MS + 200)
        assertEquals(listOf(a), local.hits)
        assertTrue(local.bodiesPartial)
        fake.serverGate.complete(Unit)
        val done = await("merged") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
        assertEquals("merged by id, local first, no duplicate", listOf(1L, 2L), done.hits.map { it.row.id })
        assertEquals(listOf(0..4), done.hits[0].subjectMatches)
        assertEquals(ServerNote.NONE, done.serverNote)
        assertFalse("the server searched everything", done.bodiesPartial)
    }

    @Test
    fun typingAgainCancelsThePreviousServerRequest() {
        val fake = Fake(local = emptyList(), server = emptyList())
        val c = SearchController(fake, scope, InMemorySearchHistory())
        c.onQuery("lun")
        await("first server call") { fake.serverCalls.takeIf { it.contains("lun") } }
        c.onQuery("lunch")
        await("the first is cancelled") { fake.cancelledServer.takeIf { it.contains("lun") } }
        await("the second asks") { fake.serverCalls.takeIf { it.contains("lunch") } }
        fake.serverGate.complete(Unit)
        val done = await("done") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
        assertEquals("lunch", done.query)
    }

    @Test
    fun keystrokesInsideThePauseSearchOnlyOnce() {
        val fake = Fake(local = emptyList(), server = emptyList()).also { it.serverGate.complete(Unit) }
        val c = SearchController(fake, scope, InMemorySearchHistory())
        for (q in listOf("l", "lu", "lun", "lunc", "lunch")) c.onQuery(q)
        await("done") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
        assertEquals(listOf("lunch"), fake.localCalls.toList())
    }

    @Test
    fun aServerProblemNeverRemovesLocalResults_andEachNoteIsShown() {
        val a = SearchHitData(row(1, "Lunch"))
        for ((result, note) in listOf(
            ServerSearchResult(emptyList(), offline = true) to ServerNote.OFFLINE,
            ServerSearchResult(emptyList(), unsupported = true) to ServerNote.DEVICE_ONLY,
            ServerSearchResult(emptyList(), failed = true) to ServerNote.FAILED,
            ServerSearchResult(emptyList(), signInNeeded = true) to ServerNote.SIGN_IN,
        )) {
            val fake = Fake(local = listOf(a), server = emptyList(), result = result).also { it.serverGate.complete(Unit) }
            val c = SearchController(fake, scope, InMemorySearchHistory())
            c.submit("lunch")
            val done = await("done") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
            assertEquals(note, done.serverNote)
            assertEquals(listOf(a), done.hits)
        }
    }

    @Test
    fun aBadDateIsShownAndNothingIsAskedOfTheServer() {
        val fake = Fake(local = emptyList(), server = emptyList())
        val c = SearchController(fake, scope, InMemorySearchHistory())
        c.submit("after:bad")
        val done = await("done") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
        assertTrue(done.badDate)
        assertTrue(fake.serverCalls.isEmpty())
    }

    @Test
    fun historyAddRemoveWithUndoAndClear_andEmptyGoesBackToIdle() {
        val fake = Fake(local = emptyList(), server = emptyList()).also { it.serverGate.complete(Unit) }
        val h = InMemorySearchHistory()
        val c = SearchController(fake, scope, h)
        c.submit("lunch")
        c.submit("budget")
        c.submit("lunch")
        assertEquals("newest first, once", listOf("lunch", "budget"), c.state.value.history)
        val at = c.removeHistory("budget")
        assertEquals(listOf("lunch"), c.state.value.history)
        c.undoRemove("budget", at)
        assertEquals(listOf("lunch", "budget"), c.state.value.history)
        c.clearHistory()
        assertTrue(c.state.value.history.isEmpty())
        c.clear()
        assertEquals(SearchPhase.IDLE, c.state.value.phase)
        assertEquals("", c.state.value.query)
    }

    @Test
    fun chipsAddAndRemoveTheirOperator() {
        assertEquals("lunch is:unread", FilterChip.UNREAD.toggle("lunch"))
        assertEquals("lunch", FilterChip.UNREAD.toggle("lunch is:unread"))
        assertEquals("has:attachment", FilterChip.ATTACHMENT.toggle(""))
        assertTrue(FilterChip.STARRED.isOn("IS:STARRED x"))
        val fake = Fake(local = emptyList(), server = emptyList()).also { it.serverGate.complete(Unit) }
        val c = SearchController(fake, scope, InMemorySearchHistory())
        c.toggle(FilterChip.UNREAD)
        await("searched") { fake.localCalls.takeIf { it.contains("is:unread") } }
        assertEquals("is:unread", c.state.value.query)
    }

    /** The real core: in airplane mode the device's results show, and the line says the server cannot be reached. */
    @Test(timeout = 60_000)
    fun offlineShowsLocalResultsAndTheCannotReachServerLine() {
        Rig().use { r ->
            val rows = r.awaitInbox(3)
            val word = rows.first().subject.split(" ").first { it.length > 2 }
            r.core.setNetworkAvailable(false)
            val c = SearchController(CoreSearchBackend(r.core), scope, InMemorySearchHistory(), account = r.account)
            c.submit(word)
            val done = await("done") { c.state.value.takeIf { it.phase == SearchPhase.DONE } }
            assertTrue("local results for $word", done.hits.isNotEmpty())
            assertEquals(ServerNote.OFFLINE, done.serverNote)
            assertTrue(done.hits.all { it.subjectMatches.isNotEmpty() })
        }
    }
}
