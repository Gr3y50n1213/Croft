// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.RustInfo
import com.example.croft.mail.MailSession
import com.example.croft.mail.accounts.Diagnostics
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import kotlinx.coroutines.launch

/**
 * S60 About and diagnostics: the version, the privacy policy and the source code (plain view intents: the link opens in the person's
 * browser, no permission is used), the open-source licences ([onOpenLicences]), "Copy diagnostics", "Erase all Croft data on this
 * device" (a confirmation that lists what goes, then [eraseAll], which removes every account and its password, clears the settings
 * and returns to S01) and the note that Croft is not affiliated with Google, Microsoft or Apple.
 *
 * The diagnostics text is [Diagnostics.build]: versions, the number of accounts and their states, nothing else.
 */
@Composable
fun AboutScreen(
    info: RustInfo?,
    session: MailSession,
    onOpenLicences: () -> Unit,
    eraseAll: suspend () -> Boolean,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    openUrl: (String) -> Boolean = LocalContext.current.let { context -> { url: String -> context.openInBrowser(url) } },
    copyText: (label: String, text: String) -> Unit = LocalContext.current.let { context -> { label: String, text: String -> context.copyToClipboard(label, text) } },
) {
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val linkFailed = stringResource(R.string.about_link_failed)
    val copied = stringResource(R.string.about_copied)
    val eraseFailed = stringResource(R.string.about_erase_failed)
    var erasing by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val version = if (info != null) stringResource(R.string.about_version_value, info.croftVersion) else stringResource(R.string.about_version_unknown)

    val open = { url: String -> if (!openUrl(url)) scope.launch { snackbar.showSnackbar(linkFailed) }; Unit }

    SettingsFrame(
        title = stringResource(R.string.about_title), onBack = onBack, snackbarHostState = snackbar,
        modifier = modifier.testTag(AccountsTags.ABOUT),
    ) {
        AboutRow(stringResource(R.string.about_version), version, Modifier.testTag(AccountsTags.VERSION), onClick = null)
        AboutRow(stringResource(R.string.about_privacy_policy), stringResource(R.string.about_privacy_hint), Modifier.testTag(AccountsTags.PRIVACY)) { open(AboutLinks.PRIVACY_POLICY_URL) }
        AboutRow(stringResource(R.string.about_licences), stringResource(R.string.about_licences_hint), Modifier.testTag(AccountsTags.LICENCES), onClick = onOpenLicences)
        AboutRow(stringResource(R.string.about_source), stringResource(R.string.about_source_hint), Modifier.testTag(AccountsTags.SOURCE)) { open(AboutLinks.SOURCE_URL) }
        AboutRow(stringResource(R.string.about_copy_diagnostics), stringResource(R.string.about_copy_hint), Modifier.testTag(AccountsTags.COPY)) {
            scope.launch {
                val accounts = try { session.repository.listAccounts() } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (e: Exception) { emptyList() }
                val text = Diagnostics.build(
                    appVersion = info?.croftVersion ?: "unknown",
                    androidVersion = "${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
                    ffiApiVersion = info?.ffiApiVersion?.toString() ?: "unknown",
                    accounts = accounts,
                )
                copyText("Croft diagnostics", text)
                snackbar.showSnackbar(copied)
            }
        }
        CroftButton(
            stringResource(R.string.about_erase), onClick = { erasing = true },
            Modifier.fillMaxWidth().testTag(AccountsTags.ERASE_ALL), kind = ButtonKind.OUTLINED, enabled = !busy,
        )
        Text(stringResource(R.string.about_erase_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            stringResource(R.string.about_not_affiliated), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 8.dp).testTag(AccountsTags.NOT_AFFILIATED),
        )
    }

    if (erasing) {
        EraseAllDialog(
            onDismiss = { erasing = false },
            onConfirm = {
                erasing = false; busy = true
                scope.launch { val ok = eraseAll(); busy = false; if (!ok) snackbar.showSnackbar(eraseFailed) }
            },
        )
    }
}

/** One line of S60: a title and a hint; tappable when [onClick] is given. */
@Composable
private fun AboutRow(title: String, hint: String, modifier: Modifier, onClick: (() -> Unit)?) {
    Surface(modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerLow) {
        ListItem(
            modifier = if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier,
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            headlineContent = { Text(title) },
            supportingContent = { Text(hint) },
        )
    }
}

/**
 * The open-source licences screen: a plain list of every library Croft is built with and its licence, grouped by where it comes from.
 * The list is [Licences] (a checked-in file CI keeps current). A list that cannot be read says so.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LicencesScreen(onBack: () -> Unit, modifier: Modifier = Modifier, entries: List<LicenceEntry> = Licences.load(LocalContext.current)) {
    val groups = remember(entries) {
        listOf(
            LicenceEntry.Origin.ANDROID to R.string.licences_group_android,
            LicenceEntry.Origin.RUST to R.string.licences_group_rust,
            LicenceEntry.Origin.NATIVE to R.string.licences_group_native,
            LicenceEntry.Origin.FONT to R.string.licences_group_font,
            LicenceEntry.Origin.OTHER to R.string.licences_group_other,
        ).mapNotNull { (origin, title) -> entries.filter { it.origin == origin }.takeIf { it.isNotEmpty() }?.let { title to it } }
    }
    Scaffold(
        modifier = modifier.testTag(AccountsTags.LICENCES_SCREEN),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.licences_title)) },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.nav_back)) } },
            )
        },
    ) { inner ->
        LazyColumn(Modifier.padding(inner).testTag(AccountsTags.LICENCE_LIST)) {
            item {
                Text(
                    stringResource(if (entries.isEmpty()) R.string.licences_unavailable else R.string.licences_intro),
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
            groups.forEach { (title, list) ->
                item(key = "group-$title") {
                    Text(
                        stringResource(title), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp).semantics { heading() },
                    )
                }
                items(list, key = { "${it.origin}-${it.name}-${it.version}" }) { entry ->
                    val description = stringResource(R.string.licences_entry_description, entry.name, entry.version, entry.licence)
                    ListItem(
                        modifier = Modifier.semantics(mergeDescendants = true) { contentDescription = description },
                        headlineContent = { Text(if (entry.version.isEmpty()) entry.name else "${entry.name} ${entry.version}") },
                        supportingContent = { Text(entry.licence) },
                    )
                }
            }
        }
    }
}
