-- OpenPGP keys (T41b, migration 15): each account's own public keys and those it learnt of other people, and what each
-- sender's Autocrypt header last said. Never a secret: an own key's secret part is in the platform keystore only.
-- Everything goes with the account.

-- `source`: how Croft has the key (made here, imported from a file, from an Autocrypt header, from WKD). `own`: its secret
-- is in the keystore. The certificate is binary OpenPGP (`public_key`), as read back by croft-crypto.
CREATE TABLE pgp_key (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    fingerprint TEXT    NOT NULL,
    public_key  BLOB    NOT NULL,
    own         INTEGER NOT NULL DEFAULT 0 CHECK (own IN (0, 1)),
    source      TEXT    NOT NULL CHECK (source IN ('made', 'imported', 'autocrypt', 'wkd')),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER,
    revoked     INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    updated_at  INTEGER NOT NULL,
    UNIQUE (account_id, fingerprint)
);

-- The addresses a key is filed under, ASCII lower-case: those of its valid user IDs (for an Autocrypt key, only the
-- header's `addr`).
CREATE TABLE pgp_key_address (
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    key_id     INTEGER NOT NULL REFERENCES pgp_key (id) ON DELETE CASCADE,
    address    TEXT    NOT NULL,
    PRIMARY KEY (key_id, address)
);
CREATE INDEX pgp_key_address_lookup ON pgp_key_address (account_id, address);

-- Autocrypt Level 1 state per sender: the key their newest header named and its `prefer-encrypt=mutual`, the date of that
-- message, and the date of the newest message from them (with or without a header).
CREATE TABLE autocrypt_peer (
    account_id    INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    address       TEXT    NOT NULL,
    fingerprint   TEXT,
    prefer_mutual INTEGER NOT NULL DEFAULT 0 CHECK (prefer_mutual IN (0, 1)),
    autocrypt_at  INTEGER,
    last_seen     INTEGER NOT NULL,
    PRIMARY KEY (account_id, address)
);
