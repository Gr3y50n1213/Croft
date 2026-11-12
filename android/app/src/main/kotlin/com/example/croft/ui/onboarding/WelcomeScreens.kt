// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Email
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.onboarding.EmailInput
import com.example.croft.mail.onboarding.ProviderShortcut
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftSuggestionChip
import com.example.croft.ui.components.CroftTextField

/**
 * What decides whether the "I run a Companion" button exists. **There is no store-build flag yet** (the app has a single
 * build type and no flavours), so the button is hidden in every build for now: a missing button, never a disabled one (Apple 2.1
 * rejects placeholder content, and `docs/tasks/T17-onboarding-accounts.md` says so). When phase 2 adds S06 and a store flag,
 * this is the one line that changes.
 */
object OnboardingFlags {
    const val COMPANION_ENTRY: Boolean = false
}

/**
 * S01 Welcome. One primary action (Add an account), and "Try with sample mail" as the quiet one. The Companion entry shows only
 * when [showCompanion] is true (see [OnboardingFlags]). Back on this screen leaves the app (the host has no handler here).
 */
@Composable
fun WelcomeScreen(
    onAddAccount: () -> Unit,
    onTrySampleMail: () -> Unit,
    modifier: Modifier = Modifier,
    showCompanion: Boolean = OnboardingFlags.COMPANION_ENTRY,
    onCompanion: () -> Unit = {},
    /** The sample mailbox could not be opened (the core or its secure storage is not available): a plain line says so. */
    sampleProblem: Boolean = false,
    /** Opens the privacy policy (a view intent); the link is not drawn when null. */
    onPrivacyPolicy: (() -> Unit)? = null,
) {
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(OnboardingTags.WELCOME), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 560.dp).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Box(Modifier.size(96.dp).background(MaterialTheme.colorScheme.primaryContainer, CircleShape), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.Email, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer, modifier = Modifier.size(48.dp))
            }
            Text(stringResource(R.string.onb_welcome_title), style = MaterialTheme.typography.displaySmall, textAlign = TextAlign.Center)
            Text(
                stringResource(R.string.onb_welcome_tagline), style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center,
            )
            CroftButton(
                stringResource(R.string.onb_add_account), onAddAccount,
                Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.ADD_BUTTON),
            )
            if (showCompanion) {
                CroftButton(
                    stringResource(R.string.onb_companion), onCompanion,
                    Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.COMPANION_BUTTON), kind = ButtonKind.OUTLINED,
                )
            }
            CroftButton(
                stringResource(R.string.onb_try_sample), onTrySampleMail,
                Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(OnboardingTags.SAMPLE_BUTTON), kind = ButtonKind.TEXT,
            )
            if (sampleProblem) {
                Text(
                    stringResource(R.string.onb_sample_problem), Modifier.testTag(OnboardingTags.SAMPLE_PROBLEM), style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center,
                )
            }
            Text(
                stringResource(R.string.onb_no_account_needed), style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center,
            )
            if (onPrivacyPolicy != null) {
                CroftButton(
                    stringResource(R.string.about_privacy_policy), onPrivacyPolicy,
                    Modifier.heightIn(min = 48.dp).testTag(OnboardingTags.PRIVACY_LINK), kind = ButtonKind.TEXT,
                )
            }
        }
    }
}

/**
 * S02 Add account. The address field has autofill hints (email address). Continue stays disabled until the address is valid
 * ([EmailInput.isValid]). The error is checked when the field loses focus and then updates live as the text changes
 * (docs/ux/controls.md); [rejected] is the core saying it is not an address. The chips fill in the provider's domain.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AddAccountScreen(
    email: String,
    rejected: Boolean,
    onEmailChange: (String) -> Unit,
    onShortcut: (ProviderShortcut) -> Unit,
    onContinue: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    autofocus: Boolean = true,
) {
    val valid = EmailInput.isValid(email)
    var hadFocus by remember { mutableStateOf(false) }
    var checked by remember { mutableStateOf(false) }
    val focus = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    if (autofocus) LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    // After the first error the field is checked live: the error clears as soon as the address is fixed.
    val showError = rejected || (checked && !valid && email.isNotEmpty())
    OnboardingFrame(stringResource(R.string.onb_add_account_title), onBack, modifier.testTag(OnboardingTags.ADD_ACCOUNT)) {
        ScreenHeading(stringResource(R.string.onb_email_heading))
        CroftTextField(
            value = email,
            onValueChange = onEmailChange,
            label = stringResource(R.string.onb_email_label),
            modifier = Modifier.focusRequester(focus).testTag(OnboardingTags.EMAIL),
            errorText = if (showError) stringResource(R.string.onb_email_error) else null,
            supportingText = stringResource(R.string.onb_email_hint),
            contentType = ContentType.EmailAddress,
            keyboardType = KeyboardType.Email,
            imeAction = ImeAction.Done,
            onImeAction = {
                // Done leaves the field, which is what checks it; a valid address goes straight on.
                focusManager.clearFocus()
                if (EmailInput.isValid(email)) onContinue() else if (email.isNotEmpty()) checked = true
            },
            onFocusChange = { focused ->
                if (focused) hadFocus = true
                else if (hadFocus) checked = !valid && email.isNotEmpty()
            },
        )
        Text(
            stringResource(R.string.onb_quick_start).uppercase(), style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
        )
        FlowRow(
            Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            ProviderShortcut.entries.forEach { shortcut ->
                val name = when (shortcut) {
                    ProviderShortcut.GMAIL -> stringResource(R.string.onb_shortcut_gmail)
                    ProviderShortcut.OUTLOOK -> stringResource(R.string.onb_shortcut_outlook)
                    ProviderShortcut.ICLOUD -> stringResource(R.string.onb_shortcut_icloud)
                    ProviderShortcut.OTHER -> stringResource(R.string.onb_shortcut_other)
                }
                CroftSuggestionChip(
                    label = name,
                    onClick = {
                        onShortcut(shortcut)
                        if (shortcut == ProviderShortcut.OTHER) runCatching { focus.requestFocus() }
                    },
                    modifier = Modifier.testTag(OnboardingTags.shortcut(shortcut.name.lowercase())),
                )
            }
        }
        CroftButton(
            stringResource(R.string.onb_continue), onContinue,
            Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OnboardingTags.CONTINUE), enabled = valid,
        )
    }
}
