// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.onboarding

import uniffi.croft_ffi.AuthKind
import uniffi.croft_ffi.DiscoverySource
import uniffi.croft_ffi.FfiDiscovered
import uniffi.croft_ffi.SecurityMode
import uniffi.croft_ffi.ServerSettings

/** Why one field of S05 cannot be used. The screen turns each into a sentence. */
enum class FieldProblem { EMPTY, BAD_HOST, BAD_PORT }

/** The fields of S05 that have a problem, by name. */
enum class FormField { HOST, PORT, USERNAME, OUT_HOST, OUT_PORT }

/**
 * What S05 (Server settings) edits: the incoming IMAP server (protocol, host, port, security, username) and, behind
 * "Advanced", the outgoing server. Ports are text so that a half-typed number is not lost. **Only the two encrypted modes
 * exist** ([SecurityMode] has no plain one): the core refuses an unencrypted connection to anything but a test server on this
 * computer, so the screen has no such option, not even for a private-network address. No password is kept here.
 */
data class ServerForm(
    val host: String = "",
    val port: String = DEFAULT_IMAP_TLS_PORT.toString(),
    val security: SecurityMode = SecurityMode.TLS,
    val username: String = "",
    val outHost: String = "",
    val outPort: String = DEFAULT_SMTP_TLS_PORT.toString(),
    val outSecurity: SecurityMode = SecurityMode.TLS,
) {
    /** The problem of each field that cannot be used; empty when the form can be turned into settings. */
    fun problems(): Map<FormField, FieldProblem> = buildMap {
        hostProblem(host)?.let { put(FormField.HOST, it) }
        portProblem(port)?.let { put(FormField.PORT, it) }
        if (username.isBlank()) put(FormField.USERNAME, FieldProblem.EMPTY)
        // The outgoing server is optional: an empty host means "none yet". If a host is there, its port must be right.
        if (outHost.isNotBlank()) {
            hostProblem(outHost)?.let { put(FormField.OUT_HOST, it) }
            portProblem(outPort)?.let { put(FormField.OUT_PORT, it) }
        }
    }

    /** The form changes the security: the port moves to the new mode's usual one, but only when it was the usual one of the old mode. */
    fun withSecurity(next: SecurityMode): ServerForm =
        copy(security = next, port = if (port == defaultImapPort(security).toString()) defaultImapPort(next).toString() else port)

    /** Same for the outgoing server. */
    fun withOutSecurity(next: SecurityMode): ServerForm =
        copy(outSecurity = next, outPort = if (outPort == defaultSmtpPort(outSecurity).toString()) defaultSmtpPort(next).toString() else outPort)

    /**
     * The settings for [email], or null while [problems] is not empty. [base] is what discovery found: when the incoming server is
     * still the one it found, its provider and source are kept; when the person changed it, the account is no longer "known
     * provider" (the core would otherwise ignore a pinned certificate for it).
     */
    fun toDiscovered(email: String, base: FfiDiscovered?): FfiDiscovered? {
        if (problems().isNotEmpty()) return null
        val imap = ServerSettings(host.trim(), port.trim().toUShort(), security, username.trim())
        val smtp = outHost.trim().takeIf { it.isNotEmpty() }?.let { ServerSettings(it, outPort.trim().toUShort(), outSecurity, username.trim()) }
        val kept = base?.takeIf { b -> b.imap?.let { it.host == imap.host && it.port == imap.port && it.security == imap.security } == true }
        return FfiDiscovered(
            email = email.trim(),
            imap = imap,
            smtp = smtp,
            jmap = base?.jmap,
            source = kept?.source ?: DiscoverySource.AUTOCONFIG,
            provider = kept?.provider,
            authHint = kept?.authHint ?: AuthKind.PASSWORD,
        )
    }

    companion object {
        const val DEFAULT_IMAP_TLS_PORT = 993
        const val DEFAULT_IMAP_STARTTLS_PORT = 143
        const val DEFAULT_SMTP_TLS_PORT = 465
        const val DEFAULT_SMTP_STARTTLS_PORT = 587

        fun defaultImapPort(s: SecurityMode): Int = if (s == SecurityMode.TLS) DEFAULT_IMAP_TLS_PORT else DEFAULT_IMAP_STARTTLS_PORT
        fun defaultSmtpPort(s: SecurityMode): Int = if (s == SecurityMode.TLS) DEFAULT_SMTP_TLS_PORT else DEFAULT_SMTP_STARTTLS_PORT

        /** The form for what discovery found. */
        fun from(d: FfiDiscovered): ServerForm {
            val imap = d.imap
            val smtp = d.smtp
            return ServerForm(
                host = imap?.host.orEmpty(),
                port = (imap?.port?.toInt() ?: DEFAULT_IMAP_TLS_PORT).toString(),
                security = imap?.security ?: SecurityMode.TLS,
                username = imap?.username ?: d.email,
                outHost = smtp?.host.orEmpty(),
                outPort = (smtp?.port?.toInt() ?: DEFAULT_SMTP_TLS_PORT).toString(),
                outSecurity = smtp?.security ?: SecurityMode.TLS,
            )
        }

        /**
         * The guess S05 starts from when nothing was found: the address as the login name and `mail.<domain>` as the incoming server, both
         * editable (the outgoing server is left empty). It is a guess and S05 says so.
         */
        fun guessFor(email: String): ServerForm {
            val domain = EmailInput.domainOf(email).orEmpty()
            return ServerForm(
                host = if (domain.isEmpty()) "" else "mail.$domain",
                username = email.trim(),
            )
        }

        /** A host name (letters, digits, dots, hyphens; at most 253 characters) or an IPv4 address. */
        fun hostProblem(text: String): FieldProblem? {
            val t = text.trim()
            if (t.isEmpty()) return FieldProblem.EMPTY
            if (t.length > 253 || t.any { it.isWhitespace() }) return FieldProblem.BAD_HOST
            val labels = t.split('.')
            val ok = labels.all { l -> l.isNotEmpty() && l.length <= 63 && !l.startsWith('-') && !l.endsWith('-') && l.all { it.isLetterOrDigit() && it.code < 128 || it == '-' } }
            return if (ok) null else FieldProblem.BAD_HOST
        }

        /** A port number from 1 to 65535. */
        fun portProblem(text: String): FieldProblem? {
            val t = text.trim()
            if (t.isEmpty()) return FieldProblem.EMPTY
            val n = t.toIntOrNull() ?: return FieldProblem.BAD_PORT
            return if (n in 1..65535 && t.all { it.isDigit() }) null else FieldProblem.BAD_PORT
        }
    }
}

/** The steps "Test connection" shows, in order. The core's `verify` is one call; the step that failed is read from its typed failure. */
enum class TestStep { REACH_SERVER, CERTIFICATE, SIGN_IN }

/** One step's state in the list on S05. */
enum class StepState { PENDING, RUNNING, DONE, FAILED }

/** The list S05 shows under Test connection. */
data class ConnectionTest(val steps: Map<TestStep, StepState>) {
    companion object {
        /** The test has started: the first step runs, the others wait. */
        fun started() = ConnectionTest(TestStep.entries.associateWith { if (it == TestStep.REACH_SERVER) StepState.RUNNING else StepState.PENDING })

        /** Everything worked. */
        fun passed() = ConnectionTest(TestStep.entries.associateWith { StepState.DONE })

        /** The test stopped at [failed]: the steps before it are done, it failed, the rest never ran. */
        fun stoppedAt(failed: TestStep) = ConnectionTest(
            TestStep.entries.associateWith {
                when {
                    it.ordinal < failed.ordinal -> StepState.DONE
                    it == failed -> StepState.FAILED
                    else -> StepState.PENDING
                }
            },
        )

        /** The step a sign-in failure belongs to: reaching the server, its certificate, or the sign-in itself. */
        fun stepOf(failure: uniffi.croft_ffi.VerifyFailure): TestStep = when (failure) {
            is uniffi.croft_ffi.VerifyFailure.UntrustedCertificate,
            is uniffi.croft_ffi.VerifyFailure.CertificateChanged,
            is uniffi.croft_ffi.VerifyFailure.ConnectionIntercepted,
            is uniffi.croft_ffi.VerifyFailure.CertificateInvalid,
            is uniffi.croft_ffi.VerifyFailure.NoSecureConnection,
            -> TestStep.CERTIFICATE
            is uniffi.croft_ffi.VerifyFailure.WrongPassword,
            is uniffi.croft_ffi.VerifyFailure.NeedsAppPasswordOrOAuth,
            is uniffi.croft_ffi.VerifyFailure.ServerProblem,
            -> TestStep.SIGN_IN
            else -> TestStep.REACH_SERVER
        }
    }
}
