// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import com.example.croft.mail.companion.CompanionRuntime

/**
 * The end of signing in to a Companion through the browser (T34b, S06): the Companion's page sends the browser to
 * `<applicationId>:/companion?code=...`. This hands the address to the connection (which ignores it unless a sign-in the person
 * started is waiting, and redeems the code only with the PKCE verifier the core keeps) and goes back to the app, closing the
 * browser tab above it. Nothing is shown and nothing else is read from the intent.
 */
class CompanionRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uri = intent?.data?.toString()
        if (uri != null && uri.length <= 2048) CompanionRuntime.redirected(applicationContext, uri)
        startActivity(
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        )
        finish()
    }
}
