// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.search

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** Where a search is (`docs/ux/state-machines.md`, Search). */
enum class SearchPhase {
    /** Nothing typed: S27 shows recent searches and chips. */
    IDLE,

    /** Typed, waiting for the 150 ms pause. */
    TYPING,

    /** Local results shown; the server is being asked ("Searching server" at the end of the list). */
    SEARCHING_SERVER,

    /** Local and server results merged (or only local, with [ServerNote] saying why). */
    DONE,
}

/** The line at the end of the results about the server. */
enum class ServerNote {
    /** Nothing to say: the servers were asked and answered. */
    NONE,

    /** A server cannot search: "Showing results from this device only". */
    DEVICE_ONLY,

    /** Offline: the servers were not asked ("Can't reach the server"). */
    OFFLINE,

    /** An account needs a sign-in. */
    SIGN_IN,

    /** A server failed: a banner with Retry; the local results stay. */
    FAILED,
}

/** What S27 and S28 draw. */
data class SearchState(
    val query: String = "",
    val phase: SearchPhase = SearchPhase.IDLE,
    val hits: List<SearchHitData> = emptyList(),
    /** Bodies were not all searched: "Searched headers and downloaded mail", with Search server. */
    val bodiesPartial: Boolean = false,
    val serverNote: ServerNote = ServerNote.NONE,
    /** `before:` or `after:` holds something that is not a date. */
    val badDate: Boolean = false,
    /** null: all accounts. */
    val account: Long? = null,
    val history: List<String> = emptyList(),
)

/**
 * The search screens' logic, without Android: the state machine Idle, Typing (150 ms), Local results, Server results merged, Done.
 * Each new query cancels the work of the previous one (its server request included); a server failure never removes local results.
 * Queries are never logged.
 */
class SearchController(
    private val backend: SearchBackend,
    private val scope: CoroutineScope,
    private val history: SearchHistory,
    account: Long? = null,
    private val debounceMs: Long = DEBOUNCE_MS,
    private val limit: Int = LIMIT,
) {
    private val _state = MutableStateFlow(SearchState(account = account, history = history.list()))
    val state: StateFlow<SearchState> = _state

    private var job: Job? = null

    /** The person typed: wait for the pause, then search. An empty query goes back to Idle at once. */
    fun onQuery(text: String) {
        job?.cancel()
        if (text.isBlank()) {
            _state.value = _state.value.copy(query = text, phase = SearchPhase.IDLE, hits = emptyList(), bodiesPartial = false, serverNote = ServerNote.NONE, badDate = false)
            return
        }
        _state.value = _state.value.copy(query = text, phase = SearchPhase.TYPING)
        job = scope.launch {
            delay(debounceMs)
            run(text)
        }
    }

    /** The search key on the keyboard, a history entry or a chip: search now (no pause) and remember the query. */
    fun submit(text: String = _state.value.query) {
        job?.cancel()
        if (text.isBlank()) return onQuery(text)
        history.add(text)
        _state.value = _state.value.copy(query = text, phase = SearchPhase.TYPING, history = history.list())
        job = scope.launch { run(text) }
    }

    /** Search server (the button in the empty state or under partial results), or Retry after a failure. */
    fun searchServerAgain() {
        val text = _state.value.query
        if (text.isBlank()) return
        job?.cancel()
        job = scope.launch { askServer(text) }
    }

    /** All accounts (null) or one. Searches again. */
    fun setAccount(account: Long?) {
        _state.value = _state.value.copy(account = account)
        if (_state.value.query.isNotBlank()) submit(_state.value.query)
    }

    /** A filter chip: its operator added to or removed from the query, and a search at once. */
    fun toggle(chip: FilterChip) {
        val next = chip.toggle(_state.value.query)
        if (next.isBlank()) onQuery("") else submit(next)
    }

    /** Back from results: the query is cleared and S27 shows again. */
    fun clear() = onQuery("")

    /** Removes a recent search; returns its old place for [undoRemove]. */
    fun removeHistory(query: String): Int {
        val i = history.remove(query)
        _state.value = _state.value.copy(history = history.list())
        return i
    }

    fun undoRemove(query: String, index: Int) {
        history.restore(query, index)
        _state.value = _state.value.copy(history = history.list())
    }

    fun clearHistory() {
        history.clear()
        _state.value = _state.value.copy(history = history.list())
    }

    private suspend fun run(text: String) {
        val account = _state.value.account
        val local = try {
            backend.local(account, text, limit)
        } catch (e: BadSearchDate) {
            _state.value = _state.value.copy(phase = SearchPhase.DONE, hits = emptyList(), badDate = true, bodiesPartial = false, serverNote = ServerNote.NONE)
            return
        }
        if (_state.value.query != text) return
        _state.value = _state.value.copy(phase = SearchPhase.SEARCHING_SERVER, hits = local.hits, bodiesPartial = local.bodiesPartial, badDate = false, serverNote = ServerNote.NONE)
        askServer(text)
    }

    private suspend fun askServer(text: String) {
        val account = _state.value.account
        _state.value = _state.value.copy(phase = SearchPhase.SEARCHING_SERVER, serverNote = ServerNote.NONE)
        val server = try {
            backend.server(account, text)
        } catch (e: CancellationException) {
            throw e
        } catch (e: BadSearchDate) {
            _state.value = _state.value.copy(phase = SearchPhase.DONE, badDate = true)
            return
        } catch (e: Exception) {
            ServerSearchResult(emptyList(), failed = true)
        }
        if (_state.value.query != text) return
        val known = _state.value.hits.map { it.row.id }.toHashSet()
        val merged = _state.value.hits + server.hits.filter { known.add(it.row.id) }
        val note = when {
            server.failed -> ServerNote.FAILED
            server.signInNeeded -> ServerNote.SIGN_IN
            server.offline -> ServerNote.OFFLINE
            server.unsupported -> ServerNote.DEVICE_ONLY
            else -> ServerNote.NONE
        }
        _state.value = _state.value.copy(
            phase = SearchPhase.DONE,
            hits = merged,
            // Once the servers searched everything, the "downloaded mail only" note no longer holds.
            bodiesPartial = _state.value.bodiesPartial && note != ServerNote.NONE,
            serverNote = note,
        )
    }

    companion object {
        const val DEBOUNCE_MS = 150L
        const val LIMIT = 200
    }
}
