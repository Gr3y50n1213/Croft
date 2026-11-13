// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.contacts

/**
 * What S42's fields hold: the names, the addresses and numbers (each keeping its kinds), one postal address, the organisation and the notes. A card
 * with no first or last name but a full name shows the full name as the first name.
 */
data class EditorFields(
    val first: String = "",
    val last: String = "",
    val emails: List<LabeledValue> = listOf(LabeledValue("")),
    val phones: List<LabeledValue> = listOf(LabeledValue("")),
    val address: PostalAddress = PostalAddress(),
    val organization: String = "",
    val note: String = "",
) {
    override fun toString(): String = "EditorFields(${emails.size} emails, ${phones.size} phones)"

    companion object {
        /** The fields for [form] (an empty form gives one empty address and one empty number to type in). */
        fun of(form: ContactForm): EditorFields {
            val noParts = form.givenName.isBlank() && form.familyName.isBlank()
            return EditorFields(
                first = if (noParts) form.formattedName else form.givenName,
                last = if (noParts) "" else form.familyName,
                emails = form.emails.ifEmpty { listOf(LabeledValue("")) },
                phones = form.phones.ifEmpty { listOf(LabeledValue("")) },
                address = form.addresses.firstOrNull() ?: PostalAddress(),
                organization = form.organization,
                note = form.note,
            )
        }
    }
}

/**
 * S42's rules over [original] (an empty form for a new contact): whether anything changed, whether it can be saved, and the form to
 * save. What S42 does not show (kinds, the other postal addresses, the birthday, everything the card has beyond these fields) is
 * kept as it was.
 */
class ContactEditor(val original: ContactForm) {
    /** The fields as they were when S42 opened. */
    val initial: EditorFields = EditorFields.of(original)

    /** Whether [fields] differ from [initial] in anything that would be saved. */
    fun changed(fields: EditorFields): Boolean = normalized(fields) != normalized(initial)

    /** Whether Save is enabled: something changed, and the contact has a name or an address. */
    fun canSave(fields: EditorFields): Boolean = changed(fields) && toForm(fields).hasIdentity

    /** The form to save. */
    fun toForm(fields: EditorFields): ContactForm {
        val namesChanged = fields.first.trim() != initial.first.trim() || fields.last.trim() != initial.last.trim()
        val noParts = original.givenName.isBlank() && original.familyName.isBlank()
        val (given, family, formatted) = when {
            !namesChanged -> Triple(original.givenName, original.familyName, original.formattedName)
            else -> Triple(fields.first.trim(), fields.last.trim(), listOf(fields.first.trim(), fields.last.trim()).filter { it.isNotEmpty() }.joinToString(" "))
        }
        // A card that only had a full name keeps it as it was while the name field is left alone.
        val keepParts = !namesChanged && noParts
        return original.copy(
            formattedName = formatted,
            givenName = if (keepParts) original.givenName else given,
            familyName = if (keepParts) original.familyName else family,
            emails = fields.emails,
            phones = fields.phones,
            addresses = if (fields.address.isBlank) original.addresses.drop(1) else listOf(fields.address) + original.addresses.drop(1),
            organization = fields.organization.trim(),
            note = fields.note.trim(),
        ).cleaned()
    }

    private fun normalized(f: EditorFields) = f.copy(
        first = f.first.trim(),
        last = f.last.trim(),
        emails = f.emails.map { it.copy(value = it.value.trim()) }.filter { it.value.isNotEmpty() },
        phones = f.phones.map { it.copy(value = it.value.trim()) }.filter { it.value.isNotEmpty() },
        address = f.address.copy(street = f.address.street.trim(), locality = f.address.locality.trim(), region = f.address.region.trim(), postalCode = f.address.postalCode.trim(), country = f.address.country.trim()),
        organization = f.organization.trim(),
        note = f.note.trim(),
    )
}
