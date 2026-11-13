// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.croft_ffi.AddressSuggestion
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.NoHandle

/** T38b: compose's suggestions put the account's contacts first, then the addresses mail was sent to, each address once. */
class ContactSuggestionsTest {
    /** The two core calls, scripted (no Rust behind it). Made-up addresses. */
    private class Scripted(private val contactsFail: Boolean = false) : Core(NoHandle) {
        val asked = ArrayList<Pair<Long?, String>>()

        override fun autocomplete(account: Long?, prefix: String, limit: UInt): List<AddressSuggestion> {
            asked += account to prefix
            if (contactsFail) throw CoreException.NoSuchAccount()
            return listOf(
                AddressSuggestion("mira@example.org", "Mira Okafor", true),
                // A recent recipient as autocomplete gives it: left to the history call, which knows the Sent folder too.
                AddressSuggestion("old@example.org", null, false),
            ).take(limit.toInt())
        }

        override fun suggestRecipients(accountId: Long, prefix: String, limit: UInt): List<FfiAddress> =
            listOf(FfiAddress(null, "MIRA@example.org"), FfiAddress("Sam Rivera", "sam@example.org"), FfiAddress(null, "old@example.org")).take(limit.toInt())
    }

    @Test
    fun contactsComeFirst_thenRecentRecipients_eachAddressOnce() = runBlocking {
        val core = Scripted()
        val got = CoreComposeBackend(core, Dispatchers.Unconfined).suggestRecipients(1, "m", 5)
        assertEquals(
            listOf(FfiAddress("Mira Okafor", "mira@example.org"), FfiAddress("Sam Rivera", "sam@example.org"), FfiAddress(null, "old@example.org")),
            got,
        )
        // Only the sending account's contacts.
        assertEquals(listOf<Pair<Long?, String>>(1L to "m"), core.asked)
    }

    @Test
    fun theLimitHolds() = runBlocking {
        assertEquals(2, CoreComposeBackend(Scripted(), Dispatchers.Unconfined).suggestRecipients(1, "m", 2).size)
    }

    @Test
    fun contactsThatCannotBeReadLeaveTheHistory() = runBlocking {
        val got = CoreComposeBackend(Scripted(contactsFail = true), Dispatchers.Unconfined).suggestRecipients(1, "m", 5)
        assertEquals(listOf("MIRA@example.org", "sam@example.org", "old@example.org"), got.map { it.address })
    }
}
