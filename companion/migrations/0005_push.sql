-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T28: push subscriptions. Each belongs to a user and to the device (session) that made it; its
-- endpoint and keys are sealed with the user's data key (the endpoint is a capability: whoever has
-- it can wake the device); its duplicate key is an HMAC of it under the user's data key. Forward-only.

CREATE TABLE push_subscription (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    session_id    INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    sealed        BLOB    NOT NULL,
    endpoint_hash BLOB    NOT NULL,
    created_ms    INTEGER NOT NULL,
    failures      INTEGER NOT NULL DEFAULT 0,
    failing_since_ms INTEGER,
    UNIQUE (user_id, endpoint_hash)
);
CREATE INDEX push_subscription_user ON push_subscription(user_id);
CREATE INDEX push_subscription_session ON push_subscription(session_id);

-- A device that signs out (or is signed out) keeps no subscription, however it was revoked.
CREATE TRIGGER push_subscription_follows_session
AFTER UPDATE OF revoked_ms ON session
WHEN NEW.revoked_ms IS NOT NULL
BEGIN
    DELETE FROM push_subscription WHERE session_id = NEW.id;
END;
