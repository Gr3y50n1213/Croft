// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.AnchorStore
import com.example.croft.mail.FolderMapping

/**
 * What the app does at every start, once the core is open, to finish a removal that a crash cut short. An account's removal is
 * several steps (the core's rows, its saved secret, the app's own entries for it); a crash between them leaves something behind for an
 * id the core no longer lists. An id is not given to another account while the database exists (the core keeps a high-water mark), so a
 * leftover could never reach another account; but it must not stay: this deletes it.
 *
 * Only runs against a list the core really gave: if reading the accounts failed, nothing is swept (an unknown is not "none").
 *
 * **The quarantine is not swept** (T11): its secret files belong to an earlier database, whose account numbers may repeat the current
 * ones, and deleting them at the first start after a key loss would make a later restore bring back accounts without passwords. It
 * expires by itself and "Erase all Croft data" deletes it.
 */
class AccountSweep(
    /** The ids the core lists now. */
    private val known: Set<AccountId>,
    /** What the sweep needs from the secret store. */
    private val secretStore: Secrets,
) {
    /** The parts of the secret store the sweep uses (`SecretStore` in the app, a fake in a test). */
    interface Secrets {
        /** The ids that have a stored secret on this device. */
        fun storedIds(): Set<AccountId>

        /** Deletes the live secrets of [id]. */
        fun deleteSecrets(id: AccountId)
    }

    /** Deletes the saved secrets of ids the core does not list. */
    fun secrets() {
        for (id in secretStore.storedIds()) if (id !in known) secretStore.deleteSecrets(id)
    }

    /** The folder mapping without the entries of accounts the core does not list. */
    fun folderMapping(mapping: FolderMapping) {
        for (id in mapping.accounts()) if (id !in known) mapping.forgetAccount(id)
    }

    /** The list positions without those of folders of accounts the core does not list. */
    fun anchors(store: AnchorStore) {
        for (id in store.accounts()) if (id !in known) store.forgetAccount(id)
    }

    /** The prefs without the entries of accounts the core does not list. */
    fun prefs(prefs: AccountPrefs): AccountPrefs = prefs.keepOnly(known)
}
