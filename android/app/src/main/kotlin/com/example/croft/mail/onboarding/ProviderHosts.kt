// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import uniffi.croft_ffi.KnownProvider

/**
 * Which large provider owns a server host. A copy of the core's `provider_for_host` (`core/protocols/src/discovery/provider.rs`), which
 * is the real guard: the core refuses a pin for such a host in `verify` and `add_account` whatever the settings say. The app keeps the
 * provider on the settings anyway, so that S04 and S05 behave the same way (no "trust this server" offer for imap.gmail.com) and the
 * settings the core receives say what they are.
 */
object ProviderHosts {
    private val domains = listOf(
        "gmail.com" to KnownProvider.GMAIL, "googlemail.com" to KnownProvider.GMAIL,
        "outlook.com" to KnownProvider.OUTLOOK, "hotmail.com" to KnownProvider.OUTLOOK, "live.com" to KnownProvider.OUTLOOK,
        "office365.com" to KnownProvider.OUTLOOK,
        "icloud.com" to KnownProvider.I_CLOUD, "me.com" to KnownProvider.I_CLOUD, "mac.com" to KnownProvider.I_CLOUD,
        "fastmail.com" to KnownProvider.FASTMAIL, "fastmail.fm" to KnownProvider.FASTMAIL,
    )

    /** The provider whose own server [host] is (the provider's domain or a name under it), or null. Case and a trailing dot do not matter. */
    fun providerFor(host: String): KnownProvider? {
        val h = host.trim().trimEnd('.').lowercase()
        return domains.firstOrNull { (d, _) -> h == d || h.endsWith(".$d") }?.second
    }
}
