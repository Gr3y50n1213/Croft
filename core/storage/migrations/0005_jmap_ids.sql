-- JMAP id map (T09). The JMAP adapter names a message by (folder, u32 UID, folder UIDVALIDITY) like
-- the rest of Croft, but JMAP ids are strings. For each mailbox the adapter keeps a UIDVALIDITY and a
-- counter, and for each (mailbox, email id) the UID it handed out.
--
-- A UID is never given twice: `next_uid` only ever goes up (a trigger refuses to lower it), removing
-- a row does not change it, an entry's UID must be below it, and (account, mailbox, uid) is unique.
-- Resetting a mailbox deletes its row, which cascades to its entries, and inserts a new one with a
-- new UIDVALIDITY. Every table carries account_id and is removed with the account.
CREATE TABLE jmap_mailbox (
    account_id   INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    mailbox_id   TEXT    NOT NULL,
    uid_validity INTEGER NOT NULL CHECK (uid_validity BETWEEN 1 AND 4294967295),
    -- One past the last UID handed out. 4294967296 means all of them are used.
    next_uid     INTEGER NOT NULL DEFAULT 1 CHECK (next_uid BETWEEN 1 AND 4294967296),
    PRIMARY KEY (account_id, mailbox_id)
) WITHOUT ROWID;

CREATE TABLE jmap_email_uid (
    account_id INTEGER NOT NULL,
    mailbox_id TEXT    NOT NULL,
    email_id   TEXT    NOT NULL,
    uid        INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    PRIMARY KEY (account_id, mailbox_id, email_id),
    UNIQUE (account_id, mailbox_id, uid),
    FOREIGN KEY (account_id, mailbox_id) REFERENCES jmap_mailbox (account_id, mailbox_id) ON DELETE CASCADE
) WITHOUT ROWID;

-- The newest `Email` state string the id map was reconciled with, one per account.
CREATE TABLE jmap_account_state (
    account_id  INTEGER PRIMARY KEY REFERENCES account (id) ON DELETE CASCADE,
    email_state TEXT NOT NULL
);

CREATE TRIGGER jmap_email_uid_below_next BEFORE INSERT ON jmap_email_uid
WHEN NEW.uid >= (SELECT next_uid FROM jmap_mailbox
                 WHERE account_id = NEW.account_id AND mailbox_id = NEW.mailbox_id)
BEGIN
    SELECT RAISE(ABORT, 'a JMAP UID must be below the mailbox next_uid');
END;

CREATE TRIGGER jmap_email_uid_is_fixed BEFORE UPDATE ON jmap_email_uid
BEGIN
    SELECT RAISE(ABORT, 'a JMAP UID is never reassigned');
END;

CREATE TRIGGER jmap_mailbox_next_uid_only_goes_up BEFORE UPDATE OF next_uid ON jmap_mailbox
WHEN NEW.next_uid < OLD.next_uid
BEGIN
    SELECT RAISE(ABORT, 'next_uid never goes down');
END;
