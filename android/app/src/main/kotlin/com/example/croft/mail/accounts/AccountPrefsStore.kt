// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow

/** DataStore's view of the file. */
object AccountPrefsSerializer : Serializer<AccountPrefs> {
    override val defaultValue: AccountPrefs = AccountPrefs()

    override suspend fun readFrom(input: InputStream): AccountPrefs = AccountPrefsJson.decodeOrDefault(input.readBytes().toString(Charsets.UTF_8))

    override suspend fun writeTo(t: AccountPrefs, output: OutputStream) { output.write(AccountPrefsJson.encode(t).toByteArray(Charsets.UTF_8)) }
}

/**
 * Persists [AccountPrefs] (the account order, the names and colours the person chose) in the app's private files, as DataStore does
 * for the theme, the swipe settings and the mail settings. Display-only, no address and no secret. The file is excluded from backups
 * like the other settings (see `res/xml/backup_rules.xml`).
 */
class AccountPrefsStore(private val dataStore: DataStore<AccountPrefs>) {
    /** The current prefs; emits again after every change. */
    val prefs: Flow<AccountPrefs> get() = dataStore.data

    /** Replace everything. */
    suspend fun set(value: AccountPrefs) { dataStore.updateData { value } }

    /** Change what is stored, from what is stored (waits until it is written). */
    suspend fun update(change: (AccountPrefs) -> AccountPrefs) { dataStore.updateData { change(it) } }

    companion object {
        @Volatile private var shared: AccountPrefsStore? = null

        /** The one store of this process. */
        fun shared(context: Context): AccountPrefsStore = shared ?: synchronized(this) {
            shared ?: onFile(File(context.applicationContext.filesDir, "datastore/croft-accounts.json"), CoroutineScope(SupervisorJob() + Dispatchers.IO)).also { shared = it }
        }

        /** A store on any file, for tests. */
        fun onFile(file: File, scope: CoroutineScope): AccountPrefsStore =
            AccountPrefsStore(DataStoreFactory.create(serializer = AccountPrefsSerializer, scope = scope, produceFile = { file }))
    }
}
