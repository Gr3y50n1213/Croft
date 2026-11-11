// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.Attempt
import com.example.croft.mail.onboarding.OnboardingState
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftTextField
import uniffi.croft_ffi.KnownProvider

/**
 * S04 Sign in. The account card, the login name (edit it if it is not the whole address), the password with a show or hide
 * button and autofill hints, and Sign in. A wrong password shows **inline under the field in plain words, with no dialog**. Gmail
 * and Outlook (and iCloud, Fastmail) show the text that they need an app password before the person types
 * (`docs/store/READINESS.md`, row 10). There is no "sign in with Google or Microsoft" button until T21 builds it. An untrusted
 * certificate asks once, in [TrustDialog].
 *
 * **The password** is typed into a field of this screen only (`remember`, never saved state, never a navigation argument). On
 * Sign in it is turned into UTF-8 bytes and handed to [onSignIn]; the flow wipes those bytes when the attempt ends. (The text
 * the keyboard field holds is a JVM String that cannot be wiped; it is dropped with the screen.)
 */
@Composable
fun SignInScreen(
    state: OnboardingState,
    onSignIn: (username: String, password: ByteArray) -> Unit,
    onServerSettings: () -> Unit,
    onTrust: () -> Unit,
    onCancelTrust: () -> Unit,
    onPasswordEdited: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val imap = state.discovered?.imap
    var username by rememberSaveable(imap?.username) { mutableStateOf(imap?.username ?: state.email) }
    var password by remember { mutableStateOf("") }
    val working = state.attempt is Attempt.Working
    val failure = (state.attempt as? Attempt.Failed)?.failure
    val focus = LocalFocusManager.current
    val canSubmit = username.isNotBlank() && password.isNotEmpty() && !working
    val submit = { if (canSubmit) onSignIn(username, password.toByteArray(Charsets.UTF_8)) }
    OnboardingFrame(stringResource(R.string.onb_sign_in_title), onBack, modifier.testTag(OnboardingTags.SIGN_IN)) {
        AccountCard(state.email, imap?.host)
        CroftTextField(
            value = username, onValueChange = { username = it },
            label = stringResource(R.string.onb_username_label),
            modifier = Modifier.testTag(OnboardingTags.USERNAME),
            enabled = !working,
            contentType = ContentType.Username,
            keyboardType = KeyboardType.Email,
            imeAction = ImeAction.Next,
        )
        CroftTextField(
            value = password,
            onValueChange = { password = it; if (failure != null) onPasswordEdited() },
            label = stringResource(R.string.onb_password_label),
            modifier = Modifier.testTag(OnboardingTags.PASSWORD),
            enabled = !working,
            password = true,
            contentType = ContentType.Password,
            imeAction = ImeAction.Done,
            onImeAction = { focus.clearFocus(); submit() },
            errorText = if (failure?.isPasswordProblem == true) failureText(failure) else null,
        )
        ProviderHint(state.discovered?.provider)
        CroftButton(
            stringResource(R.string.onb_sign_in), { submit() },
            Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.SIGN_IN_BUTTON),
            enabled = canSubmit || working,
            progress = if (working) ButtonProgress.LOADING else ButtonProgress.IDLE,
        )
        if (failure != null && !failure.isPasswordProblem) {
            FailureNotice(failure, serverDetails(imap), Modifier.testTag(OnboardingTags.SIGN_IN_FAILURE))
        }
        CroftButton(
            stringResource(R.string.onb_server_settings_link), onServerSettings,
            Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(OnboardingTags.SIGN_IN_SETTINGS), kind = ButtonKind.TEXT, enabled = !working,
        )
    }
    (state.attempt as? Attempt.Trust)?.let { TrustDialog(it, onTrust, onCancelTrust) }
}

/** The avatar letter, the address and the server of the account being added. */
@Composable
private fun AccountCard(email: String, host: String?) {
    Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            Box(Modifier.size(40.dp).background(MaterialTheme.colorScheme.primary, CircleShape), contentAlignment = Alignment.Center) {
                Text(email.firstOrNull()?.uppercaseChar()?.toString().orEmpty(), color = MaterialTheme.colorScheme.onPrimary, style = MaterialTheme.typography.titleMedium)
            }
            Column {
                Text(email, style = MaterialTheme.typography.titleMedium)
                if (host != null) Text(stringResource(R.string.onb_server_summary, host), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

/** What to say about the password before it is typed: the provider's own rule, or the general hint. */
@Composable
private fun ProviderHint(provider: KnownProvider?) {
    val text = when (provider) {
        KnownProvider.GMAIL -> R.string.onb_hint_gmail
        KnownProvider.OUTLOOK -> R.string.onb_hint_outlook
        KnownProvider.I_CLOUD -> R.string.onb_hint_icloud
        KnownProvider.FASTMAIL -> R.string.onb_hint_fastmail
        null -> R.string.onb_hint_generic
    }
    val notice = provider == KnownProvider.GMAIL || provider == KnownProvider.OUTLOOK
    if (notice) {
        Surface(Modifier.fillMaxWidth().testTag(OnboardingTags.PROVIDER_HINT), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.secondaryContainer) {
            Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
                Icon(Icons.Filled.Info, contentDescription = null, tint = MaterialTheme.colorScheme.onSecondaryContainer, modifier = Modifier.size(20.dp))
                Text(stringResource(text), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
            }
        }
    } else {
        Text(
            stringResource(text), Modifier.testTag(OnboardingTags.PROVIDER_HINT), style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * Trust on first use. Shows the server and the certificate's fingerprint (so the person can compare it with the one the server
 * shows), with "Trust this server" and Cancel. A certificate that **changed** after the person trusted one gets a stronger
 * warning (the error colour and a warning icon). Cancel and a tap outside both cancel: nothing is trusted by accident.
 */
@Composable
fun TrustDialog(trust: Attempt.Trust, onTrust: () -> Unit, onCancel: () -> Unit) {
    val host = trust.host.ifEmpty { stringResource(R.string.onb_host_fallback) }
    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(OnboardingTags.TRUST_DIALOG),
        icon = if (trust.changed) ({ Icon(Icons.Filled.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error) }) else null,
        title = { Text(stringResource(if (trust.changed) R.string.onb_trust_changed_title else R.string.onb_trust_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    stringResource(if (trust.changed) R.string.onb_trust_changed_body else R.string.onb_trust_body, host),
                    color = if (trust.changed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                )
                Text(stringResource(R.string.onb_trust_host_label), style = MaterialTheme.typography.labelMedium)
                Text(host, style = MaterialTheme.typography.bodyMedium)
                Text(stringResource(R.string.onb_trust_fingerprint_label), style = MaterialTheme.typography.labelMedium)
                Text(trust.fingerprint, Modifier.testTag(OnboardingTags.TRUST_FINGERPRINT), style = MaterialTheme.typography.bodyMedium)
            }
        },
        dismissButton = { TextButton(onCancel, Modifier.testTag(OnboardingTags.TRUST_CANCEL)) { Text(stringResource(R.string.control_cancel)) } },
        confirmButton = { TextButton(onTrust, Modifier.testTag(OnboardingTags.TRUST_CONFIRM)) { Text(stringResource(R.string.onb_trust_confirm)) } },
    )
}
