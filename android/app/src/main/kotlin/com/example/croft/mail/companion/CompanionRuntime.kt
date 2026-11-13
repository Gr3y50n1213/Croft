// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.companion

import android.content.Context
import android.os.Build
import com.example.croft.mail.MailRuntime
import com.example.croft.secrets.AppSecretKind
import com.example.croft.secrets.SecretStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** The connection's secret in the keystore-backed store (the app's own entry, apart from accounts). */
class KeystoreCompanionSecrets(private val store: SecretStore) : CompanionSecrets {
    override fun get(): ByteArray? = store.getApp(AppSecretKind.COMPANION)
    override fun put(secret: ByteArray) = store.putApp(AppSecretKind.COMPANION, secret)
    override fun remove() = store.removeApp(AppSecretKind.COMPANION)
}

/** The app's one Companion connection, made once the core is open (T34b). */
object CompanionRuntime {
    @Volatile private var connection: CompanionConnection? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** The app's redirect: `<applicationId>:/companion`. */
    fun redirect(context: Context): String = "${context.packageName}:/companion"

    /** How this phone is named in the person's list of devices. */
    fun deviceName(): String = "Croft on ${Build.MANUFACTURER} ${Build.MODEL}".filter { !it.isISOControl() }.take(60)

    /** The connection, or null while the core is not open. */
    fun connection(context: Context): CompanionConnection? {
        connection?.let { return it }
        val open = MailRuntime.core ?: return null
        return synchronized(this) {
            connection ?: CompanionConnection(
                backend = CoreCompanionBackend(open.core),
                secrets = KeystoreCompanionSecrets(open.secrets.secrets),
                redirect = redirect(context),
                deviceName = deviceName(),
            ).also { connection = it }
        }
    }

    /**
     * The connection, opening the core first when the process is cold (a push or a check woke it). Null when there is no database
     * (nothing was ever set up, or everything was erased) or the core cannot be opened.
     */
    suspend fun openedConnection(context: Context): CompanionConnection? {
        connection?.let { return it }
        if (!MailRuntime.databaseFile(context).exists()) return null
        try {
            MailRuntime.openedCore(context)
        } catch (e: Exception) {
            return null
        } catch (e: UnsatisfiedLinkError) {
            return null
        }
        return connection(context)
    }

    /** The browser came back to the app's redirect (from [com.example.croft.CompanionRedirectActivity]). */
    fun redirected(context: Context, uri: String) {
        val c = connection(context) ?: return
        val app = context.applicationContext
        scope.launch {
            c.onRedirect(uri)
            // T34d: a new connection asks for push at once (UnifiedPush may still need the person to choose a distributor in S58).
            if (c.state.value is CompanionState.Connected) com.example.croft.push.PushRuntime.start(app)
        }
    }

    /** Forgets the connection object (the core was closed: an erase, or a test). */
    fun reset() {
        synchronized(this) { connection = null }
    }
}
