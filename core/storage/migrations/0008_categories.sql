-- Inbox categories (T35, migration 8): the list and bulk-mail headers each message was fetched
-- with (the classifier reads them; T37's unsubscribe uses the last two), and the person's
-- "Always do this" rules, per account.

ALTER TABLE message ADD COLUMN list_id          TEXT;
ALTER TABLE message ADD COLUMN list_unsubscribe TEXT;
-- 1 when `List-Unsubscribe-Post: List-Unsubscribe=One-Click` came with it (RFC 8058).
ALTER TABLE message ADD COLUMN list_one_click   INTEGER NOT NULL DEFAULT 0 CHECK (list_one_click IN (0, 1));

-- Mail from `sender` (a lower-case address, or a domain) goes to `category`. Made when the person
-- moves a message to a category and chooses "Always do this".
CREATE TABLE category_rule (
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    sender     TEXT    NOT NULL,
    category   TEXT    NOT NULL CHECK (category IN ('primary', 'updates', 'promotions', 'social')),
    PRIMARY KEY (account_id, sender)
);

-- The unread count per category of a folder (S10's chips).
CREATE INDEX message_folder_category ON message (account_id, folder_id, category) WHERE is_seen = 0;
