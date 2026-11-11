// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.border
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
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.Attempt
import com.example.croft.mail.onboarding.ConnectionTest
import com.example.croft.mail.onboarding.FieldProblem
import com.example.croft.mail.onboarding.FormField
import com.example.croft.mail.onboarding.OnboardingState
import com.example.croft.mail.onboarding.ServerForm
import com.example.croft.mail.onboarding.StepState
import com.example.croft.mail.onboarding.TestStep
import com.example.croft.ui.appearance.ChoiceRow
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftSpinner
import com.example.croft.ui.components.CroftTextField
import uniffi.croft_ffi.SecurityMode

/**
 * S05 Server settings: protocol, host, port, security and username of the incoming server, a password for the test, and behind
 * "Advanced options" the outgoing server. **There is no unencrypted choice**: Croft only connects over TLS or STARTTLS, also
 * for addresses on a private network (the core refuses an unencrypted connection to any real host). "Test connection" lists its
 * steps in order (reaching the server, the certificate, signing in), stops at the one that failed and says why; "Add account"
 * does the same checks and then adds the account. An untrusted certificate asks once ([TrustDialog]).
 *
 * The password is typed only here (`remember`, never saved) and handed over as bytes, like on S04.
 */
@Composable
fun ServerSettingsScreen(
    state: OnboardingState,
    onFormChange: (ServerForm) -> Unit,
    onTest: (password: ByteArray) -> Unit,
    onAdd: (password: ByteArray) -> Unit,
    onTrust: () -> Unit,
    onCancelTrust: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val form = state.form
    var password by remember { mutableStateOf("") }
    var advanced by rememberSaveable { mutableStateOf(false) }
    val working = state.attempt is Attempt.Working
    val problems = if (state.showFormProblems) form.problems() else emptyMap()
    val failure = (state.attempt as? Attempt.Failed)?.failure
    val canRun = password.isNotEmpty() && !working
    OnboardingFrame(stringResource(R.string.onb_settings_title), onBack, modifier.testTag(OnboardingTags.SETTINGS)) {
        if (state.formGuessed) Notice(stringResource(R.string.onb_settings_guess_notice))
        SectionLabel(stringResource(R.string.onb_incoming))
        CroftTextField(
            value = stringResource(R.string.onb_protocol_imap), onValueChange = {}, label = stringResource(R.string.onb_protocol_label), readOnly = true,
        )
        CroftTextField(
            value = form.host, onValueChange = { onFormChange(form.copy(host = it)) }, label = stringResource(R.string.onb_server_label),
            modifier = Modifier.testTag(OnboardingTags.HOST), enabled = !working, keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next,
            errorText = problemText(FormField.HOST, problems[FormField.HOST]),
        )
        CroftTextField(
            value = form.port, onValueChange = { onFormChange(form.copy(port = it.filter(Char::isDigit))) }, label = stringResource(R.string.onb_port_label),
            modifier = Modifier.testTag(OnboardingTags.PORT), enabled = !working, keyboardType = KeyboardType.Number, imeAction = ImeAction.Next,
            errorText = problemText(FormField.PORT, problems[FormField.PORT]),
        )
        SectionLabel(stringResource(R.string.onb_security_label))
        ChoiceRow(
            options = listOf(SecurityMode.TLS, SecurityMode.START_TLS), selected = form.security,
            label = { securityLabel(it) }, onSelect = { onFormChange(form.withSecurity(it)) },
            modifier = Modifier.testTag(OnboardingTags.SECURITY),
        )
        Text(stringResource(R.string.onb_encrypted_only), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        CroftTextField(
            value = form.username, onValueChange = { onFormChange(form.copy(username = it)) }, label = stringResource(R.string.onb_username_label),
            modifier = Modifier.testTag(OnboardingTags.SETTINGS_USERNAME), enabled = !working, contentType = ContentType.Username,
            keyboardType = KeyboardType.Email, imeAction = ImeAction.Next, errorText = problemText(FormField.USERNAME, problems[FormField.USERNAME]),
        )
        CroftTextField(
            value = password, onValueChange = { password = it }, label = stringResource(R.string.onb_password_label),
            modifier = Modifier.testTag(OnboardingTags.SETTINGS_PASSWORD), enabled = !working, password = true, contentType = ContentType.Password,
            imeAction = ImeAction.Done,
            errorText = if (failure?.isPasswordProblem == true) failureText(failure) else null,
        )
        CroftButton(
            stringResource(if (advanced) R.string.onb_advanced_hide else R.string.onb_advanced), { advanced = !advanced },
            Modifier.testTag(OnboardingTags.ADVANCED), kind = ButtonKind.TEXT,
        )
        if (advanced) {
            SectionLabel(stringResource(R.string.onb_outgoing))
            Text(stringResource(R.string.onb_outgoing_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            CroftTextField(
                value = form.outHost, onValueChange = { onFormChange(form.copy(outHost = it)) }, label = stringResource(R.string.onb_server_label),
                modifier = Modifier.testTag(OnboardingTags.OUT_HOST), enabled = !working, keyboardType = KeyboardType.Uri,
                errorText = problemText(FormField.OUT_HOST, problems[FormField.OUT_HOST]),
            )
            CroftTextField(
                value = form.outPort, onValueChange = { onFormChange(form.copy(outPort = it.filter(Char::isDigit))) }, label = stringResource(R.string.onb_port_label),
                enabled = !working, keyboardType = KeyboardType.Number, errorText = problemText(FormField.OUT_PORT, problems[FormField.OUT_PORT]),
            )
            ChoiceRow(
                options = listOf(SecurityMode.TLS, SecurityMode.START_TLS), selected = form.outSecurity,
                label = { securityLabel(it) }, onSelect = { onFormChange(form.withOutSecurity(it)) },
            )
        }
        state.test?.let { TestSteps(it) }
        if (failure != null && !failure.isPasswordProblem) {
            FailureNotice(failure, serverDetails(state.discovered?.imap), Modifier.testTag(OnboardingTags.SETTINGS_FAILURE))
        }
        CroftButton(
            stringResource(R.string.onb_test_connection), { onTest(password.toByteArray(Charsets.UTF_8)) },
            Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.TEST_BUTTON), kind = ButtonKind.TONAL, enabled = canRun,
            progress = if (working) ButtonProgress.LOADING else ButtonProgress.IDLE,
        )
        CroftButton(
            stringResource(R.string.onb_add_this_account), { onAdd(password.toByteArray(Charsets.UTF_8)) },
            Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.ADD_THIS), enabled = canRun,
        )
    }
    (state.attempt as? Attempt.Trust)?.let { TrustDialog(it, onTrust, onCancelTrust) }
}

@Composable
private fun problemText(field: FormField, problem: FieldProblem?): String? = when {
    problem == null -> null
    problem == FieldProblem.BAD_HOST -> stringResource(R.string.onb_problem_host_bad)
    problem == FieldProblem.BAD_PORT -> stringResource(R.string.onb_problem_port)
    field == FormField.USERNAME -> stringResource(R.string.onb_problem_username)
    field == FormField.PORT || field == FormField.OUT_PORT -> stringResource(R.string.onb_problem_port)
    else -> stringResource(R.string.onb_problem_host_empty)
}

@Composable
private fun Notice(text: String) {
    Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.secondaryContainer) {
        Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
            Icon(Icons.Filled.Info, contentDescription = null, tint = MaterialTheme.colorScheme.onSecondaryContainer, modifier = Modifier.size(20.dp))
            Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
        }
    }
}

/** The steps of Test connection in order, each with an icon, its name and its state in words (a check, a spinner, a warning, an empty ring). */
@Composable
private fun TestSteps(test: ConnectionTest) {
    Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            TestStep.entries.forEach { step ->
                val s = test.steps[step] ?: StepState.PENDING
                val name = stringResource(
                    when (step) {
                        TestStep.REACH_SERVER -> R.string.onb_test_step_reach
                        TestStep.CERTIFICATE -> R.string.onb_test_step_certificate
                        TestStep.SIGN_IN -> R.string.onb_test_step_sign_in
                    },
                )
                val stateText = stringResource(
                    when (s) {
                        StepState.PENDING -> R.string.onb_step_pending
                        StepState.RUNNING -> R.string.onb_step_running
                        StepState.DONE -> R.string.onb_step_done
                        StepState.FAILED -> R.string.onb_step_failed
                    },
                )
                Row(
                    Modifier.fillMaxWidth().semantics(mergeDescendants = true) { stateDescription = stateText }.testTag(OnboardingTags.testStep(step.name)),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    when (s) {
                        StepState.DONE -> Icon(Icons.Filled.Check, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(24.dp))
                        StepState.RUNNING -> CroftSpinner(Modifier.size(24.dp), color = MaterialTheme.colorScheme.primary)
                        StepState.FAILED -> Icon(Icons.Filled.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(24.dp))
                        StepState.PENDING -> Box(Modifier.size(24.dp).border(2.dp, MaterialTheme.colorScheme.outline, CircleShape))
                    }
                    Text(name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                    Text(stateText, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (test.steps.values.all { it == StepState.DONE }) {
                Text(stringResource(R.string.onb_test_passed), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.primary)
            }
        }
    }
}
