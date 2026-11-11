// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import androidx.activity.ComponentActivity

/** The web addresses S60 and S01 open. Each is one constant, so changing one is a one-line edit. */
object AboutLinks {
    /**
     * **PLACEHOLDER, not a real address yet.** The privacy policy has to be hosted at a stable public address before the first store
     * upload (docs/store/READINESS.md row 8, docs/OPEN-ITEMS.md): replace this with that address. `example.org` is the project's placeholder
     * domain (see "Placeholders to replace" in docs/OPEN-ITEMS.md).
     */
    const val PRIVACY_POLICY_URL = "https://example.org/croft/privacy"

    /** The project's source code. */
    const val SOURCE_URL = "https://github.com/Gr3y50n1213/Croft"
}

/**
 * Opens [url] in the person's browser with a plain view intent: no permission, nothing is sent from Croft itself. Returns false when
 * no app can open it (the screen then says so).
 */
fun Context.openInBrowser(url: String): Boolean {
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addCategory(Intent.CATEGORY_BROWSABLE)
    // From a non-activity context (a test, a service) the intent needs its own task.
    if (generateSequence(this) { (it as? ContextWrapper)?.baseContext }.none { it is ComponentActivity }) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    return try {
        startActivity(intent)
        true
    } catch (e: ActivityNotFoundException) {
        false
    } catch (e: SecurityException) {
        false
    }
}

/** Puts [text] on the clipboard as plain text. */
fun Context.copyToClipboard(label: String, text: String) {
    val manager = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    manager.setPrimaryClip(ClipData.newPlainText(label, text))
}
