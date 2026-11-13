-- One-tap unsubscribe (T37, migration 11): the senders the person unsubscribed from, per account,
-- so S21 says "Unsubscribed" on their later mail instead of offering it again. The address is
-- lower-case. Removed with the account.

CREATE TABLE unsubscribed (
    account_id INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    sender     TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, sender)
);

-- The older mail from one sender (S31's "Also delete the N older messages").
CREATE INDEX message_account_sender ON message (account_id, from_addr COLLATE NOCASE);
