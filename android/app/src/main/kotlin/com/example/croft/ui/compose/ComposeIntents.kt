// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.ClipData
import android.content.Intent
import android.net.Uri
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.Mailto

/**
 * The system intents compose handles (the manifest declares exactly these): `ACTION_SENDTO` and `ACTION_VIEW` with a `mailto:` URI,
 * `ACTION_SEND` and `ACTION_SEND_MULTIPLE` with text, a subject, recipients and `content:` streams. Anything else is not ours.
 *
 * - A `mailto:` link is read by [Mailto] (the `attach` field is never honoured).
 * - Streams are taken only when they are `content:` URIs ([ComposeFiles.resolve] checks again when it copies); a `file:` stream is dropped.
 * - Nothing here logs an address, a subject or a text.
 */
object ComposeIntents {
    private const val MAX_STREAMS = 20
    private const val MAX_TEXT = 20_000

    /** The compose start for [intent], or null when it is not one of ours. */
    fun parse(intent: Intent?): ComposeStart? {
        intent ?: return null
        return when (intent.action) {
            Intent.ACTION_SENDTO, Intent.ACTION_VIEW -> {
                val data = intent.data ?: return null
                if (!data.scheme.equals("mailto", ignoreCase = true)) return null
                val fromLink = Mailto.parse(data.toString()) ?: return null
                // The extras of a SENDTO may add to the link (a subject and a text from the sending app).
                ComposeStart.Fill(fromLink.merge(extrasOf(intent, withStreams = intent.action == Intent.ACTION_SENDTO)))
            }
            Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE -> ComposeStart.Fill(extrasOf(intent, withStreams = true))
            else -> null
        }
    }

    private fun extrasOf(intent: Intent, withStreams: Boolean): ComposeFill {
        val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT).orEmpty().replace('\r', ' ').replace('\n', ' ').take(MAX_TEXT)
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty().take(MAX_TEXT)
        val streams = if (withStreams) streamsOf(intent) else emptyList()
        return ComposeFill(
            to = intent.getStringArrayExtra(Intent.EXTRA_EMAIL)?.toList().orEmpty(),
            cc = intent.getStringArrayExtra(Intent.EXTRA_CC)?.toList().orEmpty(),
            bcc = intent.getStringArrayExtra(Intent.EXTRA_BCC)?.toList().orEmpty(),
            subject = subject, body = text, attachments = streams,
        )
    }

    @Suppress("DEPRECATION")
    private fun streamsOf(intent: Intent): List<String> {
        val uris = LinkedHashSet<Uri>()
        try {
            intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris += it }
            intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris += it }
        } catch (e: RuntimeException) {
            // A malformed extra from another app: no files.
        }
        val clip: ClipData? = intent.clipData
        if (clip != null) for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { uris += it }
        return uris.filter { it.scheme.equals("content", ignoreCase = true) }.map { it.toString() }.take(MAX_STREAMS)
    }

    private fun ComposeFill.merge(other: ComposeFill) = ComposeFill(
        to = to + other.to, cc = cc + other.cc, bcc = bcc + other.bcc,
        subject = subject.ifEmpty { other.subject }, body = body.ifEmpty { other.body }, attachments = attachments + other.attachments,
    )
}
