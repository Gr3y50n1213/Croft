// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import uniffi.croft_ffi.DraftCheck
import uniffi.croft_ffi.DraftProblem

/**
 * The Send button's condition table (`docs/ux/controls.md`), row by row, from the facts of the core's `check_draft`. A disabled row
 * wins over an enabled one; among the enabled ones the first that applies is shown. Offline and "needs sign-in" never disable Send.
 */
object SendRules {
    /**
     * @param check the core's facts for the saved draft.
     * @param uploading a picked file is still being copied into the draft (Send waits for it, and never blocks).
     * @param encryptionWarning encryption is required and a recipient has no key ([EncryptionPolicy]).
     * @param offlineOrSignIn the account is offline or needs sign-in.
     */
    fun stateOf(check: DraftCheck, uploading: Boolean = false, encryptionWarning: Boolean = false, offlineOrSignIn: Boolean = false): SendState = when {
        DraftProblem.NO_RECIPIENT in check.problems -> SendState.NO_RECIPIENT
        DraftProblem.INVALID_RECIPIENT in check.problems -> SendState.INVALID_RECIPIENT
        DraftProblem.ATTACHMENTS_TOO_LARGE in check.problems -> SendState.OVER_SIZE_LIMIT
        uploading -> SendState.UPLOADING
        encryptionWarning -> SendState.ENCRYPTION_NO_KEY
        check.subjectEmpty && check.bodyEmpty -> SendState.EMPTY_CONFIRM
        offlineOrSignIn -> SendState.OFFLINE_OR_SIGN_IN
        else -> SendState.READY
    }

    /** Whether a tap on Send in [state] does nothing but explain. */
    fun isDisabled(state: SendState): Boolean = state == SendState.NO_RECIPIENT || state == SendState.INVALID_RECIPIENT || state == SendState.OVER_SIZE_LIMIT || state == SendState.SENDING

    /** The size limit in whole MB for the warning text. */
    fun limitMb(check: DraftCheck): Int = (check.sizeLimitBytes / (1024UL * 1024UL)).toInt().coerceAtLeast(1)
}
