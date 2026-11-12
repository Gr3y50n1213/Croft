// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.annotation.StringRes
import com.example.croft.R
import com.example.croft.mail.reading.ReadProblem

/**
 * The plain-language text of every [ReadProblem] (the core's `ReadError`): what happened and, where it helps, what to do. No raw
 * codes, and never message text, an address or a file name. `ReadCopyTest` checks that every problem has a text.
 */
object ReadCopy {
    /** The string resource that says what [problem] means. */
    @StringRes
    fun resFor(problem: ReadProblem): Int = when (problem) {
        ReadProblem.NO_SUCH_MESSAGE -> R.string.read_error_no_such_message
        ReadProblem.NO_SUCH_ATTACHMENT -> R.string.read_error_no_such_attachment
        ReadProblem.NO_SUCH_ACCOUNT -> R.string.read_error_no_such_account
        ReadProblem.ACCOUNT_UNAVAILABLE -> R.string.read_error_account_unavailable
        ReadProblem.OFFLINE -> R.string.read_error_offline
        ReadProblem.SIGN_IN_NEEDED -> R.string.read_error_sign_in_needed
        ReadProblem.NOT_ON_SERVER -> R.string.read_error_not_on_server
        ReadProblem.TOO_LARGE -> R.string.read_error_too_large
        ReadProblem.UNREADABLE -> R.string.read_error_unreadable
        ReadProblem.SERVER_UNAVAILABLE -> R.string.read_error_server_unavailable
        ReadProblem.SERVER_REFUSED -> R.string.read_error_server_refused
        ReadProblem.ATTACHMENT_BLOCKED -> R.string.read_error_attachment_blocked
        ReadProblem.BAD_DESTINATION -> R.string.read_error_bad_destination
        ReadProblem.WRITE_FAILED -> R.string.read_error_write_failed
        ReadProblem.INVALID_ADDRESS -> R.string.read_error_invalid_address
        ReadProblem.STORAGE -> R.string.read_error_storage
        ReadProblem.INTERNAL -> R.string.read_error_internal
    }

    /** The name of the resource of [problem] (`read_error_<problem>`), for the test that reads the strings file. */
    fun resourceName(problem: ReadProblem): String = "read_error_" + problem.name.lowercase()
}
