// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.contacts

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** An account as S40's chips and S42's picker name it. */
data class ContactsAccount(val id: Long, val label: String, val isDemo: Boolean = false) {
    override fun toString(): String = "ContactsAccount($id)"
}

/** One line of S40 (a heading or a contact). */
sealed interface ContactsItem {
    /** A letter heading. */
    data class Header(val letter: String) : ContactsItem

    /** A contact. */
    data class Row(val row: ContactRow) : ContactsItem
}

/** What S40 shows. */
data class ContactsState(
    val query: String = "",
    /** The account chip chosen; null is All. */
    val filter: Long? = null,
    /** The accounts that have an address book (the chips, and S42's "Save to"). */
    val accounts: List<ContactsAccount> = emptyList(),
    /** Each account's books, for S42. */
    val books: Map<Long, List<AddressBook>> = emptyMap(),
    val rows: List<ContactRow> = emptyList(),
    /** False until the first list was read. */
    val loaded: Boolean = false,
    /** A sync is running (the pull indicator). */
    val refreshing: Boolean = false,
    /** The accounts whose last sync could not reach the server or needs a sign-in, by id. */
    val problems: Map<Long, ContactsSync> = emptyMap(),
    /** True once every account was synced at least once since S40 opened. */
    val syncedOnce: Boolean = false,
) {
    /** The list with its letter headings. */
    val items: List<ContactsItem> get() = buildList {
        var last: String? = null
        for (r in rows) {
            val s = ContactSections.of(r.sortKey)
            if (s != last) { add(ContactsItem.Header(s)); last = s }
            add(ContactsItem.Row(r))
        }
    }

    /** The letters that have a section. */
    val letters: Set<String> get() = rows.mapTo(LinkedHashSet()) { ContactSections.of(it.sortKey) }

    override fun toString(): String = "ContactsState(${rows.size} rows, filter=$filter, loaded=$loaded, refreshing=$refreshing)"
}

/**
 * S40's state: the list (A to Z, the search, the account filter) and the syncs. [accounts] lists the session's accounts; every one
 * except the sample mailbox is synced on [start] and on [refresh], one after another, and the list is read again after each.
 */
class ContactsController(
    private val backend: ContactsBackend,
    private val accounts: suspend () -> List<ContactsAccount>,
    private val scope: CoroutineScope,
) {
    private val mutable = MutableStateFlow(ContactsState())

    /** What the screen draws. */
    val state: StateFlow<ContactsState> get() = mutable

    private var syncJob: Job? = null
    private var loadJob: Job? = null
    private var all: List<ContactsAccount> = emptyList()

    /** Reads the list and the books, then syncs every account. Called when S40 opens. */
    fun start() {
        scope.launch {
            all = runCatching { accounts() }.getOrDefault(emptyList())
            readBooks()
            reload()
            refresh()
        }
    }

    /** Pull to refresh: syncs every account again (nothing when a sync is running). */
    fun refresh() {
        if (syncJob?.isActive == true) return
        syncJob = scope.launch {
            mutable.update { it.copy(refreshing = true) }
            try {
                for (a in all.filterNot { it.isDemo }) {
                    val result = try {
                        backend.sync(a.id)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        ContactsSync.UNREACHABLE
                    }
                    mutable.update { s ->
                        val problems = if (result == ContactsSync.UNREACHABLE || result == ContactsSync.SIGN_IN_NEEDED) s.problems + (a.id to result) else s.problems - a.id
                        s.copy(problems = problems)
                    }
                    readBooks()
                    reload()
                }
            } finally {
                mutable.update { it.copy(refreshing = false, syncedOnce = true) }
            }
        }
    }

    /** Syncs one account in the background (after a save or a delete), then reads the list again. */
    fun syncInBackground(account: Long) {
        scope.launch {
            val result = try { backend.sync(account) } catch (e: CancellationException) { throw e } catch (e: Exception) { ContactsSync.UNREACHABLE }
            mutable.update { s ->
                s.copy(problems = if (result == ContactsSync.UNREACHABLE || result == ContactsSync.SIGN_IN_NEEDED) s.problems + (account to result) else s.problems - account)
            }
            reload()
        }
    }

    /** The search text changed. */
    fun setQuery(query: String) {
        if (query == mutable.value.query) return
        mutable.update { it.copy(query = query) }
        scope.launch { reload() }
    }

    /** An account chip was tapped (null for All). */
    fun setFilter(account: Long?) {
        mutable.update { it.copy(filter = account) }
        scope.launch { reload() }
    }

    /** Reads the list again (after S42 saved, or S41 deleted). */
    fun reloadNow() { scope.launch { reload() } }

    private suspend fun readBooks() {
        val books = HashMap<Long, List<AddressBook>>()
        for (a in all) {
            val b = try { backend.books(a.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { emptyList() }
            if (b.isNotEmpty()) books[a.id] = b
        }
        mutable.update { s ->
            val withBooks = all.filter { it.id in books }
            s.copy(books = books, accounts = withBooks, filter = s.filter?.takeIf { f -> withBooks.any { it.id == f } })
        }
    }

    private suspend fun reload() {
        loadJob?.cancel()
        val job = scope.launch {
            val s = mutable.value
            val out = ArrayList<ContactRow>()
            try {
                while (out.size < MAX_ROWS) {
                    val page = backend.list(s.filter, s.query.trim(), backend.pageSize, out.size)
                    out.addAll(page)
                    if (page.size < backend.pageSize) break
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // The list could not be read: what was shown stays.
                mutable.update { it.copy(loaded = true) }
                return@launch
            }
            // A newer query or filter started meanwhile: its own reload sets the list.
            if (mutable.value.query != s.query || mutable.value.filter != s.filter) return@launch
            mutable.update { it.copy(rows = ContactSections.order(out.take(MAX_ROWS)), loaded = true) }
        }
        loadJob = job
        job.join()
    }

    private companion object {
        /** The most rows S40 reads (the core keeps at most this many per account in one list). */
        const val MAX_ROWS = 10_000
    }
}
