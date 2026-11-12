// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import com.example.croft.mail.compose.ComposeFill
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.Mailto

/**
 * The system intents compose handles (the manifest declares exactly these): `ACTION_SENDTO` and `ACTION_VIEW` with a `mailto:` URI,
 * `ACTION_SEND` and `ACTION_SEND_MULTIPLE` with text, a subject, recipients and `content:` streams. Anything else is not ours.
 *
 * **[parse] never throws.** The intent comes from another app, and its extras are theirs: a null where a Uri belongs, an `Intent` in a list
 * of Uris, an array with a null entry, or (on API 26 to 32, where the first read unparcels every extra) a class this app does not have
 * (`BadParcelableException`). Each extra is read on its own, keeping only what has the right type; if reading fails anyway, compose opens
 * empty. A hostile intent can at worst open an empty compose.
 *
 * - A `mailto:` link is read by [Mailto] (the `attach` field is never honoured).
 * - Streams are taken only when they are `content:` URIs of **another app's** provider ([ComposeFiles.isForeign], when a [Context] is
 *   given; [ComposeFiles.resolve] checks again when it copies): a `file:` stream, or one of Croft's own providers, is dropped.
 * - Recipients are capped at [Mailto.MAX_RECIPIENTS] and texts at [Mailto.MAX_BODY]; when something was cut, [ComposeFill.truncated] says so.
 * - Nothing here logs an address, a subject or a text.
 */
object ComposeIntents {
    private const val MAX_STREAMS = 20

    /** The compose start for [intent], or null when it is not one of ours. */
    fun parse(intent: Intent?, context: Context? = null): ComposeStart? {
        val action = try { intent?.action } catch (e: RuntimeException) { return null }
        if (intent == null || action == null) return null
        if (action != Intent.ACTION_SENDTO && action != Intent.ACTION_VIEW && action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return null
        return try {
            parseOurs(intent, action, context)
        } catch (e: RuntimeException) {
            // Whatever it was (a bad parcel, a wrong type): an empty compose, never a crash.
            ComposeStart.Fill(ComposeFill())
        }
    }

    private fun parseOurs(intent: Intent, action: String, context: Context?): ComposeStart? {
        if (action == Intent.ACTION_SENDTO || action == Intent.ACTION_VIEW) {
            val data = intent.data ?: return null
            if (!data.scheme.equals("mailto", ignoreCase = true)) return null
            val fromLink = Mailto.parse(data.toString()) ?: return null
            // The extras of a SENDTO may add to the link (a subject and a text from the sending app); a VIEW takes only the link.
            return ComposeStart.Fill(if (action == Intent.ACTION_SENDTO) fromLink.merge(extrasOf(intent, context)) else fromLink)
        }
        return ComposeStart.Fill(extrasOf(intent, context))
    }

    private fun extrasOf(intent: Intent, context: Context?): ComposeFill {
        val extras: Bundle? = try { intent.extras } catch (e: RuntimeException) { null }
        if (extras == null) return ComposeFill(attachments = streamsOf(intent, null, context))
        val rawSubject = text(extras, Intent.EXTRA_SUBJECT)?.replace('\r', ' ')?.replace('\n', ' ').orEmpty()
        val rawText = text(extras, Intent.EXTRA_TEXT).orEmpty()
        val subject = rawSubject.take(Mailto.MAX_FIELD)
        val body = rawText.take(Mailto.MAX_BODY)
        return ComposeFill(
            to = strings(extras, Intent.EXTRA_EMAIL), cc = strings(extras, Intent.EXTRA_CC), bcc = strings(extras, Intent.EXTRA_BCC),
            subject = subject, body = body, attachments = streamsOf(intent, extras, context),
            truncated = rawSubject.length > Mailto.MAX_FIELD || rawText.length > Mailto.MAX_BODY,
        )
    }

    /** A CharSequence extra as text, or null when it is missing or of another type. */
    @Suppress("DEPRECATION")
    private fun text(extras: Bundle, key: String): String? = try { (extras.get(key) as? CharSequence)?.toString() } catch (e: RuntimeException) { null }

    /** A String array (or a single String, which some apps send) of addresses: null entries and other types are dropped, the list is capped. */
    @Suppress("DEPRECATION")
    private fun strings(extras: Bundle, key: String): List<String> = try {
        when (val v = extras.get(key)) {
            is String -> listOf(v)
            is Array<*> -> v.filterIsInstance<String>()
            is Collection<*> -> v.filterIsInstance<String>()
            else -> emptyList()
        }.filter { it.isNotBlank() }.take(Mailto.MAX_RECIPIENTS)
    } catch (e: RuntimeException) {
        emptyList()
    }

    /** The `content:` streams of another app's providers, from `EXTRA_STREAM` (one Uri or a list: only Uri items count) and the clip. */
    @Suppress("DEPRECATION")
    private fun streamsOf(intent: Intent, extras: Bundle?, context: Context?): List<String> {
        val uris = LinkedHashSet<Uri>()
        if (extras != null) {
            try {
                when (val v = extras.get(Intent.EXTRA_STREAM)) {
                    is Uri -> uris += v
                    is Collection<*> -> v.forEach { if (it is Uri) uris += it }
                }
            } catch (e: RuntimeException) {
                // A malformed extra from another app: no files from it.
            }
        }
        try {
            val clip: ClipData? = intent.clipData
            if (clip != null) for (i in 0 until clip.itemCount) clip.getItemAt(i)?.uri?.let { uris += it }
        } catch (e: RuntimeException) {
            // Same.
        }
        return uris
            .filter { it.scheme.equals("content", ignoreCase = true) }
            .filter { context == null || ComposeFiles.isForeign(context, it) }
            .map { it.toString() }
            .take(MAX_STREAMS)
    }

    private fun ComposeFill.merge(other: ComposeFill) = ComposeFill(
        to = (to + other.to).take(Mailto.MAX_RECIPIENTS), cc = (cc + other.cc).take(Mailto.MAX_RECIPIENTS), bcc = (bcc + other.bcc).take(Mailto.MAX_RECIPIENTS),
        subject = subject.ifEmpty { other.subject }, body = body.ifEmpty { other.body }, attachments = attachments + other.attachments,
        truncated = truncated || other.truncated,
    )
}
