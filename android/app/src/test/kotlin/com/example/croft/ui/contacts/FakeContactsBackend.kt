// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import com.example.croft.mail.contacts.AddressBook
import com.example.croft.mail.contacts.ContactCard
import com.example.croft.mail.contacts.ContactForm
import com.example.croft.mail.contacts.ContactRow
import com.example.croft.mail.contacts.ContactsBackend
import com.example.croft.mail.contacts.ContactsSync
import com.example.croft.mail.contacts.LabeledValue

/**
 * An in-memory [ContactsBackend]: contacts by account, books by account, a scripted sync result per account, and what was saved,
 * deleted and synced. The sort key is the core's rule (family name first, else the name shown, lower case). Made-up names only.
 */
class FakeContactsBackend : ContactsBackend {
    data class Stored(val account: Long, val id: Long, val book: Long, val form: ContactForm, val hasKey: Boolean = false, val conflict: Boolean = false, val pending: Boolean = false)

    val contacts = ArrayList<Stored>()
    val books = HashMap<Long, List<AddressBook>>()
    val syncResults = HashMap<Long, ContactsSync>()
    val synced = ArrayList<Long>()
    val saved = ArrayList<Triple<Long, Long?, Long>>()
    val deleted = ArrayList<Pair<Long, Long>>()
    var failSave = false
    override var pageSize: Int = 500
    private var nextId = 100L

    fun book(account: Long, id: Long, name: String) { books[account] = books[account].orEmpty() + AddressBook(id, account, name) }

    fun add(account: Long, id: Long, given: String, family: String = "", email: String? = null, phone: String? = null, book: Long = account * 10, hasKey: Boolean = false, organization: String = "", conflict: Boolean = false) {
        contacts += Stored(
            account, id, book,
            ContactForm(
                formattedName = listOf(given, family).filter { it.isNotEmpty() }.joinToString(" "),
                givenName = given, familyName = family,
                emails = listOfNotNull(email?.let { LabeledValue(it, listOf("work")) }),
                phones = listOfNotNull(phone?.let { LabeledValue(it, listOf("cell")) }),
                organization = organization,
            ),
            hasKey = hasKey, conflict = conflict,
        )
    }

    private fun sortKey(f: ContactForm) = (if (f.familyName.isNotBlank()) "${f.familyName} ${f.givenName}" else f.displayName).trim().lowercase()

    override suspend fun sync(account: Long): ContactsSync { synced += account; return syncResults[account] ?: ContactsSync.DONE }

    override suspend fun books(account: Long): List<AddressBook> = books[account].orEmpty()

    override suspend fun list(account: Long?, query: String, limit: Int, offset: Int): List<ContactRow> {
        val q = query.lowercase()
        return contacts.filter { account == null || it.account == account }
            .filter { q.isEmpty() || it.form.displayName.lowercase().contains(q) || it.form.emails.any { e -> e.value.lowercase().contains(q) } }
            .map { ContactRow(it.id, it.account, it.form.displayName, sortKey(it.form), it.form.emails.firstOrNull()?.value) }
            .sortedWith(compareBy({ it.sortKey }, { it.id }))
            .drop(offset).take(limit)
    }

    override suspend fun get(account: Long, id: Long): ContactCard? =
        contacts.firstOrNull { it.account == account && it.id == id }?.let { ContactCard(it.id, it.account, it.book, it.form.displayName, it.form, it.hasKey, it.conflict, it.pending) }

    override suspend fun save(account: Long, id: Long?, book: Long, form: ContactForm): Long {
        if (failSave) throw IllegalStateException("refused")
        saved += Triple(account, id, book)
        return if (id == null) {
            val n = nextId++
            contacts += Stored(account, n, book, form, pending = true)
            n
        } else {
            val i = contacts.indexOfFirst { it.account == account && it.id == id }
            contacts[i] = contacts[i].copy(form = form, pending = true)
            id
        }
    }

    override suspend fun delete(account: Long, id: Long): Boolean {
        deleted += account to id
        return contacts.removeIf { it.account == account && it.id == id }
    }

    /** The stored form of a contact. */
    fun form(account: Long, id: Long): ContactForm = contacts.first { it.account == account && it.id == id }.form
}
