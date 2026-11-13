// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import com.example.croft.BuildConfig
import com.google.android.gms.tasks.Task
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * The Play build's push (T34d): Firebase Cloud Messaging through Croft's push gateway (`gateway/`). The app's FCM token goes to the
 * gateway, bound to the person's Companion's VAPID key; the gateway answers with an endpoint, which is subscribed at the Companion.
 *
 * Only when the build was made with a Firebase project and a gateway (`BuildConfig.FCM_CONFIGURED`, android/README.md): otherwise push
 * is "not in this build" and the periodic check is all there is.
 */
object StorePush : StorePushProvider {
    override val kind: PushKind = PushKind.FCM

    private fun ready(context: Context) = BuildConfig.FCM_CONFIGURED && FirebaseApp.getApps(context).isNotEmpty()

    override suspend fun start(context: Context, vapid: String): StoreStart {
        if (!ready(context)) return StoreStart.Status(PushStatus.NOT_IN_THIS_BUILD)
        val token = FirebaseMessaging.getInstance().token.await() ?: return StoreStart.Status(PushStatus.FAILED)
        return FcmRegistration.endpoint(context, token, vapid) ?: StoreStart.Status(PushStatus.FAILED)
    }

    override fun stop(context: Context) {
        FcmRegistration.forget(context)
        if (!ready(context)) return
        FirebaseMessaging.getInstance().deleteToken()
    }
}

/** The result of a Play services task, or null when it failed or was cancelled. */
internal suspend fun <T> Task<T>.await(): T? = suspendCancellableCoroutine { cont ->
    addOnCompleteListener { t -> if (cont.isActive) cont.resume(if (t.isSuccessful) t.result else null) }
}
