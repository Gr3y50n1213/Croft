// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.DraftSyncOutcome
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.FfiDraft

/** When the draft is saved (docs/ux/state-machines.md, "Outgoing message"): on the device 1 s after typing stops, on the server 5 s after, at most every 20 s. */
data class AutosaveTiming(
    val localMs: Long = 1_000,
    val serverIdleMs: Long = 5_000,
    val serverMinIntervalMs: Long = 20_000,
)

/**
 * The logic of the compose screen (S23), apart from drawing it: the fields, the recipient chips, the attachments, the autosave, the
 * Send button's state, the questions before sending (empty message, S24 encryption check) and the Back rules. It talks to the core
 * only through [ComposeBackend]; the screen draws [state] and calls the functions.
 *
 * - **Saving.** Typing (subject, body) is saved on the device [AutosaveTiming.localMs] after the last keystroke and copied to the server
 *   after [AutosaveTiming.serverIdleMs], never more often than [AutosaveTiming.serverMinIntervalMs]; adding or removing a recipient or
 *   a file is saved at once, so the Send button (from `check_draft`) is never behind. [flush] (the app pauses) saves and copies now.
 * - **Send** goes through [requestSend]: a disabled Send only explains; an empty subject and body ask first ([SendPrompt.EmptyConfirm]);
 *   then the [EncryptionPolicy] is asked, and when it says a key is missing the message goes only after [confirmUnencrypted]. The
 *   core's `send` is called nowhere else.
 * - **Back** ([back]): an untouched compose closes; an emptied one discards its draft and closes; with content it is saved, copied
 *   to the server and compose closes with [ComposeClosed.DraftSaved] (the screen shows "Draft saved").
 *
 * Nothing printed or logged holds an address, a subject, a body or a file name.
 */
class ComposeController(
    private val backend: ComposeBackend,
    private val scope: CoroutineScope,
    private val accounts: suspend () -> List<AccountInfo>,
    private val intake: AttachmentIntake,
    private val tools: AddressTools = CoreAddressTools,
    private val policy: EncryptionPolicy = NoEncryptionPolicy,
    private val cursors: CursorMemory = CursorMemory(),
    private val timing: AutosaveTiming = AutosaveTiming(),
    private val clock: () -> Long = System::currentTimeMillis,
    /** Turns a content URI (a share's or a `mailto:`'s file) into something to copy; null when none can be read. */
    private val files: (String) -> PickedFile? = { null },
    /** Work that must outlive the screen (the copy to the server when compose closes). */
    private val background: CoroutineScope = defaultBackground,
) {
    private val mutable = MutableStateFlow(ComposeState())

    /** What the screen draws. */
    val state: StateFlow<ComposeState> = mutable.asStateFlow()

    private val ops = Mutex()
    private var localJob: Job? = null
    private var serverJob: Job? = null
    private val intakeJobs = ArrayList<Job>()
    private var dirty = false
    private var userEdited = false
    private var lastSyncAt = 0L
    private var emptyConfirmed = false
    private var inReplyTo: String? = null
    private var references: List<String> = emptyList()
    private var bodyCursor: BodyCursor? = null
    private var lastCheck: uniffi.croft_ffi.DraftCheck? = null

    // ---- Opening ----

    /** Opens compose as [start] says. Call once. The state is [ComposeState.Phase.EDITING] when it returns, or says why it is not. */
    suspend fun open(start: ComposeStart) {
        openLocked(start)
        // The files of a share are copied right away (its read grant lasts only as long as its task), after the lock is free.
        val uris = pendingUris
        pendingUris = emptyList()
        if (uris.isNotEmpty() && state.value.phase == ComposeState.Phase.EDITING) addFromUris(uris)
    }

    private var pendingUris: List<String> = emptyList()

    private suspend fun openLocked(start: ComposeStart) = ops.withLock {
        try {
            when (start) {
                is ComposeStart.New -> openFresh(start.account, ComposeFill())
                is ComposeStart.Fill -> openFresh(start.account, start.fill)
                is ComposeStart.Reply -> openReply(start.account) { backend.replyDraft(start.account, start.messageId) }
                is ComposeStart.ReplyAll -> openReply(start.account) { backend.replyAllDraft(start.account, start.messageId) }
                is ComposeStart.Forward -> openReply(start.account) { backend.forwardDraft(start.account, start.messageId) }
                is ComposeStart.Draft -> openDraft(start)
            }
        } catch (e: ComposeFailure) {
            mutable.update { it.copy(phase = ComposeState.Phase.FAILED_TO_OPEN) }
        }
    }

    private suspend fun defaultAccount(wanted: AccountId?): AccountId? {
        val all = accounts()
        return wanted?.takeIf { w -> all.any { it.id == w } } ?: all.firstOrNull()?.id
    }

    private suspend fun openFresh(wanted: AccountId?, fill: ComposeFill) {
        val account = defaultAccount(wanted) ?: throw ComposeFailure(ComposeProblem.NO_SUCH_ACCOUNT)
        val identities = backend.identities(account)
        val to = chipsOfText(fill.to)
        val cc = chipsOfText(fill.cc)
        val bcc = chipsOfText(fill.bcc)
        mutable.update {
            it.copy(
                phase = ComposeState.Phase.EDITING, account = account, identities = identities,
                identity = identities.firstOrNull { i -> i.isDefault }?.id ?: identities.firstOrNull()?.id,
                to = to, cc = cc, bcc = bcc, showCcBcc = cc.isNotEmpty() || bcc.isNotEmpty(),
                subject = fill.subject, body = fill.body,
                initialCursor = if (fill.body.isEmpty()) null else BodyCursor(0, 0),
            )
        }
        refreshStatus()
        pendingUris = fill.attachments
    }

    private suspend fun openReply(wanted: AccountId, make: suspend () -> uniffi.croft_ffi.ReplyDraft) {
        val account = defaultAccount(wanted) ?: throw ComposeFailure(ComposeProblem.NO_SUCH_ACCOUNT)
        val identities = backend.identities(account)
        val reply = make()
        val c = reply.content
        inReplyTo = c.inReplyTo
        references = c.references
        mutable.update {
            it.copy(
                phase = ComposeState.Phase.EDITING, account = account, identities = identities,
                identity = c.identity ?: identities.firstOrNull { i -> i.isDefault }?.id ?: identities.firstOrNull()?.id,
                to = chipsOf(c.to), cc = chipsOf(c.cc), bcc = chipsOf(c.bcc),
                showCcBcc = c.cc.isNotEmpty() || c.bcc.isNotEmpty(),
                subject = c.subject, body = c.body, quoteMissing = !reply.quoteIncluded,
                // A reply starts above the quote.
                initialCursor = BodyCursor(0, 0),
            )
        }
        refreshStatus()
    }

    private suspend fun openDraft(start: ComposeStart.Draft) {
        val draft: FfiDraft = backend.loadDraft(start.account, start.draftId)
        val identities = backend.identities(start.account)
        val c = draft.content
        inReplyTo = c.inReplyTo
        references = c.references
        val cursor = start.cursor ?: cursors.recall(draft.id)
        mutable.update {
            it.copy(
                phase = if (draft.outboxId != null) ComposeState.Phase.LOCKED else ComposeState.Phase.EDITING,
                account = start.account, draftId = draft.id, identities = identities,
                identity = c.identity ?: identities.firstOrNull { i -> i.isDefault }?.id ?: identities.firstOrNull()?.id,
                to = chipsOf(c.to), cc = chipsOf(c.cc), bcc = chipsOf(c.bcc), showCcBcc = c.cc.isNotEmpty() || c.bcc.isNotEmpty(),
                subject = c.subject, body = c.body, attachments = draft.attachments,
                initialCursor = cursor ?: BodyCursor(c.body.length, c.body.length),
                saveStatus = when (draft.serverCopy) {
                    uniffi.croft_ffi.ServerCopy.CURRENT -> SaveStatus.SAVED
                    else -> SaveStatus.SAVED_ON_DEVICE
                },
            )
        }
        bodyCursor = cursor
        if (draft.outboxId == null) refreshStatus()
    }

    private fun chipsOf(list: List<FfiAddress>): List<RecipientChipData> = list.map { RecipientChipData(it, tools.isValid(it.address)) }

    private fun chipsOfText(texts: List<String>): List<RecipientChipData> =
        texts.flatMap { parseChips(it) }.distinctBy { it.address.address.lowercase() }

    private fun parseChips(text: String): List<RecipientChipData> {
        val t = text.trim()
        if (t.isEmpty()) return emptyList()
        val parsed = tools.parse(t)
        val list = parsed.ifEmpty { listOf(FfiAddress(null, t)) }
        return list.map { RecipientChipData(it, tools.isValid(it.address)) }
    }

    // ---- Fields ----

    /** The subject changed (typing): saved after a pause. */
    fun onSubject(text: String) {
        if (!editable()) return
        userEdited = true
        mutable.update { it.copy(subject = text, sendProblem = null) }
        touch()
    }

    /** The body changed (typing); [selection] is where the cursor is, kept for Undo. */
    fun onBody(text: String, selection: BodyCursor? = null) {
        if (!editable()) return
        userEdited = true
        if (selection != null) bodyCursor = selection
        mutable.update { it.copy(body = text, sendProblem = null) }
        touch()
    }

    /** The cursor moved without a change of text. */
    fun onSelection(selection: BodyCursor) { bodyCursor = selection }

    /** Shows the Cc and Bcc fields. */
    fun showCcBcc() { mutable.update { it.copy(showCcBcc = true) } }

    /**
     * Text typed or pasted into a recipient field was committed (comma, space, Enter, the field lost focus, a suggestion was picked).
     * It becomes chips: an address the core does not accept becomes an error chip (never dropped). Saved at once.
     */
    suspend fun commitRecipients(field: RecipientField, text: String) {
        val chips = parseChips(text)
        if (chips.isEmpty() || !editable()) return
        userEdited = true
        mutate(field) { existing ->
            val have = existing.map { it.address.address.lowercase() }.toSet()
            existing + chips.filter { it.address.address.lowercase() !in have }.distinctBy { it.address.address.lowercase() }
        }
        saveStructural()
    }

    /** A picked suggestion becomes a chip. */
    suspend fun addRecipient(field: RecipientField, address: FfiAddress) {
        if (!editable()) return
        userEdited = true
        mutate(field) { existing ->
            if (existing.any { it.address.address.equals(address.address, ignoreCase = true) }) existing
            else existing + RecipientChipData(address, tools.isValid(address.address))
        }
        saveStructural()
    }

    suspend fun removeRecipient(field: RecipientField, chip: RecipientChipData) {
        if (!editable()) return
        userEdited = true
        mutate(field) { existing -> existing - chip }
        saveStructural()
    }

    /** Move a chip to another field (the chip menu: Move to Cc, Move to Bcc; a drag). */
    suspend fun moveRecipient(from: RecipientField, chip: RecipientChipData, to: RecipientField) {
        if (from == to || !editable()) return
        userEdited = true
        mutate(from) { it - chip }
        mutate(to) { existing -> if (existing.any { it.address.address.equals(chip.address.address, true) }) existing else existing + chip }
        if (to != RecipientField.TO) mutable.update { it.copy(showCcBcc = true) }
        saveStructural()
    }

    private fun mutate(field: RecipientField, change: (List<RecipientChipData>) -> List<RecipientChipData>) {
        mutable.update {
            when (field) {
                RecipientField.TO -> it.copy(to = change(it.to), sendProblem = null)
                RecipientField.CC -> it.copy(cc = change(it.cc), sendProblem = null)
                RecipientField.BCC -> it.copy(bcc = change(it.bcc), sendProblem = null)
            }
        }
    }

    /** Autocomplete from the addresses of mail sent from this account (no contacts permission). */
    suspend fun suggest(prefix: String): List<FfiAddress> {
        val p = prefix.trim()
        if (p.isEmpty()) return emptyList()
        return try { backend.suggestRecipients(state.value.account, p, SUGGESTIONS) } catch (e: ComposeFailure) { emptyList() }
    }

    /** Picks another sender. A different account starts a new draft there (attachments stay with the old one, so it is refused while there are any). */
    suspend fun setSender(account: AccountId, identity: Long?) = ops.withLock {
        val s = state.value
        if (!editable()) return@withLock
        if (account == s.account) {
            mutable.update { it.copy(identity = identity) }
            userEdited = true
            saveLocked()
            return@withLock
        }
        if (!s.canSwitchAccount) return@withLock
        val identities = try { backend.identities(account) } catch (e: ComposeFailure) { return@withLock }
        s.draftId?.let { id -> try { backend.discardDraft(s.account, id) } catch (e: ComposeFailure) { /* gone already */ } }
        userEdited = true
        mutable.update {
            it.copy(account = account, draftId = null, identities = identities, identity = identity ?: identities.firstOrNull { i -> i.isDefault }?.id, saveStatus = SaveStatus.NONE)
        }
        saveLocked()
    }

    // ---- Attachments ----

    /** Pick results: each file is copied into the attachment folder and added to the draft. Send waits for them. */
    fun addFiles(picked: List<PickedFile>) {
        if (!editable()) return
        for (file in picked) {
            val job = scope.launch { attachOne(file) }
            synchronized(intakeJobs) { intakeJobs += job }
        }
    }

    /** Adds one picked file and returns when it is in the draft (or refused). The screen uses [addFiles], which does not wait. */
    suspend fun addFromFile(file: PickedFile) = attachOne(file)

    /** Content URIs of a share or a `mailto:`, copied as soon as compose opens. */
    suspend fun addFromUris(uris: List<String>) {
        for (uri in uris) {
            val file = files(uri)
            if (file == null) mutable.update { it.copy(attachmentError = AttachmentError.UNREADABLE) } else attachOne(file)
        }
    }

    private suspend fun attachOne(file: PickedFile) {
        mutable.update { it.copy(attachmentsBusy = it.attachmentsBusy + 1, attachmentError = null) }
        try {
            userEdited = true
            val id = ops.withLock { ensureDraft() } ?: return
            val s = state.value
            val result = intake.add(s.account, id, file, s.attachments.size, s.attachments.sumOf { it.sizeBytes.toLong() })
            when (result) {
                is AttachmentIntake.Result.Added -> mutable.update { it.copy(attachments = it.attachments + result.attachment) }
                is AttachmentIntake.Result.Refused -> mutable.update { it.copy(attachmentError = result.error) }
            }
        } finally {
            mutable.update { it.copy(attachmentsBusy = (it.attachmentsBusy - 1).coerceAtLeast(0)) }
            ops.withLock { refreshStatus() }
            scheduleServer(timing.serverIdleMs)
        }
    }

    suspend fun removeAttachment(attachmentId: Long) = ops.withLock {
        val s = state.value
        val id = s.draftId ?: return@withLock
        if (!editable()) return@withLock
        try {
            backend.removeAttachment(s.account, id, attachmentId)
            mutable.update { it.copy(attachments = it.attachments.filter { a -> a.id != attachmentId }, attachmentError = null) }
        } catch (e: ComposeFailure) {
            mutable.update { it.copy(attachmentError = AttachmentError.FAILED) }
        }
        refreshStatus()
        scheduleServer(timing.serverIdleMs)
    }

    /** The attachment error was shown. */
    fun attachmentErrorShown() { mutable.update { it.copy(attachmentError = null) } }

    // ---- Autosave ----

    private fun editable() = state.value.phase == ComposeState.Phase.EDITING

    private fun touch() {
        dirty = true
        localJob?.cancel()
        localJob = scope.launch {
            delay(timing.localMs)
            ops.withLock { saveLocked() }
        }
        scheduleServer(timing.serverIdleMs)
    }

    private fun scheduleServer(afterMs: Long) {
        serverJob?.cancel()
        serverJob = scope.launch {
            delay(afterMs)
            syncToServer(force = false)
        }
    }

    private suspend fun saveStructural() {
        ops.withLock { saveLocked() }
        scheduleServer(timing.serverIdleMs)
    }

    private fun isBlank(s: ComposeState) = s.allRecipients.isEmpty() && s.subject.isBlank() && s.body.isBlank() && s.attachments.isEmpty()

    private fun contentOf(s: ComposeState) = DraftContent(
        identity = s.identity,
        to = s.to.map { it.address }, cc = s.cc.map { it.address }, bcc = s.bcc.map { it.address },
        subject = s.subject, body = s.body, inReplyTo = inReplyTo, references = references,
    )

    /** Makes sure a draft exists on the device (a file can only be added to one). Call with [ops] held. */
    private suspend fun ensureDraft(): Long? {
        state.value.draftId?.let { return it }
        saveLocked(force = true)
        return state.value.draftId
    }

    /** Saves the draft on the device now and re-reads the Send button's facts. Call with [ops] held. */
    private suspend fun saveLocked(force: Boolean = false) {
        val s = state.value
        if (s.phase != ComposeState.Phase.EDITING) return
        if (s.draftId == null && !force && isBlank(s)) { dirty = false; return }
        try {
            val saved = backend.saveDraft(s.account, s.draftId, contentOf(s))
            dirty = false
            mutable.update {
                it.copy(
                    draftId = saved.id,
                    saveStatus = SaveStatus.SAVED_ON_DEVICE,
                )
            }
            bodyCursor?.let { c -> cursors.remember(saved.id, c) }
        } catch (e: ComposeFailure) {
            mutable.update {
                if (e.problem == ComposeProblem.IN_OUTBOX) it.copy(phase = ComposeState.Phase.LOCKED) else it.copy(saveStatus = SaveStatus.COULD_NOT_SAVE)
            }
            return
        }
        refreshStatus()
    }

    /** Re-reads `check_draft`, the policy and the account's state into the Send button. Call with [ops] held (or from a single caller). */
    private suspend fun refreshStatus() {
        val s = state.value
        val offline = try {
            accounts().firstOrNull { it.id == s.account }?.status.let { it == AccountState.OFFLINE || it == AccountState.NEEDS_SIGN_IN }
        } catch (e: Exception) { false }
        val addresses = s.allRecipients.map { it.address.address }
        val warn = addresses.isNotEmpty() && policy.check(s.account, addresses) is EncryptionDecision.RequiredKeysMissing
        val id = s.draftId
        if (id == null) {
            mutable.update { it.copy(sendState = SendState.NO_RECIPIENT, offlineOrSignIn = offline, encryptionWarning = warn) }
            return
        }
        try {
            val check = backend.checkDraft(s.account, id)
            lastCheck = check
            mutable.update {
                it.copy(
                    sendState = if (it.phase == ComposeState.Phase.SENDING) SendState.SENDING else SendRules.stateOf(check, it.attachmentsBusy > 0, warn, offline),
                    sizeLimitMb = SendRules.limitMb(check), offlineOrSignIn = offline, encryptionWarning = warn,
                )
            }
        } catch (e: ComposeFailure) {
            // Keep what the button showed.
        }
    }

    /** The account's state changed (a change notice): re-reads the Send button's offline and sign-in condition. */
    suspend fun refreshState() = ops.withLock { if (state.value.phase == ComposeState.Phase.EDITING) refreshStatus() }

    private suspend fun syncToServer(force: Boolean) {
        val s = state.value
        if (s.phase != ComposeState.Phase.EDITING) return
        if (dirty) ops.withLock { saveLocked() }
        val id = state.value.draftId ?: return
        val since = clock() - lastSyncAt
        if (!force && since < timing.serverMinIntervalMs) {
            scheduleServer(timing.serverMinIntervalMs - since)
            return
        }
        lastSyncAt = clock()
        val outcome = try { backend.syncDraftToServer(s.account, id, force) } catch (e: ComposeFailure) { null }
        if (outcome == DraftSyncOutcome.DEFERRED) { scheduleServer(timing.serverMinIntervalMs); return }
        mutable.update {
            if (it.phase != ComposeState.Phase.EDITING) it else it.copy(
                saveStatus = when (outcome) {
                    DraftSyncOutcome.SAVED, DraftSyncOutcome.ALREADY_CURRENT -> SaveStatus.SAVED
                    DraftSyncOutcome.OFFLINE -> SaveStatus.WAITING_FOR_CONNECTION
                    else -> SaveStatus.SAVED_ON_DEVICE
                },
            )
        }
    }

    /** The app pauses (or compose is left): save on the device and copy to the server now. */
    suspend fun flush() {
        localJob?.cancel()
        serverJob?.cancel()
        if (state.value.phase != ComposeState.Phase.EDITING) return
        ops.withLock { saveLocked() }
        syncToServer(force = true)
    }

    // ---- Send ----

    /** The person tapped Send. See the class comment for the order of the questions. */
    suspend fun requestSend() {
        if (!editable()) return
        localJob?.cancel()
        serverJob?.cancel()
        // A file still being copied is waited for: the message must never go without it.
        val pending = synchronized(intakeJobs) { intakeJobs.toList() }
        pending.joinAll()
        val check = ops.withLock {
            saveLocked(force = state.value.draftId == null && !isBlank(state.value))
            state.value.draftId?.let { id -> try { backend.checkDraft(state.value.account, id) } catch (e: ComposeFailure) { null } }
        }
        if (check == null) {
            mutable.update { it.copy(prompt = SendPrompt.Explain(SendState.NO_RECIPIENT), sendState = SendState.NO_RECIPIENT) }
            return
        }
        val warn = state.value.encryptionWarning
        val shown = SendRules.stateOf(check, false, warn, state.value.offlineOrSignIn)
        if (SendRules.isDisabled(shown)) {
            mutable.update { it.copy(prompt = SendPrompt.Explain(shown), sendState = shown, sizeLimitMb = SendRules.limitMb(check)) }
            return
        }
        emptyConfirmed = false
        if (check.subjectEmpty && check.bodyEmpty) {
            mutable.update { it.copy(prompt = SendPrompt.EmptyConfirm) }
            return
        }
        askPolicyThenSend()
    }

    /** "Send anyway" for a message without subject or text. */
    suspend fun confirmEmpty() {
        if (state.value.prompt !is SendPrompt.EmptyConfirm) return
        mutable.update { it.copy(prompt = null) }
        emptyConfirmed = true
        askPolicyThenSend()
    }

    /** S24 "Send unencrypted": the only way past a missing key. */
    suspend fun confirmUnencrypted() {
        if (state.value.prompt !is SendPrompt.EncryptionCheck) return
        mutable.update { it.copy(prompt = null) }
        doSend()
    }

    /** Cancel on any question: nothing is sent, compose stays open. */
    fun cancelPrompt() { mutable.update { it.copy(prompt = null) } }

    private suspend fun askPolicyThenSend() {
        val s = state.value
        val decision = policy.check(s.account, s.allRecipients.map { it.address.address })
        when (decision) {
            EncryptionDecision.NotRequired -> doSend()
            is EncryptionDecision.RequiredKeysMissing -> mutable.update { it.copy(prompt = SendPrompt.EncryptionCheck(decision.missing), encryptionWarning = true) }
        }
    }

    private suspend fun doSend() = ops.withLock {
        val s = state.value
        val id = s.draftId ?: return@withLock
        if (s.phase != ComposeState.Phase.EDITING) return@withLock
        mutable.update { it.copy(phase = ComposeState.Phase.SENDING, sendState = SendState.SENDING, prompt = null) }
        try {
            val started = backend.send(s.account, id)
            val cursor = bodyCursor
            if (cursor != null) cursors.remember(id, cursor)
            mutable.update { it.copy(phase = ComposeState.Phase.CLOSED, closed = ComposeClosed.Sent(s.account, started.outboxId.toLong(), started.sendAtMs, cursor)) }
        } catch (e: ComposeFailure) {
            mutable.update { it.copy(phase = ComposeState.Phase.EDITING, sendProblem = e.problem) }
            refreshStatus()
        }
    }

    // ---- Leaving ----

    /** Back (or the close button). See the class comment. */
    suspend fun back() {
        val s0 = state.value
        if (s0.phase == ComposeState.Phase.LOCKED || s0.phase == ComposeState.Phase.FAILED_TO_OPEN || s0.phase == ComposeState.Phase.LOADING) {
            mutable.update { it.copy(phase = ComposeState.Phase.CLOSED, closed = ComposeClosed.Nothing) }
            return
        }
        if (s0.phase != ComposeState.Phase.EDITING) return
        localJob?.cancel()
        serverJob?.cancel()
        synchronized(intakeJobs) { intakeJobs.toList() }.joinAll()
        val closed = ops.withLock {
            val s = state.value
            when {
                s.draftId == null && !userEdited && s.attachments.isEmpty() -> ComposeClosed.Nothing
                isBlank(s) -> {
                    s.draftId?.let { id -> try { backend.discardDraft(s.account, id); cursors.forget(id) } catch (e: ComposeFailure) { /* gone */ } }
                    ComposeClosed.Nothing
                }
                else -> {
                    if (dirty || s.draftId == null) saveLocked(force = true)
                    val saved = state.value
                    if (saved.saveStatus == SaveStatus.COULD_NOT_SAVE || saved.draftId == null) ComposeClosed.Nothing else ComposeClosed.DraftSaved
                }
            }
        }
        val s = state.value
        if (closed == ComposeClosed.DraftSaved && s.draftId != null) {
            // The copy to the server goes on after the screen is gone.
            background.launch { try { backend.syncDraftToServer(s.account, s.draftId, true) } catch (e: ComposeFailure) { /* the core copies it later */ } }
        }
        mutable.update { it.copy(phase = ComposeState.Phase.CLOSED, closed = closed) }
    }

    /** The person chose Discard (asked about first by the screen). */
    suspend fun discard() = ops.withLock {
        localJob?.cancel()
        serverJob?.cancel()
        val s = state.value
        if (s.phase != ComposeState.Phase.EDITING && s.phase != ComposeState.Phase.LOCKED) return@withLock
        if (s.phase == ComposeState.Phase.EDITING) s.draftId?.let { id -> try { backend.discardDraft(s.account, id); cursors.forget(id) } catch (e: ComposeFailure) { /* gone */ } }
        mutable.update { it.copy(phase = ComposeState.Phase.CLOSED, closed = ComposeClosed.Discarded) }
    }

    /** Stops the timers (the screen left without Back: the draft stays as it was last saved). */
    fun stop() {
        localJob?.cancel()
        serverJob?.cancel()
    }

    companion object {
        private const val SUGGESTIONS = 5
        private val defaultBackground = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
