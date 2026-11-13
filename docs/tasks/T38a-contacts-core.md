# T38a: Contacts in the core (CardDAV)

Folders: `core/` (protocols: a CardDAV client; mail-core: vCard; storage; engine; ffi). The Android screens are T38b.

Depends on: phase 2. Read first: `docs/PLAN.md` ("Calendar and contacts", the stack table), `docs/ux/screens.md` (S40 to S42),
RFC 6352 (CardDAV), RFC 6578 (sync-collection), RFC 6764 (finding the service), RFC 6350 and 2426 (vCard 4 and 3).

## Goal
Each account's address books are kept on the device, in sync with the server both ways, so the contacts screens and compose's
autocomplete work offline.

## Spec
- **Finding the service.** From the account's mail host: `/.well-known/carddav` (one redirect followed, same scheme and host
  family), then `current-user-principal`, `addressbook-home-set`, and the address books under it. The same credential as mail
  (password or OAuth token); the same certificate rules and pin; the process's address policy (the Companion). A server without
  CardDAV is fine: the account simply has no address books ("Contacts: not offered by this server").
- **The client (protocols).** A small CardDAV client over the existing HTTPS client, not a new HTTP stack: `PROPFIND`, `REPORT`
  (`sync-collection` with a sync token, else `getetag` listing and `addressbook-multiget`), `PUT` with `If-Match` /
  `If-None-Match: *`, `DELETE` with `If-Match`. XML with quick-xml, bounded (sizes, depth, entries). Redirects other than the
  well-known one are not followed. Nothing about a contact is logged.
- **vCard (mail-core, pure).** Read vCard 3.0 and 4.0 into a contact (name, formatted name, emails, phones, postal addresses,
  organisation, notes, birthday, photo URI only, `KEY` presence) while keeping every other property line as it was, so an edit
  saved back loses nothing. Writing produces vCard 4.0 when the card was 4.0 or new, else 3.0. Bounded (card size, lines,
  properties); folded lines, quoted-printable (3.0), escaping and parameters handled. Photos are not downloaded.
- **Storage (migration 0012).** `address_book` and `contact` tables scoped by account (contact: book, href, etag, uid, display
  name, sort key, the raw card, what waits to be sent, a conflict mark); a `contact_email` table for autocomplete and "is this
  sender a contact". Search is an indexed `LIKE` on names and addresses (address books are small). Removed with the account.
- **Sync (`sync_contacts`).** Changes made here go out first (create, update, delete, each conditional on the etag), then the
  server's come in; a 412 (changed on the server) keeps the server's card and reports the conflict. Local edits are marked on the
  contact row, not in the mail operation queue (a contact has no Undo window). The app asks for a sync (S40 opening, pull to
  refresh, after an account is added); scheduling it in the background is T38b's.
- **Recent recipients.** Already kept per account since T18d (`sent_recipient`); autocomplete merges them after the contacts.
- **The core (FFI v19).** List and search contacts (per account or all; A to Z with a sort key), get one, create, update, delete,
  the address books of an account, `autocomplete(prefix, limit)` over contacts and recent recipients (contacts first, then by use),
  sync now, and the change notices the lists already use.
- **Not in this task:** JMAP Contacts (RFC 9610) and Google's and Microsoft's contacts (their OAuth scopes); groups; photos;
  calendar (T39). In OPEN-ITEMS.

## Done when
- vCard tests: 3.0 and 4.0, folding, escaping, quoted-printable, unknown properties kept byte for byte through an edit, bounds.
- Protocol tests against an in-process DAV fake: discovery, sync-collection with and without a token, multiget, conditional PUT
  and DELETE, 412, redirects refused, a hostile XML answer bounded.
- An integration test against Stalwart's CardDAV (already in the CI's integration job); Radicale later.
- Storage and FFI tests: scoping per account, search, autocomplete order, recent recipients, conflict handling, migration 0012.
- An independent review (credentials sent to a new endpoint, a new parser of server data).
- CI green.
