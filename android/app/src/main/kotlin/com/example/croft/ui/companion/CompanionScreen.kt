// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.companion

import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.companion.CompanionConnection
import com.example.croft.mail.companion.CompanionProblem
import com.example.croft.mail.companion.CompanionState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Test tags of S06 and S58. */
object CompanionTags {
    const val SCREEN = "companion"
    const val ADDRESS = "companion-address"
    const val CONNECT = "companion-connect"
    const val DISCONNECT = "companion-disconnect"
}

@Composable
private fun problemText(p: CompanionProblem): String = stringResource(
    when (p) {
        CompanionProblem.BAD_ADDRESS -> R.string.companion_problem_bad_address
        CompanionProblem.NOT_A_COMPANION -> R.string.companion_problem_not_a_companion
        CompanionProblem.UNREACHABLE -> R.string.companion_problem_unreachable
        CompanionProblem.SIGN_IN_FAILED -> R.string.companion_problem_sign_in_failed
        CompanionProblem.SIGNED_OUT -> R.string.companion_problem_signed_out
        CompanionProblem.RATE_LIMITED -> R.string.companion_problem_rate_limited
        CompanionProblem.SERVER -> R.string.companion_problem_server
        CompanionProblem.STORAGE -> R.string.companion_problem_storage
    },
)

/**
 * S06 Connect a Companion and, once connected, S58 Companion and devices (T34b). Connecting opens the Companion's own sign-in page in
 * a Custom Tab; the browser comes back through [com.example.croft.CompanionRedirectActivity].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CompanionScreen(connection: CompanionConnection, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val state by connection.state.collectAsState()
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var address by rememberSaveable { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) { withContext(Dispatchers.IO) { connection.load() } }
    LaunchedEffect(state is CompanionState.Connected) {
        if (state is CompanionState.Connected) withContext(Dispatchers.IO) { connection.loadDevices() }
    }

    Scaffold(
        modifier = modifier.testTag(CompanionTags.SCREEN),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.companion_title)) },
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
            when (val s = state) {
                CompanionState.Loading -> Unit
                is CompanionState.NotConnected -> {
                    Text(stringResource(R.string.companion_intro), style = MaterialTheme.typography.bodyLarge)
                    OutlinedTextField(
                        value = address,
                        onValueChange = { address = it },
                        label = { Text(stringResource(R.string.companion_address)) },
                        placeholder = { Text(stringResource(R.string.companion_address_hint)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                        modifier = Modifier.fillMaxWidth().testTag(CompanionTags.ADDRESS),
                    )
                    s.problem?.let { Text(problemText(it), color = MaterialTheme.colorScheme.error) }
                    Button(
                        enabled = !busy && address.isNotBlank(),
                        onClick = {
                            busy = true
                            scope.launch {
                                val page = withContext(Dispatchers.IO) { connection.start(address) }
                                busy = false
                                if (page != null) {
                                    CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, Uri.parse(page))
                                }
                            }
                        },
                        modifier = Modifier.testTag(CompanionTags.CONNECT),
                    ) { Text(stringResource(R.string.companion_connect)) }
                }
                is CompanionState.Waiting -> {
                    Text(stringResource(R.string.companion_waiting), style = MaterialTheme.typography.bodyLarge)
                    OutlinedButton(onClick = { connection.cancel() }) { Text(stringResource(R.string.companion_cancel)) }
                }
                is CompanionState.Connected -> {
                    Text(stringResource(R.string.companion_connected_to, s.url), style = MaterialTheme.typography.titleMedium)
                    if (s.userName.isNotEmpty()) Text(stringResource(R.string.companion_signed_in_as, s.userName))
                    s.problem?.let { Text(problemText(it), color = MaterialTheme.colorScheme.error) }
                    Text(stringResource(R.string.companion_devices), style = MaterialTheme.typography.titleSmall)
                    s.devices?.forEach { d ->
                        ListItem(
                            headlineContent = { Text(d.name) },
                            supportingContent = { if (d.current) Text(stringResource(R.string.companion_this_device)) },
                            trailingContent = {
                                if (!d.current) {
                                    TextButton(onClick = { scope.launch(Dispatchers.IO) { connection.signOutDevice(d.id) } }) {
                                        Text(stringResource(R.string.companion_sign_out_device))
                                    }
                                }
                            },
                        )
                    }
                    OutlinedButton(onClick = { confirming = true }, modifier = Modifier.testTag(CompanionTags.DISCONNECT)) {
                        Text(stringResource(R.string.companion_disconnect))
                    }
                }
            }
        }
    }

    if (confirming) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text(stringResource(R.string.companion_disconnect_title)) },
            text = { Text(stringResource(R.string.companion_disconnect_body)) },
            confirmButton = {
                TextButton(onClick = {
                    confirming = false
                    scope.launch(Dispatchers.IO) { connection.disconnect() }
                }) { Text(stringResource(R.string.companion_disconnect_confirm)) }
            },
            dismissButton = { TextButton(onClick = { confirming = false }) { Text(stringResource(R.string.companion_cancel)) } },
        )
    }
}
