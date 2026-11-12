-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T28: push subscriptions. Each belongs to a user and to the device (session) that made it; its
-- endpoint and keys are sealed with the user's data key (the endpoint is a capability: whoever has
-- it can wake the device). Forward-only.

CREATE TABLE push_subscription (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    session_id    INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    sealed        BLOB    NOT NULL,
    endpoint_hash BLOB    NOT NULL,
    created_ms    INTEGER NOT NULL,
    failures      INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, endpoint_hash)
);
CREATE INDEX push_subscription_user ON push_subscription(user_id);
