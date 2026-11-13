// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

/**
 * How a store build of Croft is woken when mail comes (T34c; registering comes with T34d). Each flavour's `StorePush` names its
 * kind: `play` uses FCM through the push gateway, `fdroid` and `github` use UnifiedPush (no Google services in those builds).
 */
enum class PushKind {
    /** Firebase Cloud Messaging, through Croft's push gateway (`gateway/`). Play only. */
    FCM,

    /** A UnifiedPush distributor the person chose (ntfy, for example), straight from their Companion. */
    UNIFIED_PUSH,
}
