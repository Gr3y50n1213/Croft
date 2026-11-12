# T29: The thin push gateway (FCM) and the Android flavours

Folders: `gateway/` (T29a, a separate service, AGPL-3.0) and `android/` (T29b).

Depends on: T28 (the Companion sends Web Push pings). Blocks T34 (phones register for push through the Companion).

Built in two pull requests: **T29a** the gateway, **T29b** the Android build flavours and their checks.

## Read first
`docs/PLAN.md` ("Review decisions": run a thin push gateway, never a mail-watching service), `docs/tasks/T28-companion-push-relay.md`,
`docs/store/READINESS.md` (rows 15 and the Data safety table: the push token), RFC 8030 (Web Push), the FCM HTTP v1 API
(`projects.messages.send`) and Google's OAuth 2.0 for service accounts (a signed JWT exchanged for an access token).

## Goal
Only the app's publisher can send to the official app through Google's push (FCM): a Companion cannot. The gateway lets every Companion
wake Play-store phones without learning anything itself: it takes the Companion's Web Push message, already encrypted for the device
(RFC 8291), and hands the same bytes to FCM. The Companion needs no change (the gateway is one more Web Push endpoint); the phone
decrypts with its own Web Push keys (T34). F-Droid and GitHub builds never contain Google code and use UnifiedPush.

## Spec (T29a, the gateway)
- **A Web Push endpoint per device, without state.** `POST /register` with `{"token": "<FCM registration token>"}` answers
  `{"endpoint": "<public URL>/wp/<sealed>"}`: the token sealed (AES-256-GCM, the gateway's key, a fixed context) and base64url
  encoded. The gateway stores nothing: the endpoint carries the token, and only the gateway can open it. A token is at most 4096
  characters of `[A-Za-z0-9:_-]`.
- **Forwarding.** `POST /wp/{sealed}` (what a Companion sends, T28): `Content-Encoding: aes128gcm` required, the body at most 2048
  bytes (FCM's data limit is 4 KB after base64). The body goes as an FCM data message (`data: {"wp": <base64url body>}`), with
  `android.priority` from `Urgency` (`high` for `high`, else `normal`) and `android.ttl` from `TTL` (at most 1 day). Answers `201` when
  FCM accepted it; `410` when FCM says the token is no more (`UNREGISTERED`, or `INVALID_ARGUMENT` about the token), so the Companion
  drops the subscription; `404` for an endpoint the gateway did not make; `413` for a large body; `429` over the limits; `502` when FCM
  failed otherwise or could not be reached.
- **Limits.** Per endpoint (60 an hour, burst 10) and per client address for `/register` (30 an hour); all in memory, bounded.
- **FCM authentication.** A Google service account JSON (path in the config, never in the repository): `client_email`, `private_key`
  (PKCS#8 PEM, RSA), `token_uri`, `project_id`. An RS256 JWT for the scope `https://www.googleapis.com/auth/firebase.messaging` is
  exchanged at `token_uri` for an access token, kept until 5 minutes before it expires. Requests go through the core's HTTPS client
  (`croft-protocols` `fetch`), to the admin's configured hosts (Google's by default).
- **Privacy.** The gateway never logs a token, an endpoint or a body; it logs counts and status codes. It sees only that some device is
  pinged at some time (as PLAN says: device tokens and timing, never mail or addresses).
- **Running it.** Config from a TOML file and environment: listen address, public URL, the key file (made by
  `croft-push-gateway generate-key`, 32 random bytes, hex), the service account path, the FCM base URL. `GET /health`. A Docker image.
  Its own CI job (fmt, clippy, tests).

## Spec (T29b, the Android flavours)
- Three product flavours: **`play`** (FCM, `com.google.firebase:firebase-messaging`, without the google-services plugin: Firebase is
  set up from build values, empty by default so a build without them has no push), **`fdroid`** and **`github`** (no Google libraries).
- Checks in CI: the `fdroid` and `github` runtime classpaths contain no `com.google.firebase`, `com.google.android.gms` or
  `com.google.android.datatransport` artefact, and their merged manifests declare no FCM service or receiver; the `play` manifest has
  no permission beyond what READINESS lists.
- `docs/store/READINESS.md` Data safety: the push token row says when it is collected (Play build, with a Companion, push turned on).
- CI, the preview build and `docs/TESTING-ON-A-PHONE.md` move to the flavour task names.
- **Left out:** registering for push on the phone and decrypting pings (T34, where phones first connect to a Companion; the same for
  UnifiedPush).

## Done when
- T29a: against a fake Google token endpoint and a fake FCM on this machine: a registered endpoint forwards a Web Push message whose
  bytes FCM receives unchanged, with the access token from a JWT that verifies with the service account's public key; the token is
  fetched once and reused; priority and TTL are mapped; `UNREGISTERED` gives `410`; a forged, altered or other gateway's endpoint gives
  `404`; the limits give `429`; nothing secret is logged. Clippy is clean. An independent review (security-sensitive: it holds Google
  credentials and is open to the internet).
- T29b: the four builds assemble in CI and the checks fail when a Google library is added to `fdroid` (shown once, then removed).
- Not checkable here: real FCM delivery (needs the owner's Firebase project and the Play build on a phone, OPEN-ITEMS).
