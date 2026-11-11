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
check("removing an account removes its folders, messages and queue",
      one("SELECT count(*) FROM folder") + one("SELECT count(*) FROM message") + one("SELECT count(*) FROM op_queue") == 0)

sys.exit(1 if check.failed else 0)
