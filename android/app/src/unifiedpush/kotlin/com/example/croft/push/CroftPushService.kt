// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * UnifiedPush's events for this app (T34d). Not exported: the connector's own receiver accepts the distributor's broadcasts only with
 * the token of this app's registration and hands them here. A message counts only when it decrypted with this device's Web Push keys
 * (so it came from the Companion it was subscribed at); its content (the Companion's ping, empty) is not used.
 */
class CroftPushService : PushService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        val keys = endpoint.pubKeySet
        val url = endpoint.url
        scope.launch {
            val push = PushRuntime.registrar(applicationContext)
            if (push == null || keys == null) {
                // Not connected (or a distributor without Web Push keys, which the Companion cannot encrypt for): nothing to subscribe.
                PushRuntime.setStatus(if (push == null) PushStatus.NOT_CONNECTED else PushStatus.FAILED)
                return@launch
            }
            PushRuntime.setStatus(if (push.onEndpoint(url, keys.pubKey, keys.auth)) PushStatus.ON else PushStatus.FAILED)
        }
    }

    override fun onMessage(message: PushMessage, instance: String) {
        val authentic = message.decrypted
        scope.launch { PushRuntime.registrar(applicationContext)?.onPing(authentic) }
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        PushRuntime.setStatus(PushStatus.FAILED)
    }

    override fun onUnregistered(instance: String) {
        scope.launch {
            PushRuntime.registrar(applicationContext)?.onEndpointGone()
            PushRuntime.setStatus(PushStatus.FAILED)
        }
    }
}
