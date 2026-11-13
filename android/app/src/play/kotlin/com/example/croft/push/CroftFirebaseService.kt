// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * FCM's events for the Play build (T34d). Not exported (Play services delivers through its own exported receiver, guarded by
 * `com.google.android.c2dm.permission.SEND`, which only Google Play services holds). A message is a ping the gateway forwarded; its
 * content is not read.
 */
class CroftFirebaseService : FirebaseMessagingService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onNewToken(token: String) {
        scope.launch {
            // The token is asked of Firebase again rather than taken from the call, and registered only for a connected Companion.
            val vapid = FcmRegistration.savedVapid(applicationContext) ?: return@launch
            val current = FirebaseMessaging.getInstance().token.await() ?: return@launch
            val endpoint = FcmRegistration.endpoint(applicationContext, current, vapid)
            if (endpoint == null) {
                PushRuntime.setStatus(PushStatus.FAILED)
                return@launch
            }
            PushRuntime.onEndpoint(applicationContext, endpoint.url, endpoint.p256dh, endpoint.auth)
        }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        if (message.data.containsKey("wp")) PushRuntime.onPing(applicationContext, authentic = true)
    }
}
