# T38b: The contacts screens on Android

Folders: `android/` (S40, S41, S42, compose's suggestions, the bottom bar).

Depends on: T38a (contacts in the core, FFI v19). Read first: `docs/ux/screens.md` (S40 to S42), the contacts wireframe
(`design/wireframes/16_F1_contacts.png`), `docs/tasks/T38a-contacts-core.md`.

## Goal
The person can open Contacts from the bottom bar, find a contact, see it, mail or call it, and add, edit or delete contacts in an
account's address book. Compose suggests contacts first, then recent recipients.

## Spec
- **Bottom bar.** Contacts is enabled where the session has the core's contacts (`session.contacts`). Mail and Contacts switch
  between the two destinations; Calendar stays disabled until T39. Back from Contacts goes to the Inbox.
- **S40 Contacts.** A search field ("Search contacts"), account chips (All, then one per account with an address book; only shown
  with more than one such account), the list A to Z with a header per first letter (anything that is not a letter goes under
  "#"), each row with an avatar letter, the name and the first address. An A to Z index on the trailing edge: tap or drag to a
  letter jumps to its section, a letter bubble shows while dragging, and the Tick haptic plays at each new letter (through
  `Haptics`). It is hidden from screen readers (they scroll the list). An Add button opens S42 empty. Opening S40 and pulling to
  refresh sync each account's contacts (`sync_contacts`), one account after another; the list reloads after each. A sync that
  could not reach the server says so in a line under the chips ("Couldn't reach <account>"); "sign in needed" says that too;
  "not offered" is quiet. Empty states: no contacts ("No contacts yet"), no match ("No contacts match"), and no account with an
  address book ("Your accounts don't offer contacts").
- **S41 Contact.** Avatar letter, name, organisation. Actions: Mail (compose to the first address), Call (the dialler with the
  first number, `ACTION_DIAL`: no permission), and Edit; the overflow menu has Delete (asks first). Rows for each address and
  number with their kind; an address row has "Key attached" when the card carries a key (encryption is T41 and T42). The postal
  address, the birthday and the notes when there are any. "Recent conversations": the last three messages from or to the first
  address, from the device's search index, each opening the conversation. A conflict with the server shows a line: "Changed on
  the server too; this copy was kept as a new contact". Invite (calendar) waits for T39; star (favourite) and photos are not in
  this task (OPEN-ITEMS).
- **S42 Edit contact.** First and last name, emails (add, remove), phones (add, remove), one postal address, organisation, notes,
  and "Save to" (the address book, chosen for a new contact only; the first book of the first account is the default). Save is
  disabled until something changed and needs a name or an address; Close with changes asks "Discard changes?". Saving stores at
  once and starts a sync of that account in the background; a failure to save says so and keeps the form.
- **Compose.** Suggestions come from `autocomplete` (contacts first, then recent recipients) for the sending account.
- **Privacy.** No contacts permission and no access to the phone's own contacts. Names, addresses and numbers are never logged;
  the state classes' `toString` leaves them out.

## Done when
- Unit tests (Robolectric): S40 lists A to Z with sections, search narrows the list, the account chips filter, the index jumps
  to a letter, the sync messages, the empty states; S41 shows the fields and its actions open compose and the dialler, Delete
  asks then deletes; S42 Save is disabled until a change, adding a contact saves to the chosen book, editing keeps the id,
  Discard asks; compose suggestions come from the contacts backend.
- The A to Z index has a gallery entry.
- CI green.
