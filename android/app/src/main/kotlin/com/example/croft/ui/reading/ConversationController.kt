// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import android.content.res.Resources
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.example.croft.R
import com.example.croft.mail.AccountId
import com.example.croft.mail.FolderInfo
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MailError
import com.example.croft.mail.MailException
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.mail.MessageId
import com.example.croft.mail.OpId
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.Mailto
import com.example.croft.mail.reading.AttachmentData
import com.example.croft.mail.reading.BodyData
import com.example.croft.mail.reading.ConversationData
import com.example.croft.mail.reading.ConversationMessageData
import com.example.croft.mail.reading.LinkAction
import com.example.croft.mail.reading.ReadFailure
import com.example.croft.mail.reading.ReadProblem
import com.example.croft.mail.reading.ReadingRules
import com.example.croft.mail.reading.RemoteImages
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.mail.InboxTexts
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Where the conversation itself is: being opened, shown, or not openable (a message that is gone, an account that was removed). */
sealed interface ConversationPhase {
    data object Loading : ConversationPhase
    data object Ready : ConversationPhase
    data class Failed(val problem: ReadProblem) : ConversationPhase
}

/** Where the body of one message is (S21). */
sealed interface BodyPhase {
    data object Loading : BodyPhase
    data class Ready(val body: BodyData) : BodyPhase
    data class Failed(val problem: ReadProblem) : BodyPhase
}

/** An attachment chip's own state while the screen is open; "blocked" comes from the attachment itself. */
enum class ChipPhase { NOT_DOWNLOADED, DOWNLOADING, DOWNLOADED, FAILED }

/** One attachment of one message. */
data class ChipKey(val messageId: MessageId, val attachmentId: Long)

/**
 * What S20 hands to the Inbox when it closes after an action that has an Undo: the text and the operations Undo cancels. [ops] null: no
 * Undo button (delete forever, which cannot be undone). Compared by identity: two equal results are two notices.
 */
class ConversationNotice(val text: String, val ops: List<OpId>?)

/** The Move to question of S20: the messages in [account] that would move, and the folder they are in. */
data class ConversationMove(val account: AccountId, val excluding: Set<Long>)

/** The first Archive or Delete of an account without such a folder: which folder is yours (docs/ux/message-list.md, "Folder rules")? */
data class ConversationFolderRequest(val role: FolderRole, val account: AccountId, val retry: suspend () -> Unit)

/** Texts of S20 and S21 that are not drawn by a composable, built from the resources so the controller holds no Context. */
class ReadingTexts(private val res: Resources) {
    val blockedAttachment: String get() = res.getString(R.string.reading_attachment_blocked_snack)
    val noApp: String get() = res.getString(R.string.reading_attachment_no_app)
    val trustedSender: String get() = res.getString(R.string.reading_remote_trusted_snack)
    fun problem(p: ReadProblem): String = res.getString(ReadCopy.resFor(p))
}

/**
 * The state and the logic of the conversation screen (S20) and its message bodies (S21), apart from drawing them. It reads through
 * [MailSession.reading] and acts through [MailSession.rowActions], so it knows nothing about the fake or the core.
 *
 * - **Opening**: the conversation (oldest first) comes from the database, offline. The opened message and the newest are expanded and
 *   their bodies are loaded; the others are collapsed to sender, date and snippet until the person expands them.
 * - **Bodies**: [BodyPhase] per message. A body that is not stored while offline is [ReadProblem.OFFLINE]: the screen says it will load
 *   when the person is online, and the controller tries again by itself when the account's status changes.
 * - **Mark read**: [markReadSoon] after the S49 delay counted from when the body is shown (immediately, 2 s, never), with `markSeen`.
 *   Loading a body never marks it read. After Mark as unread nothing is marked again.
 * - **Actions**: archive, delete, move, spam and mark unread run through the same row actions as the Inbox, on the messages of the
 *   conversation that are in the folder it was opened from. After a move the screen closes (`onClose`) and the Inbox shows the Undo.
 * - **Attachments**: [onChip] saves the attachment into [attachmentDir] (the core refuses a program or script), then opens it through
 *   [opener]; [clearSharedFiles] removes what was saved when the viewer returns and when the screen is left.
 * - **Links**: [onLink] only records the tapped link; the screen shows its real destination and [confirmLink] opens it.
 */
@Stable
class ConversationController(
    private val session: MailSession,
    private val scope: CoroutineScope,
    val openedId: MessageId,
    private val texts: InboxTexts,
    private val readingTexts: ReadingTexts,
    /** The folder the core saves opened attachments to; null when it is not available (opening then says so). */
    private val attachmentDir: File?,
) {
    /** The mail settings in force (S49), set by the screen on every composition. */
    var settings: MailSettings = MailSettings()

    /** Closes the screen; the notice, when there is one, is shown by the Inbox. Set by the screen. */
    var onClose: (ConversationNotice?) -> Unit = {}

    /** Opens compose (reply, reply all, forward, a `mailto:` link). Set by the screen. */
    var onCompose: (ComposeStart) -> Unit = {}

    /** Opens a saved attachment in another app. Set by the screen. */
    var opener: AttachmentOpener = AttachmentOpener { _, _ -> false }

    /** Opens a web link in the browser (a chooser). Set by the screen. */
    var browser: (String) -> Boolean = { false }

    /** The one snackbar of the screen. */
    val snackbar = CroftSnackbarState()

    var phase by mutableStateOf<ConversationPhase>(ConversationPhase.Loading)
        private set

    var conversation by mutableStateOf<ConversationData?>(null)
        private set

    /** The messages shown expanded. */
    var expanded by mutableStateOf<Set<MessageId>>(emptySet())
        private set

    /** The body of each expanded message that was asked for. */
    var bodies by mutableStateOf<Map<MessageId, BodyPhase>>(emptyMap())
        private set

    /** The state of each attachment chip that is not simply "not downloaded". */
    var chips by mutableStateOf<Map<ChipKey, ChipPhase>>(emptyMap())
        private set

    /** Messages whose "Remote images blocked" bar was dismissed. */
    var dismissedBars by mutableStateOf<Set<MessageId>>(emptySet())
        private set

    /** The folders of the account, for the folder chip, Move to and Report spam. */
    var folders by mutableStateOf<List<FolderInfo>>(emptyList())
        private set

    /** The account's own address, to leave it out when counting the people of a message. */
    var myAddress by mutableStateOf<String?>(null)
        private set

    /** A link was tapped and waits for the person's answer. */
    var pendingLink by mutableStateOf<LinkAction?>(null)
        private set

    var moveRequest by mutableStateOf<ConversationMove?>(null)
        private set

    var folderRequest by mutableStateOf<ConversationFolderRequest?>(null)
        private set

    /** Delete forever (Trash and Spam) asks first. */
    var confirmDeleteForever by mutableStateOf(false)
        private set

    private val readJobs = HashMap<MessageId, Job>()
    private val markedRead = HashSet<MessageId>()
    private var keepUnread = false
    private val chipJobs = HashMap<ChipKey, Job>()
    private val savedFiles = HashMap<ChipKey, File>()

    private val reading get() = session.reading

    // ---- Derived state ----

    /** The message the person opened. */
    val opened: ConversationMessageData? get() = conversation?.opened

    /** What this folder is for, from the folder the message is in. */
    val openedRole: FolderRole get() = opened?.folderRole ?: FolderRole.OTHER

    /** The folder the message is in, for the chip under the subject. */
    val openedFolder: FolderInfo? get() = opened?.let { m -> folders.firstOrNull { it.id == m.folder } }

    /** The message the reply bar answers: the one that was opened. */
    val replyTarget: ConversationMessageData? get() = opened

    /** Whether Reply all is offered for the reply bar. */
    val showReplyAll: Boolean get() = replyTarget?.let { ReadingRules.showReplyAll(it, myAddress) } ?: false

    // ---- Running ----

    /** Opens the conversation, loads the bodies it shows, then follows the repository's changes until cancelled. Call from a `LaunchedEffect`. */
    suspend fun run() = coroutineScope {
        val repo = reading
        if (repo == null) { phase = ConversationPhase.Failed(ReadProblem.INTERNAL); return@coroutineScope }
        val first = try {
            repo.openConversation(openedId)
        } catch (e: ReadFailure) {
            phase = ConversationPhase.Failed(e.problem)
            return@coroutineScope
        }
        conversation = first
        expanded = first.initiallyExpanded
        phase = ConversationPhase.Ready
        launch { loadFolders(first.account) }
        first.initiallyExpanded.forEach { id -> launch { loadBody(id) } }
        observeChanges(first.account)
    }

    private suspend fun loadFolders(account: AccountId) {
        try {
            folders = session.repository.listFolders(account)
            myAddress = session.repository.listAccounts().firstOrNull { it.id == account }?.email
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The chip and the folder-based actions wait; nothing else depends on this.
        }
    }

    private suspend fun observeChanges(account: AccountId) {
        var after = 0L
        while (true) {
            val batch = try {
                session.repository.nextChange(after)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                return
            }
            after = batch.seq
            if (batch.everything || account in batch.accounts || batch.folders.any { it.account == account }) {
                refresh()
                retryOffline()
            }
        }
    }

    /** Reads the conversation again (a reply arrived, a flag changed elsewhere). The expanded set and the bodies stay as they are. */
    private suspend fun refresh() {
        val repo = reading ?: return
        try {
            val fresh = repo.openConversation(openedId)
            conversation = fresh.copy(messages = fresh.messages.map { m -> if (m.id in markedRead) m.copy(seen = true) else m })
        } catch (e: ReadFailure) {
            // The message went away (moved or deleted elsewhere): what is shown stays until the person leaves.
        }
    }

    private fun retryOffline() {
        bodies.filter { (_, p) -> p is BodyPhase.Failed && p.problem == ReadProblem.OFFLINE }.keys.forEach { id -> scope.launch { loadBody(id) } }
    }

    // ---- Bodies ----

    /**
     * Loads the body of [id] with the S49 remote-images setting. [quiet]: keep showing the body that is there (Show images, Retry of a
     * reload) instead of the loading state.
     */
    suspend fun loadBody(id: MessageId, quiet: Boolean = false) {
        val repo = reading ?: return
        if (!quiet || bodies[id] == null) bodies = bodies + (id to BodyPhase.Loading)
        try {
            val body = repo.loadBody(id, settings.remoteImages)
            bodies = bodies + (id to BodyPhase.Ready(body))
            markReadSoon(id)
        } catch (e: CancellationException) {
            throw e
        } catch (e: ReadFailure) {
            bodies = bodies + (id to BodyPhase.Failed(e.problem))
        }
    }

    /** Retry of a body that did not load. */
    fun retry(id: MessageId) { scope.launch { loadBody(id) } }

    /** Expands or collapses [id]; an expanded message whose body was not asked for yet loads it. */
    fun toggle(id: MessageId) {
        if (id in expanded) {
            expanded = expanded - id
        } else {
            expanded = expanded + id
            if (bodies[id] == null) scope.launch { loadBody(id) }
        }
    }

    // ---- Mark read ----

    /**
     * Marks [id] read after the S49 delay (immediately, 2 s, never), counted from now: the body is on the screen. Not when it is read
     * already, when the person chose Mark as unread, or when the delay is Never. The job ends with the screen: a message left before the
     * time is not marked.
     */
    private fun markReadSoon(id: MessageId) {
        val message = conversation?.messages?.firstOrNull { it.id == id } ?: return
        if (message.seen || id in markedRead || keepUnread) return
        val wait = settings.markRead.delayMs ?: return
        if (readJobs[id]?.isActive == true) return
        readJobs[id] = scope.launch {
            if (wait > 0) delay(wait)
            if (keepUnread) return@launch
            try {
                session.rowActions.markSeen(ReadingRules.rowOf(message, conversation?.localOnly ?: false), true)
                markedRead += id
                setSeen(id, true)
            } catch (e: MailException) {
                // Quiet: a message that could not be marked is simply still unread in the list.
            }
        }
    }

    private fun setSeen(id: MessageId, seen: Boolean) {
        val c = conversation ?: return
        conversation = c.copy(messages = c.messages.map { if (it.id == id) it.copy(seen = seen) else it })
    }

    // ---- Actions of the action bar and the menu ----

    private enum class Kind { ARCHIVE, TRASH, SPAM, NOT_SPAM }

    private fun targetRows() = conversation?.let { c -> ReadingRules.targets(c).map { ReadingRules.rowOf(it, c.localOnly) } }.orEmpty()

    fun archive() = act(Kind.ARCHIVE)

    /** Delete: to Trash with Undo; in Trash and Spam it is Delete forever and asks first. */
    fun delete() {
        if (openedRole == FolderRole.TRASH || openedRole == FolderRole.SPAM) confirmDeleteForever = true else act(Kind.TRASH)
    }

    fun reportSpam() = act(if (openedRole == FolderRole.SPAM) Kind.NOT_SPAM else Kind.SPAM)

    fun dismissDeleteForever() { confirmDeleteForever = false }

    fun deleteForever() {
        confirmDeleteForever = false
        val rows = targetRows()
        scope.launch {
            try {
                rows.forEach { session.rowActions.deleteForever(it) }
                onClose(ConversationNotice(texts.deletedForever(rows.size), null))
            } catch (e: MailException) {
                snackbar.show(errorText(e.error))
            }
        }
    }

    private fun act(kind: Kind) {
        val rows = targetRows()
        if (rows.isEmpty()) return
        scope.launch {
            try {
                val actions = session.rowActions
                val (ops, text) = when (kind) {
                    Kind.ARCHIVE -> rows.flatMap { actions.archive(it) } to texts.archived(rows.size)
                    Kind.TRASH -> rows.flatMap { actions.trash(it) } to texts.deleted(rows.size)
                    Kind.SPAM -> rows.flatMap { actions.markSpam(it) } to texts.spammed(rows.size)
                    Kind.NOT_SPAM -> {
                        val inbox = folders.firstOrNull { it.role == FolderRole.INBOX }?.id ?: throw MailException(MailError.NOT_FOUND)
                        rows.flatMap { actions.moveTo(it, inbox) } to texts.moved(rows.size)
                    }
                }
                onClose(ConversationNotice(text, ops))
            } catch (e: MailException) {
                if (!askForFolder(e) { act(kind) }) snackbar.show(errorText(e.error))
            }
        }
    }

    /** Mark as unread: the opened message goes back to unread, nothing marks it read again, and the screen closes (back to the list). */
    fun markUnread() {
        val target = opened ?: return
        keepUnread = true
        readJobs.values.forEach { it.cancel() }
        scope.launch {
            try {
                session.rowActions.markSeen(ReadingRules.rowOf(target, conversation?.localOnly ?: false), false)
                onClose(null)
            } catch (e: MailException) {
                keepUnread = false
                snackbar.show(errorText(e.error))
            }
        }
    }

    /** Star or unstar the opened message. */
    fun toggleStar() {
        val target = opened ?: return
        val now = !target.flagged
        scope.launch {
            try {
                session.rowActions.setFlagged(ReadingRules.rowOf(target, conversation?.localOnly ?: false), now)
                val c = conversation ?: return@launch
                conversation = c.copy(messages = c.messages.map { if (it.id == target.id) it.copy(flagged = now) else it })
            } catch (e: MailException) {
                snackbar.show(errorText(e.error))
            }
        }
    }

    // ---- Move to ----

    fun openMove() {
        val c = conversation ?: return
        val folder = opened?.folder ?: return
        moveRequest = ConversationMove(c.account, setOf(folder))
    }

    fun dismissMove() { moveRequest = null }

    /** The folders the Move to sheet lists: those of the account. */
    val moveFolders: List<FolderInfo> get() = moveRequest?.let { r -> folders.filter { it.account == r.account } }.orEmpty()

    val moveRecents: List<Long> get() = moveRequest?.let { session.folderMapping.recents(it.account) }.orEmpty()

    fun pickMove(folder: FolderInfo) {
        moveRequest = null
        val rows = targetRows()
        scope.launch {
            try {
                val ops = rows.flatMap { session.rowActions.moveTo(it, folder.id) }
                onClose(ConversationNotice(texts.moved(rows.size), ops))
            } catch (e: MailException) {
                snackbar.show(errorText(e.error))
            }
        }
    }

    // ---- The folder question ----

    private fun errorText(error: MailError): String = when (error) {
        MailError.NO_ARCHIVE_FOLDER -> texts.noArchive
        MailError.NO_TRASH_FOLDER -> texts.noTrash
        MailError.NO_SPAM_FOLDER -> texts.noSpam
        MailError.NOT_FOUND, MailError.FAILED -> texts.failed
    }

    private fun askForFolder(e: MailException, retry: suspend () -> Unit): Boolean {
        val account = e.account ?: return false
        val role = when (e.error) {
            MailError.NO_ARCHIVE_FOLDER -> FolderRole.ARCHIVE
            MailError.NO_TRASH_FOLDER -> FolderRole.TRASH
            else -> return false
        }
        if (session.folderMapping.isDeclined(account, role)) return false
        folderRequest = ConversationFolderRequest(role, account, retry)
        return true
    }

    fun answerFolderRequest(folder: FolderInfo) {
        val request = folderRequest ?: return
        folderRequest = null
        session.folderMapping.remember(request.account, request.role, folder.id)
        scope.launch { request.retry() }
    }

    fun declineFolderRequest() {
        val request = folderRequest ?: return
        folderRequest = null
        session.folderMapping.decline(request.account, request.role)
        snackbar.show(if (request.role == FolderRole.TRASH) texts.noTrash else texts.noArchive)
    }

    fun dismissFolderRequest() {
        val request = folderRequest ?: return
        folderRequest = null
        snackbar.show(if (request.role == FolderRole.TRASH) texts.noTrash else texts.noArchive)
    }

    // ---- Reply bar ----

    fun reply(to: MessageId = openedId) { conversation?.let { onCompose(ComposeStart.Reply(it.account, to)) } }

    fun replyAll(to: MessageId = openedId) { conversation?.let { onCompose(ComposeStart.ReplyAll(it.account, to)) } }

    fun forward(to: MessageId = openedId) { conversation?.let { onCompose(ComposeStart.Forward(it.account, to)) } }

    // ---- Remote images ----

    /** "Show images": the person allows remote content on this one message, and its body loads again. */
    fun showImages(id: MessageId) {
        scope.launch {
            try {
                reading?.allowRemoteContent(id)
                loadBody(id, quiet = true)
            } catch (e: ReadFailure) {
                snackbar.show(readingTexts.problem(e.problem))
            }
        }
    }

    /** "Always for this sender": the sender joins the known senders and the body loads again. Only offered under Known senders. */
    fun alwaysForSender(id: MessageId) {
        val message = conversation?.messages?.firstOrNull { it.id == id } ?: return
        val address = message.senderAddress ?: return
        scope.launch {
            try {
                reading?.trustSender(message.account, address)
                loadBody(id, quiet = true)
                snackbar.show(readingTexts.trustedSender)
            } catch (e: ReadFailure) {
                snackbar.show(readingTexts.problem(e.problem))
            }
        }
    }

    fun dismissBar(id: MessageId) { dismissedBars = dismissedBars + id }

    // ---- Links ----

    /** A link was tapped in a message: [LinkAction.Ignore] does nothing; the others wait for the person's answer. */
    fun onLink(action: LinkAction) { if (action !is LinkAction.Ignore) pendingLink = action }

    fun cancelLink() { pendingLink = null }

    /** The person confirmed: a web link opens in the browser chooser, a `mailto:` link opens Croft's own compose. */
    fun confirmLink() {
        val link = pendingLink ?: return
        pendingLink = null
        when (link) {
            is LinkAction.Web -> if (!browser(link.url)) snackbar.show(readingTexts.noApp)
            is LinkAction.Mail -> Mailto.parse(link.url)?.let { fill -> conversation?.let { onCompose(ComposeStart.Fill(fill, it.account)) } }
            LinkAction.Ignore -> Unit
        }
    }

    // ---- Attachments ----

    /** The state a chip shows. */
    fun chipPhase(key: ChipKey): ChipPhase = chips[key] ?: ChipPhase.NOT_DOWNLOADED

    /**
     * A tap on a chip. A blocked attachment (a program or script) is never saved or opened: the tap only says so. Otherwise the file is
     * saved into the shared folder and opened in another app; a tap while it downloads cancels, a tap on a failed one tries again.
     */
    fun onChip(message: MessageId, attachment: AttachmentData) {
        val key = ChipKey(message, attachment.id)
        if (attachment.blocked) { snackbar.show(readingTexts.blockedAttachment); return }
        when (chipPhase(key)) {
            ChipPhase.DOWNLOADING -> { chipJobs.remove(key)?.cancel(); setChip(key, ChipPhase.NOT_DOWNLOADED) }
            ChipPhase.DOWNLOADED -> {
                val file = savedFiles[key]
                if (file != null && file.isFile) openFile(file, attachment) else download(key, attachment)
            }
            ChipPhase.NOT_DOWNLOADED, ChipPhase.FAILED -> download(key, attachment)
        }
    }

    private fun download(key: ChipKey, attachment: AttachmentData) {
        val repo = reading ?: return
        val dir = attachmentDir
        if (dir == null || !(dir.isDirectory || dir.mkdirs())) {
            setChip(key, ChipPhase.FAILED)
            snackbar.show(readingTexts.problem(ReadProblem.BAD_DESTINATION))
            return
        }
        setChip(key, ChipPhase.DOWNLOADING)
        chipJobs[key] = scope.launch {
            try {
                val file = repo.saveAttachment(key.messageId, key.attachmentId, dir)
                savedFiles[key] = file
                setChip(key, ChipPhase.DOWNLOADED)
                openFile(file, attachment)
            } catch (e: CancellationException) {
                throw e
            } catch (e: ReadFailure) {
                setChip(key, ChipPhase.FAILED)
                snackbar.show(readingTexts.problem(e.problem))
            } finally {
                chipJobs.remove(key)
            }
        }
    }

    private fun openFile(file: File, attachment: AttachmentData) {
        if (!opener.open(file, attachment.mimeType)) snackbar.show(readingTexts.noApp)
    }

    private fun setChip(key: ChipKey, phase: ChipPhase) {
        chips = if (phase == ChipPhase.NOT_DOWNLOADED) chips - key else chips + (key to phase)
    }

    /** The viewer returned, or the screen is left: what was saved for opening is deleted and the chips are "not downloaded" again. */
    fun clearSharedFiles() {
        chipJobs.values.forEach { it.cancel() }
        chipJobs.clear()
        savedFiles.values.forEach { try { it.delete() } catch (e: SecurityException) { /* the folder is emptied at the next start */ } }
        savedFiles.clear()
        chips = emptyMap()
    }

    /** The conversation screen is left: nothing that was saved stays, and the mark-read waits are over. */
    fun leave() {
        readJobs.values.forEach { it.cancel() }
        clearSharedFiles()
    }

    /** Whether the person's setting allows the "Always for this sender" button. */
    val offersTrustSender: Boolean get() = settings.remoteImages == RemoteImages.KNOWN_SENDERS
}
