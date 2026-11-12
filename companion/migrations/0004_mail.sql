-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T26: the credentials of a user's mail accounts, sealed with the user's data key (bound to the user
-- and the account in the associated data). The accounts themselves live in the user's own mail
-- database (the core's, SQLCipher). Forward-only.

CREATE TABLE mail_credential (
    user_id       INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    account_id    INTEGER NOT NULL,
    sealed        BLOB    NOT NULL,
    needs_sign_in INTEGER NOT NULL DEFAULT 0 CHECK (needs_sign_in IN (0, 1)),
    updated_ms    INTEGER NOT NULL,
    PRIMARY KEY (user_id, account_id)
);
