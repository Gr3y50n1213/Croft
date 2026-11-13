// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.contacts

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.ContactAddress
import uniffi.croft_ffi.ContactFields
import uniffi.croft_ffi.ContactValue
import uniffi.croft_ffi.ContactsSyncResult
import uniffi.croft_ffi.Core

/** [ContactsBackend] over the core (FFI v19). Database calls run off the main thread; the sync is the core's own async call. */
class CoreContactsBackend(private val core: Core, private val io: CoroutineDispatcher = Dispatchers.IO) : ContactsBackend {
    override suspend fun sync(account: Long): ContactsSync = when (core.syncContacts(account).result) {
        ContactsSyncResult.DONE -> ContactsSync.DONE
        ContactsSyncResult.NOT_OFFERED -> ContactsSync.NOT_OFFERED
        ContactsSyncResult.UNREACHABLE -> ContactsSync.UNREACHABLE
        ContactsSyncResult.SIGN_IN_NEEDED -> ContactsSync.SIGN_IN_NEEDED
    }

    override suspend fun books(account: Long): List<AddressBook> = withContext(io) {
        core.addressBooks(account).map { AddressBook(it.id, account, it.name) }
    }

    override suspend fun list(account: Long?, query: String, limit: Int, offset: Int): List<ContactRow> = withContext(io) {
        core.listContacts(account, query, limit.coerceIn(0, pageSize).toUInt(), offset.coerceAtLeast(0).toUInt())
            .map { ContactRow(it.id, it.account, it.displayName, it.sortKey, it.email) }
    }

    override suspend fun get(account: Long, id: Long): ContactCard? = withContext(io) {
        core.getContact(account, id)?.let { d ->
            ContactCard(d.id, d.account, d.book, d.displayName, d.fields.toForm(), hasKey = d.hasKey, conflict = d.conflict, pending = d.pending)
        }
    }

    override suspend fun save(account: Long, id: Long?, book: Long, form: ContactForm): Long = withContext(io) {
        core.saveContact(account, id, book, form.cleaned().toFields())
    }

    override suspend fun delete(account: Long, id: Long): Boolean = withContext(io) { core.deleteContact(account, id) }

    companion object {
        internal fun ContactFields.toForm() = ContactForm(
            formattedName = formattedName,
            givenName = givenName,
            familyName = familyName,
            emails = emails.map { LabeledValue(it.value, it.kinds, it.preferred) },
            phones = phones.map { LabeledValue(it.value, it.kinds, it.preferred) },
            addresses = addresses.map { PostalAddress(it.street, it.locality, it.region, it.postalCode, it.country, it.kinds) },
            organization = organization,
            note = note,
            birthday = birthday,
        )

        internal fun ContactForm.toFields() = ContactFields(
            formattedName = formattedName,
            givenName = givenName,
            familyName = familyName,
            emails = emails.map { ContactValue(it.value, it.kinds, it.preferred) },
            phones = phones.map { ContactValue(it.value, it.kinds, it.preferred) },
            addresses = addresses.map { ContactAddress(it.street, it.locality, it.region, it.postalCode, it.country, it.kinds) },
            organization = organization,
            note = note,
            birthday = birthday,
        )
    }
}
