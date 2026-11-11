// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.onboarding.AccountAdder
import com.example.croft.mail.onboarding.Attempt
import com.example.croft.mail.onboarding.ConnectionTest
import com.example.croft.mail.onboarding.CoreOnboardingBackend
import com.example.croft.mail.onboarding.OnboardingBackend
import com.example.croft.mail.onboarding.OnboardingFailure
import com.example.croft.mail.onboarding.OnboardingFlow
import com.example.croft.mail.onboarding.OnboardingState
import com.example.croft.mail.onboarding.ProbeRow
import com.example.croft.mail.onboarding.RowState
import com.example.croft.mail.onboarding.ServerForm
import com.example.croft.mail.onboarding.Step
import com.example.croft.mail.onboarding.StepState
import com.example.croft.mail.onboarding.TestStep
import java.io.File
import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.DiscoveryObserver
import uniffi.croft_ffi.DiscoveryProbe
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.ProbeProgress
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings
import uniffi.croft_ffi.TestImapServer
import uniffi.croft_ffi.VerifyFailure

/**
 * The sign-in flow (`OnboardingFlow`, S01 to S05 and S09) over the **real Rust core** and a real in-process IMAP server with a
 * self-signed certificate: the trust prompt, the pinned fingerprint, a wrong password, a changed certificate, Test connection,
 * and that the password never reaches the saved state and is wiped. Only discovery is scripted (it would need DNS and
 * published settings): the backend answers `discover` with the in-process server's settings and reports the probes, and
 * hands everything else to the core. Made-up names, `example.org` addresses and an obviously fake password.
 */
class OnboardingFlowTest {
    private val dir: File = Files.createTempDirectory("croft-onboarding-test").toFile()
    private val provider = MemoryCredentials()
    private val server: TestImapServer = TestImapServer.start(2u, TEST_PASSWORD)
    private val core: Core = Core.open(File(dir, "croft.db").path, null, provider)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** Every password array the flow was handed, to check they were wiped. */
    private val handedOver = mutableListOf<ByteArray>()

    /** Held by a test to stop the add between "the flow asked" and "the core inserted the account"; [addStarted] says it got there. */
    @Volatile private var addGate: CompletableDeferred<Unit>? = null
    private val addStarted = AtomicBoolean(false)

    /** What the adder stored under each new account's id, as text: a wiped array would show as zeros here. */
    private val storedPasswords = ConcurrentHashMap<Long, String>()

    private val adder: AccountAdder = { d, name, pin, pw ->
        addStarted.set(true)
        addGate?.await()
        val credential = FfiCredential.password(pw.copyOf())
        try {
            core.addAccount(d, name, pin, credential).also { provider.store(it, String(pw)); storedPasswords[it] = String(pw) }
        } finally {
            credential.close()
        }
    }
    private val real = CoreOnboardingBackend(core, adder)

    @After
    fun tearDown() {
        verifyHook = null; discoverHook = null; addGate?.complete(Unit)
        scope.cancel()
        core.close()
        server.stop()
        dir.deleteRecursively()
    }

    private fun found(port: UShort = server.port(), host: String = server.host()) = FfiDiscovered(
        email = "alice@example.org",
        imap = ServerSettings(host, port, SecurityMode.TLS, server.username()),
        smtp = null, jmap = null, source = DiscoverySource.AUTOCONFIG, provider = null, authHint = AuthKind.PASSWORD,
    )

    /** Set by a test to hold or watch a call: before the real `verify`, before the real add (the adder), and in `discover`. */
    @Volatile private var verifyHook: (suspend (ByteArray) -> Unit)? = null
    @Volatile private var discoverHook: (suspend (DiscoveryObserver) -> Unit)? = null

    /** Discovery is scripted; sign-in checks and adding are the real core. */
    private val backend = object : OnboardingBackend {
        override suspend fun discover(email: String, observer: DiscoveryObserver): FfiDiscovered {
            discoverHook?.invoke(observer)
            observer.onProbe(DiscoveryProbe.PROVIDER_TABLE, ProbeProgress.Skipped)
            observer.onProbe(DiscoveryProbe.AUTOCONFIG_SUBDOMAIN, ProbeProgress.Found)
            return found()
        }
        override suspend fun verify(discovered: FfiDiscovered, password: ByteArray, pinnedFingerprint: String?) {
            verifyHook?.invoke(password)
            real.verify(discovered, password, pinnedFingerprint)
        }
        override suspend fun addAccount(discovered: FfiDiscovered, displayName: String, pinnedFingerprint: String?, password: ByteArray) =
            real.addAccount(discovered, displayName, pinnedFingerprint, password)
    }

    private fun flow(saved: List<String>? = null) = OnboardingFlow(backend, scope, saved)

    private fun pw(text: String = TEST_PASSWORD): ByteArray = text.toByteArray().also { handedOver += it }

    private fun OnboardingFlow.awaitState(what: String, check: (OnboardingState) -> Boolean): OnboardingState =
        await(what) { state.value.takeIf(check) }

    /** S01 to S04 as a person goes: Add an account, the address, Continue; the lookup ends at S04. */
    private fun OnboardingFlow.toSignIn(): OnboardingFlow {
        openAddAccount()
        setEmail("alice@example.org")
        continueWithEmail()
        awaitState("S04") { it.step == Step.SIGN_IN }
        return this
    }

    private fun assertWiped() = handedOver.forEach { assertTrue("a password array was not wiped", it.all { b -> b == 0.toByte() }) }

    @Test(timeout = 60_000)
    fun theLookupEndsAtSignInWithTheFoundSettings_andBackFromThereIsTheAddressScreen() {
        val f = flow().toSignIn()
        val s = f.state.value
        assertEquals(listOf(Step.WELCOME, Step.ADD_ACCOUNT, Step.SIGN_IN), s.stack)
        assertEquals(server.port(), s.discovered!!.imap!!.port)
        assertTrue(f.back())
        assertEquals(Step.ADD_ACCOUNT, f.state.value.step)
        assertTrue(f.back())
        assertEquals(Step.WELCOME, f.state.value.step)
        assertFalse("Back on the first screen is left to the system", f.back())
    }

    @Test(timeout = 60_000)
    fun aSelfSignedCertificateAsksFirst_andTrustingItPinsThatCertificateAndAddsTheAccount() {
        val f = flow().toSignIn()
        f.signIn("alice", pw())

        // The core refuses the unknown certificate before any password is sent, and says which one it saw.
        val asked = f.awaitState("the trust question") { it.attempt is Attempt.Trust }.attempt as Attempt.Trust
        assertEquals(server.certificateFingerprint(), asked.fingerprint)
        assertFalse(asked.changed)
        assertEquals(server.host(), asked.host)
        assertNull("nothing is trusted yet", f.state.value.pinnedFingerprint)
        assertTrue("no account yet", core.listAccounts().isEmpty())

        f.trustServer()
        val done = f.awaitState("the account was added") { it.addedAccount != null }
        assertEquals(listOf(Step.FIRST_SYNC), done.stack)
        assertEquals(server.certificateFingerprint(), done.pinnedFingerprint)
        val accounts = core.listAccounts()
        assertEquals(1, accounts.size)
        assertEquals(done.addedAccount, accounts.single().id)
        // The pin is stored and used: the account syncs over the same self-signed certificate.
        await("the first sync") { core.listAccounts().single().lastSyncedMs }
        assertEquals(2, await("the Inbox messages") { core.listFolders(accounts.single().id).firstOrNull { it.specialUse == "inbox" }?.totalCount?.takeIf { it == 2u } }.toInt())
        assertWiped()
    }

    @Test(timeout = 60_000)
    fun aWrongPasswordIsAnInlineFailureAndAddsNothing() {
        val f = flow().toSignIn()
        f.signIn("alice", pw("not-the-password-0000"))
        f.awaitState("the trust question") { it.attempt is Attempt.Trust }
        f.trustServer()
        val failed = f.awaitState("the failure") { it.attempt is Attempt.Failed }.attempt as Attempt.Failed
        assertTrue(failed.failure is OnboardingFailure.Verify)
        assertTrue((failed.failure as OnboardingFailure.Verify).failure is VerifyFailure.WrongPassword)
        assertTrue("the screen shows it under the password field", failed.failure.isPasswordProblem)
        assertEquals("still on S04", Step.SIGN_IN, f.state.value.step)
        assertTrue(core.listAccounts().isEmpty())
        assertWiped()

        // Typing again clears the message, and the right password then works (the trusted certificate is kept).
        f.dismissFailure()
        assertEquals(Attempt.Idle, f.state.value.attempt)
        f.signIn("alice", pw())
        f.awaitState("the account was added") { it.addedAccount != null }
        assertWiped()
    }

    @Test(timeout = 60_000)
    fun cancellingTheTrustQuestionTrustsNothingAndWipesThePassword() {
        val f = flow().toSignIn()
        val bytes = pw()
        f.signIn("alice", bytes)
        f.awaitState("the trust question") { it.attempt is Attempt.Trust }
        f.cancelTrust()
        assertEquals(Attempt.Idle, f.state.value.attempt)
        assertNull(f.state.value.pinnedFingerprint)
        assertTrue(bytes.all { it == 0.toByte() })
        assertTrue(core.listAccounts().isEmpty())
    }

    @Test(timeout = 60_000)
    fun aCertificateThatChangedAfterItWasTrustedAsksAgainWithTheStrongerWarning() {
        // The person trusted a certificate earlier (the flow was saved with that pin); the server now shows another one.
        val first = flow().toSignIn()
        val saved = ArrayList(first.snapshot())
        saved[3] = "AB".repeat(32)
        val f = flow(saved)
        assertEquals("AB".repeat(32), f.state.value.pinnedFingerprint)
        f.signIn("alice", pw())
        val asked = f.awaitState("the second question") { it.attempt is Attempt.Trust }.attempt as Attempt.Trust
        assertTrue("a changed certificate is marked as changed", asked.changed)
        assertEquals(server.certificateFingerprint(), asked.fingerprint)
        // Trusting the new one works and replaces the pin.
        f.trustServer()
        val done = f.awaitState("the account was added") { it.addedAccount != null }
        assertEquals(server.certificateFingerprint(), done.pinnedFingerprint)
    }

    @Test(timeout = 60_000)
    fun noServerThereIsACannotConnectFailureNotAHang() {
        val f = flow().toSignIn()
        // A port nobody listens on, through the manual route.
        f.openServerSettings()
        val s = f.state.value
        f.updateForm(s.form.copy(port = "1"))
        f.testConnection(pw())
        val failed = f.awaitState("the failure") { it.attempt is Attempt.Failed }
        val v = (failed.attempt as Attempt.Failed).failure as OnboardingFailure.Verify
        assertTrue(v.failure is VerifyFailure.CannotConnect)
        assertEquals(ConnectionTest.stoppedAt(TestStep.REACH_SERVER), failed.test)
        assertWiped()
    }

    @Test(timeout = 60_000)
    fun testConnectionListsItsStepsInOrder_andAddsNothing() {
        val f = flow().toSignIn()
        f.openServerSettings()
        assertEquals(listOf(Step.WELCOME, Step.ADD_ACCOUNT, Step.SIGN_IN, Step.SERVER_SETTINGS), f.state.value.stack)
        f.testConnection(pw())
        // The certificate is the second step: the first one (reaching the server) worked.
        val asked = f.awaitState("the trust question") { it.attempt is Attempt.Trust }
        assertEquals(ConnectionTest.stoppedAt(TestStep.CERTIFICATE), asked.test)
        assertEquals(StepState.DONE, asked.test!!.steps[TestStep.REACH_SERVER])
        assertEquals(StepState.FAILED, asked.test!!.steps[TestStep.CERTIFICATE])
        assertEquals(StepState.PENDING, asked.test!!.steps[TestStep.SIGN_IN])
        f.trustServer()
        val passed = f.awaitState("the test passed") { it.test == ConnectionTest.passed() }
        assertEquals(Attempt.Idle, passed.attempt)
        assertEquals(Step.SERVER_SETTINGS, passed.step)
        assertTrue("Test connection adds nothing", core.listAccounts().isEmpty())
        // Then the same screen adds the account.
        f.addFromSettings(pw())
        f.awaitState("the account was added") { it.addedAccount != null }
        assertEquals(1, core.listAccounts().size)
        assertWiped()
    }

    @Test(timeout = 60_000)
    fun theSavedStateNeverHoldsThePassword_andMovingToAnotherServerForgetsTheTrust() {
        val f = flow().toSignIn()
        f.signIn("alice", pw())
        f.awaitState("the trust question") { it.attempt is Attempt.Trust }
        // While the question waits the password is held; it is in neither the state nor the saved snapshot.
        assertFalse(f.snapshot().joinToString("|").contains(TEST_PASSWORD))
        assertFalse(f.state.value.toString().contains(TEST_PASSWORD))
        f.trustServer()
        f.awaitState("the account was added") { it.addedAccount != null }

        // Another flow: trust, then change the host on S05: the pin belonged to the other server.
        val g = flow().toSignIn()
        g.signIn("alice", pw())
        g.awaitState("the trust question") { it.attempt is Attempt.Trust }
        g.trustServer()
        g.awaitState("pinned") { it.pinnedFingerprint != null }
        g.cancelTrust()
        g.openServerSettings()
        g.updateForm(g.state.value.form.copy(host = "other.example.org"))
        assertNull(g.state.value.pinnedFingerprint)
    }

    @Test(timeout = 60_000)
    fun aSnapshotRestoresThePathAndWhatWasFound_butStartsAnInterruptedLookupAgain() {
        val f = flow().toSignIn()
        val again = flow(f.snapshot())
        assertEquals(f.state.value.stack, again.state.value.stack)
        assertEquals(f.state.value.discovered, again.state.value.discovered)
        assertEquals(f.state.value.form, again.state.value.form)
        // A saved path that was in the middle of the lookup starts it again and ends at S04.
        val mid = ArrayList(f.snapshot()).also { it[1] = "WELCOME,ADD_ACCOUNT,FINDING" }
        val restored = flow(mid)
        restored.awaitState("S04") { it.step == Step.SIGN_IN }
        // A broken snapshot gives a fresh flow, never a crash.
        assertEquals(OnboardingState(), flow(listOf("v1", "NOT_A_STEP", "", "", "")).state.value)
    }

    /** A flow at S04 that already trusted the test server's certificate (as a saved flow does), so the check goes straight on. */
    private fun trustedAtSignIn(): OnboardingFlow {
        val saved = ArrayList(flow().toSignIn().snapshot())
        saved[3] = server.certificateFingerprint()
        return flow(saved)
    }

    @Test(timeout = 60_000)
    fun backDuringTheCheckCancelsItAndWipesThePassword() {
        val entered = AtomicBoolean(false)
        verifyHook = { entered.set(true); awaitCancellation() }
        val f = trustedAtSignIn()
        val bytes = pw()
        f.signIn("alice", bytes)
        await("the check started") { entered.get().takeIf { it } }
        assertEquals(Attempt.Working, f.state.value.attempt)
        assertTrue("Back works while the password is being checked", f.back())
        assertEquals(Step.ADD_ACCOUNT, f.state.value.step)
        assertTrue("the password was wiped", bytes.all { it == 0.toByte() })
        assertEquals(Attempt.Idle, f.state.value.attempt)
        assertTrue(core.listAccounts().isEmpty())
    }

    @Test(timeout = 60_000)
    fun theAddFinishesEvenWhenTheScreenGoesAway_withTheRightPasswordStored_andARetryDoesNotDuplicateTheAccount() {
        addGate = CompletableDeferred()
        val f = trustedAtSignIn()
        val bytes = pw()
        f.signIn("alice", bytes)
        await("the add started") { addStarted.get().takeIf { it } }
        assertTrue(f.state.value.adding)
        // Back waits for the add: the press is taken and nothing changes.
        assertTrue(f.back())
        assertEquals(Step.SIGN_IN, f.state.value.step)
        // The screen is destroyed (a rotation, the system) in the middle of the add.
        f.close()
        addGate!!.complete(Unit)
        val id = await("the account exists") { core.listAccounts().singleOrNull()?.id }
        // Its password was stored under its id, complete: the copy the add works on was not wiped with the attempt's array.
        await("the password was stored") { storedPasswords[id] }
        assertEquals(TEST_PASSWORD, storedPasswords[id])
        assertTrue("the flow's own array was wiped", bytes.all { it == 0.toByte() })

        // The person signs in again (the new screen starts from the saved path): the account is found, not added twice.
        addGate = null
        val again = trustedAtSignIn()
        again.signIn("alice", pw())
        val done = again.awaitState("on to the first sync") { it.addedAccount != null }
        assertEquals(id, done.addedAccount)
        assertEquals(1, core.listAccounts().size)
        assertWiped()
    }

    @Test(timeout = 60_000)
    fun aCancelledAttemptCannotWipeTheNextAttemptsPassword() {
        val entered = java.util.concurrent.atomic.AtomicInteger(0)
        verifyHook = { entered.incrementAndGet(); awaitCancellation() }
        val f = flow().toSignIn()
        val first = pw("first-fake-password-1")
        f.signIn("alice", first)
        await("the first check started") { entered.get().takeIf { it >= 1 } }
        // Straight on to another attempt (S05's test) while the first one is still unwinding from its cancel.
        f.openServerSettings()
        val second = pw("second-fake-password-2")
        f.testConnection(second)
        await("the second check started") { entered.get().takeIf { it >= 2 } }
        Thread.sleep(300)
        assertTrue("the first attempt's password was wiped", first.all { it == 0.toByte() })
        assertEquals("the second attempt's password is untouched", "second-fake-password-2", String(second))
        f.close()
        assertTrue("and wiped when it ends", second.all { it == 0.toByte() })
    }

    @Test(timeout = 60_000)
    fun aLateProbeCallbackOfAnOlderLookupCannotOverwriteTheNewOnesRows() {
        val observers = java.util.concurrent.CopyOnWriteArrayList<DiscoveryObserver>()
        discoverHook = { o -> observers += o; awaitCancellation() }
        val f = flow()
        f.openAddAccount()
        f.setEmail("alice@example.org")
        f.continueWithEmail()
        await("the first lookup") { observers.size.takeIf { it >= 1 } }
        assertTrue(f.back())
        f.continueWithEmail()
        await("the second lookup") { observers.size.takeIf { it >= 2 } }
        observers[1].onProbe(DiscoveryProbe.SRV, ProbeProgress.Running)
        // The old lookup's board speaks late.
        observers[0].onProbe(DiscoveryProbe.SRV, ProbeProgress.Found)
        observers[0].onProbe(DiscoveryProbe.PROVIDER_TABLE, ProbeProgress.Found)
        Thread.sleep(100)
        val rows = f.state.value.probes
        assertEquals(RowState.Running, rows[ProbeRow.SRV])
        assertEquals(RowState.Waiting, rows[ProbeRow.BUILT_IN_TABLE])
        f.close()
    }

    @Test
    fun aProvidersHostKeepsItsProviderWhateverThePortOrTheSecurity_andNoOtherHostHasOne() {
        val base = FfiDiscovered(
            email = "sam@gmail.com",
            imap = ServerSettings("imap.gmail.com", 993u, SecurityMode.TLS, "sam@gmail.com"),
            smtp = null, jmap = null, source = DiscoverySource.PROVIDER_TABLE, provider = uniffi.croft_ffi.KnownProvider.GMAIL, authHint = AuthKind.O_AUTH2_OR_APP_PASSWORD,
        )
        val form = ServerForm.from(base)
        // STARTTLS on port 143, and the host typed by hand with another case.
        assertEquals(uniffi.croft_ffi.KnownProvider.GMAIL, form.withSecurity(SecurityMode.START_TLS).toDiscovered(base.email, base)!!.provider)
        assertEquals(uniffi.croft_ffi.KnownProvider.GMAIL, form.copy(port = "2993").toDiscovered(base.email, base)!!.provider)
        assertEquals(uniffi.croft_ffi.KnownProvider.GMAIL, form.copy(host = "IMAP.Gmail.com").toDiscovered(base.email, null)!!.provider)
        assertEquals(uniffi.croft_ffi.KnownProvider.OUTLOOK, form.copy(host = "outlook.office365.com").toDiscovered(base.email, base)!!.provider)
        // Another server: no provider, and a pin is possible there.
        assertNull(form.copy(host = "mail.example.org").toDiscovered(base.email, base)!!.provider)
        assertNull(form.copy(host = "imap.gmail.com.evil.example").toDiscovered(base.email, null)!!.provider)
    }

    @Test
    fun theFormTurnsIntoSettingsOnlyWhenEveryFieldIsUsable() {
        val good = ServerForm(host = "mail.example.org", port = "993", username = "alice@example.org")
        assertTrue(good.problems().isEmpty())
        val d = good.toDiscovered("alice@example.org", null)!!
        assertEquals(993.toUShort(), d.imap!!.port)
        assertNull("an empty outgoing server means none", d.smtp)
        assertEquals(setOf(com.example.croft.mail.onboarding.FormField.HOST, com.example.croft.mail.onboarding.FormField.PORT, com.example.croft.mail.onboarding.FormField.USERNAME),
            ServerForm(host = "bad host", port = "70000", username = " ").problems().keys)
        assertNull(ServerForm(host = "", port = "993", username = "a").toDiscovered("a@example.org", null))
        // STARTTLS moves the usual port with it, but not a port the person chose.
        assertEquals("143", good.withSecurity(SecurityMode.START_TLS).port)
        assertEquals("1993", good.copy(port = "1993").withSecurity(SecurityMode.START_TLS).port)
        assertNotNull(good.copy(outHost = "smtp.example.org").toDiscovered("alice@example.org", null)!!.smtp)
    }
}
