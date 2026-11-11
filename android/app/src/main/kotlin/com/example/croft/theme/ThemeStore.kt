// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

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

/**
 * The stored theme file as DataStore sees it: JSON text. Anything unreadable (corrupt, empty, newer than this build,
 * an unknown option) is read as Moss, Standard, System, never as an error, so the app always starts.
 */
object ThemeSettingsSerializer : Serializer<ThemeSettings> {
    override val defaultValue: ThemeSettings = ThemeSettings.Default

    override suspend fun readFrom(input: InputStream): ThemeSettings =
        ThemeJson.decodeOrDefault(input.readBytes().toString(Charsets.UTF_8))

    override suspend fun writeTo(t: ThemeSettings, output: OutputStream) {
        output.write(ThemeJson.encode(t).toByteArray(Charsets.UTF_8))
    }
}

/** Persists [ThemeSettings] as JSON in the app's private files (DataStore). The theme holds no secrets. */
class ThemeStore(private val dataStore: DataStore<ThemeSettings>) {
    /** The current settings; emits again after every change. */
    val settings: Flow<ThemeSettings> get() = dataStore.data

    /** Apply [change] to the stored settings. */
    suspend fun update(change: (ThemeSettings) -> ThemeSettings) {
        dataStore.updateData(change)
    }

    /** Replace everything. Used by Undo, Import and Reset. */
    suspend fun set(value: ThemeSettings) {
        dataStore.updateData { value }
    }

    companion object {
        /** The store in this app's private `files/datastore/croft-theme.json`. */
        fun create(context: Context, scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)): ThemeStore =
            ThemeStore(
                DataStoreFactory.create(
                    serializer = ThemeSettingsSerializer,
                    scope = scope,
                    produceFile = { File(context.applicationContext.filesDir, "datastore/croft-theme.json") },
                ),
            )

        @Volatile private var shared: ThemeStore? = null

        /**
         * The one store of this process. DataStore refuses two instances on one file, and an activity can be created many
         * times, so the activity asks for this one.
         */
        fun shared(context: Context): ThemeStore =
            shared ?: synchronized(this) { shared ?: create(context).also { shared = it } }

        /** A store on any file, for tests. */
        fun onFile(file: File, scope: CoroutineScope): ThemeStore =
            ThemeStore(DataStoreFactory.create(serializer = ThemeSettingsSerializer, scope = scope, produceFile = { file }))
    }
}
