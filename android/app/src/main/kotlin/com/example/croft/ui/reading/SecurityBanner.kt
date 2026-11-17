// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MessageId
import com.example.croft.mail.reading.BodySecurity
import com.example.croft.theme.statusColors
import com.example.croft.ui.components.CroftIcons

/** Test tags of S56. */
object SecurityTags {
    fun banner(id: MessageId) = "s56-$id"
    fun signature(id: MessageId) = "s56-signature-$id"
    fun failure(id: MessageId) = "s56-failure-$id"
}

/** The key as S54 shows it: its last eight characters, in two groups. */
private fun shortKey(fingerprint: String?): String = fingerprint.orEmpty().takeLast(8).chunked(4).joinToString(" ")

@Composable
private fun Banner(icon: ImageVector, container: Color, content: Color, title: String, text: String?, modifier: Modifier) {
    // One element for screen readers: the title, then the text.
    Surface(color = container, contentColor = content, shape = MaterialTheme.shapes.medium, modifier = modifier.fillMaxWidth().semantics(mergeDescendants = true) {}) {
        Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(24.dp))
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                if (text != null) Text(text, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

/**
 * S56 Encrypted message (T42b), above the body: a shield for an encrypted message (with the signer when the signature is valid), the
 * signature's state, or why it could not be decrypted and what fixes it. Amber for a signature that is not confirmed or not verified,
 * wax red for a failure; the words always say it, never colour alone. Only the body is ever called signed.
 */
@Composable
fun SecurityBanner(id: MessageId, security: BodySecurity, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val status = statusColors()
    Column(modifier.fillMaxWidth().testTag(SecurityTags.banner(id)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        when (security.encryption) {
            BodySecurity.Encryption.FAILED -> Banner(
                Icons.Filled.Lock, scheme.errorContainer, scheme.onErrorContainer,
                stringResource(R.string.s56_failed_title),
                stringResource(
                    when (security.problem) {
                        BodySecurity.DecryptIssue.NO_OWN_KEY -> R.string.s56_failed_no_key
                        BodySecurity.DecryptIssue.NOT_FOR_THIS_KEY -> R.string.s56_failed_not_for_key
                        BodySecurity.DecryptIssue.TOO_LARGE -> R.string.s56_failed_too_large
                        BodySecurity.DecryptIssue.DAMAGED, null -> R.string.s56_failed_damaged
                    },
                ),
                Modifier.testTag(SecurityTags.failure(id)),
            )
            BodySecurity.Encryption.DECRYPTED -> {
                val valid = security.signature == BodySecurity.Signature.VALID
                Banner(
                    CroftIcons.Shield, scheme.primaryContainer, scheme.onPrimaryContainer,
                    stringResource(if (valid) R.string.s56_encrypted_signed else R.string.s56_encrypted),
                    if (valid) stringResource(R.string.s56_signed_by, security.signer.orEmpty(), shortKey(security.signerKey))
                    else stringResource(R.string.s56_encrypted_hint),
                    Modifier,
                )
            }
            BodySecurity.Encryption.NONE -> if (security.signature == BodySecurity.Signature.VALID) {
                Banner(
                    CroftIcons.Shield, scheme.secondaryContainer, scheme.onSecondaryContainer,
                    stringResource(R.string.s56_signed),
                    stringResource(R.string.s56_signed_by, security.signer.orEmpty(), shortKey(security.signerKey)),
                    Modifier.testTag(SecurityTags.signature(id)),
                )
            }
        }
        when (security.signature) {
            BodySecurity.Signature.UNCONFIRMED -> Banner(
                Icons.Filled.Warning, status.amberContainer, status.onAmberContainer,
                stringResource(R.string.s56_unconfirmed_title),
                stringResource(R.string.s56_unconfirmed, security.signer.orEmpty()),
                Modifier.testTag(SecurityTags.signature(id)),
            )
            BodySecurity.Signature.NOT_VERIFIED -> Banner(
                Icons.Filled.Warning, status.amberContainer, status.onAmberContainer,
                stringResource(R.string.s56_not_verified_title),
                stringResource(R.string.s56_not_verified),
                Modifier.testTag(SecurityTags.signature(id)),
            )
            else -> Unit
        }
    }
}
