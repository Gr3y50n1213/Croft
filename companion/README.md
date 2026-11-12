# Croft Companion

The optional, self-hosted, multi-user server of Croft (roadmap phase 2). It will serve the web app, instant push and server-side
search; phones work without it. Licence: AGPL-3.0-or-later (see `LICENSES.md`).

**Status:** the foundation (T23) and sign-in (T24): passkeys, an authenticator app with recovery codes, tokens for the apps, and the
admin's user management. No mail yet; see `docs/tasks/PHASE-2.md`.

## Run it with Docker

```
cp companion/docker-compose.example.yml docker-compose.yml   # then set CROFT_COMPANION_PUBLIC_URL
docker compose run --rm companion init                        # creates the master key and the database
docker compose run --rm companion user add --name admin --admin   # prints a one-time enrolment link
docker compose up -d
```

Open the printed link in a browser to make the admin's passkey; the page then shows an authenticator-app key and ten recovery codes,
once. The link works once, for 24 hours. Its secret part is after the `#`, which browsers never send to a server, so it does not end
up in the proxy's logs; still, pass it to the person privately.

The server speaks plain HTTP on port 8480 and must sit behind a reverse proxy that terminates TLS (Caddy, nginx, Traefik). The
public address must be `https://` (plain `http` is accepted only for localhost).

## Settings

A TOML file (`--config FILE`) and environment variables; the variable wins.

| Setting | Variable | Default |
|---|---|---|
| `listen` | `CROFT_COMPANION_LISTEN` | `127.0.0.1:8480` |
| `public_url` | `CROFT_COMPANION_PUBLIC_URL` | required |
| `data_dir` | `CROFT_COMPANION_DATA_DIR` | `/data` |
| `master_key_file` | `CROFT_COMPANION_MASTER_KEY_FILE` | `/keys/master.key` (must be outside `data_dir`) |
| `trusted_proxies` | `CROFT_COMPANION_TRUSTED_PROXIES` (comma-separated) | none |

`public_url` must use a domain name (passkeys do not work with an IP address) and is the address passkeys are bound to: changing it
later means everyone enrols again. `trusted_proxies` lists the reverse proxy's own addresses (as the Companion sees them, for example
the proxy container's address); only from those is `X-Forwarded-For` believed, so rate limits count the real client and not the proxy.

## The master key

Every user's data (from later tasks: mail passwords, tokens, cached mail) is encrypted with that user's own data key, and every data
key is encrypted with the **master key**. The master key is a file, never in the database.

- **Back it up**, separately from the data. Without it no user's data can be read, by anyone.
- Keep it readable by the Companion only: a file other users can read, or a symbolic link, is refused (`chmod 600`).
- It must be outside the data folder (the Docker image keeps it on its own `/keys` volume), so one stolen backup does not hold both
  and nobody who can write the data folder can replace it.
- A key is accepted only if it opens every user's key.
- `croft-companion rotate-master-key` (with the server stopped) re-wraps every user's key under a new master key and replaces the
  file; back up the new one. If a rotation is cut short, the Companion will not start and says so: check the key file and run
  `croft-companion rotate-master-key --finish`. A leftover key is never adopted on its own.
- The Companion drops a client that does not send its request headers within 10 seconds or its body within 10 seconds, closes any
  connection after two minutes (once its current request is done), and serves at most 512 connections at once and 32 from one
  address (a trusted proxy is not limited). Still keep it behind the reverse proxy and do not publish port 8480 to the internet.

## Signing in

- **Passkeys** are how people sign in. Each person can register more than one (another phone, a security key) once signed in.
- **Authenticator app and recovery codes** are the backup for a device that cannot use a passkey. A code from the app is never enough on
  its own: it must come with one of the ten recovery codes, each usable once, and that sign-in can only register a new passkey (it
  lasts 15 minutes and cannot be renewed).
- **Devices.** Each sign-in is a device the person can see and sign out remotely. An app holds an access token (15 minutes) and a
  refresh token (30 days, replaced on every use); if a used refresh token ever comes back, that device is signed out at once.
- **Limits.** Ten failed authenticator-app and recovery-code attempts in 15 minutes lock that way of signing in for that user for 15
  minutes; a passkey still works (passkey answers cannot be guessed, so they never lock anyone). One address gets 30 sign-in requests a
  minute and 20 failures in 15 minutes.
- **Lost everything?** Whoever runs the server gives the person a new enrolment link: `croft-companion user enrol --name NAME`.
  Using it replaces the user's passkeys, authenticator key and recovery codes and signs out every device. This is also how an admin who
  lost their own passkeys gets back in. It is deliberately not in the admin API: whoever holds a user's link can become that user, and
  an admin must never be able to act as someone else. (The server's operator holds the master key, so nothing is hidden from them.)
- **Admins** can list, disable and enable users and read the instance's audit log. They cannot read anyone's data or sign in as them.

## Commands

```
croft-companion init
croft-companion user add --name NAME [--admin]
croft-companion user enrol --name NAME
croft-companion user list
croft-companion rotate-master-key [--finish]
croft-companion serve
croft-companion healthcheck
```

## API (for the apps)

JSON over HTTPS; errors are `{"error": "<code>"}` and never say which part of a sign-in was wrong.

| Route | Who |
|---|---|
| `POST /api/enrol/start`, `/api/enrol/finish` | the enrolment page |
| `POST /api/sign-in/passkey/start` (`{"user"?}`), `/api/sign-in/passkey/finish` | anyone |
| `POST /api/sign-in/recovery` (`{"user", "totp", "recovery_code"}`) | anyone |
| `POST /api/token/refresh` (`{"refresh_token"}`) | anyone |
| `GET /api/me`, `POST /api/sign-out`, `POST /api/passkeys/start`, `/api/passkeys/finish` | signed in (also a recovery sign-in) |
| `GET /api/passkeys`, `DELETE /api/passkeys/{id}`, `GET /api/sessions`, `DELETE /api/sessions/{id}`, `GET /api/audit` | signed in |
| `GET /api/admin/users`, `POST /api/admin/users/{id}/disable`, `/enable`, `GET /api/admin/audit` | admins |
