-- Pin and follow-up (T36b, migration 10). Both are keywords on the server (`$Pinned`, `$FollowUp`)
-- and stay in `keywords_json`; these two columns mirror them so the Inbox's Pinned section and the
-- Follow up folder are plain indexed queries. The core keeps them in step on every write.

ALTER TABLE message ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE message ADD COLUMN follows_up INTEGER NOT NULL DEFAULT 0;

UPDATE message
   SET is_pinned  = EXISTS (SELECT 1 FROM json_each(message.keywords_json) WHERE lower(value) = '$pinned'),
       follows_up = EXISTS (SELECT 1 FROM json_each(message.keywords_json) WHERE lower(value) = '$followup');

CREATE INDEX message_account_pinned ON message (account_id, date_ts) WHERE is_pinned = 1;
CREATE INDEX message_account_follow_up ON message (account_id, date_ts) WHERE follows_up = 1;
