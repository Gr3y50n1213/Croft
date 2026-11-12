-- Reading messages (T18a, migration 7): the fetched message itself, what the sanitiser counted, more about each
-- attachment, and the person's remote-content choices. Every new table carries account_id and goes
-- with its account; message_body and attachment stay keyed by message (scoped through it).

-- The whole message as the server sent it (at most 50 MiB, the protocol layer's limit). Croft
-- fetches a message once and keeps every part from this copy: attachments are saved from it,
-- "Show original" reads it, and the HTML is sanitised again from it when remote images are
-- allowed. It lives in the (encrypted) database, never in a plain file.
CREATE TABLE message_raw (
    message_id  INTEGER PRIMARY KEY REFERENCES message (id) ON DELETE CASCADE,
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    raw         BLOB    NOT NULL,
    fetched_at  INTEGER NOT NULL
);
CREATE INDEX message_raw_account ON message_raw (account_id);

-- Conversations follow Message-IDs as well as thread keys (a reply whose References were cut).
CREATE INDEX message_msgid ON message (account_id, message_id_hdr) WHERE message_id_hdr IS NOT NULL;

-- Remote images the stored html_clean left out (it is always the blocked version), and tracking
-- pixels removed, for the "Remote images blocked" bar.
ALTER TABLE message_body ADD COLUMN remote_blocked   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE message_body ADD COLUMN trackers_removed INTEGER NOT NULL DEFAULT 0;
-- 1 when a body was cut at a limit (too long, too many parts, HTML too deep).
ALTER TABLE message_body ADD COLUMN truncated        INTEGER NOT NULL DEFAULT 0;

-- attachment.part_id is the part's number in the stored message (croft-reader's numbering).
ALTER TABLE attachment ADD COLUMN content_id   TEXT;
-- 1 when the part is drawn inside the HTML body (a cid: image): no chip for it.
ALTER TABLE attachment ADD COLUMN shown_inline INTEGER NOT NULL DEFAULT 0;
-- 1 for a program or script: shown as blocked, never saved or opened.
ALTER TABLE attachment ADD COLUMN executable   INTEGER NOT NULL DEFAULT 0;

-- The person tapped "Show images" on this message.
ALTER TABLE message ADD COLUMN remote_allowed INTEGER NOT NULL DEFAULT 0;

-- Known senders: "Always show images from this sender", per account. address is lower case.
CREATE TABLE trusted_sender (
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    address     TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, address)
) WITHOUT ROWID;
