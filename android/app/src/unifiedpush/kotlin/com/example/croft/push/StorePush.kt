// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import org.unifiedpush.android.connector.UnifiedPush

/**
 * The F-Droid and GitHub builds' push (T34d): UnifiedPush, no Google services. The person's distributor app (ntfy, for example) gives
 * an endpoint; [CroftPushService] subscribes it at their Companion, which pings it straight (Web Push, encrypted for this device).
 */
object StorePush : StorePushProvider {
    override val kind: PushKind = PushKind.UNIFIED_PUSH

    override suspend fun start(context: Context, vapid: String): PushStatus {
        if (UnifiedPush.getDistributors(context).isEmpty()) return PushStatus.NO_DISTRIBUTOR
        // The saved distributor, else the person's default; choosing one needs an activity (S58), so a start from the background
        // with none chosen yet ends here.
        val chosen = suspendCancellableCoroutine { cont ->
            UnifiedPush.tryUseCurrentOrDefaultDistributor(context) { ok -> if (cont.isActive) cont.resume(ok) }
        }
        if (!chosen) return PushStatus.NO_DISTRIBUTOR
        // The Companion's VAPID key: distributors that need one (and Web Push servers) accept only its signed messages.
        UnifiedPush.register(context, vapid = vapid)
        return PushStatus.WAITING
    }

    override fun stop(context: Context) {
        UnifiedPush.unregister(context)
    }
}
