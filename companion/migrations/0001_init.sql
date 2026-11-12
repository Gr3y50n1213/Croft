-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T23: the Companion's first schema. Forward-only: never edit a released migration.

-- Global settings of the instance (the master key check value). No user data.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value BLOB NOT NULL
);

-- The people of this Companion. `name` is a login name, not an address.
CREATE TABLE user (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL UNIQUE,
    role       TEXT    NOT NULL CHECK (role IN ('admin', 'user')),
    created_ms INTEGER NOT NULL,
    disabled   INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1))
);

-- Each user's data key, only ever wrapped by the master key (envelope encryption).
CREATE TABLE user_key (
    user_id     INTEGER PRIMARY KEY REFERENCES user(id) ON DELETE CASCADE,
    wrapped     BLOB    NOT NULL,
    key_version INTEGER NOT NULL
);

-- What happened, for the admin. Never a secret, an address or a message.
CREATE TABLE audit_log (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms   INTEGER NOT NULL,
    user_id INTEGER REFERENCES user(id) ON DELETE SET NULL,
    action  TEXT    NOT NULL,
    outcome TEXT    NOT NULL
);
CREATE INDEX audit_log_user ON audit_log(user_id, at_ms);
