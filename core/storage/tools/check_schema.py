#!/usr/bin/env python3
"""Applies every migration to an in-memory SQLite database and checks the behaviour the Rust
code will rely on: foreign keys, counter triggers, full-text search and the due-operation query.
Run: python3 core/storage/tools/check_schema.py"""
import glob, os, sqlite3, sys

here = os.path.dirname(os.path.abspath(__file__))
db = sqlite3.connect(":memory:")
db.execute("PRAGMA foreign_keys = ON")
for path in sorted(glob.glob(os.path.join(here, "..", "migrations", "*.sql"))):
    db.executescript(open(path).read())

def one(sql, *a):
    return db.execute(sql, a).fetchone()[0]

def check(name, cond):
    print(("ok   " if cond else "FAIL ") + name)
    if not cond:
        check.failed = True
check.failed = False

db.execute("INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (1,'Sam','sam@example.org','imap','{}',0)")
db.execute("INSERT INTO folder (id,account_id,path,special_use) VALUES (1,1,'INBOX','inbox')")
for uid, subj, seen, ts in [(1, "Lunch on Friday?", 0, 300), (2, "Server migration notes", 1, 200), (3, "Bin collection changes", 0, 100)]:
    db.execute("INSERT INTO message (account_id,folder_id,uid,thread_key,subject,from_name,from_addr,date_ts,is_seen) "
               "VALUES (1,1,?,?,?,?,?,?,?)", (uid, f"t{uid}", subj, "Sam Rivera", "sam@example.org", ts, seen))

check("folder counters after insert: total 3, unread 2",
      db.execute("SELECT total_count, unread_count FROM folder WHERE id=1").fetchone() == (3, 2))
db.execute("UPDATE message SET is_seen = 1 WHERE uid = 1")
check("marking read lowers unread to 1", one("SELECT unread_count FROM folder WHERE id=1") == 1)
db.execute("UPDATE message SET is_seen = 0 WHERE uid = 1")
check("marking unread raises unread to 2", one("SELECT unread_count FROM folder WHERE id=1") == 2)
db.execute("DELETE FROM message WHERE uid = 3")
check("deleting an unread message updates both counters",
      db.execute("SELECT total_count, unread_count FROM folder WHERE id=1").fetchone() == (2, 1))

check("list query is newest first",
      [r[0] for r in db.execute("SELECT uid FROM message WHERE folder_id=1 ORDER BY date_ts DESC")] == [1, 2])
plan = " ".join(r[3] for r in db.execute("EXPLAIN QUERY PLAN SELECT uid FROM message WHERE folder_id=1 ORDER BY date_ts DESC"))
check("list query uses message_folder_date (no sort step)", "message_folder_date" in plan and "TEMP B-TREE" not in plan)

db.execute("INSERT INTO message_body (message_id, text_plain) VALUES (2, 'Thanks, I will test the new DNS records tomorrow')")
check("search finds subject words", one("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'lunch'") == 1)
check("search finds sender", one("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'rivera'") == 2)
check("search finds body text added later", one("SELECT rowid FROM message_fts WHERE message_fts MATCH 'dns'") == 2)
db.execute("UPDATE message SET subject = 'Lunch moved to Thursday' WHERE uid = 1")
check("search follows subject edits", one("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'thursday'") == 1
      and one("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'friday'") == 0)
db.execute("DELETE FROM message WHERE uid = 2")
check("search drops deleted messages", one("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'dns'") == 0)

db.execute("INSERT INTO op_queue (account_id,kind,payload_json,target_key,created_at,not_before) VALUES (1,'set_seen','{}','m1',1000,6000)")
db.execute("INSERT INTO op_queue (account_id,kind,payload_json,target_key,created_at,not_before,state) VALUES (1,'move','{}','m2',1000,500,'confirmed')")
due = lambda now: one("SELECT count(*) FROM op_queue WHERE state='queued' AND not_before <= ?", now)
check("op inside undo window is not due", due(5999) == 0)
check("op becomes due at not_before", due(6000) == 1)
db.execute("UPDATE op_queue SET state='cancelled' WHERE id=1")
check("cancelled op is never due", due(99999) == 0)

# Migration 0002: hidden rows and the awaiting-UID table.
check("message.local_hidden exists and defaults to 0",
      one("SELECT count(*) FROM message WHERE local_hidden = 0") == one("SELECT count(*) FROM message"))
db.execute("UPDATE message SET local_hidden = 1 WHERE uid = 1")
check("a hidden row is left out of the visible list",
      [r[0] for r in db.execute("SELECT uid FROM message WHERE folder_id=1 AND local_hidden = 0")] == [])
db.execute("UPDATE message SET local_hidden = 0 WHERE uid = 1")
db.execute("INSERT INTO op_awaiting_uid (account_id,folder,uid,dest_folder,message_id_hdr,created_at) VALUES (1,'INBOX',1,'Archive','<a@example.org>',5)")
check("op_awaiting_uid keeps destination and Message-ID, lookups start at 0",
      db.execute("SELECT dest_folder, message_id_hdr, lookups FROM op_awaiting_uid").fetchone() == ("Archive", "<a@example.org>", 0))
try:
    db.execute("INSERT INTO op_awaiting_uid (account_id,folder,uid,dest_folder,created_at) VALUES (1,'INBOX',1,'Trash',6)")
    check("op_awaiting_uid has one row per old address", False)
except sqlite3.IntegrityError:
    check("op_awaiting_uid has one row per old address", True)
try:
    db.execute("INSERT INTO op_awaiting_uid (account_id,folder,uid,dest_folder,created_at) VALUES (99,'INBOX',2,'Trash',6)")
    check("op_awaiting_uid foreign key rejects a missing account", False)
except sqlite3.IntegrityError:
    check("op_awaiting_uid foreign key rejects a missing account", True)

# Migration 0003: hidden rows do not count towards the folder counters.
counts = lambda: db.execute("SELECT total_count, unread_count FROM folder WHERE id=1").fetchone()
before = counts()  # uid 1 is visible and unread
db.execute("UPDATE message SET local_hidden = 1 WHERE uid = 1")
check("hiding an unread row lowers total and unread by one", counts() == (before[0] - 1, before[1] - 1))
db.execute("UPDATE message SET is_seen = 1 WHERE uid = 1")
check("marking a hidden row read leaves the counters alone", counts() == (before[0] - 1, before[1] - 1))
db.execute("UPDATE message SET local_hidden = 0, is_seen = 0 WHERE uid = 1")
check("showing it again and marking unread in one update restores both counters", counts() == before)
db.execute("UPDATE message SET local_hidden = 1 WHERE uid = 1")
db.execute("DELETE FROM message WHERE uid = 1")
check("deleting a hidden row leaves the counters alone", counts() == (before[0] - 1, before[1] - 1))
db.execute("INSERT INTO message (account_id,folder_id,uid,thread_key,date_ts,is_seen,local_hidden) VALUES (1,1,1,'t1',300,0,1)")
check("inserting a hidden row leaves the counters alone", counts() == (before[0] - 1, before[1] - 1))
db.execute("UPDATE message SET local_hidden = 0 WHERE uid = 1")
check("counters match a fresh count of visible rows",
      counts() == (one("SELECT count(*) FROM message WHERE folder_id=1 AND local_hidden=0"),
                   one("SELECT count(*) FROM message WHERE folder_id=1 AND local_hidden=0 AND is_seen=0")))

# Migration 0004: the Outbox keeps the envelope and has the submitted and filing states.
db.execute("INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (2,'Kim','kim@example.org','imap','{}',0)")
db.execute("INSERT INTO identity (id,account_id,display_name,address,is_default) VALUES (20,2,'Kim','kim@example.org',1)")
db.execute("INSERT INTO outbox (account_id,identity_id,mime_blob,send_at,envelope_from,recipients) "
           "VALUES (2,20,?,6000,'kim@example.org','[\"a@example.org\",\"b@example.org\"]')", (b"Subject: x\r\n\r\n\x00\xff",))
check("outbox row keeps bytes, envelope and defaults to queued with 0 attempts",
      db.execute("SELECT mime_blob, envelope_from, recipients, state, attempts FROM outbox").fetchone()
      == (b"Subject: x\r\n\r\n\x00\xff", "kim@example.org", '["a@example.org","b@example.org"]', "queued", 0))
for st in ("sending", "submitted", "filing", "sent", "failed", "queued"):
    db.execute("UPDATE outbox SET state = ?", (st,))
check("outbox accepts submitted and filing", one("SELECT state FROM outbox") == "queued")
try:
    db.execute("UPDATE outbox SET state = 'bogus'")
    check("outbox state check rejects unknown states", False)
except sqlite3.IntegrityError:
    check("outbox state check rejects unknown states", True)
try:
    db.execute("INSERT INTO outbox (account_id,mime_blob,send_at) VALUES (99,x'00',0)")
    check("outbox foreign key rejects a missing account", False)
except sqlite3.IntegrityError:
    check("outbox foreign key rejects a missing account", True)
first_id = one("SELECT id FROM outbox")
db.execute("DELETE FROM outbox")
db.execute("INSERT INTO outbox (account_id,mime_blob,send_at) VALUES (2,x'00',0)")
check("outbox ids are never reused after a delete", one("SELECT id FROM outbox") > first_id)
db.execute("DELETE FROM identity WHERE id = 20")
check("removing an identity keeps the outbox row", one("SELECT count(*) FROM outbox") == 1
      and one("SELECT identity_id IS NULL FROM outbox") == 1)
db.execute("DELETE FROM account WHERE id = 2")
check("removing an account removes its outbox rows", one("SELECT count(*) FROM outbox") == 0)

try:
    db.execute("INSERT INTO message (account_id,folder_id,uid,thread_key,date_ts,category) VALUES (1,1,9,'t9',0,'spam')")
    check("category check rejects unknown values", False)
except sqlite3.IntegrityError:
    check("category check rejects unknown values", True)
try:
    db.execute("INSERT INTO folder (account_id,path) VALUES (99,'X')")
    check("foreign key rejects a missing account", False)
except sqlite3.IntegrityError:
    check("foreign key rejects a missing account", True)
db.execute("DELETE FROM account WHERE id = 1")
check("removing an account removes its folders, messages, queue and awaiting marks",
      one("SELECT count(*) FROM folder") + one("SELECT count(*) FROM message") + one("SELECT count(*) FROM op_queue")
      + one("SELECT count(*) FROM op_awaiting_uid") == 0)

# Upgrading a version-2 database that already has hidden rows corrects its counters once.
mig = sorted(glob.glob(os.path.join(here, "..", "migrations", "*.sql")))
old = sqlite3.connect(":memory:")
for path in mig[:2]:
    old.executescript(open(path).read())
old.executescript("""INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (1,'S','s@example.org','imap','{}',0);
INSERT INTO folder (id,account_id,path) VALUES (1,1,'INBOX');
INSERT INTO message (account_id,folder_id,uid,thread_key,date_ts,is_seen) VALUES (1,1,1,'t1',1,0),(1,1,2,'t2',2,1);
UPDATE message SET local_hidden = 1 WHERE uid = 1;""")
old.executescript(open(mig[2]).read())
check("migration 0003 takes already hidden rows out of the counters",
      old.execute("SELECT total_count, unread_count FROM folder WHERE id=1").fetchone() == (1, 0))

# Upgrading a version-3 database that already has an outbox row keeps the row and the id counter.
old = sqlite3.connect(":memory:")
old.execute("PRAGMA foreign_keys = ON")
for path in mig[:3]:
    old.executescript(open(path).read())
old.executescript("""INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (1,'S','s@example.org','imap','{}',0);
INSERT INTO outbox (id,account_id,mime_blob,state,send_at,attempts,last_error) VALUES (41,1,x'414243','failed',99,3,'old');""")
old.executescript(open(mig[3]).read())
check("migration 0004 keeps existing outbox rows and gives them an empty envelope",
      old.execute("SELECT id, mime_blob, state, send_at, attempts, last_error, envelope_from, recipients FROM outbox").fetchone()
      == (41, b"ABC", "failed", 99, 3, "old", "", "[]"))
old.execute("INSERT INTO outbox (account_id,mime_blob,send_at) VALUES (1,x'00',0)")
check("migration 0004 continues the id counter after the old rows",
      old.execute("SELECT max(id) FROM outbox").fetchone()[0] == 42)

sys.exit(1 if check.failed else 0)
