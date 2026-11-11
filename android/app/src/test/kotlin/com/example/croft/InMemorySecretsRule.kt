// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import com.example.croft.mail.MailRuntime
import com.example.croft.secrets.AesGcmSecretBox
import com.example.croft.secrets.AppSecrets
import com.example.croft.secrets.MemoryKeySource
import com.example.croft.secrets.secretsDirectory
import org.junit.rules.ExternalResource

/**
 * For a test that starts the real [MainActivity]: the app's secrets use a wrapping key in memory instead of the AndroidKeyStore (a
 * simulated device has none that works, see `KeystoreWiringTest`), so the real Rust core opens in the activity and "Try with sample
 * mail" can add the core's own sample account. [MailRuntime] is cleared before and after, so each test is a fresh process-like state.
 * Use it with a lower `order` than the activity rule (a rule with order 0, the activity rule with order 1): the activity starts
 * the core as soon as it is created.
 */
class InMemorySecretsRule : ExternalResource() {
    override fun before() {
        MailRuntime.resetForTests()
        MailRuntime.secretsFactory = { context, db ->
            val keys = MemoryKeySource()
            AppSecrets(secretsDirectory(context), AesGcmSecretBox(keys), keys, db)
        }
    }

    override fun after() = MailRuntime.resetForTests()
}
