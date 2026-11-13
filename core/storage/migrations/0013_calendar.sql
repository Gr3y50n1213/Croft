-- Calendars (T39a, migration 13): each account's CalDAV calendars and their objects, kept on the
-- device. An object (an event with its changed occurrences) is kept whole so an edit sent back
-- loses nothing; the columns beside it are what the queries read. Everything goes with the account.

CREATE TABLE calendar (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    path       TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    color      TEXT,
    sort_order INTEGER,
    sync_token TEXT,
    ctag       TEXT,
    shown      INTEGER NOT NULL DEFAULT 1 CHECK (shown IN (0, 1)),
    synced_at  INTEGER,
    UNIQUE (account_id, path)
);

-- `href` is NULL until an event made on the device is created on the server. `pending`, `revision`
-- and `conflict` work as for contacts (0 nothing, 1 create, 2 update, 3 delete). `first_start` and
-- `last_end` (Unix ms, UTC; NULL `last_end` for a repeat without end) bound every occurrence, read
-- in UTC: queries widen the window by a day for all-day and floating times.
CREATE TABLE event (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    calendar_id INTEGER NOT NULL REFERENCES calendar (id) ON DELETE CASCADE,
    href        TEXT,
    etag        TEXT,
    uid         TEXT    NOT NULL,
    object      TEXT    NOT NULL,
    first_start INTEGER NOT NULL,
    last_end    INTEGER,
    pending     INTEGER NOT NULL DEFAULT 0 CHECK (pending IN (0, 1, 2, 3)),
    revision    INTEGER NOT NULL DEFAULT 0,
    conflict    INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1)),
    updated_at  INTEGER NOT NULL,
    UNIQUE (calendar_id, href)
);
CREATE INDEX event_account_span ON event (account_id, first_start, last_end);
CREATE INDEX event_pending ON event (account_id, pending) WHERE pending <> 0;
