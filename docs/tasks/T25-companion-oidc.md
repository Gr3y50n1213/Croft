# T25: Companion sign-in with OIDC

Crate / folder: `companion/`

Depends on: T24. Blocks nothing (an alternative to passkeys), but T30's web sign-in offers it when it is set up.

## Read first
`docs/PLAN.md` (Security: "Companion sign-in"), `docs/tasks/T24-companion-sign-in.md`, `companion/README.md` ("Signing in", "API"),
`docs/DECISIONS.md` (the T24 rows: tokens, sessions, limits, the recovery session).

## Goal
A Companion run next to an identity provider (Authelia, Authentik, Keycloak, Microsoft Entra ID) lets people sign in through that
provider instead of a passkey. What the provider proves ends in the same Companion session and tokens as T24, with the same devices,
limits, audit and admin rules.

## Spec
- **Settings.** An optional `[oidc]` section: the issuer URL (https only), the client id, a display name for the button, and whether
  unknown people may be created (`create_users = false` by default; when true, only members of a configured group claim). The client
  secret is a file (like the master key: outside the data folder, mode 0600, never in the TOML, never printed). Unknown keys refused.
- **Discovery.** `/.well-known/openid-configuration` from the issuer, fetched at start and cached; its `issuer` must equal the
  configured one exactly; every endpoint must be https on any host (Entra uses several). The JWKS is cached and refetched at most once a
  minute when a token names an unknown key id.
- **Flow.** Authorization Code with PKCE (S256) and `state` and `nonce`, each random, single use, 10 minutes, kept in memory like T24's
  ceremonies. Only `response_type=code`; `response_mode` query. The redirect URI is `{public_url}/oidc/callback`, fixed.
- **ID token.** Signature checked with the provider's JWKS (RS256, ES256, EdDSA; `none` and every HMAC algorithm refused, and the key's
  type must match the algorithm); `iss` exact, `aud` contains the client id (and `azp` equals it when there are several audiences),
  `exp`, `iat` and `nbf` with 60 seconds of leeway, `nonce` equal to the one sent. The identity is (`iss`, `sub`), never the e-mail.
- **Linking.** A signed-in user links their provider identity on a settings page (it starts the flow and stores `iss`/`sub` against
  their user id); an admin can unlink. With `create_users`, a first sign-in by a member of the group creates a user (role `user`, a name
  from `preferred_username` cleaned to T24's name rule, made unique). An unknown identity otherwise gets "not linked", audited.
- **Handing over to an app.** The browser lands on the callback; the Companion then hands a one-time code (60 seconds, single use, bound
  to the PKCE challenge the app sent at the start) to the app's redirect: the web app's own origin (`public_url`), or the phones' claimed
  `https` link / loopback address (T34). The app exchanges code + verifier at `/api/sign-in/oidc/exchange` for T24 tokens. Tokens never
  appear in a URL.
- **Rules kept from T24.** A disabled or locked user is refused after the provider says yes; failures count towards the lockout and
  the address limits; every sign-in, link, unlink and refusal is audited; OIDC sessions are full sessions (not recovery ones).
- **No passkey needed.** A user created by OIDC can work with OIDC alone; they may add a passkey later. Enrolment links still work.

## Done when
- An in-process fake provider (made-up keys; discovery, JWKS, token endpoint) drives the whole flow in tests, from start to the app's
  token exchange.
- Refused, each with a test: wrong `iss`, wrong `aud`, a missing or wrong `azp` with several audiences, expired, not yet valid, a wrong
  or replayed `nonce`, a replayed or unknown `state`, `alg: none`, HS256 signed with the public key (algorithm confusion), an unknown key
  id after one refetch, a discovery document whose issuer differs, an http endpoint, a replayed or expired hand-over code, a wrong PKCE
  verifier at the exchange.
- `create_users` off: an unknown identity is refused and audited; on: only with the group claim.
- The cross-user test of T24 covers the new routes; no secret (client secret, codes, tokens, ID token) reaches a log, an error or the
  audit log.
- Not checkable here: real providers (Authelia, Authentik, Keycloak, Entra). Reported as unverified, with the settings each needs in the
  README.
