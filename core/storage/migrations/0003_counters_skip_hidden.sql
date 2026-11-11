-- Folder counters leave out rows hidden by a pending move or delete (message.local_hidden, added in 0002),
-- so the unread badge drops the moment the person archives, not when the server confirms.
-- Each trigger subtracts a row's old contribution and adds its new one, so one UPDATE that changes both
-- is_seen and local_hidden is counted exactly once. A visible row counts 1 towards total_count and
-- (1 - is_seen) towards unread_count; a hidden row counts 0 towards both.

DROP TRIGGER message_count_insert;
DROP TRIGGER message_count_delete;
DROP TRIGGER message_count_seen;

CREATE TRIGGER message_count_insert AFTER INSERT ON message BEGIN
    UPDATE folder SET total_count  = total_count  + (1 - new.local_hidden),
                      unread_count = unread_count + (1 - new.local_hidden) * (1 - new.is_seen)
     WHERE id = new.folder_id;
END;

CREATE TRIGGER message_count_delete AFTER DELETE ON message BEGIN
    UPDATE folder SET total_count  = total_count  - (1 - old.local_hidden),
                      unread_count = unread_count - (1 - old.local_hidden) * (1 - old.is_seen)
     WHERE id = old.folder_id;
END;

CREATE TRIGGER message_count_update AFTER UPDATE OF is_seen, local_hidden ON message
WHEN old.is_seen <> new.is_seen OR old.local_hidden <> new.local_hidden BEGIN
    UPDATE folder SET total_count  = total_count  - (1 - old.local_hidden) + (1 - new.local_hidden),
                      unread_count = unread_count - (1 - old.local_hidden) * (1 - old.is_seen)
                                                  + (1 - new.local_hidden) * (1 - new.is_seen)
     WHERE id = new.folder_id;
END;

-- Rows already hidden when this migration runs were counted by the old triggers; take them out once.
UPDATE folder SET
    total_count  = total_count  - (SELECT count(*) FROM message m WHERE m.folder_id = folder.id AND m.local_hidden = 1),
    unread_count = unread_count - (SELECT count(*) FROM message m WHERE m.folder_id = folder.id AND m.local_hidden = 1 AND m.is_seen = 0);
