-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T24: sign-in. Every token and code is stored only as a SHA-256 hash; the TOTP secret is sealed
-- with the user's data key. Forward-only.

-- One-time enrolment links (from `user add` or an admin's "re-enrol").
CREATE TABLE enrolment (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    token_hash BLOB    NOT NULL UNIQUE,
    expires_ms INTEGER NOT NULL,
    used_ms    INTEGER
);

-- Registered passkeys (WebAuthn credentials: public keys and counters, no secret).
CREATE TABLE passkey (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    credential_id BLOB    NOT NULL UNIQUE,
    credential    TEXT    NOT NULL,
    created_ms    INTEGER NOT NULL,
    last_used_ms  INTEGER
);

-- The TOTP secret (sealed with the user's data key) and the last step used (no code twice).
CREATE TABLE totp (
    user_id   INTEGER PRIMARY KEY REFERENCES user(id) ON DELETE CASCADE,
    secret    BLOB    NOT NULL,
    last_step INTEGER
);

-- Recovery codes, each usable once.
CREATE TABLE recovery_code (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    code_hash BLOB    NOT NULL,
    used_ms   INTEGER
);

-- A signed-in device. `family` groups its refresh tokens: reusing a rotated one revokes it.
CREATE TABLE session (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    device       TEXT    NOT NULL,
    created_ms   INTEGER NOT NULL,
    last_seen_ms INTEGER NOT NULL,
    revoked_ms   INTEGER,
    -- Signed in with TOTP and a recovery code: may only register a new passkey.
    recovery     INTEGER NOT NULL DEFAULT 0 CHECK (recovery IN (0, 1))
);

CREATE TABLE access_token (
    token_hash BLOB    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    expires_ms INTEGER NOT NULL
);

CREATE TABLE refresh_token (
    token_hash BLOB    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    expires_ms INTEGER NOT NULL,
    rotated_ms INTEGER
);

-- Failed sign-ins, for the lockout (kept 15 minutes).
CREATE TABLE sign_in_failure (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    at_ms   INTEGER NOT NULL
);
CREATE INDEX sign_in_failure_user ON sign_in_failure(user_id, at_ms);
