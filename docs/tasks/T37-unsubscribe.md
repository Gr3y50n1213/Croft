# T37: One-tap unsubscribe

Folders: `core/` (protocols, mail-core, storage, ffi), `android/` (S21's banner, S31).

Depends on: T35 (the stored `List-Unsubscribe` and `List-Unsubscribe-Post` headers). Read first: `docs/ux/server-contract.md`
("Unsubscribe"), `docs/ux/screens.md` (S21, S31), the S31 wireframe (`design/wireframes/08_B5_inbox_5.png`), RFC 2369 and
RFC 8058. Security-sensitive: an independent review before merging.

## Goal
From a mailing-list message, the person can unsubscribe in one step after confirming: Croft uses the sender's one-click address
when there is one, else sends the unsubscribe email, else opens the sender's page in the browser. Optionally it also deletes the
older mail from that sender.

## Spec
- **Reading the header (mail-core, pure).** `List-Unsubscribe` is a list of `<URI>` entries (RFC 2369). Croft takes the first
  `https:` URI and the first `mailto:` URI and ignores everything else (`http:`, other schemes, text outside the brackets). A URI
  longer than 2048 bytes, with control characters, spaces or credentials (`user@host`) in an `https` authority, or a `mailto:`
  whose address does not parse as one plain address, is ignored. `mailto:` keeps only `subject` and `body` from its query
  (percent-decoded, subject at most 200 and body at most 500 characters, control characters other than line breaks in the body removed); any other field (`cc`, `bcc`, `to`,
  headers) is dropped.
- **Choosing the way.**
  1. One-click: an `https:` URI and `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (stored as `list_one_click`).
  2. Email: a `mailto:` URI.
  3. Website: an `https:` URI without one-click. Croft never fetches it; the app opens it in the browser after showing the host.
- **One-click request (protocols).** A single `POST` with the body `List-Unsubscribe=One-Click` and
  `Content-Type: application/x-www-form-urlencoded`, over the existing small HTTPS client: certificates always checked, no
  cookies, no `Authorization`, no other header from the message, no redirect followed, bounded waits, answer body not read beyond a
  small cap and never kept, and **public addresses only** (the `Internet` reach: a message can never make the phone or the
  Companion post to a private network). 2xx is done; anything else is "Couldn't unsubscribe" with the website or email offered if
  the header had one. No retry loop: one attempt per tap.
- **Email.** A new message from the account to that address with the URI's subject (else "unsubscribe") and body, sent through the
  Outbox like any other (Undo while it waits).
- **No silent contact.** Nothing is sent without the person confirming in S31; the dialog names the sender, says how (the host for
  one-click and website; the address, subject and body for email), and the confirmed call hands back what S31 showed (nothing is
  done if the message changed). "Also delete the N older messages from <address>" trashes them with Undo, never the account's
  own or sent mail.
- **After.** The sender's address is remembered per account (`unsubscribed` table, migration 0011, scoped by account), so the banner
  shows "Unsubscribed" instead of the action for that sender's later mail. Forgotten when the account is removed.
- **DKIM.** RFC 8058 asks that the two headers be covered by a valid DKIM signature. Croft does not verify DKIM yet; the
  confirmation (with the host shown) and the public-addresses-only rule are what stand in its place. Recorded in DECISIONS and
  OPEN-ITEMS.
- **The core (FFI v18).** `unsubscribe_info(message_id) -> Option<UnsubscribeInfo>` (the way, the host or address, the sender, the
  count of older mail from that sender in the account, and whether it was already unsubscribed) and
  `unsubscribe(message_id, delete_older) -> UnsubscribeOutcome` (done, the website URL to open, or failed with a fixed reason, plus
  the operation ids of the deletes for Undo). Nothing about the URI or the answer is logged.
- **Android.** S21's banner on a message that can be unsubscribed from ("Unsubscribe from <sender>"; "Unsubscribed" once done);
  S31 as the confirmation dialog of the wireframe; the result as a snackbar; a website opens in the browser only after S31.
- **The web (T43).** Not in this task.

## Done when
- mail-core tests: parsing (order, schemes, brackets, bounds, credentials, control characters, `mailto:` fields), and the choice.
- protocols tests (in-process HTTPS server): the exact request (method, body, content type, no cookies or extra headers), 2xx, 4xx and
  5xx, a redirect not followed, a private address refused before connecting, an `http:` URI refused.
- storage and FFI tests: the info, the count, the email path through the Outbox, the delete of older mail with Undo, the
  remembered sender per account.
- Android unit tests: the banner, S31 with and without the delete, the outcomes.
- An independent review of the request path, the parsing and the Android browser hand-off, with its findings fixed.
- CI green.
