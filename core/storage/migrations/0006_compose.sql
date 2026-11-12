-- Composing and sending (T18c): drafts, their attachments, the people the person has written to,
-- and the link from an Outbox row to the draft it was made from.
--
-- A draft is stored on the device first and copied to the server's Drafts folder later. It keeps
-- one Message-ID for its whole life, so the copy on the server can be recognised and replaced.
--   state    draft   being written; shown in the Drafts list
--            queued  Send was tapped; the Outbox row with draft_id = this id owns it
--            done    sent or discarded; kept (and hidden) only until its copy on the server is
--                    deleted, then the row is deleted too
--   revision goes up with every change the person makes; synced_revision is the revision the server
--            copy has. They differ when a copy is still to be made (offline, or a pause).
--   stale_message_ids are earlier Message-IDs of this draft (a draft taken back from a message that
--            may have been sent gets a new one, so the corrected message is not mistaken for the
--            old one); their copies on the server are deleted at the next copy.
--   outbox_id is a hint written after the Outbox row (which carries draft_id itself, so the two can
--            never be separated by a crash). It lets the core tell "sent" from "taken back".
CREATE TABLE draft (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    identity_id     INTEGER REFERENCES identity (id) ON DELETE SET NULL,
    message_id      TEXT    NOT NULL,
    to_json         TEXT    NOT NULL DEFAULT '[]',
    cc_json         TEXT    NOT NULL DEFAULT '[]',
    bcc_json        TEXT    NOT NULL DEFAULT '[]',
    subject         TEXT    NOT NULL DEFAULT '',
    body            TEXT    NOT NULL DEFAULT '',
    in_reply_to     TEXT,
    references_json TEXT    NOT NULL DEFAULT '[]',
    state           TEXT    NOT NULL DEFAULT 'draft' CHECK (state IN ('draft', 'queued', 'done')),
    outbox_id       INTEGER,
    revision        INTEGER NOT NULL DEFAULT 1,
    synced_revision INTEGER NOT NULL DEFAULT 0,
    server_folder   TEXT,
    server_uid      INTEGER,
    stale_message_ids TEXT  NOT NULL DEFAULT '[]',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX draft_account ON draft (account_id, state);

CREATE TABLE draft_attachment (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    draft_id    INTEGER NOT NULL REFERENCES draft (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    mime_type   TEXT    NOT NULL,
    size        INTEGER NOT NULL,
    data        BLOB    NOT NULL
);
CREATE INDEX draft_attachment_draft ON draft_attachment (account_id, draft_id);

-- Addresses the person has sent mail to, for the suggestions in the recipient fields. This is the
-- only source of suggestions (no device contacts). `address` is lower case.
CREATE TABLE sent_recipient (
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    address     TEXT    NOT NULL,
    name        TEXT,
    last_used   INTEGER NOT NULL,
    uses        INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (account_id, address)
) WITHOUT ROWID;

-- The draft an Outbox message was made from (NULL for a message with no draft). Written in the
-- same statement as the row.
ALTER TABLE outbox ADD COLUMN draft_id INTEGER;
