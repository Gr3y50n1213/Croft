# T06: SMTP send and Outbox

Crate / folder: `core/protocols + core/mail-core`

## Goal
Sending, with the undo window and the Outbox screen (S26).

## Spec
- `lettre` (tokio + rustls) with XOAUTH2 support.
- Send writes to the `outbox` table with `send_at = now + undo window`; Undo before `send_at` removes the row; after it the message is sent and then appended to Sent if the server does not do it.
- States: queued, sending, sent, failed with a reason; failed rows can Retry, Edit (returns to a draft) or Discard.

## Done when
- Send with no network queues and sends later exactly once.
- Undo inside the window sends nothing (test on the fake SMTP server).
