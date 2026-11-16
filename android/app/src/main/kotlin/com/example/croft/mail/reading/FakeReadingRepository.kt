// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.AccountId
import com.example.croft.mail.MessageId
import com.example.croft.mail.calendar.ReplyWords
import com.example.croft.mail.calendar.RsvpChoice
import com.example.croft.mail.calendar.RsvpResult
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList

/**
 * A message of the fake: what the conversation lists, and the two bodies the core would give it ([blocked] with remote images left out,
 * [allowed] with them). All text is made up.
 */
class FakeMail(val message: ConversationMessageData, val blocked: BodyData, val allowed: BodyData = blocked)

/**
 * [ReadingRepository] for tests, previews and screenshots: made-up conversations, the core's decision about remote images re-done in
 * the same words (Never wins; Ask needs the person's tap; Known senders also takes the known senders list), saved attachments written
 * as small files, a record of every call, and failures a test can switch on. It keeps no real mail.
 */
class FakeReadingRepository(
    mail: List<FakeMail> = emptyList(),
    private val localOnly: Boolean = false,
) : ReadingRepository {
    private val mail = LinkedHashMap<MessageId, FakeMail>().also { m -> mail.forEach { m[it.message.id] = it } }
    private val allowedMessages = HashSet<MessageId>()
    private val trusted = HashMap<AccountId, java.util.TreeSet<String>>()

    /** Every call, in order, as `name:ids` (no content). */
    val calls = CopyOnWriteArrayList<String>()

    /** While set, [loadBody] fails with this problem (a test switches it on and off). */
    @Volatile var loadFails: ReadProblem? = null

    /** While set, [saveAttachment] fails with this problem. */
    @Volatile var saveFails: ReadProblem? = null

    /** How many times [loadBody] ran. */
    val loads: Int get() = calls.count { it.startsWith("loadBody") }

    /** Add or replace a message. */
    fun put(m: FakeMail) { mail[m.message.id] = m }

    override suspend fun openConversation(messageId: MessageId): ConversationData {
        calls += "openConversation:$messageId"
        val opened = mail[messageId] ?: throw ReadFailure(ReadProblem.NO_SUCH_MESSAGE)
        val same = mail.values.map { it.message }.filter { it.account == opened.message.account }.sortedWith(compareBy({ it.dateMs }, { it.id }))
        return ConversationData(opened.message.account, messageId, same, localOnly)
    }

    override suspend fun loadBody(messageId: MessageId, setting: RemoteImages): BodyData {
        calls += "loadBody:$messageId:${setting.key}"
        loadFails?.let { throw ReadFailure(it) }
        val m = mail[messageId] ?: throw ReadFailure(ReadProblem.NO_SUCH_MESSAGE)
        val known = m.message.senderAddress?.lowercase()?.let { it in trusted[m.message.account].orEmpty() } ?: false
        val allow = when (setting) {
            RemoteImages.NEVER -> false
            RemoteImages.ASK -> messageId in allowedMessages
            RemoteImages.KNOWN_SENDERS -> messageId in allowedMessages || known
        }
        return (if (allow) m.allowed else m.blocked).copy(senderKnown = known)
    }

    override suspend fun saveAttachment(messageId: MessageId, attachmentId: Long, destDir: File): File {
        calls += "saveAttachment:$messageId:$attachmentId"
        saveFails?.let { throw ReadFailure(it) }
        val attachment = mail[messageId]?.blocked?.attachments?.firstOrNull { it.id == attachmentId } ?: throw ReadFailure(ReadProblem.NO_SUCH_ATTACHMENT)
        if (attachment.blocked) throw ReadFailure(ReadProblem.ATTACHMENT_BLOCKED)
        val file = File(destDir, "attachment-$attachmentId.bin")
        file.writeBytes(ByteArray(16) { it.toByte() })
        return file
    }

    override suspend fun allowRemoteContent(messageId: MessageId) {
        calls += "allowRemoteContent:$messageId"
        if (messageId !in mail) throw ReadFailure(ReadProblem.NO_SUCH_MESSAGE)
        allowedMessages += messageId
    }

    override suspend fun trustSender(account: AccountId, address: String) {
        calls += "trustSender:$account"
        trusted.getOrPut(account) { java.util.TreeSet() } += address.trim().lowercase()
    }

    override suspend fun untrustSender(account: AccountId, address: String): Boolean {
        calls += "untrustSender:$account"
        return trusted[account]?.remove(address.trim().lowercase()) ?: false
    }

    override suspend fun trustedSenders(account: AccountId): List<String> = trusted[account]?.toList().orEmpty()

    // ---- T37: unsubscribe, set up by a test ----

    /** What [unsubscribeInfo] answers, per message. */
    val unsubscribeInfos = HashMap<MessageId, UnsubscribeInfoData>()

    /** What [unsubscribe] answers (by default: done). */
    @Volatile var unsubscribeResult: UnsubscribeResultData = UnsubscribeResultData(done = true, openUrl = null, problem = null, deletedOps = emptyList())

    override suspend fun unsubscribeInfo(messageId: MessageId): UnsubscribeInfoData? = unsubscribeInfos[messageId]

    override suspend fun unsubscribe(messageId: MessageId, shown: UnsubscribeInfoData, deleteOlder: Boolean): UnsubscribeResultData {
        calls += "unsubscribe:$messageId:$deleteOlder"
        val result = unsubscribeResult
        if (result.done) unsubscribeInfos[messageId]?.let { unsubscribeInfos[messageId] = it.copy(already = true) }
        return result
    }

    // ---- T40: invitations, set up by a test ----

    /** What [invitation] answers, per message. */
    val invitations = HashMap<MessageId, InvitationData>()

    /** What [respondToInvitation] answers (by default: sent, stored as event 900). */
    @Volatile var rsvpResult: RsvpResult = RsvpResult(sent = true, eventId = 900, problem = null)

    /** The calendars offered. */
    val calendarChoices = ArrayList<CalendarChoice>()

    /** The wording of the last answer. */
    @Volatile var lastWords: ReplyWords? = null

    override suspend fun invitation(messageId: MessageId, zone: String): InvitationData? = invitations[messageId]

    override suspend fun respondToInvitation(messageId: MessageId, answer: RsvpChoice, calendar: Long?, words: ReplyWords): RsvpResult {
        calls += "respond:$messageId:$answer:$calendar"
        lastWords = words
        val result = rsvpResult
        if (result.problem == null) invitations[messageId]?.let { invitations[messageId] = it.copy(myAnswer = answer, storedEvent = result.eventId) }
        return result
    }

    override suspend fun applyCancellation(messageId: MessageId): Boolean {
        calls += "cancel:$messageId"
        val can = invitations[messageId]?.canRemove == true
        if (can) invitations[messageId]?.let { invitations[messageId] = it.copy(canRemove = false, storedEvent = null) }
        return can
    }

    override suspend fun writableCalendars(account: AccountId): List<CalendarChoice> = calendarChoices.toList()
}
