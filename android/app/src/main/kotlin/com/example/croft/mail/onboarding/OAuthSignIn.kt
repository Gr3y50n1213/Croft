// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.OAuthFailure

/** The provider of a bring-your-own OAuth sign-in (T21): the core's key, and whether its client type has a secret. */
enum class OAuthProviderChoice(val key: String, val takesSecret: Boolean) {
    /** Gmail: a Google "Desktop app" client (Google gives that type a secret, which belongs to the person's own project). */
    GOOGLE("google", takesSecret = true),

    /** Outlook and Microsoft 365: a "Mobile and desktop applications" client; no secret. */
    MICROSOFT("microsoft", takesSecret = false),
    ;

    companion object {
        /** The OAuth provider for a discovered provider, or null (other providers sign in with a password). */
        fun of(provider: KnownProvider?): OAuthProviderChoice? = when (provider) {
            KnownProvider.GMAIL -> GOOGLE
            KnownProvider.OUTLOOK -> MICROSOFT
            else -> null
        }
    }
}

/** What the OAuth sign-in needs from the core (`oauth_begin`, `oauth_finish`, `oauth_cancel`, `oauth_client_id`). A fake in tests. */
interface OAuthBackend {
    /** Starts a sign-in; returns `(flowId, url)`. The secret is the person's own client secret (Google only), wiped by the caller. */
    suspend fun begin(provider: OAuthProviderChoice, clientId: String, clientSecret: ByteArray?, email: String, donePage: String): Pair<String, String>

    /** Waits for the browser and returns the sign-in secret (the caller wipes it). Throws [OAuthFailure]. */
    suspend fun finish(flowId: String): ByteArray

    fun cancel(flowId: String)

    /** The client id used last for [provider], to fill in the field. */
    suspend fun rememberedClientId(provider: OAuthProviderChoice): String?
}

/** [OAuthBackend] over the real core. */
class CoreOAuthBackend(private val core: suspend () -> Core) : OAuthBackend {
    /** The core a sign-in was started on (cancel needs it without waiting). */
    @Volatile private var started: Core? = null

    override suspend fun begin(provider: OAuthProviderChoice, clientId: String, clientSecret: ByteArray?, email: String, donePage: String): Pair<String, String> {
        val copy = clientSecret?.copyOf()
        try {
            val c = core()
            started = c
            val start = c.oauthBegin(provider.key, clientId, copy, email, donePage)
            return start.flowId to start.url
        } finally {
            copy?.fill(0)
        }
    }

    override suspend fun finish(flowId: String): ByteArray = core().oauthFinish(flowId).secret

    override fun cancel(flowId: String) {
        // The core's call does not block: safe from the main thread.
        try { started?.oauthCancel(flowId) } catch (e: Exception) { /* already over */ }
    }

    override suspend fun rememberedClientId(provider: OAuthProviderChoice): String? = withContext(Dispatchers.IO) {
        try { core().oauthClientId(provider.key) } catch (e: Exception) { null }
    }

    override fun toString(): String = "[redacted]"
}

/** Where the browser part of the sign-in is. */
sealed interface OAuthPhase {
    data object Idle : OAuthPhase

    /** The provider's page is open in a browser tab; Croft waits for it (Cancel ends it). */
    data object Waiting : OAuthPhase

    /** It did not work; [failure] is the plain message to show (Cancel shows none: it is [Idle]). */
    data class Failed(val failure: OAuthFailure) : OAuthPhase
}

/**
 * The browser part of S04's "Use your own client" (T21): asks the core for the provider's URL, has [openTab] show it in a browser tab
 * (Custom Tabs, never a WebView), waits for the core to receive the redirect, and hands the sign-in secret to [onSignedIn] (the flow
 * then checks and adds the account like a password, and wipes the secret). A second start ends the first. Cancel ends the wait
 * quietly. The secret and the client secret are wiped here once handed over.
 */
class OAuthSignIn(
    private val backend: OAuthBackend,
    private val scope: CoroutineScope,
    private val openTab: (String) -> Unit,
    private val onSignedIn: (ByteArray) -> Unit,
    /** After the browser handed the sign-in back: bring Croft to the front (it closes the tab). */
    private val onReturned: () -> Unit = {},
) {
    private val mutable = MutableStateFlow<OAuthPhase>(OAuthPhase.Idle)
    val phase: StateFlow<OAuthPhase> = mutable.asStateFlow()

    private var flowId: String? = null
    private var job: Job? = null

    fun start(provider: OAuthProviderChoice, clientId: String, clientSecret: ByteArray?, email: String, donePage: String) {
        cancel()
        mutable.value = OAuthPhase.Waiting
        job = scope.launch {
            try {
                val (id, url) = try {
                    backend.begin(provider, clientId.trim(), clientSecret, email, donePage)
                } finally {
                    clientSecret?.fill(0)
                }
                flowId = id
                openTab(url)
                val secret = backend.finish(id)
                flowId = null
                mutable.value = OAuthPhase.Idle
                onReturned()
                onSignedIn(secret)
            } catch (e: CancellationException) {
                throw e
            } catch (f: OAuthFailure) {
                flowId = null
                mutable.value = if (f is OAuthFailure.Cancelled) OAuthPhase.Idle else OAuthPhase.Failed(f)
            } catch (e: Exception) {
                flowId = null
                mutable.value = OAuthPhase.Failed(OAuthFailure.Internal())
            }
        }
    }

    /** The person cancelled (or left the screen): no message. */
    fun cancel() {
        flowId?.let { backend.cancel(it) }
        flowId = null
        job?.cancel()
        job = null
        mutable.value = OAuthPhase.Idle
    }

    /** The failure was read (the person changed a field). */
    fun clearFailure() {
        if (mutable.value is OAuthPhase.Failed) mutable.value = OAuthPhase.Idle
    }
}
