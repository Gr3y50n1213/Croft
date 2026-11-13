-- Contacts (T38a, migration 12): each account's CardDAV address books and their cards, kept on the
-- device. The card text is kept whole so an edit sent back loses nothing; the columns beside it are
-- what the lists and autocomplete read. Everything goes with the account.

CREATE TABLE address_book (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    path       TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    sync_token TEXT,
    ctag       TEXT,
    synced_at  INTEGER,
    UNIQUE (account_id, path)
);

-- `href` is NULL until a card made on the device is created on the server. `pending`: 0 nothing,
-- 1 create, 2 update, 3 delete (the row stays hidden until the server confirms). `revision` grows
-- with every change made on the device, so a sync confirms only the version it sent. `conflict` is
-- set when the server had changed the card: the server's version was kept.
CREATE TABLE contact (
    id           INTEGER PRIMARY KEY,
    account_id   INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    book_id      INTEGER NOT NULL REFERENCES address_book (id) ON DELETE CASCADE,
    href         TEXT,
    etag         TEXT,
    uid          TEXT    NOT NULL,
    display_name TEXT    NOT NULL,
    sort_key     TEXT    NOT NULL,
    card         TEXT    NOT NULL,
    pending      INTEGER NOT NULL DEFAULT 0 CHECK (pending IN (0, 1, 2, 3)),
    revision     INTEGER NOT NULL DEFAULT 0,
    conflict     INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1)),
    updated_at   INTEGER NOT NULL,
    UNIQUE (book_id, href)
);
CREATE INDEX contact_account_sort ON contact (account_id, sort_key);
CREATE INDEX contact_pending ON contact (account_id, pending) WHERE pending <> 0;

-- A contact's addresses, lower-case: autocomplete and "is this sender a contact".
CREATE TABLE contact_email (
    contact_id INTEGER NOT NULL REFERENCES contact (id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    address    TEXT    NOT NULL,
    PRIMARY KEY (contact_id, address)
) WITHOUT ROWID;
CREATE INDEX contact_email_address ON contact_email (account_id, address);
