// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.OnboardingFailure
import com.example.croft.mail.onboarding.ProbeRow
import com.example.croft.mail.onboarding.ProbeRows
import com.example.croft.mail.onboarding.RowState
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/** Test tags of the sign-in screens. Screen IDs are the ones of design/screens.csv. */
object OnboardingTags {
    const val WELCOME = "S01"
    const val ADD_ACCOUNT = "S02"
    const val FINDING = "S03"
    const val SIGN_IN = "S04"
    const val SETTINGS = "S05"
    const val FIRST_SYNC = "S09"

    const val ADD_BUTTON = "S01-add-account"
    const val SAMPLE_BUTTON = "S01-try-sample"
    const val COMPANION_BUTTON = "S01-companion"
    const val SAMPLE_PROBLEM = "S01-sample-problem"
    const val PRIVACY_LINK = "S01-privacy-policy"

    const val EMAIL = "S02-email"
    const val CONTINUE = "S02-continue"
    fun shortcut(name: String) = "S02-shortcut-$name"

    fun probeRow(row: ProbeRow) = "S03-probe-${row.name.lowercase()}"
    const val FINDING_FAILURE = "S03-failure"
    const val FINDING_RETRY = "S03-retry"
    const val FINDING_MANUAL = "S03-manual"
    const val FINDING_CANCEL = "S03-cancel"

    const val USERNAME = "S04-username"
    const val PASSWORD = "S04-password"
    const val SIGN_IN_BUTTON = "S04-sign-in"
    const val SIGN_IN_SETTINGS = "S04-server-settings"
    const val PROVIDER_HINT = "S04-hint"
    const val SIGN_IN_FAILURE = "S04-failure"

    const val TRUST_DIALOG = "trust-dialog"
    const val TRUST_CONFIRM = "trust-confirm"
    const val TRUST_CANCEL = "trust-cancel"
    const val TRUST_FINGERPRINT = "trust-fingerprint"

    const val HOST = "S05-host"
    const val PORT = "S05-port"
    const val SECURITY = "S05-security"
    const val SETTINGS_USERNAME = "S05-username"
    const val SETTINGS_PASSWORD = "S05-password"
    const val ADVANCED = "S05-advanced"
    const val OUT_HOST = "S05-out-host"
    const val TEST_BUTTON = "S05-test"
    const val ADD_THIS = "S05-add"
    const val SETTINGS_FAILURE = "S05-failure"
    fun testStep(name: String) = "S05-step-${name.lowercase()}"

    const val SYNC_CONTINUE = "S09-continue"
    const val SYNC_SKELETON = "S09-skeleton"
}

private val ContentMaxWidth = 560.dp
private val Gutter = 16.dp

/**
 * The frame of every sign-in screen: a top bar (with a Back arrow unless [onBack] is null, as on S09), and a body that is one
 * scrolling column no wider than 560 dp, centred on wide windows. The body moves above the keyboard.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun OnboardingFrame(
    title: String,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.appearance_back))
                        }
                    }
                },
            )
        },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner).imePadding(), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(
                Modifier.widthIn(max = ContentMaxWidth).fillMaxWidth().weight(1f).verticalScroll(rememberScrollState())
                    .padding(horizontal = Gutter, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
                content = content,
            )
        }
    }
}

/** A large heading inside a screen. */
@Composable
internal fun ScreenHeading(text: String, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.headlineSmall, modifier = modifier.semantics { heading() })
}

/**
 * A failure in words, with its warning icon (never colour alone) and a Details link that opens the technical lines. Announced
 * politely when it appears. [details] are plain sentences (a host and port, the reason a probe found nothing); none is a raw code.
 */
@Composable
internal fun FailureNotice(failure: OnboardingFailure, details: List<String>, modifier: Modifier = Modifier) {
    var open by rememberSaveable { mutableStateOf(false) }
    Column(modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite }, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
            Icon(Icons.Filled.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(20.dp))
            Text(failureText(failure), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
        }
        TextButton(onClick = { open = !open }) { Text(stringResource(if (open) R.string.onb_hide_details else R.string.onb_details)) }
        if (open) {
            val lines = details.ifEmpty { listOf(stringResource(R.string.onb_details_none)) }
            lines.forEach { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
    }
}

/** The label of a security mode. */
@Composable
internal fun securityLabel(mode: SecurityMode): String =
    stringResource(if (mode == SecurityMode.TLS) R.string.onb_security_tls else R.string.onb_security_starttls)

/** The technical lines behind Details for a sign-in or add failure: where Croft tried to connect. */
@Composable
internal fun serverDetails(server: ServerSettings?): List<String> =
    if (server == null) emptyList() else listOf(stringResource(R.string.onb_details_server, server.host, server.port.toInt(), securityLabel(server.security)))

/** The technical lines behind Details for a failed lookup: what each place said, in words. */
@Composable
internal fun probeDetails(rows: ProbeRows): List<String> = ProbeRow.entries.mapNotNull { row ->
    val state = rows[row]
    if (state is RowState.NoMatch) {
        stringResource(R.string.onb_details_probe_line, probeLabel(row), stringResource(FailureCopy.probeReasonRes(state.reason)))
    } else null
}

/** The name of a probe row. */
@Composable
internal fun probeLabel(row: ProbeRow): String = stringResource(
    when (row) {
        ProbeRow.BUILT_IN_TABLE -> R.string.onb_probe_built_in
        ProbeRow.AUTOCONFIG -> R.string.onb_probe_autoconfig
        ProbeRow.SRV -> R.string.onb_probe_srv
        ProbeRow.JMAP_WELL_KNOWN -> R.string.onb_probe_jmap
    },
)
