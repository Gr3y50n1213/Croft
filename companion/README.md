# Croft Companion

The optional, self-hosted, multi-user server of Croft (roadmap phase 2). It will serve the web app, instant push and server-side
search; phones work without it. Licence: AGPL-3.0-or-later (see `LICENSES.md`).

**Status:** foundation only (T23): settings, storage, per-user encryption, the command line and a health endpoint. No sign-in and no
mail yet; see `docs/tasks/PHASE-2.md`.

## Run it with Docker

```
cp companion/docker-compose.example.yml docker-compose.yml   # then set CROFT_COMPANION_PUBLIC_URL
docker compose run --rm companion init                        # creates the master key and the database
docker compose run --rm companion user add --name admin --admin
docker compose up -d
```

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
- The Companion has no slow-client protection of its own yet: keep it behind the reverse proxy (which has header timeouts) and do not
  publish port 8480 to the internet.

## Commands

```
croft-companion init
croft-companion user add --name NAME [--admin]
croft-companion user list
croft-companion rotate-master-key [--finish]
croft-companion serve
croft-companion healthcheck
```
