// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import org.unifiedpush.android.connector.UnifiedPush

/**
 * The F-Droid and GitHub builds' push (T34d): UnifiedPush, no Google services. The person's distributor app (ntfy, for example) gives
 * an endpoint; [CroftPushService] hands it to [PushRuntime], which subscribes it at their Companion. The Companion pings it straight
 * (Web Push, encrypted for this device).
 */
object StorePush : StorePushProvider {
    override val kind: PushKind = PushKind.UNIFIED_PUSH

    /** How long S58 waits for the person to choose a distributor before giving up (the chooser may never answer). */
    private const val CHOOSE_MS = 120_000L

    override suspend fun start(context: Context, vapid: String): StoreStart {
        if (UnifiedPush.getDistributors(context).isEmpty()) return StoreStart.Status(PushStatus.NO_DISTRIBUTOR)
        // The saved distributor, else the person's default; choosing one needs an activity (S58), so a start from the background
        // with none chosen yet ends here.
        val chosen = withTimeoutOrNull(CHOOSE_MS) {
            suspendCancellableCoroutine { cont ->
                UnifiedPush.tryUseCurrentOrDefaultDistributor(context) { ok -> if (cont.isActive) cont.resume(ok) }
            }
        } ?: false
        if (!chosen) return StoreStart.Status(PushStatus.NO_DISTRIBUTOR)
        // The Companion's VAPID key (checked by PushRuntime): distributors that need one accept only its signed messages. The
        // distributor answers with the endpoint, again for an existing registration, so the subscription is renewed at each start.
        UnifiedPush.register(context, vapid = vapid)
        return StoreStart.Status(PushStatus.WAITING)
    }

    override fun stop(context: Context) {
        UnifiedPush.unregister(context)
    }
}
