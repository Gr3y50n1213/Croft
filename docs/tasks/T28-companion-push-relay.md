# T28: Push relay in the Companion

Crate / folder: `companion/` (and a small extension of the HTTPS client in `core/protocols`).

Depends on: T26 (each user's core watches their accounts), T27 (the apps use JMAP). Blocks T29 (the FCM gateway forwards these pings)
and T34 (phones register for them).

## Read first
`docs/PLAN.md` ("Notifications", "Review decisions": the thin push gateway), `docs/tasks/PHASE-2.md`, RFC 8030 (Web Push), RFC 8291
(message encryption), RFC 8292 (VAPID), the UnifiedPush specification (an app server POSTs Web Push messages to the endpoint the
distributor gave the app), `docs/DECISIONS.md` (T26 rows: the address policy).

## Goal
When a user's mail changes, the Companion wakes that user's apps with a ping that says nothing about the mail (the app then syncs
itself). One mechanism covers browsers (Web Push, for the web client of T30) and Android without Google (UnifiedPush, which uses the
same Web Push messages); the FCM gateway of T29 will relay the same ping for the Play build.

## Spec
- **Subscriptions** (full sessions only): `POST /api/push/subscriptions` (`{endpoint, keys: {p256dh, auth}}`, as a browser's
  `PushSubscription.toJSON()` and UnifiedPush give them) → an id; `GET /api/push/subscriptions`; `DELETE /api/push/subscriptions/{id}`;
  `GET /api/push/vapid` → the Companion's VAPID public key (the `applicationServerKey` a browser needs). A subscription belongs to the
  device (session) that made it: signing that device out, or disabling the user, stops its pings. At most 10 per user. The endpoint and
  keys are sealed with the user's data key (the endpoint is a capability: whoever has it can wake the device).
- **The ping**: `{"v":1,"type":"changed"}` and nothing else (no account, address, subject or count), encrypted per RFC 8291 (`aes128gcm`,
  a new ephemeral P-256 key and salt per message), with VAPID (RFC 8292: an ES256 JWT for the endpoint's origin, 12 hours, `sub` the
  Companion's public URL) and `TTL: 3600`, `Urgency: normal`.
- **When**: each user's core reports changes (`next_change`); a change starts a ping to all that user's live subscriptions, at most one per
  30 seconds per user (later changes in the window fold into one ping at its end).
- **Delivery**: through the core's HTTPS client to **public addresses only** (an endpoint is chosen by a client, so it must not reach
  the Companion's own network, even where `mail_allowed_networks` lets mail servers do so); https only; no redirects; to all the user's
  devices at once; answers `2xx` ok, `404`/`410` remove the subscription, other refusals counted and the subscription removed after 10 in a
  row over at least 3 days; an unreachable service, `5xx`, `408` or `429` counts nothing. A device that signs out (however it is signed
  out) loses its subscriptions.
- **The VAPID key**: an ES256 key made at first use, kept in the Companion's database sealed with the master key.
- **Left out**: the Android flavours registering a UnifiedPush distributor and the phones subscribing (T34, where phones connect to the
  Companion); the FCM gateway (T29).

## Done when
- RFC 8291's example (Appendix A) is reproduced by the encryption; a VAPID JWT verifies with the published key and has the right `aud`,
  `exp` and `sub`.
- With a fake push service on this machine: a change in a user's core leads to one encrypted ping that the subscription's private key
  decrypts to the fixed content-free body; changes within the window give one ping; `410` removes the subscription; signing the device
  out stops its pings.
- Isolation: one user's subscriptions never receive another user's pings and cannot be listed or deleted by another user.
- An endpoint on the Companion's own network is refused, even with every address allowed for mail.
- Not checkable here: real push services (Mozilla, Google, Apple for browsers; ntfy and other UnifiedPush distributors).
