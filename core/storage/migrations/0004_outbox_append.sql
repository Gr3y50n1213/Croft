-- Outbox, part two (T06). Version 1 of the table could not hold what sending needs:
--   * the envelope (who the server should deliver to). The recipients are not always in the
--     message headers (Bcc), and the headers are not parsed to find them. `recipients` is a JSON
--     array of bare addresses, never a delimited string, so one address can never become two.
--   * the two states after the server has accepted the message: `submitted` (accepted, the copy in
--     the Sent folder is still to be saved) and `filing` (the copy is being appended). A crash
--     between the two steps must neither send the message again nor forget the copy.
--   * ids that are never reused (AUTOINCREMENT), so an Undo button that is still on screen can
--     never act on a newer message.
-- SQLite cannot change a CHECK constraint, so the table is rebuilt. Nothing refers to it.
-- `send_at` keeps its meaning and also serves as the retry time after a temporary failure.
CREATE TABLE outbox_new (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    identity_id   INTEGER REFERENCES identity (id) ON DELETE SET NULL,
    mime_blob     BLOB    NOT NULL,
    state         TEXT    NOT NULL DEFAULT 'queued'
                  CHECK (state IN ('queued', 'sending', 'submitted', 'filing', 'sent', 'failed')),
    send_at       INTEGER NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    envelope_from TEXT    NOT NULL DEFAULT '',
    recipients    TEXT    NOT NULL DEFAULT '[]'
);

INSERT INTO outbox_new (id, account_id, identity_id, mime_blob, state, send_at, attempts, last_error)
SELECT id, account_id, identity_id, mime_blob, state, send_at, attempts, last_error FROM outbox;

DROP TABLE outbox;
ALTER TABLE outbox_new RENAME TO outbox;

CREATE INDEX outbox_due ON outbox (state, send_at) WHERE state IN ('queued', 'submitted');
CREATE INDEX outbox_account ON outbox (account_id, state);
