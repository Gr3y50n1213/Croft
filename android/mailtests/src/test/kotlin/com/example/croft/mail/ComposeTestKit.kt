// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import com.example.croft.mail.compose.AttachmentIntake
import com.example.croft.mail.compose.AutosaveTiming
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.ComposeState
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.CoreAddressTools
import com.example.croft.mail.compose.CursorMemory
import com.example.croft.mail.compose.EncryptionPolicy
import com.example.croft.mail.compose.NoEncryptionPolicy
import com.example.croft.mail.compose.PickedFile
import java.io.ByteArrayInputStream
import java.io.InputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/** Compose controllers over a [Rig]'s real core. The scope is cancelled with [close]. */
class ComposeHarness(private val rig: Rig) : AutoCloseable {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    val cursors = CursorMemory()

    fun controller(
        policy: EncryptionPolicy = NoEncryptionPolicy,
        timing: AutosaveTiming = AutosaveTiming(),
        accounts: suspend () -> List<AccountInfo> = { rig.repo.listAccounts() },
        files: (String) -> PickedFile? = { null },
    ) = ComposeController(
        backend = rig.compose, scope = scope, accounts = accounts, intake = AttachmentIntake(rig.compose, rig.attachDir),
        tools = CoreAddressTools, policy = policy, cursors = cursors, timing = timing, files = files,
    )

    /** Opens a controller on [start] and waits until it is editing. */
    fun open(start: ComposeStart = ComposeStart.New(rig.account), policy: EncryptionPolicy = NoEncryptionPolicy, timing: AutosaveTiming = AutosaveTiming()): ComposeController =
        controller(policy, timing).also { c -> rig.run { c.open(start) }; check(c.state.value.phase == ComposeState.Phase.EDITING) { "not editing: ${c.state.value.phase}" } }

    override fun close() { scope.cancel() }
}

/** A picked file of [bytes] (the content resolver's answer, faked). */
class MemoryFile(
    private val bytes: ByteArray,
    override val displayName: String? = "notes.txt",
    override val mimeType: String? = "text/plain",
    override val declaredSize: Long? = bytes.size.toLong(),
) : PickedFile {
    override fun open(): InputStream = ByteArrayInputStream(bytes)
}

/** A picked file that claims [declared] bytes and gives [stream], for the limit checks. */
class ClaimedFile(private val declared: Long?, private val stream: () -> InputStream?) : PickedFile {
    override val displayName: String? = "big.bin"
    override val mimeType: String? = null
    override val declaredSize: Long? get() = declared
    override fun open(): InputStream? = stream()
}
