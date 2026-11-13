-- Calendar links (T39c, migration 14): a read-only calendar of an account made from a link (an
-- iCalendar file over https) the person added. `source_url` is the address as fetched (it may hold
-- a private token: it is never logged); `link_etag` and `link_modified` let the next fetch ask
-- whether the file changed; `link_fetched_at` spaces the fetches. A calendar from CalDAV has none.

ALTER TABLE calendar ADD COLUMN source_url TEXT;
ALTER TABLE calendar ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1));
ALTER TABLE calendar ADD COLUMN link_etag TEXT;
ALTER TABLE calendar ADD COLUMN link_modified TEXT;
ALTER TABLE calendar ADD COLUMN link_fetched_at INTEGER;
