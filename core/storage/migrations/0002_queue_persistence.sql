-- While a move or delete is waiting (Undo window, or not yet confirmed), the message is hidden from
-- lists but kept. Its row is deleted only once the server confirms it is gone (never delete locally first).
ALTER TABLE message ADD COLUMN local_hidden INTEGER NOT NULL DEFAULT 0;

-- Messages that were moved on the server but whose new UID is not known yet (no UIDPLUS, or the
-- server's answer did not include it). Later operations on the old address wait until the engine
-- finds the new UID by Message-ID (then repoints them) or gives up (then they fail).
CREATE TABLE op_awaiting_uid (
    account_id      INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    folder          TEXT    NOT NULL,     -- old folder path
    uid             INTEGER NOT NULL,     -- old UID
    dest_folder     TEXT    NOT NULL,     -- where it was moved to
    message_id_hdr  TEXT,                 -- to find it again; NULL means it cannot be found
    lookups         INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (account_id, folder, uid)
);
