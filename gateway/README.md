# Croft push gateway

A small, stateless service (AGPL-3.0-or-later) that lets Croft Companions wake the **Play build** of the Croft app through Google's
Firebase Cloud Messaging (FCM). Only the app's publisher can send to the official app through FCM, so a Companion cannot do it
itself; the gateway does it for every Companion, without learning anything about mail.

F-Droid and GitHub builds do not use it: they get the Companion's pings through UnifiedPush directly. People who build their own
copy of the app with their own Firebase project run their own gateway.

## How it works

1. The app (Play build) gets an FCM registration token and sends it to the gateway: `POST /register` with `{"token": "..."}`. The
   gateway answers `{"endpoint": "https://<gateway>/wp/<sealed>"}`: the token sealed with the gateway's key (AES-256-GCM). The
   gateway stores nothing.
2. The app subscribes at its Companion with that endpoint and its own Web Push keys, exactly as a browser would (T28).
3. When the user's mail changes, the Companion sends its Web Push message (RFC 8291, encrypted for the device) to the endpoint. The
   gateway opens the endpoint, and hands the encrypted bytes to FCM unchanged as a data message (`data.wp`, base64url). The phone
   decrypts them with its own keys and syncs.

The gateway sees that some device is woken at some time. It never sees a mail, an address or a Companion's user, and it never
logs a token, an endpoint, a message or a client address: only counts every ten minutes.

| Request | Answer |
|---|---|
| `POST /register` `{"token": "<FCM token>"}` | `200` `{"endpoint": ...}`; `400` for a token that is not one; `429` over the limit (30 an hour per address) |
| `POST /wp/{sealed}` (a Web Push message: `Content-Encoding: aes128gcm`, `TTL`, optional `Urgency`; body at most 2048 bytes) | `201` FCM took it; `410` the token is no more (the Companion drops the subscription); `404` not this gateway's endpoint; `400` not a Web Push message; `413` too large; `429` over the limit (60 an hour per device, 10 at once); `502` FCM failed |
| `GET /health` | `200` `ok` |

`Urgency: high` becomes FCM's high priority (anything else normal); `TTL` is passed on, at most one day.

## Running it

It needs:
- **A key**: `croft-push-gateway generate-key > gateway.key` (64 hexadecimal characters). Keep it: endpoints made with one key do not
  work with another, so a new key means every phone registers again.
- **A Google service account key** (JSON) of the Firebase project the app is built with, with the Firebase Cloud Messaging API
  enabled and a role that may send (Firebase Cloud Messaging API Admin). Never commit it.
- **A public https address**, through a reverse proxy that terminates TLS.

```
docker build -f gateway/Dockerfile -t croft-push-gateway .
docker run -d -p 127.0.0.1:8490:8490 -v /srv/croft-gateway/keys:/keys:ro \
  -e CROFT_GATEWAY_PUBLIC_URL=https://push.example.org croft-push-gateway
```

Settings (environment, or a TOML file named by `CROFT_GATEWAY_CONFIG` with the same names in lower case without the prefix):

| Variable | Default | Meaning |
|---|---|---|
| `CROFT_GATEWAY_PUBLIC_URL` | (required) | The gateway's public `https` address |
| `CROFT_GATEWAY_LISTEN` | `0.0.0.0:8490` | Where to listen (plain HTTP) |
| `CROFT_GATEWAY_KEY_FILE` | `/keys/gateway.key` | The key |
| `CROFT_GATEWAY_SERVICE_ACCOUNT_FILE` | `/keys/service-account.json` | The service account key |
| `CROFT_GATEWAY_FCM_BASE` | `https://fcm.googleapis.com` | FCM's address |
| `CROFT_GATEWAY_TRUSTED_PROXIES` | (none) | Comma-separated proxy addresses whose `X-Forwarded-For` is believed |
| `CROFT_GATEWAY_PUSH_PER_HOUR`, `CROFT_GATEWAY_PUSH_BURST` | `60`, `10` | Messages per device |
| `CROFT_GATEWAY_REGISTER_PER_HOUR` | `30` | Registrations per client address |

Behind a reverse proxy, list it in `CROFT_GATEWAY_TRUSTED_PROXIES`, or every client counts as the proxy's address.

## Development

```
cd gateway && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests run against a fake Google token endpoint (which verifies the signed JWT) and a fake FCM on this machine, with a made-up
service account generated at test time. Real delivery through FCM needs the publisher's Firebase project and the Play build on a
phone (`docs/OPEN-ITEMS.md`).
