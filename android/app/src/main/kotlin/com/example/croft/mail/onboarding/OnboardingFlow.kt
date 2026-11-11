// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import java.util.concurrent.CancellationException
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DiscoverException
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.KnownProvider
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings
import uniffi.croft_ffi.VerifyFailure

/** The screens of the sign-in flow, in the IDs of design/screens.csv. */
enum class Step { WELCOME, ADD_ACCOUNT, FINDING, SIGN_IN, SERVER_SETTINGS, FIRST_SYNC }

/** The lookup of S03. */
sealed interface Lookup {
    data object Idle : Lookup
    data object Running : Lookup
    data class Failed(val failure: OnboardingFailure) : Lookup
}

/** A sign-in or a connection test (S04, S05). [Trust] is the trust-on-first-use question: the password is kept only until it is answered. */
sealed interface Attempt {
    data object Idle : Attempt
    data object Working : Attempt
    data class Failed(val failure: OnboardingFailure) : Attempt

    /** The server's certificate is not one Croft knows. [changed] is true when a certificate the person trusted before is not the one shown now. */
    data class Trust(val host: String, val fingerprint: String, val changed: Boolean) : Attempt
}

/**
 * Everything the screens draw. **It holds no password**: that is in [OnboardingFlow] as a byte array until the attempt ends.
 * Everything here is safe to save across a rotation ([OnboardingFlow.snapshot]).
 */
data class OnboardingState(
    /** The path the person took, first screen first. Back pops it. S03 is replaced by what it found, so Back from S04 lands on S02. */
    val stack: List<Step> = listOf(Step.WELCOME),
    val email: String = "",
    /** The core said the address is not an address (S02 shows it). */
    val emailRejected: Boolean = false,
    val probes: ProbeRows = waitingRows(),
    val lookup: Lookup = Lookup.Idle,
    /** What the lookup found (or what S05 built), for S04. */
    val discovered: FfiDiscovered? = null,
    val form: ServerForm = ServerForm(),
    /** S05 was reached because nothing was found: its values are a guess and the screen says so. */
    val formGuessed: Boolean = false,
    /** S05 shows the problems of its fields (after the person pressed a button with a bad form). */
    val showFormProblems: Boolean = false,
    val attempt: Attempt = Attempt.Idle,
    val test: ConnectionTest? = null,
    /** The certificate the person trusted, as `pinned_fingerprint` for the core. Public data. */
    val pinnedFingerprint: String? = null,
    /** The new account, once it was added (S09). */
    val addedAccount: Long? = null,
    /**
     * The account is being added right now (the core call and the storing of the password). That step cannot be cancelled and Back
     * waits for it (see [OnboardingFlow.back]).
     */
    val adding: Boolean = false,
) {
    val step: Step get() = stack.last()
}

/**
 * The sign-in flow S01 to S05 and S09 as one state machine over an [OnboardingBackend]. Plain Kotlin and coroutines (no Android),
 * so it is tested over the real core on the JVM and over a scripted backend in the Compose tests.
 *
 * **The password.** A screen hands it over as a `ByteArray` in [signIn], [testConnection] or [addFromSettings]; the flow owns that
 * array from then on and wipes it (`fill(0)`) as soon as the attempt ends: on success, on any failure, on Back, and on [close].
 * The one case where it is kept longer is the trust question (an untrusted certificate): the same bytes are used again after
 * "Trust this server", because the person should not have to type the password twice, and wiped on Cancel. Each attempt owns
 * its own array, so a cancelled attempt cannot wipe the next one's. It is never put in
 * [state], [snapshot], a log line or an exception.
 */
class OnboardingFlow(
    private val backend: OnboardingBackend,
    private val scope: CoroutineScope,
    saved: List<String>? = null,
) {
    private val mutable = MutableStateFlow(OnboardingState())

    /** What to draw. */
    val state: StateFlow<OnboardingState> = mutable.asStateFlow()

    private var job: Job? = null

    /**
     * One sign-in attempt and the password it owns. Only the attempt's own code, its superseding by the next one, [back], [cancelTrust]
     * and [close] wipe [password]; a cancelled attempt that is still unwinding can never wipe the next attempt's array, because each
     * attempt has its own and only touches the shared fields while it is still [run].
     */
    private class Run(val discovered: FfiDiscovered, val add: Boolean, val password: ByteArray) {
        fun wipe() = password.fill(0)
    }

    /** The attempt that is current (running, or waiting on the trust question). */
    @Volatile private var run: Run? = null

    /** The number of the lookup that is current; a probe callback or an answer of an older lookup is ignored. */
    private val lookups = AtomicLong()

    init {
        if (saved != null) restore(saved)
    }

    // ---- S01, S02 ----

    /** S01: "Add an account". */
    fun openAddAccount() {
        if (mutable.value.step == Step.WELCOME) push(Step.ADD_ACCOUNT)
    }

    /** S02: the text of the address field. */
    fun setEmail(text: String) = mutable.update { it.copy(email = text, emailRejected = false) }

    /** S02: a shortcut chip. */
    fun applyShortcut(shortcut: ProviderShortcut) = setEmail(EmailInput.applyShortcut(mutable.value.email, shortcut))

    /** S02: Continue. Starts the lookup (S03). Does nothing while the address is not valid. */
    fun continueWithEmail() {
        val s = mutable.value
        if (s.step != Step.ADD_ACCOUNT || !EmailInput.isValid(s.email)) return
        mutable.update { it.copy(discovered = null, attempt = Attempt.Idle, test = null, pinnedFingerprint = null) }
        push(Step.FINDING)
        startLookup()
    }

    // ---- S03 ----

    /** S03: "Try again" after a failure. */
    fun retryLookup() {
        if (mutable.value.step == Step.FINDING) startLookup()
    }

    private fun startLookup() {
        job?.cancel()
        val me = lookups.incrementAndGet()
        val email = mutable.value.email.trim()
        mutable.update { it.copy(lookup = Lookup.Running, probes = waitingRows(), emailRejected = false) }
        // The board belongs to this lookup: a late callback of an older one (the core delivers from a thread of its own) is dropped.
        val board = ProbeBoard { rows -> mutable.update { if (lookups.get() == me) it.copy(probes = rows) else it } }
        job = scope.launch {
            try {
                val found = backend.discover(email, board)
                if (lookups.get() == me) onDiscovered(found, email)
            } catch (e: CancellationException) {
                throw e
            } catch (e: DiscoverException) {
                if (lookups.get() == me) onDiscoverFailed(e, email)
            } catch (e: Exception) {
                if (lookups.get() == me) mutable.update { it.copy(lookup = Lookup.Failed(OnboardingFailure.Unexpected())) }
            }
        }
    }

    private fun onDiscovered(found: FfiDiscovered, email: String) {
        mutable.update { s ->
            if (s.step != Step.FINDING) return@update s
            if (found.imap == null) {
                // JMAP only: Croft cannot sign in over JMAP yet. S03 says so and offers the manual route.
                s.copy(lookup = Lookup.Failed(OnboardingFailure.Verify(VerifyFailure.JmapOnly(), EmailInput.domainOf(email))))
            } else {
                s.copy(
                    discovered = found, form = ServerForm.from(found), formGuessed = false, lookup = Lookup.Idle,
                    attempt = Attempt.Idle, test = null, pinnedFingerprint = null,
                    stack = s.stack.dropLast(1) + Step.SIGN_IN,
                )
            }
        }
    }

    private fun onDiscoverFailed(e: DiscoverException, email: String) {
        mutable.update { s ->
            if (s.step != Step.FINDING) return@update s
            when (e) {
                // Nothing published: S05, filled in with what is known (the address and a guess at the server).
                is DiscoverException.NothingFound -> s.copy(
                    form = ServerForm.guessFor(email), formGuessed = true, lookup = Lookup.Idle, discovered = null,
                    attempt = Attempt.Idle, test = null, pinnedFingerprint = null, showFormProblems = false,
                    stack = s.stack.dropLast(1) + Step.SERVER_SETTINGS,
                )
                is DiscoverException.InvalidAddress -> s.copy(emailRejected = true, lookup = Lookup.Idle, stack = s.stack.dropLast(1))
                else -> s.copy(lookup = Lookup.Failed(OnboardingFailure.Discover(e, EmailInput.domainOf(email))))
            }
        }
    }

    /** S03 failure or S04: "Enter settings manually" / "Server settings". Opens S05 with what is known. */
    fun openServerSettings() {
        val s = mutable.value
        if (s.step != Step.FINDING && s.step != Step.SIGN_IN) return
        job?.cancel(); lookups.incrementAndGet(); endRun()
        val form = s.discovered?.let { ServerForm.from(it) } ?: ServerForm.guessFor(s.email)
        mutable.update {
            val next = if (s.step == Step.FINDING) it.stack.dropLast(1) + Step.SERVER_SETTINGS else it.stack + Step.SERVER_SETTINGS
            it.copy(
                form = form, formGuessed = s.discovered == null, showFormProblems = false, lookup = Lookup.Idle, attempt = Attempt.Idle,
                test = null, stack = next,
            )
        }
    }

    // ---- S04 ----

    /**
     * S04: Sign in. [username] may have been edited; [password] is wiped by the flow when the attempt ends (see the class text).
     * Verifies the settings with the credential, then adds the account; an untrusted certificate asks first ([Attempt.Trust]).
     */
    fun signIn(username: String, password: ByteArray) {
        val s = mutable.value
        val found = s.discovered
        if (s.step != Step.SIGN_IN || found == null || s.attempt is Attempt.Working || username.isBlank() || password.isEmpty()) {
            password.fill(0)
            return
        }
        val named = found.withUsername(username.trim())
        mutable.update { it.copy(discovered = named, form = it.form.copy(username = username.trim())) }
        begin(named, add = true, password)
    }

    // ---- S05 ----

    /** S05: any field changed. A new host or port drops a trusted certificate (it was trusted for the other server). */
    fun updateForm(next: ServerForm) {
        mutable.update { s ->
            val moved = s.form.host.trim() != next.host.trim() || s.form.port.trim() != next.port.trim()
            s.copy(
                form = next, pinnedFingerprint = if (moved) null else s.pinnedFingerprint,
                test = null, attempt = if (s.attempt is Attempt.Failed) Attempt.Idle else s.attempt,
            )
        }
    }

    /** S05: Test connection. Checks reaching the server, its certificate and the sign-in, and adds nothing. */
    fun testConnection(password: ByteArray) = fromSettings(password, add = false)

    /** S05: Add account, after the form is filled in. Same checks, then adds the account. */
    fun addFromSettings(password: ByteArray) = fromSettings(password, add = true)

    private fun fromSettings(password: ByteArray, add: Boolean) {
        val s = mutable.value
        val built = s.form.toDiscovered(s.email, s.discovered)
        if (s.step != Step.SERVER_SETTINGS || s.attempt is Attempt.Working || password.isEmpty()) { password.fill(0); return }
        if (built == null) {
            password.fill(0)
            mutable.update { it.copy(showFormProblems = true) }
            return
        }
        mutable.update { it.copy(discovered = built, showFormProblems = false) }
        begin(built, add, password)
    }

    // ---- the attempt (S04 and S05) ----

    private fun begin(d: FfiDiscovered, add: Boolean, password: ByteArray) {
        // The attempt before this one (waiting on a trust question, say) is over: its password is wiped.
        run?.wipe()
        val r = Run(d, add, password)
        run = r
        job?.cancel()
        job = scope.launch { execute(r) }
    }

    private suspend fun execute(r: Run) {
        val d = r.discovered
        val host = d.imap?.host
        val pin = mutable.value.pinnedFingerprint
        mutable.update { if (run === r) it.copy(attempt = Attempt.Working, test = if (it.step == Step.SERVER_SETTINGS) ConnectionTest.started() else null) else it }
        try {
            backend.verify(d, r.password, pin)
            if (run !== r) { r.wipe(); return }
            if (r.add) {
                mutable.update { it.copy(adding = true) }
                val id = addAccountToCompletion(r, pin)
                endRun(r)
                mutable.update { it.copy(attempt = Attempt.Idle, test = null, addedAccount = id, stack = listOf(Step.FIRST_SYNC), adding = false) }
            } else {
                endRun(r)
                mutable.update { it.copy(attempt = Attempt.Idle, test = ConnectionTest.passed()) }
            }
        } catch (e: CancellationException) {
            r.wipe()
            // Cancelled while adding: the add still ran to its end (see addAccountToCompletion), so nothing is half done.
            mutable.update { if (it.adding && run === r) it.copy(adding = false) else it }
            throw e
        } catch (f: VerifyFailure) {
            onVerifyFailed(r, f, host)
        } catch (e: CoreException) {
            fail(r, OnboardingFailure.Add(e, host))
        } catch (e: StorageUnavailableException) {
            fail(r, OnboardingFailure.Storage())
        } catch (e: Exception) {
            fail(r, OnboardingFailure.Unexpected())
        }
    }

    /**
     * Adds the account **and keeps the password** as one step that nothing can cancel: the core inserts the account even if the caller
     * is cancelled (its call does not stop on cancel), so a cancel between that and the storing of the password would leave an
     * account with no secret, and a retry would then add a second one. The step works on its own copy of the password, so that
     * wiping the attempt's array meanwhile (Back, rotation) cannot store zeros. The result is the account's id; if nobody is
     * listening any more, the account is there with its secret and the next start shows it (the app starts on the Inbox once an
     * account exists), and a second sign-in for the same address finds it ([CoreOnboardingBackend.addAccount]).
     */
    private suspend fun addAccountToCompletion(r: Run, pin: String?): Long = withContext(NonCancellable) {
        val copy = r.password.copyOf()
        try {
            backend.addAccount(r.discovered, EmailInput.localPartOf(r.discovered.email), pin, copy)
        } finally {
            copy.fill(0)
        }
    }

    private fun onVerifyFailed(r: Run, f: VerifyFailure, host: String?) {
        if (run !== r) { r.wipe(); return }
        val trust = when (f) {
            is VerifyFailure.UntrustedCertificate -> Attempt.Trust(host.orEmpty(), f.fingerprint, changed = false)
            is VerifyFailure.CertificateChanged -> Attempt.Trust(host.orEmpty(), f.fingerprint, changed = true)
            else -> null
        }
        mutable.update { it.copy(test = if (it.step == Step.SERVER_SETTINGS) ConnectionTest.stoppedAt(ConnectionTest.stepOf(f)) else null) }
        if (trust != null) {
            // The password stays in the run until the question is answered.
            mutable.update { it.copy(attempt = trust) }
        } else {
            fail(r, OnboardingFailure.Verify(f, host))
        }
    }

    private fun fail(r: Run, failure: OnboardingFailure) {
        if (run !== r) { r.wipe(); return }
        endRun(r)
        mutable.update { it.copy(attempt = Attempt.Failed(failure), adding = false) }
    }

    /** Wipes the run's password and, if it is still the current one, drops it. */
    private fun endRun(r: Run? = run) {
        r?.wipe()
        if (r != null && run === r) run = null
    }

    /** "Trust this server": the certificate shown is pinned for this server, and the attempt runs again with it. */
    fun trustServer() {
        val trust = mutable.value.attempt as? Attempt.Trust ?: return
        val r = run ?: return
        mutable.update { it.copy(pinnedFingerprint = trust.fingerprint) }
        job?.cancel()
        job = scope.launch { execute(r) }
    }

    /** Cancel on the trust question: nothing is trusted, the password is wiped. */
    fun cancelTrust() {
        job?.cancel(); endRun()
        mutable.update { it.copy(attempt = Attempt.Idle, test = null) }
    }

    /** The inline failure was read (the person edits the password again). */
    fun dismissFailure() = mutable.update { if (it.attempt is Attempt.Failed) it.copy(attempt = Attempt.Idle) else it }

    // ---- Back ----

    /**
     * Back goes one step back. Returns false when the flow cannot go back (S01: the system leaves the app; S09: the screen
     * continues in the background instead). Going back cancels what is running and wipes the password, except while the account is
     * being added: that step finishes first, and Back does nothing meanwhile.
     */
    fun back(): Boolean {
        val s = mutable.value
        if (s.stack.size <= 1 || s.step == Step.FIRST_SYNC) return false
        // Adding the account cannot be stopped half way, so Back waits for it (the press is taken and does nothing). Checking the
        // password can be cancelled, and is.
        if (s.adding) return true
        job?.cancel(); job = null; lookups.incrementAndGet(); endRun()
        mutable.update { it.copy(stack = it.stack.dropLast(1), attempt = Attempt.Idle, lookup = Lookup.Idle, test = null) }
        return true
    }

    /** The screen is going away: stops what runs and wipes the password. */
    fun close() {
        job?.cancel(); job = null; lookups.incrementAndGet(); endRun()
    }

    private fun push(step: Step) = mutable.update { it.copy(stack = it.stack + step) }

    // ---- saving across a rotation (never the password) ----

    /** What survives the activity being recreated: the path, the address, what was found, the form, a trusted certificate. No secret. */
    fun snapshot(): ArrayList<String> {
        val s = mutable.value
        val out = arrayListOf("v1", s.stack.joinToString(",") { it.name }, s.email, s.pinnedFingerprint.orEmpty(), s.addedAccount?.toString().orEmpty())
        out += encode(s.discovered)
        out += listOf(s.form.host, s.form.port, s.form.security.name, s.form.username, s.form.outHost, s.form.outPort, s.form.outSecurity.name, s.formGuessed.toString())
        return out
    }

    private fun restore(saved: List<String>) {
        try {
            if (saved.size < 5 || saved[0] != "v1") return
            val stack = saved[1].split(",").map { Step.valueOf(it) }.ifEmpty { listOf(Step.WELCOME) }
            val reader = saved.drop(5)
            val (found, rest) = decode(reader)
            val f = rest
            val form = ServerForm(
                host = f[0], port = f[1], security = SecurityMode.valueOf(f[2]), username = f[3],
                outHost = f[4], outPort = f[5], outSecurity = SecurityMode.valueOf(f[6]),
            )
            val added = saved[4].toLongOrNull()
            // A step whose data did not survive goes back to the step before it.
            val fixed = stack.toMutableList()
            while (fixed.size > 1 && (
                (fixed.last() == Step.SIGN_IN && found == null) || (fixed.last() == Step.FIRST_SYNC && added == null)
                )) fixed.removeAt(fixed.size - 1)
            mutable.value = OnboardingState(
                stack = fixed, email = saved[2], discovered = found, form = form, formGuessed = f[7].toBoolean(),
                pinnedFingerprint = saved[3].ifEmpty { null }, addedAccount = added,
            )
            // The lookup is not saved: it starts again.
            if (fixed.last() == Step.FINDING) startLookup()
        } catch (e: Exception) {
            mutable.value = OnboardingState()
        }
    }

    private fun encode(d: FfiDiscovered?): List<String> {
        if (d == null) return listOf("0")
        fun server(s: ServerSettings?) = if (s == null) listOf("0") else listOf("1", s.host, s.port.toString(), s.security.name, s.username)
        return listOf("1", d.email) + server(d.imap) + server(d.smtp) +
            listOf(d.jmap.orEmpty(), d.source.name, d.provider?.name.orEmpty(), d.authHint.name)
    }

    /** Reads what [encode] wrote; returns the discovered settings and the items after them. */
    private fun decode(items: List<String>): Pair<FfiDiscovered?, List<String>> {
        if (items[0] == "0") return null to items.drop(1)
        var i = 1
        val email = items[i++]
        fun server(): ServerSettings? {
            if (items[i++] == "0") return null
            val s = ServerSettings(items[i], items[i + 1].toUShort(), SecurityMode.valueOf(items[i + 2]), items[i + 3])
            i += 4
            return s
        }
        val imap = server()
        val smtp = server()
        val jmap = items[i++].ifEmpty { null }
        val source = DiscoverySource.valueOf(items[i++])
        val provider = items[i++].ifEmpty { null }?.let { KnownProvider.valueOf(it) }
        val auth = AuthKind.valueOf(items[i++])
        return FfiDiscovered(email, imap, smtp, jmap, source, provider, auth) to items.drop(i)
    }
}

/** The settings with another login name (the person edited it on S04). */
fun FfiDiscovered.withUsername(username: String): FfiDiscovered =
    copy(imap = imap?.copy(username = username), smtp = smtp?.copy(username = username))
