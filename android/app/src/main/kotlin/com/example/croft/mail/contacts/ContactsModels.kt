// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.contacts

import java.text.Normalizer

// Names, addresses and numbers never reach a log: every class here prints counts only (CLAUDE.md, security rules).

/** One row of S40. [sortKey] is the core's (family name first, lower case); [email] is the first address. */
data class ContactRow(val id: Long, val account: Long, val name: String, val sortKey: String, val email: String?) {
    override fun toString(): String = "ContactRow($account/$id)"
}

/** An address or a number with its kinds ("work", "home", "cell") as the card has them. */
data class LabeledValue(val value: String, val kinds: List<String> = emptyList(), val preferred: Boolean = false) {
    override fun toString(): String = "LabeledValue(${kinds.size} kinds)"
}

/** A postal address. */
data class PostalAddress(
    val street: String = "",
    val locality: String = "",
    val region: String = "",
    val postalCode: String = "",
    val country: String = "",
    val kinds: List<String> = emptyList(),
) {
    /** Whether every part is blank. */
    val isBlank: Boolean get() = listOf(street, locality, region, postalCode, country).all { it.isBlank() }

    /** The parts that are filled, one line each (street, then town with region and code, then country). */
    fun lines(): List<String> = listOf(
        street,
        listOf(locality, region, postalCode).filter { it.isNotBlank() }.joinToString(" "),
        country,
    ).map { it.trim() }.filter { it.isNotEmpty() }

    override fun toString(): String = "PostalAddress"
}

/** What S42 edits (and S41 shows). Blank values are dropped by [cleaned]. */
data class ContactForm(
    val formattedName: String = "",
    val givenName: String = "",
    val familyName: String = "",
    val emails: List<LabeledValue> = emptyList(),
    val phones: List<LabeledValue> = emptyList(),
    val addresses: List<PostalAddress> = emptyList(),
    val organization: String = "",
    val note: String = "",
    val birthday: String? = null,
) {
    /** The form as it is saved: values trimmed, blank addresses, numbers and postal addresses left out. */
    fun cleaned(): ContactForm = copy(
        formattedName = formattedName.trim(),
        givenName = givenName.trim(),
        familyName = familyName.trim(),
        emails = emails.map { it.copy(value = it.value.trim()) }.filter { it.value.isNotEmpty() },
        phones = phones.map { it.copy(value = it.value.trim()) }.filter { it.value.isNotEmpty() },
        addresses = addresses.map { it.copy(street = it.street.trim(), locality = it.locality.trim(), region = it.region.trim(), postalCode = it.postalCode.trim(), country = it.country.trim()) }.filterNot { it.isBlank },
        organization = organization.trim(),
        note = note.trim(),
    )

    /** Whether there is something to call the contact by: a name or an address. */
    val hasIdentity: Boolean get() = cleaned().let { it.formattedName.isNotEmpty() || it.givenName.isNotEmpty() || it.familyName.isNotEmpty() || it.emails.isNotEmpty() }

    /** The name shown: the full name, else given and family, else the first address. */
    val displayName: String get() = cleaned().let { c ->
        c.formattedName.ifEmpty { listOf(c.givenName, c.familyName).filter { it.isNotEmpty() }.joinToString(" ") }.ifEmpty { c.emails.firstOrNull()?.value.orEmpty() }
    }

    override fun toString(): String = "ContactForm(${emails.size} emails, ${phones.size} phones, ${addresses.size} addresses)"
}

/** One contact as S41 shows it. */
data class ContactCard(
    val id: Long,
    val account: Long,
    val book: Long,
    val name: String,
    val form: ContactForm,
    /** The card carries a public key. */
    val hasKey: Boolean,
    /** The server changed it too: this copy was kept as a new contact. */
    val conflict: Boolean,
    /** Changed here and not sent yet. */
    val pending: Boolean,
) {
    override fun toString(): String = "ContactCard($account/$id)"
}

/** An address book of an account. */
data class AddressBook(val id: Long, val account: Long, val name: String)

/** How a contacts sync of one account went. */
enum class ContactsSync {
    /** Done. */
    DONE,

    /** The account's server offers no contacts. */
    NOT_OFFERED,

    /** The server could not be reached (or refused the request for now). */
    UNREACHABLE,

    /** The server wants the person to sign in again. */
    SIGN_IN_NEEDED,
}

/** The contacts calls of the core (FFI v19). */
interface ContactsBackend {
    /** Sends this device's changes for [account], then brings in the server's. */
    suspend fun sync(account: Long): ContactsSync

    /** The address books of [account] (known after its first sync). */
    suspend fun books(account: Long): List<AddressBook>

    /** Contacts of [account] (every account when null) whose name or address contains [query]: at most [limit] after [offset], by sort key. */
    suspend fun list(account: Long?, query: String, limit: Int, offset: Int): List<ContactRow>

    /** One contact, or null when it is gone. */
    suspend fun get(account: Long, id: Long): ContactCard?

    /** Saves a new contact in [book] (when [id] is null) or changes [id]. Returns the contact's id. */
    suspend fun save(account: Long, id: Long?, book: Long, form: ContactForm): Long

    /** Deletes a contact (sent at the next sync). False when it was already gone. */
    suspend fun delete(account: Long, id: Long): Boolean

    /** The page size the core allows for [list]. */
    val pageSize: Int get() = 500
}

/** The A to Z sections of S40. */
object ContactSections {
    /** The heading for things that do not start with a letter. */
    const val OTHER = "#"

    /** The letters of the index, then [OTHER]. */
    val INDEX: List<String> = ('A'..'Z').map { it.toString() } + OTHER

    /** The section of [sortKey]: its first letter without accents, upper case; else [OTHER]. */
    fun of(sortKey: String): String {
        val first = sortKey.trim().firstOrNull() ?: return OTHER
        val base = Normalizer.normalize(first.toString(), Normalizer.Form.NFD).firstOrNull() ?: return OTHER
        val upper = base.uppercaseChar()
        return if (upper in 'A'..'Z') upper.toString() else OTHER
    }

    /** [rows] in the order S40 shows them: by section (A to Z, then [OTHER]), then by sort key with accents ignored, then id. */
    fun order(rows: List<ContactRow>): List<ContactRow> {
        val collator = java.text.Collator.getInstance(java.util.Locale.ROOT).apply { strength = java.text.Collator.PRIMARY }
        return rows.sortedWith(
            compareBy<ContactRow> { INDEX.indexOf(of(it.sortKey)) }
                .then { a, b -> collator.compare(a.sortKey, b.sortKey) }
                .thenBy { it.account }
                .thenBy { it.id },
        )
    }
}
