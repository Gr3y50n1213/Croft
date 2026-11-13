-- Snooze on this device (T36, migration 9). `message.snoozed_until` (migration 1) hides a message
-- until its wake time; when it wakes, `snooze_back_at` keeps when, for the Snoozed chip it shows for
-- 24 hours.

ALTER TABLE message ADD COLUMN snooze_back_at INTEGER;

-- The Snoozed folder and the next wake time.
CREATE INDEX message_account_snoozed ON message (account_id, snoozed_until) WHERE snoozed_until IS NOT NULL;
