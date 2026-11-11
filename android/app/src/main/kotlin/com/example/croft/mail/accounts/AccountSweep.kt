// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.FolderMapping

/**
 * What the app does at every start, once the core is open, to finish a removal that a crash cut short. An account's removal is
 * several steps (the core's rows, its saved secret and quarantined copies, the app's own entries for it); a crash between them leaves
 * something behind for an id the core no longer lists. Ids are never reused, so a leftover could never reach another account, but it
 * must not stay: this deletes it.
 *
 * Only runs against a list the core really gave: if reading the accounts failed, nothing is swept (an unknown is not "none").
 */
class AccountSweep(
    /** The ids the core lists now. */
    private val known: Set<AccountId>,
    /** What the sweep needs from the secret store. */
    private val secretStore: Secrets,
) {
    /** The parts of the secret store the sweep uses (`SecretStore` and `AccountSecrets` in the app, a fake in a test). */
    interface Secrets {
        /** The ids that have a stored secret on this device. */
        fun storedIds(): Set<AccountId>

        /** The ids whose removal was begun and not finished (the removal marks). */
        fun unfinishedRemovals(): Set<AccountId>

        /** Deletes only the live secrets of [id]. */
        fun deleteSecrets(id: AccountId)

        /** Deletes the live secrets, the quarantined copies and the removal mark of [id]. */
        fun finishRemoval(id: AccountId)

        /** Deletes only the removal mark of [id]. */
        fun clearMark(id: AccountId)
    }

    /**
     * Finishes every removal that was begun and is not in [known] (its saved secrets **and quarantined copies** go; an id that is still
     * listed only loses its mark, the removal never reached the core), and deletes saved secrets of ids the core does not list. The
     * quarantined copies of an id are left alone unless its removal was begun: they belong to an earlier database, whose accounts may
     * use the same numbers (T11's restore).
     */
    fun secrets() {
        for (id in secretStore.unfinishedRemovals()) {
            if (id in known) secretStore.clearMark(id) else secretStore.finishRemoval(id)
        }
        for (id in secretStore.storedIds()) if (id !in known) secretStore.deleteSecrets(id)
    }

    /** The folder mapping without the entries of accounts the core does not list. */
    fun folderMapping(mapping: FolderMapping) {
        for (id in mapping.accounts()) if (id !in known) mapping.forgetAccount(id)
    }

    /** The prefs without the entries of accounts the core does not list. */
    fun prefs(prefs: AccountPrefs): AccountPrefs = prefs.keepOnly(known)
}
