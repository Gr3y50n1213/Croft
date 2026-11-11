-- Croft local database, schema version 1.
-- Rules (see CLAUDE.md): every row belongs to an account; every query is scoped by account_id;
-- secrets (passwords, OAuth tokens, private keys) never live here, only in the platform keystore.
-- Timestamps are Unix milliseconds (INTEGER). Booleans are 0/1.

CREATE TABLE account (
    id            INTEGER PRIMARY KEY,
    label         TEXT    NOT NULL,
    email         TEXT    NOT NULL,
    protocol      TEXT    NOT NULL CHECK (protocol IN ('imap', 'jmap')),
    provider      TEXT    NOT NULL DEFAULT 'generic',      -- generic, gmail, outlook, icloud, ...
    config_json   TEXT    NOT NULL,                        -- hosts, ports, security, auth kind (no secrets)
    color         INTEGER,                                 -- ARGB, optional override
    position      INTEGER NOT NULL DEFAULT 0,              -- order in the account list
    status        TEXT    NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'syncing', 'needs_sign_in', 'offline', 'error')),
    created_at    INTEGER NOT NULL
);

CREATE TABLE identity (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    display_name  TEXT    NOT NULL,
    address       TEXT    NOT NULL,
    signature     TEXT,
    is_default    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, address)
);

CREATE TABLE folder (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    path            TEXT    NOT NULL,
    delimiter       TEXT,
    special_use     TEXT    NOT NULL DEFAULT 'other'
                    CHECK (special_use IN ('inbox', 'sent', 'drafts', 'trash', 'junk', 'archive', 'all', 'flagged', 'other')),
    uid_validity    INTEGER,
    uid_next        INTEGER,
    highest_modseq  INTEGER,
    unread_count    INTEGER NOT NULL DEFAULT 0,
    total_count     INTEGER NOT NULL DEFAULT 0,
    last_synced_at  INTEGER,
    UNIQUE (account_id, path)
);

CREATE TABLE message (
    id               INTEGER PRIMARY KEY,
    account_id       INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    folder_id        INTEGER NOT NULL REFERENCES folder (id) ON DELETE CASCADE,
    uid              INTEGER NOT NULL,
    message_id_hdr   TEXT,                                  -- RFC 5322 Message-ID
    thread_key       TEXT    NOT NULL,                      -- stable key from References / In-Reply-To / subject
    subject          TEXT    NOT NULL DEFAULT '',
    from_name        TEXT,
    from_addr        TEXT,
    to_json          TEXT    NOT NULL DEFAULT '[]',
    cc_json          TEXT    NOT NULL DEFAULT '[]',
    date_ts          INTEGER NOT NULL,
    size             INTEGER NOT NULL DEFAULT 0,
    snippet          TEXT    NOT NULL DEFAULT '',
    is_seen          INTEGER NOT NULL DEFAULT 0,
    is_flagged       INTEGER NOT NULL DEFAULT 0,
    is_answered      INTEGER NOT NULL DEFAULT 0,
    is_draft         INTEGER NOT NULL DEFAULT 0,
    keywords_json    TEXT    NOT NULL DEFAULT '[]',
    has_attachments  INTEGER NOT NULL DEFAULT 0,
    category         TEXT    NOT NULL DEFAULT 'primary'
                     CHECK (category IN ('primary', 'updates', 'promotions', 'social')),
    snoozed_until    INTEGER,                               -- hidden from the list until this time
    pending_ops      INTEGER NOT NULL DEFAULT 0,            -- >0 means a local change has not reached the server
    body_state       TEXT    NOT NULL DEFAULT 'none' CHECK (body_state IN ('none', 'partial', 'full')),
    UNIQUE (folder_id, uid)
);

-- The inbox list: newest first within a folder, hiding snoozed rows.
CREATE INDEX message_folder_date ON message (folder_id, date_ts DESC);
CREATE INDEX message_thread      ON message (account_id, thread_key, date_ts DESC);
CREATE INDEX message_unread      ON message (folder_id) WHERE is_seen = 0;
CREATE INDEX message_snoozed     ON message (snoozed_until) WHERE snoozed_until IS NOT NULL;

CREATE TABLE message_body (
    message_id    INTEGER PRIMARY KEY REFERENCES message (id) ON DELETE CASCADE,
    headers_raw   TEXT,
    text_plain    TEXT,
    html_clean    TEXT                                        -- already sanitised; remote content blocked
);

CREATE TABLE attachment (
    id            INTEGER PRIMARY KEY,
    message_id    INTEGER NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    part_id       TEXT    NOT NULL,                           -- IMAP part number or JMAP blobId
    filename      TEXT,
    mime_type     TEXT    NOT NULL,
    size          INTEGER NOT NULL DEFAULT 0,
    cached_path   TEXT,                                       -- null until downloaded
    UNIQUE (message_id, part_id)
);

-- Offline action queue. Every change is applied locally first, then confirmed by the server.
-- not_before implements the undo window: Undo cancels a row that has not started yet.
CREATE TABLE op_queue (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL,                           -- set_seen, set_flagged, move, delete, send, ...
    payload_json  TEXT    NOT NULL,
    target_key    TEXT,                                       -- message/folder the op acts on, used for coalescing
    state         TEXT    NOT NULL DEFAULT 'queued'
                  CHECK (state IN ('queued', 'applying', 'confirmed', 'failed', 'cancelled')),
    created_at    INTEGER NOT NULL,
    not_before    INTEGER NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT
);
CREATE INDEX op_queue_due ON op_queue (state, not_before) WHERE state IN ('queued', 'applying');

-- Outgoing mail shown in the Outbox screen. Drafts live in the server Drafts folder, not here.
CREATE TABLE outbox (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    identity_id   INTEGER REFERENCES identity (id) ON DELETE SET NULL,
    mime_blob     BLOB    NOT NULL,
    state         TEXT    NOT NULL DEFAULT 'queued'
                  CHECK (state IN ('queued', 'sending', 'sent', 'failed')),
    send_at       INTEGER NOT NULL,                           -- undo window or scheduled time
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT
);
CREATE INDEX outbox_due ON outbox (state, send_at) WHERE state IN ('queued', 'sending');

-- App settings stored as JSON documents: theme (croft-theme.json shape), swipe actions, notification options.
CREATE TABLE setting (
    key           TEXT PRIMARY KEY,
    value_json    TEXT NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- Full-text search over subject, sender and body. rowid equals message.id.
CREATE VIRTUAL TABLE message_fts USING fts5 (
    subject, sender, body,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER message_fts_insert AFTER INSERT ON message BEGIN
    INSERT INTO message_fts (rowid, subject, sender, body)
    VALUES (new.id, new.subject, coalesce(new.from_name, '') || ' ' || coalesce(new.from_addr, ''), '');
END;

CREATE TRIGGER message_fts_update AFTER UPDATE OF subject, from_name, from_addr ON message BEGIN
    UPDATE message_fts
       SET subject = new.subject,
           sender  = coalesce(new.from_name, '') || ' ' || coalesce(new.from_addr, '')
     WHERE rowid = new.id;
END;

CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
    DELETE FROM message_fts WHERE rowid = old.id;
END;

CREATE TRIGGER message_body_fts AFTER INSERT ON message_body BEGIN
    UPDATE message_fts SET body = coalesce(new.text_plain, '') WHERE rowid = new.message_id;
END;

-- Folder counters are kept by triggers so the UI never counts rows.
CREATE TRIGGER message_count_insert AFTER INSERT ON message BEGIN
    UPDATE folder SET total_count  = total_count + 1,
                      unread_count = unread_count + (1 - new.is_seen)
     WHERE id = new.folder_id;
END;

CREATE TRIGGER message_count_delete AFTER DELETE ON message BEGIN
    UPDATE folder SET total_count  = total_count - 1,
                      unread_count = unread_count - (1 - old.is_seen)
     WHERE id = old.folder_id;
END;

CREATE TRIGGER message_count_seen AFTER UPDATE OF is_seen ON message
WHEN old.is_seen <> new.is_seen BEGIN
    UPDATE folder SET unread_count = unread_count + (old.is_seen - new.is_seen)
     WHERE id = new.folder_id;
END;
