// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Phone
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MessageRowData
import com.example.croft.mail.contacts.ContactCard
import com.example.croft.mail.contacts.ContactsBackend
import com.example.croft.mail.contacts.LabeledValue
import com.example.croft.mail.search.SearchBackend
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.DestructiveDialog
import com.example.croft.ui.components.MenuItemSpec
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Test tags of S41. */
object ContactTags {
    const val SCREEN = "contact-screen"
    const val NAME = "contact-name"
    const val MAIL = "contact-mail"
    const val CALL = "contact-call"
    const val EDIT = "contact-edit"
    const val MORE = "contact-more"
    const val KEY = "contact-key"
    const val CONFLICT = "contact-conflict"
    const val PENDING = "contact-pending"
    const val GONE = "contact-gone"
    const val RECENT = "contact-recent"
    fun email(i: Int) = "contact-email-$i"
    fun phone(i: Int) = "contact-phone-$i"
    fun recent(id: Long) = "contact-recent-$id"
}

/** How many recent conversations S41 lists. */
private const val RECENT_COUNT = 3

/**
 * S41 Contact: the avatar, name and organisation; Mail, Call and Edit; the addresses (with "Key attached" when the card carries a
 * key) and numbers with their kinds; the postal address, birthday and notes; the recent conversations from the device's index;
 * Delete in the overflow menu, which asks first. [onMail] gets the address to write to; the dialler gets the number (no
 * permission: `ACTION_DIAL` only fills it in).
 */
@Composable
fun ContactScreen(
    backend: ContactsBackend,
    account: Long,
    id: Long,
    search: SearchBackend?,
    onBack: () -> Unit,
    onEdit: (ContactCard) -> Unit,
    onMail: (account: Long, address: String) -> Unit,
    onOpenMessage: (Long) -> Unit,
    onDeleted: (account: Long) -> Unit,
    modifier: Modifier = Modifier,
) {
    var card by remember(account, id) { mutableStateOf<ContactCard?>(null) }
    var gone by remember(account, id) { mutableStateOf(false) }
    var recent by remember(account, id) { mutableStateOf<List<MessageRowData>>(emptyList()) }
    var menuOpen by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }
    val snackbar = remember { CroftSnackbarState() }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val noDialler = stringResource(R.string.contact_no_dialler)
    val deleteFailed = stringResource(R.string.contact_delete_failed)
    BackHandler { onBack() }
    LaunchedEffect(account, id) {
        val c = try { backend.get(account, id) } catch (e: CancellationException) { throw e } catch (e: Exception) { null }
        card = c
        gone = c == null
        val address = c?.form?.emails?.firstOrNull()?.value
        if (address != null && search != null) recent = recentWith(search, account, address)
    }
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(ContactTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.contact_back), onBack)
                Box(Modifier.weight(1f))
                val c = card
                if (c != null) {
                    CroftIconButton(Icons.Filled.Edit, stringResource(R.string.contact_edit), { onEdit(c) }, Modifier.testTag(ContactTags.EDIT))
                    Box {
                        CroftIconButton(Icons.Filled.MoreVert, stringResource(R.string.contact_more), { menuOpen = true }, Modifier.testTag(ContactTags.MORE))
                        CroftMenu(
                            expanded = menuOpen,
                            onDismiss = { menuOpen = false },
                            items = listOf(MenuItemSpec(stringResource(R.string.contact_delete), { menuOpen = false; confirmDelete = true }, Icons.Filled.Delete, destructive = true)),
                        )
                    }
                }
            }
            val c = card
            when {
                gone -> Text(stringResource(R.string.contact_gone), Modifier.padding(24.dp).testTag(ContactTags.GONE), style = MaterialTheme.typography.bodyLarge)
                c != null -> ContactBody(
                    card = c,
                    recent = recent,
                    onMail = { address -> onMail(c.account, address) },
                    onCall = { number ->
                        try {
                            context.startActivity(Intent(Intent.ACTION_DIAL, Uri.fromParts("tel", number, null)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        } catch (e: ActivityNotFoundException) {
                            snackbar.show(noDialler)
                        }
                    },
                    onOpenMessage = onOpenMessage,
                )
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars))
    }
    val c = card
    if (confirmDelete && c != null) {
        DestructiveDialog(
            title = stringResource(R.string.contact_delete_title),
            message = stringResource(R.string.contact_delete_message),
            confirmLabel = stringResource(R.string.contact_delete_confirm),
            onConfirm = {
                confirmDelete = false
                scope.launch {
                    val done = try { backend.delete(c.account, c.id); true } catch (e: CancellationException) { throw e } catch (e: Exception) { false }
                    if (done) onDeleted(c.account) else snackbar.show(deleteFailed)
                }
            },
            onDismiss = { confirmDelete = false },
        )
    }
}

/** The last [RECENT_COUNT] messages from or to [address] in the device's index, newest first. */
private suspend fun recentWith(search: SearchBackend, account: Long, address: String): List<MessageRowData> {
    // The address is quoted as one value; a quote inside it could end the value early, so it is dropped.
    val a = address.replace("\"", "")
    if (a.isBlank()) return emptyList()
    val rows = ArrayList<MessageRowData>()
    for (q in listOf("from:\"$a\"", "to:\"$a\"")) {
        try {
            rows += search.local(account, q, RECENT_COUNT).hits.map { it.row }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Nothing to show from this query.
        }
    }
    return rows.distinctBy { it.id }.sortedByDescending { it.dateMs }.take(RECENT_COUNT)
}

@Composable
private fun ContactBody(
    card: ContactCard,
    recent: List<MessageRowData>,
    onMail: (String) -> Unit,
    onCall: (String) -> Unit,
    onOpenMessage: (Long) -> Unit,
) {
    val form = card.form
    val email = form.emails.firstOrNull()?.value
    val phone = form.phones.firstOrNull()?.value
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        LetterAvatar(card.name, HERO_AVATAR)
        Text(card.name, Modifier.testTag(ContactTags.NAME).semantics { heading() }, style = MaterialTheme.typography.headlineSmall, textAlign = TextAlign.Center)
        if (form.organization.isNotBlank()) Text(form.organization, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
        Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
            if (email != null) ActionButton(Icons.Filled.Email, stringResource(R.string.contact_mail), stringResource(R.string.contact_mail_to, email), Modifier.testTag(ContactTags.MAIL)) { onMail(email) }
            if (phone != null) ActionButton(Icons.Filled.Call, stringResource(R.string.contact_call), stringResource(R.string.contact_call_number, phone), Modifier.testTag(ContactTags.CALL)) { onCall(phone) }
        }
        if (card.conflict) Note(stringResource(R.string.contact_conflict), Modifier.testTag(ContactTags.CONFLICT))
        if (card.pending) Note(stringResource(R.string.contact_pending), Modifier.testTag(ContactTags.PENDING))
        if (form.emails.isNotEmpty() || form.phones.isNotEmpty()) {
            Surface(Modifier.fillMaxWidth().widthIn(max = 640.dp), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow) {
                Column(Modifier.padding(vertical = 4.dp)) {
                    form.emails.forEachIndexed { i, v ->
                        ValueRow(Icons.Filled.Email, v, Modifier.testTag(ContactTags.email(i)), key = card.hasKey && i == 0, onClick = { onMail(v.value) })
                    }
                    form.phones.forEachIndexed { i, v ->
                        ValueRow(Icons.Filled.Phone, v, Modifier.testTag(ContactTags.phone(i)), onClick = { onCall(v.value) })
                    }
                }
            }
        }
        form.addresses.firstOrNull()?.let { a -> Field(stringResource(R.string.contact_address), a.lines().joinToString("\n")) }
        form.birthday?.takeIf { it.isNotBlank() }?.let { Field(stringResource(R.string.contact_birthday), it) }
        if (form.note.isNotBlank()) Field(stringResource(R.string.contact_notes), form.note)
        if (email != null) {
            Column(Modifier.fillMaxWidth().widthIn(max = 640.dp).testTag(ContactTags.RECENT), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.contact_recent), Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                if (recent.isEmpty()) {
                    Text(stringResource(R.string.contact_recent_none), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                for (m in recent) {
                    Surface(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(MaterialTheme.shapes.large).clickable(role = Role.Button) { onOpenMessage(m.id) }.testTag(ContactTags.recent(m.id)),
                        shape = MaterialTheme.shapes.large,
                        color = MaterialTheme.colorScheme.surfaceContainerLow,
                    ) {
                        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                            Text(m.subject, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (m.snippet.isNotBlank()) Text(m.snippet, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ActionButton(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, description: String, modifier: Modifier, onClick: () -> Unit) {
    Column(
        modifier.clip(MaterialTheme.shapes.large).clickable(role = Role.Button, onClickLabel = description, onClick = onClick).padding(8.dp).widthIn(min = 64.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Box(Modifier.size(56.dp).clip(MaterialTheme.shapes.extraLarge).background(MaterialTheme.colorScheme.secondaryContainer), contentAlignment = Alignment.Center) {
            Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSecondaryContainer)
        }
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
private fun ValueRow(icon: androidx.compose.ui.graphics.vector.ImageVector, value: LabeledValue, modifier: Modifier, key: Boolean = false, onClick: () -> Unit) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Column(Modifier.weight(1f)) {
            Text(value.value, style = MaterialTheme.typography.bodyLarge)
            kindLabel(value.kinds)?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        if (key) {
            Surface(Modifier.testTag(ContactTags.KEY), shape = MaterialTheme.shapes.small, color = MaterialTheme.colorScheme.secondaryContainer) {
                Text(stringResource(R.string.contact_key), Modifier.padding(horizontal = 12.dp, vertical = 6.dp), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSecondaryContainer)
            }
        }
    }
}

/** The kind of an address or number in words, for the kinds Croft knows; null otherwise. */
@Composable
private fun kindLabel(kinds: List<String>): String? {
    val k = kinds.map { it.lowercase() }
    return when {
        "cell" in k || "mobile" in k -> stringResource(R.string.contact_kind_mobile)
        "work" in k -> stringResource(R.string.contact_kind_work)
        "home" in k -> stringResource(R.string.contact_kind_home)
        "other" in k -> stringResource(R.string.contact_kind_other)
        else -> null
    }
}

@Composable
private fun Field(label: String, value: String) {
    Column(Modifier.fillMaxWidth().widthIn(max = 640.dp)) {
        Text(label, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
        Text(value, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun Note(text: String, modifier: Modifier) {
    Text(text, modifier.fillMaxWidth().widthIn(max = 640.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

private val HERO_AVATAR = 96.dp
