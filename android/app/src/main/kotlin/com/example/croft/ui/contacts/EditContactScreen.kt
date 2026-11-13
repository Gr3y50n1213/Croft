// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.contacts.AddressBook
import com.example.croft.mail.contacts.ContactCard
import com.example.croft.mail.contacts.ContactEditor
import com.example.croft.mail.contacts.ContactForm
import com.example.croft.mail.contacts.ContactsAccount
import com.example.croft.mail.contacts.ContactsBackend
import com.example.croft.mail.contacts.EditorFields
import com.example.croft.mail.contacts.LabeledValue
import com.example.croft.mail.contacts.PostalAddress
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.components.DestructiveDialog
import com.example.croft.ui.components.MenuItemSpec
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Test tags of S42. */
object EditContactTags {
    const val SCREEN = "edit-contact-screen"
    const val CLOSE = "edit-contact-close"
    const val SAVE = "edit-contact-save"
    const val FIRST = "edit-contact-first"
    const val LAST = "edit-contact-last"
    const val ADD_EMAIL = "edit-contact-add-email"
    const val ADD_PHONE = "edit-contact-add-phone"
    const val STREET = "edit-contact-street"
    const val ORGANIZATION = "edit-contact-organization"
    const val NOTES = "edit-contact-notes"
    const val SAVE_TO = "edit-contact-save-to"
    fun email(i: Int) = "edit-contact-email-$i"
    fun phone(i: Int) = "edit-contact-phone-$i"
    fun book(id: Long) = "edit-contact-book-$id"
}

/** Where S42 saves to: an account's address book. */
data class BookChoice(val account: Long, val book: Long)

/**
 * S42 Edit contact, for contact [id] of [account], or a new contact when [id] is null (saved to the book chosen under "Save to",
 * among [books]). Save is disabled until something changed and the contact has a name or an address. Close and Back with changes ask
 * "Discard changes?". [onSaved] gets the account and the contact's id; [onClose] leaves without saving.
 */
@Composable
fun EditContactScreen(
    backend: ContactsBackend,
    account: Long?,
    id: Long?,
    accounts: List<ContactsAccount>,
    books: Map<Long, List<AddressBook>>,
    onSaved: (account: Long, id: Long) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val isNew = id == null || account == null
    var original by remember(account, id) { mutableStateOf<ContactCard?>(null) }
    var loaded by remember(account, id) { mutableStateOf(isNew) }
    LaunchedEffect(account, id) {
        if (!isNew) {
            original = try { backend.get(account!!, id!!) } catch (e: CancellationException) { throw e } catch (e: Exception) { null }
            loaded = true
        }
    }
    if (!loaded) return
    val o = original
    if (!isNew && o == null) {
        // The contact went away (deleted elsewhere, or a sync removed it): nothing to edit.
        LaunchedEffect(Unit) { onClose() }
        return
    }
    EditContactForm(
        backend = backend,
        original = o,
        accounts = accounts,
        books = books,
        onSaved = onSaved,
        onClose = onClose,
        modifier = modifier,
    )
}

@Composable
private fun EditContactForm(
    backend: ContactsBackend,
    original: ContactCard?,
    accounts: List<ContactsAccount>,
    books: Map<Long, List<AddressBook>>,
    onSaved: (account: Long, id: Long) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier,
) {
    val editor = remember(original) { ContactEditor(original?.form ?: ContactForm()) }
    var fields by rememberSaveable(original?.id, stateSaver = EditorFieldsSaver) { mutableStateOf(editor.initial) }
    val firstBook = remember(accounts, books) { accounts.firstNotNullOfOrNull { a -> books[a.id]?.firstOrNull()?.let { BookChoice(a.id, it.id) } } }
    var choiceAccount by rememberSaveable { mutableStateOf(firstBook?.account) }
    var choiceBook by rememberSaveable { mutableStateOf(firstBook?.book) }
    val target: BookChoice? = when {
        original != null -> BookChoice(original.account, original.book)
        choiceAccount != null && choiceBook != null -> BookChoice(choiceAccount!!, choiceBook!!)
        else -> null
    }
    var saving by remember { mutableStateOf(false) }
    var askDiscard by remember { mutableStateOf(false) }
    var pickerOpen by remember { mutableStateOf(false) }
    val snackbar = remember { CroftSnackbarState() }
    val scope = rememberCoroutineScope()
    val failed = stringResource(R.string.edit_contact_save_failed)
    val changed = editor.changed(fields)
    val canSave = editor.canSave(fields) && target != null && !saving
    val close: () -> Unit = {
        if (changed) {
            askDiscard = true
        } else {
            onClose()
        }
    }
    BackHandler { close() }
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(EditContactTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars).imePadding()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                CroftIconButton(Icons.Filled.Close, stringResource(R.string.edit_contact_close), close, Modifier.testTag(EditContactTags.CLOSE))
                Text(
                    stringResource(if (original == null) R.string.edit_contact_title_new else R.string.edit_contact_title),
                    Modifier.weight(1f).padding(start = 8.dp).semantics { heading() },
                    style = MaterialTheme.typography.titleLarge,
                )
                CroftButton(
                    label = stringResource(R.string.edit_contact_save),
                    onClick = {
                        val t = target ?: return@CroftButton
                        saving = true
                        scope.launch {
                            try {
                                val saved = backend.save(t.account, original?.id, t.book, editor.toForm(fields))
                                onSaved(t.account, saved)
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                snackbar.show(failed)
                            } finally {
                                saving = false
                            }
                        }
                    },
                    modifier = Modifier.padding(end = 8.dp).testTag(EditContactTags.SAVE),
                    enabled = canSave || saving,
                    progress = if (saving) ButtonProgress.LOADING else ButtonProgress.IDLE,
                    disabledReason = when {
                        !changed -> stringResource(R.string.edit_contact_save_unchanged)
                        !editor.toForm(fields).hasIdentity -> stringResource(R.string.edit_contact_save_needs_name)
                        else -> null
                    },
                    onExplain = { snackbar.show(it) },
                )
            }
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                val w = Modifier.fillMaxWidth().widthIn(max = 640.dp)
                Row(w, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    CroftTextField(fields.first, { fields = fields.copy(first = it) }, stringResource(R.string.edit_contact_first), Modifier.weight(1f).testTag(EditContactTags.FIRST))
                    CroftTextField(fields.last, { fields = fields.copy(last = it) }, stringResource(R.string.edit_contact_last), Modifier.weight(1f).testTag(EditContactTags.LAST))
                }
                fields.emails.forEachIndexed { i, v ->
                    CroftTextField(
                        v.value, { t -> fields = fields.copy(emails = fields.emails.toMutableList().also { it[i] = v.copy(value = t) }) },
                        stringResource(R.string.edit_contact_email), w.testTag(EditContactTags.email(i)),
                        keyboardType = KeyboardType.Email,
                        trailingIcon = if (fields.emails.size > 1 || v.value.isNotEmpty()) Icons.Filled.Close else null,
                        trailingIconDescription = stringResource(R.string.edit_contact_remove_email),
                        onTrailingClick = { fields = fields.copy(emails = fields.emails.filterIndexed { j, _ -> j != i }.ifEmpty { listOf(LabeledValue("")) }) },
                    )
                }
                Box(w) { CroftButton(stringResource(R.string.edit_contact_add_email), { fields = fields.copy(emails = fields.emails + LabeledValue("")) }, Modifier.testTag(EditContactTags.ADD_EMAIL), kind = ButtonKind.TEXT, leadingIcon = Icons.Filled.Add) }
                fields.phones.forEachIndexed { i, v ->
                    CroftTextField(
                        v.value, { t -> fields = fields.copy(phones = fields.phones.toMutableList().also { it[i] = v.copy(value = t) }) },
                        stringResource(R.string.edit_contact_phone), w.testTag(EditContactTags.phone(i)),
                        keyboardType = KeyboardType.Phone,
                        trailingIcon = if (fields.phones.size > 1 || v.value.isNotEmpty()) Icons.Filled.Close else null,
                        trailingIconDescription = stringResource(R.string.edit_contact_remove_phone),
                        onTrailingClick = { fields = fields.copy(phones = fields.phones.filterIndexed { j, _ -> j != i }.ifEmpty { listOf(LabeledValue("")) }) },
                    )
                }
                Box(w) { CroftButton(stringResource(R.string.edit_contact_add_phone), { fields = fields.copy(phones = fields.phones + LabeledValue("")) }, Modifier.testTag(EditContactTags.ADD_PHONE), kind = ButtonKind.TEXT, leadingIcon = Icons.Filled.Add) }
                val a = fields.address
                CroftTextField(a.street, { fields = fields.copy(address = a.copy(street = it)) }, stringResource(R.string.edit_contact_street), w.testTag(EditContactTags.STREET))
                Row(w, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    CroftTextField(a.locality, { fields = fields.copy(address = a.copy(locality = it)) }, stringResource(R.string.edit_contact_locality), Modifier.weight(1f))
                    CroftTextField(a.postalCode, { fields = fields.copy(address = a.copy(postalCode = it)) }, stringResource(R.string.edit_contact_postal_code), Modifier.weight(1f))
                }
                Row(w, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    CroftTextField(a.region, { fields = fields.copy(address = a.copy(region = it)) }, stringResource(R.string.edit_contact_region), Modifier.weight(1f))
                    CroftTextField(a.country, { fields = fields.copy(address = a.copy(country = it)) }, stringResource(R.string.edit_contact_country), Modifier.weight(1f))
                }
                CroftTextField(fields.organization, { fields = fields.copy(organization = it) }, stringResource(R.string.edit_contact_organization), w.testTag(EditContactTags.ORGANIZATION))
                CroftTextField(fields.note, { fields = fields.copy(note = it) }, stringResource(R.string.edit_contact_notes), w.testTag(EditContactTags.NOTES))
                if (original == null) {
                    Column(w, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(stringResource(R.string.edit_contact_save_to), Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                        val label = target?.let { t -> bookLabel(accounts, books, t) }.orEmpty()
                        Box {
                            CroftButton(label, { pickerOpen = true }, Modifier.testTag(EditContactTags.SAVE_TO), kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.ArrowDropDown)
                            CroftMenu(
                                expanded = pickerOpen,
                                onDismiss = { pickerOpen = false },
                                items = accounts.flatMap { acc ->
                                    books[acc.id].orEmpty().map { b ->
                                        MenuItemSpec(bookLabel(accounts, books, BookChoice(acc.id, b.id)), { choiceAccount = acc.id; choiceBook = b.id; pickerOpen = false })
                                    }
                                },
                            )
                        }
                    }
                }
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars))
    }
    if (askDiscard) {
        DestructiveDialog(
            title = stringResource(R.string.edit_contact_discard_title),
            message = stringResource(R.string.edit_contact_discard_message),
            confirmLabel = stringResource(R.string.edit_contact_discard),
            onConfirm = { askDiscard = false; onClose() },
            onDismiss = { askDiscard = false },
        )
    }
}

/** "Book (Account)" for the picker. */
@Composable
private fun bookLabel(accounts: List<ContactsAccount>, books: Map<Long, List<AddressBook>>, choice: BookChoice): String {
    val account = accounts.firstOrNull { it.id == choice.account }?.label.orEmpty()
    val book = books[choice.account]?.firstOrNull { it.id == choice.book }?.name.orEmpty()
    return if (book.isBlank()) account else stringResource(R.string.edit_contact_book, book, account)
}

/**
 * Keeps S42's fields across a rotation or process death: a flat list of strings (the saved state is the app's own; it holds what
 * the person typed, as the text fields' own saved state would).
 */
private val EditorFieldsSaver: Saver<EditorFields, Any> = Saver(
    save = { f ->
        val out = arrayListOf(f.first, f.last, f.organization, f.note, f.address.street, f.address.locality, f.address.region, f.address.postalCode, f.address.country, f.address.kinds.joinToString(SEP))
        for (list in listOf(f.emails, f.phones)) {
            out += list.size.toString()
            for (v in list) { out += v.value; out += v.kinds.joinToString(SEP); out += v.preferred.toString() }
        }
        out
    },
    restore = { saved ->
        @Suppress("UNCHECKED_CAST")
        val s = saved as List<String>
        var i = FIXED
        fun values(): List<LabeledValue> {
            val n = s[i++].toInt()
            return List(n) { LabeledValue(s[i++], s[i++].split(SEP).filter { it.isNotEmpty() }, s[i++].toBoolean()) }
        }
        val emails = values()
        val phones = values()
        EditorFields(
            first = s[0], last = s[1], organization = s[2], note = s[3],
            address = PostalAddress(s[4], s[5], s[6], s[7], s[8], s[9].split(SEP).filter { it.isNotEmpty() }),
            emails = emails, phones = phones,
        )
    },
)

private const val SEP = "\u001f"
private const val FIXED = 10
