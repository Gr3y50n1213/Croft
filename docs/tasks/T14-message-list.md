# T14: Message list on fake data

Crate / folder: `android/`

Depends on: T12, T13. Does not need the core: it runs on a fake repository behind an interface that T16 later replaces with the T10 surface. Blocks: T15, T16.

## Read first
`docs/ux/build-order.md` (step 4), `docs/ux/message-list.md` (all), `docs/ux/navigation.md` (back rules, bottom bar, drawer), `docs/ux/state-machines.md` (selection mode, pull to refresh, account connection), `docs/ux/controls.md` (message row, banners, snackbar), `docs/ux/accessibility.md` (row announcement order, live regions, focus after removal), `docs/ux/theme.md` (density, list style), `docs/ux/motion.md`, `design/wireframes/` for S10 to S13, S17 to S19.

## Goal
Screens S10 Inbox, S11 Selection mode, S12 Navigation drawer, S13 Account switcher, S17 Pull to refresh and new mail, S18 Offline and sync problems, S19 Inbox zero.

## Spec
- A `MessageRepository` interface (list query as a paged flow with an anchor, change notifications, account list with status) and a `FakeMessageRepository` that can hold 5,000 rows, several accounts and every row state; it can inject new mail, go offline, and flip an account to Needs sign-in. T16 implements the same interface over the core.
- Row: avatar or initial, sender, subject, preview, time, markers (unread dot + bold, star, clip, lock, thread count chip, pending clock). Density (Comfortable, Default, Compact) and list style (Container, Flat) from the theme settings. Date headers are sticky and double as fast-scroller labels.
- Paging: 50 rows first, then 100 at a time when 15 rows from the end. Skeleton rows only for the first load of a folder, never on refresh; static when animations are reduced.
- Selection mode: long-press or avatar tap; contextual bar with count, select all (loaded page, plus "Select all N" offer), actions that apply to all selected; Back exits first; swipe and FAB disabled. (Swipe itself is T15; here the hooks only.)
- New mail: new rows above the viewport never move what the person reads; an "N new" pill appears (S17); at the very top rows slide in. Pull to refresh with the 80 dp arming threshold.
- Banners (one at a time, most urgent wins): Offline, Needs sign-in, server problem; account status chips (S13).
- Empty states: S19 with last-synced time; different copy for Trash, Spam and Search.
- Drawer (S12) with unread counts, accounts, Settings entry; account switcher (S13) from the avatar in the search pill. Category chips show only "All" until the later smart-inbox phase; leave the row of chips as a component.
- Scroll offset and anchor survive returning from a conversation, rotation and process death: keep it in `rememberSaveable` plus a persisted per-folder anchor (message id + offset), not a raw pixel value.
- Accessibility: row announcement order (state first), custom actions placeholder list, polite announcements for N new messages, focus moves to the next row after a removal.

## Done when
- Screenshot tests (T12 tooling): S10, S11, S12, S13, S17, S18, S19 in compact, medium and expanded widths, light and dark, font scale 1.0 and 2.0, LTR and RTL; plus each density.
- Compose tests: paging requests 100 more at 15 rows from the end; skeleton shows on first load and not on refresh; the "N new" pill appears when scrolled down and the first visible row stays put (assert its top offset before and after injection); selection mode rules (Back order, toolbar actions follow the selection's read state).
- State-restoration test: `StateRestorationTester` for rotation, and a Robolectric activity recreate that simulates process death, both keep the anchor row and offset; the persisted anchor test covers a cold start.
- Reduced-motion tests for row collapse, new-row slide-in and skeleton shimmer; an interrupt test for the pill.
- Semantics tests: row announcement order, 48 dp targets, labels on icon buttons.
- Airplane-mode test: with the fake offline, the list stays usable and the Offline banner shows.
- A 5,000-row fake renders its first frame under a generous JVM bound (a smoke test, not a performance claim).
- On a device (not checkable here): 120 or 60 fps scrolling with 5,000 rows on a mid-range phone (build the macrobenchmark module so it runs on a device later, but report the result as unverified), fast-scroller and pull-to-refresh feel, predictive-back animation.
