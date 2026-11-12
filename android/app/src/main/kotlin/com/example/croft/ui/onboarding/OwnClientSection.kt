// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.example.croft.MainActivity
import com.example.croft.R
import com.example.croft.mail.onboarding.OAuthPhase
import com.example.croft.mail.onboarding.OAuthProviderChoice
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftTextField
import uniffi.croft_ffi.OAuthFailure

/** Test tags of S04's own-client section. */
object OwnClientTags {
    const val SECTION = "S04-own-client"
    const val CLIENT_ID = "S04-own-client-id"
    const val CLIENT_SECRET = "S04-own-client-secret"
    const val SIGN_IN = "S04-own-client-sign-in"
    const val CANCEL = "S04-own-client-cancel"
    const val WAITING = "S04-own-client-waiting"
    const val HELP = "S04-own-client-help"
    const val HELP_TEXT = "S04-own-client-help-text"
    const val COPY = "S04-own-client-copy"
    const val FAILURE = "S04-own-client-failure"
}

/**
 * S04 for Gmail and Outlook (T21): sign in through the provider's own page with **the person's own** OAuth client (Croft ships none).
 * The client ID (and, for Google's "Desktop app" type, its secret) is typed here; the help says where to register, which client type,
 * the redirect address to copy, and that a Google project left in Testing gives sign-ins that stop after 7 days. The page opens in a
 * browser tab ([openInBrowserTab]), never in a WebView. While it is open the section says Croft is waiting, with Cancel.
 */
@Composable
fun OwnClientSection(
    provider: OAuthProviderChoice,
    phase: OAuthPhase,
    rememberedClientId: String?,
    onStart: (clientId: String, clientSecret: ByteArray?) -> Unit,
    onCancel: () -> Unit,
    onEdited: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    /** The help starts open (screenshots). */
    helpOpen: Boolean = false,
) {
    var clientId by rememberSaveable(provider) { mutableStateOf("") }
    LaunchedEffect(rememberedClientId) { if (clientId.isEmpty() && rememberedClientId != null) clientId = rememberedClientId }
    var clientSecret by remember { mutableStateOf("") }
    var showHelp by rememberSaveable { mutableStateOf(helpOpen) }
    val waiting = phase is OAuthPhase.Waiting
    val context = LocalContext.current
    Column(modifier.fillMaxWidth().testTag(OwnClientTags.SECTION), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            stringResource(if (provider == OAuthProviderChoice.GOOGLE) R.string.onb_own_client_title_google else R.string.onb_own_client_title_microsoft),
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.semantics { heading() },
        )
        Text(stringResource(R.string.onb_own_client_intro), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        CroftButton(
            stringResource(if (showHelp) R.string.onb_own_client_help_hide else R.string.onb_own_client_help_show),
            { showHelp = !showHelp },
            Modifier.heightIn(min = 48.dp).testTag(OwnClientTags.HELP),
            kind = ButtonKind.TEXT,
        )
        if (showHelp) {
            Text(
                stringResource(if (provider == OAuthProviderChoice.GOOGLE) R.string.onb_own_client_help_google else R.string.onb_own_client_help_microsoft),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.testTag(OwnClientTags.HELP_TEXT),
            )
            if (provider == OAuthProviderChoice.MICROSOFT) {
                val redirect = stringResource(R.string.onb_own_client_redirect)
                Text(redirect, style = MaterialTheme.typography.bodyLarge)
                var copied by remember { mutableStateOf(false) }
                CroftButton(
                    stringResource(if (copied) R.string.onb_own_client_copied else R.string.onb_own_client_copy),
                    { copyText(context, redirect); copied = true },
                    Modifier.heightIn(min = 48.dp).testTag(OwnClientTags.COPY),
                    kind = ButtonKind.TONAL,
                )
            }
        }
        CroftTextField(
            value = clientId,
            onValueChange = { clientId = it; onEdited() },
            label = stringResource(R.string.onb_own_client_id),
            modifier = Modifier.testTag(OwnClientTags.CLIENT_ID),
            enabled = enabled && !waiting,
            imeAction = if (provider.takesSecret) ImeAction.Next else ImeAction.Done,
        )
        if (provider.takesSecret) {
            CroftTextField(
                value = clientSecret,
                onValueChange = { clientSecret = it; onEdited() },
                label = stringResource(R.string.onb_own_client_secret),
                modifier = Modifier.testTag(OwnClientTags.CLIENT_SECRET),
                enabled = enabled && !waiting,
                password = true,
                imeAction = ImeAction.Done,
            )
        }
        if (waiting) {
            Text(
                stringResource(R.string.onb_own_client_waiting),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.testTag(OwnClientTags.WAITING).semantics { liveRegion = LiveRegionMode.Polite },
            )
            CroftButton(stringResource(R.string.onb_own_client_cancel), onCancel, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(OwnClientTags.CANCEL), kind = ButtonKind.OUTLINED)
        } else {
            CroftButton(
                stringResource(if (provider == OAuthProviderChoice.GOOGLE) R.string.onb_own_client_sign_in_google else R.string.onb_own_client_sign_in_microsoft),
                {
                    val secret = clientSecret.takeIf { provider.takesSecret && it.isNotBlank() }?.toByteArray(Charsets.UTF_8)
                    onStart(clientId, secret)
                },
                Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(OwnClientTags.SIGN_IN),
                kind = ButtonKind.TONAL,
                enabled = enabled && clientId.isNotBlank(),
                progress = ButtonProgress.IDLE,
            )
        }
        (phase as? OAuthPhase.Failed)?.let { failed ->
            Text(
                oauthFailureText(failed.failure),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.testTag(OwnClientTags.FAILURE).semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
    }
}

/** The plain message for a failed sign-in. Cancel has none (the section just goes back to its start). */
@Composable
fun oauthFailureText(f: OAuthFailure): String = when (f) {
    is OAuthFailure.AccessDenied -> stringResource(R.string.onb_oauth_denied)
    is OAuthFailure.StateMismatch -> stringResource(R.string.onb_oauth_stale)
    is OAuthFailure.Expired -> stringResource(R.string.onb_oauth_expired)
    is OAuthFailure.Refused -> stringResource(R.string.onb_oauth_refused, f.code)
    is OAuthFailure.Network -> stringResource(R.string.onb_oauth_network)
    is OAuthFailure.BadClient -> stringResource(R.string.onb_oauth_bad_client)
    is OAuthFailure.Cancelled, is OAuthFailure.Internal -> stringResource(R.string.onb_oauth_failed)
}

private fun copyText(context: Context, text: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText(text, text))
}

/**
 * Opens [url] in a browser tab: Custom Tabs when a browser offers them, the default browser otherwise. **Never a WebView** (store rule
 * and docs/ux/screens.md S04): the person types their Google or Microsoft password into the browser, not into Croft. Throws
 * [ActivityNotFoundException] when there is no browser.
 */
fun openInBrowserTab(context: Context, url: String) {
    val uri = Uri.parse(url)
    require(uri.scheme == "https") { "only https pages open" }
    CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, uri)
}

/** Brings Croft back to the front once the browser handed the sign-in back; it closes the tab above it in Croft's task. */
fun returnToCroft(context: Context) {
    val intent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    if (context !is android.app.Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try { context.startActivity(intent) } catch (e: Exception) { /* the person switches back themselves */ }
}
