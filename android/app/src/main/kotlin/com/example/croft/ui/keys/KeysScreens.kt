// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.keys

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.accounts.AccountPrefs
import com.example.croft.mail.keys.KeyAccount
import com.example.croft.mail.keys.KeyState
import com.example.croft.mail.keys.KeysBackend
import com.example.croft.mail.keys.KeysFailure
import com.example.croft.mail.keys.KeysProblem
import com.example.croft.mail.keys.OwnKey
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.launch

/** Test tags of S54 and S55. */
object KeysTags {
    const val S54 = "keys"
    const val S55 = "key"
    fun card(account: Long) = "keys-card-$account"
    fun status(account: Long) = "keys-status-$account"
    fun open(account: Long) = "keys-open-$account"
    fun sign(account: Long) = "keys-sign-$account"
    fun encrypt(account: Long) = "keys-encrypt-$account"
    fun generate(account: Long) = "keys-generate-$account"
    fun import(account: Long) = "keys-import-$account"
    const val FINGERPRINT = "key-fingerprint"
    const val EXPORT = "key-export"
    const val BACKUP = "key-backup"
    const val BACKUP_ONE = "key-backup-one"
    const val BACKUP_TWO = "key-backup-two"
    const val BACKUP_SAVE = "key-backup-save"
    const val REMOVE = "key-remove"
    const val REMOVE_CONFIRM = "key-remove-confirm"
    const val CHOOSE_FILE = "key-choose-file"
    const val PASTE = "key-paste"
    const val IMPORT_PASSPHRASE = "key-import-passphrase"
    const val IMPORT_GO = "key-import-go"
    const val MESSAGE = "key-message"
}

/** Where key files go and come from: the system's save and open dialogs in the app, a fake in tests. */
interface KeyFiles {
    /** Asks where to save [text] as [name]; [done] gets true once it is written. */
    fun save(name: String, text: String, done: (Boolean) -> Unit)

    /** Asks for a file; [done] gets its bytes (at most [MAX_KEY_FILE]), or null when none was chosen or it could not be read. */
    fun open(done: (ByteArray?) -> Unit)

    companion object {
        /** The largest key file read (the core takes 1 MiB). */
        const val MAX_KEY_FILE = 1024 * 1024
    }
}

/** [KeyFiles] through the system's document dialogs (the app's own storage access, no permission needed). */
@Composable
fun rememberSystemKeyFiles(): KeyFiles {
    val context = LocalContext.current
    var pendingSave by remember { mutableStateOf<Pair<String, (Boolean) -> Unit>?>(null) }
    var pendingOpen by remember { mutableStateOf<((ByteArray?) -> Unit)?>(null) }
    val saver = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/pgp-keys")) { uri: Uri? ->
        val (text, done) = pendingSave ?: return@rememberLauncherForActivityResult
        pendingSave = null
        done(
            uri != null && try {
                context.contentResolver.openOutputStream(uri, "wt")?.use { it.write(text.toByteArray(Charsets.UTF_8)) } != null
            } catch (e: Exception) {
                false
            },
        )
    }
    val opener = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        val done = pendingOpen ?: return@rememberLauncherForActivityResult
        pendingOpen = null
        done(
            uri?.let {
                try {
                    context.contentResolver.openInputStream(it)?.use { input -> readAtMost(input, KeyFiles.MAX_KEY_FILE) }
                } catch (e: Exception) {
                    null
                }
            },
        )
    }
    return remember(saver, opener) {
        object : KeyFiles {
            override fun save(name: String, text: String, done: (Boolean) -> Unit) {
                pendingSave = text to done
                saver.launch(name)
            }

            override fun open(done: (ByteArray?) -> Unit) {
                pendingOpen = done
                opener.launch(arrayOf("application/pgp-keys", "application/pgp-encrypted", "text/plain", "application/octet-stream", "*/*"))
            }
        }
    }
}

/** The stream's bytes, or null when there are more than [limit] (minSdk 26 has no `readNBytes`). */
internal fun readAtMost(input: java.io.InputStream, limit: Int): ByteArray? {
    val out = java.io.ByteArrayOutputStream()
    val buf = ByteArray(8192)
    while (true) {
        val n = input.read(buf)
        if (n < 0) break
        out.write(buf, 0, n)
        if (out.size() > limit) {
            buf.fill(0)
            return null
        }
    }
    buf.fill(0)
    return out.toByteArray()
}

@Composable
private fun problemText(p: KeysProblem): String = stringResource(
    when (p) {
        KeysProblem.NO_VAULT -> R.string.key_problem_no_vault
        KeysProblem.VAULT_FAILED -> R.string.key_problem_vault
        KeysProblem.UNREADABLE -> R.string.key_problem_unreadable
        KeysProblem.NOT_ONE_KEY -> R.string.key_problem_not_one
        KeysProblem.TOO_LARGE -> R.string.key_problem_too_large
        KeysProblem.WRONG_PASSPHRASE -> R.string.key_problem_passphrase
        KeysProblem.NO_USABLE_KEY -> R.string.key_problem_unusable
        KeysProblem.GONE -> R.string.key_problem_gone
        KeysProblem.FAILED -> R.string.key_problem_failed
    },
)

@Composable
private fun stateText(state: KeyState?): String = stringResource(
    when (state) {
        KeyState.ACTIVE -> R.string.keys_status_active
        KeyState.EXPIRED -> R.string.keys_status_expired
        KeyState.REVOKED -> R.string.keys_status_revoked
        KeyState.UNUSABLE -> R.string.keys_status_unusable
        null -> R.string.keys_status_none
    },
)

private fun date(ms: Long, zone: ZoneId): String =
    DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withZone(zone).format(Instant.ofEpochMilli(ms))

/**
 * S54 Encryption keys (T42a): a card per account with its key's state, the defaults for new messages, and Generate or Import when it
 * has no key. Tapping a key opens S55.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EncryptionKeysScreen(
    backend: KeysBackend,
    prefs: AccountPrefs,
    onPrefsChange: (AccountPrefs) -> Unit,
    onOpenKey: (account: Long) -> Unit,
    onImport: (account: Long) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
) {
    val scope = rememberCoroutineScope()
    var accounts by remember { mutableStateOf<List<KeyAccount>?>(null) }
    var keys by remember { mutableStateOf<Map<Long, OwnKey?>>(emptyMap()) }
    var problem by remember { mutableStateOf<KeysProblem?>(null) }
    var busy by remember { mutableStateOf(false) }
    suspend fun load() {
        try {
            val list = backend.accounts()
            keys = list.associate { it.id to backend.ownKey(it.id) }
            accounts = list
        } catch (e: KeysFailure) {
            accounts = accounts ?: emptyList()
            problem = e.problem
        }
    }
    LaunchedEffect(backend) { load() }
    Scaffold(
        modifier = modifier.testTag(KeysTags.S54),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.keys_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.nav_back)) }
                },
            )
        },
    ) { inner ->
        Column(
            Modifier.padding(inner).padding(horizontal = 16.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            problem?.let { Text(problemText(it), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag(KeysTags.MESSAGE)) }
            val list = accounts
            if (list != null && list.isEmpty()) Text(stringResource(R.string.keys_none))
            list?.forEach { account ->
                val key = keys[account.id]
                Card(Modifier.fillMaxWidth().testTag(KeysTags.card(account.id))) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(account.address, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                            Text(stateText(key?.state), style = MaterialTheme.typography.labelLarge, modifier = Modifier.testTag(KeysTags.status(account.id)))
                        }
                        if (key != null) {
                            ListItem(
                                headlineContent = { Text(stringResource(R.string.keys_key_line, key.shortFingerprint)) },
                                supportingContent = {
                                    Text(key.expiresMs?.let { stringResource(R.string.keys_expires, date(it, zone)) } ?: stringResource(R.string.keys_no_expiry))
                                },
                                modifier = Modifier.testTag(KeysTags.open(account.id)).clickable(role = Role.Button, onClickLabel = stringResource(R.string.keys_open)) {
                                    onOpenKey(account.id)
                                },
                            )
                            DefaultSwitch(
                                label = stringResource(R.string.keys_sign_default),
                                checked = account.id in prefs.signByDefault,
                                tag = KeysTags.sign(account.id),
                            ) { onPrefsChange(prefs.withSignByDefault(account.id, it)) }
                            DefaultSwitch(
                                label = stringResource(R.string.keys_encrypt_default),
                                checked = account.id in prefs.encryptByDefault,
                                tag = KeysTags.encrypt(account.id),
                            ) { onPrefsChange(prefs.withEncryptByDefault(account.id, it)) }
                        } else {
                            Text(stringResource(R.string.keys_no_key_text))
                            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Button(
                                    enabled = !busy,
                                    onClick = {
                                        busy = true
                                        problem = null
                                        scope.launch {
                                            try {
                                                backend.generate(account.id)
                                                load()
                                            } catch (e: KeysFailure) {
                                                problem = e.problem
                                            } finally {
                                                busy = false
                                            }
                                        }
                                    },
                                    modifier = Modifier.testTag(KeysTags.generate(account.id)),
                                ) { Text(stringResource(R.string.keys_generate)) }
                                OutlinedButton(onClick = { onImport(account.id) }, modifier = Modifier.testTag(KeysTags.import(account.id))) {
                                    Text(stringResource(R.string.keys_import))
                                }
                            }
                        }
                    }
                }
            }
            Text(stringResource(R.string.keys_footer), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun DefaultSwitch(label: String, checked: Boolean, tag: String, onChange: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Text(label, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange, modifier = Modifier.testTag(tag))
    }
}

/** What S55 is doing. */
private sealed interface KeyMode {
    data object Details : KeyMode
    data object Backup : KeyMode

    /** A protected key file waiting for its passphrase. The bytes are wiped once used or dropped. */
    class Passphrase(val bytes: ByteArray) : KeyMode
}

/**
 * S55 Key details and import (T42a) for one account: its key's fingerprint, identity and dates; export, a passphrase-protected backup
 * and removal; and import of a key file or pasted key. With no key yet it shows the import part only.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyDetailScreen(
    backend: KeysBackend,
    account: Long,
    files: KeyFiles,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
    /** Reads the text to paste; the system clipboard when null (tests give their own). */
    paste: (() -> String?)? = null,
) {
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    var key by remember { mutableStateOf<OwnKey?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var mode by remember { mutableStateOf<KeyMode>(KeyMode.Details) }
    var message by remember { mutableStateOf<String?>(null) }
    var problem by remember { mutableStateOf<KeysProblem?>(null) }
    var fileProblem by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var one by remember { mutableStateOf("") }
    var two by remember { mutableStateOf("") }
    var importPass by remember { mutableStateOf("") }
    val savedText = stringResource(R.string.key_saved)
    val publicText = stringResource(R.string.key_imported_public)
    val nothingToPaste = stringResource(R.string.key_nothing_to_paste)

    fun clearNotes() {
        message = null
        problem = null
        fileProblem = false
    }

    fun run(block: suspend () -> Unit) {
        busy = true
        clearNotes()
        scope.launch {
            try {
                block()
            } catch (e: KeysFailure) {
                problem = e.problem
            } finally {
                busy = false
            }
        }
    }

    suspend fun reload() {
        key = backend.ownKey(account)
        loaded = true
    }

    fun import(bytes: ByteArray, passphrase: String) = run {
        try {
            val own = backend.import(account, bytes.copyOf(), passphrase)
            (mode as? KeyMode.Passphrase)?.bytes?.fill(0)
            bytes.fill(0)
            mode = KeyMode.Details
            importPass = ""
            if (own == null) message = publicText
            reload()
        } catch (e: KeysFailure) {
            if (e.problem == KeysProblem.WRONG_PASSPHRASE && mode !is KeyMode.Passphrase) {
                // A protected key: ask for its passphrase, keeping the bytes until then.
                mode = KeyMode.Passphrase(bytes)
            } else {
                if (mode !is KeyMode.Passphrase) bytes.fill(0)
                throw e
            }
        }
    }

    // However the screen is left (Back, the system's Back, a route change), a key file waiting for its passphrase is wiped.
    androidx.compose.runtime.DisposableEffect(Unit) {
        onDispose { (mode as? KeyMode.Passphrase)?.bytes?.fill(0) }
    }
    LaunchedEffect(backend, account) {
        try {
            reload()
        } catch (e: KeysFailure) {
            problem = e.problem
            loaded = true
        }
    }
    Scaffold(
        modifier = modifier.testTag(KeysTags.S55),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(if (key != null) R.string.key_title else R.string.key_import_title)) },
                navigationIcon = {
                    IconButton(onClick = {
                        (mode as? KeyMode.Passphrase)?.bytes?.fill(0)
                        onBack()
                    }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.nav_back)) }
                },
            )
        },
    ) { inner ->
        Column(
            Modifier.padding(inner).padding(horizontal = 16.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val shown = message ?: problem?.let { problemText(it) } ?: if (fileProblem) stringResource(R.string.key_problem_file) else null
            shown?.let {
                Text(
                    it,
                    color = if (problem != null || fileProblem) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.testTag(KeysTags.MESSAGE),
                )
            }
            val k = key
            if (k != null) {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.key_fingerprint), style = MaterialTheme.typography.labelLarge)
                        Text(k.groupedFingerprint, fontFamily = FontFamily.Monospace, modifier = Modifier.testTag(KeysTags.FINGERPRINT))
                        HorizontalDivider()
                        Detail(stringResource(R.string.key_identity), k.addresses.joinToString(", "))
                        Detail(stringResource(R.string.key_created), date(k.createdMs, zone))
                        Detail(stringResource(R.string.key_expires), k.expiresMs?.let { date(it, zone) } ?: stringResource(R.string.key_never))
                        Detail(stringResource(R.string.key_status_label), stateText(k.state))
                    }
                }
                ListItem(
                    headlineContent = { Text(stringResource(R.string.key_export_public)) },
                    supportingContent = { Text(stringResource(R.string.key_export_public_hint)) },
                    modifier = Modifier.testTag(KeysTags.EXPORT).clickable(role = Role.Button, enabled = !busy) {
                        run {
                            val text = backend.exportPublic(account, k.fingerprint)
                            files.save("${k.addresses.firstOrNull() ?: "key"}.public.asc", text) { ok ->
                                if (ok) message = savedText else fileProblem = true
                            }
                        }
                    },
                )
                ListItem(
                    headlineContent = { Text(stringResource(R.string.key_backup)) },
                    supportingContent = { Text(stringResource(R.string.key_backup_hint)) },
                    modifier = Modifier.testTag(KeysTags.BACKUP).clickable(role = Role.Button, enabled = !busy) {
                        clearNotes()
                        mode = KeyMode.Backup
                    },
                )
                if (mode == KeyMode.Backup) {
                    Text(stringResource(R.string.key_backup_note), style = MaterialTheme.typography.bodySmall)
                    PassField(one, { one = it }, stringResource(R.string.key_backup_passphrase), KeysTags.BACKUP_ONE)
                    PassField(two, { two = it }, stringResource(R.string.key_backup_again), KeysTags.BACKUP_TWO)
                    val issue = when {
                        two.isNotEmpty() && one != two -> R.string.key_backup_mismatch
                        one.isNotEmpty() && one.length < MIN_PASSPHRASE -> R.string.key_backup_short
                        else -> null
                    }
                    issue?.let { Text(stringResource(it), color = MaterialTheme.colorScheme.error) }
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Button(
                            enabled = !busy && issue == null && one.length >= MIN_PASSPHRASE && one == two,
                            onClick = {
                                val pass = one
                                run {
                                    val text = backend.exportSecret(account, k.fingerprint, pass)
                                    one = ""
                                    two = ""
                                    mode = KeyMode.Details
                                    files.save("${k.addresses.firstOrNull() ?: "key"}.private-backup.asc", text) { ok ->
                                        if (ok) message = savedText else fileProblem = true
                                    }
                                }
                            },
                            modifier = Modifier.testTag(KeysTags.BACKUP_SAVE),
                        ) { Text(stringResource(R.string.key_backup_save)) }
                        TextButton(onClick = { one = ""; two = ""; mode = KeyMode.Details }) { Text(stringResource(R.string.key_cancel)) }
                    }
                }
                ListItem(
                    headlineContent = { Text(stringResource(R.string.key_remove), color = MaterialTheme.colorScheme.error) },
                    supportingContent = { Text(stringResource(R.string.key_remove_hint)) },
                    modifier = Modifier.testTag(KeysTags.REMOVE).clickable(role = Role.Button, enabled = !busy) { confirmRemove = true },
                )
            }
            if (loaded) {
                Text(stringResource(R.string.key_import_section), style = MaterialTheme.typography.titleSmall)
                Text(stringResource(R.string.key_import_text))
                val pending = mode as? KeyMode.Passphrase
                if (pending != null) {
                    Text(stringResource(R.string.key_import_passphrase))
                    PassField(importPass, { importPass = it }, stringResource(R.string.key_backup_passphrase), KeysTags.IMPORT_PASSPHRASE)
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Button(
                            enabled = !busy && importPass.isNotEmpty(),
                            onClick = { import(pending.bytes, importPass) },
                            modifier = Modifier.testTag(KeysTags.IMPORT_GO),
                        ) { Text(stringResource(R.string.key_import_go)) }
                        TextButton(onClick = {
                            pending.bytes.fill(0)
                            importPass = ""
                            mode = KeyMode.Details
                        }) { Text(stringResource(R.string.key_cancel)) }
                    }
                } else {
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Button(
                            enabled = !busy,
                            onClick = {
                                clearNotes()
                                files.open { bytes -> if (bytes == null) fileProblem = true else import(bytes, "") }
                            },
                            modifier = Modifier.testTag(KeysTags.CHOOSE_FILE),
                        ) { Text(stringResource(R.string.key_choose_file)) }
                        OutlinedButton(
                            enabled = !busy,
                            onClick = {
                                clearNotes()
                                val text = (paste ?: { clipboard.getText()?.text })()
                                if (text.isNullOrBlank()) message = nothingToPaste else import(text.toByteArray(Charsets.UTF_8), "")
                            },
                            modifier = Modifier.testTag(KeysTags.PASTE),
                        ) { Text(stringResource(R.string.key_paste)) }
                    }
                }
            }
        }
    }
    val k = key
    if (confirmRemove && k != null) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text(stringResource(R.string.key_remove_title)) },
            text = { Text(stringResource(R.string.key_remove_text)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmRemove = false
                        run {
                            backend.remove(account, k.fingerprint)
                            reload()
                        }
                    },
                    modifier = Modifier.testTag(KeysTags.REMOVE_CONFIRM),
                ) { Text(stringResource(R.string.key_remove_confirm), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text(stringResource(R.string.key_cancel)) } },
        )
    }
}

/** The shortest backup passphrase accepted. */
private const val MIN_PASSPHRASE = 8

@Composable
private fun Detail(label: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(0.4f))
        Text(value, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(0.6f))
    }
}

@Composable
private fun PassField(value: String, onChange: (String) -> Unit, label: String, tag: String) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
        modifier = Modifier.fillMaxWidth().testTag(tag),
    )
}
