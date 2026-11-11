#!/usr/bin/env bash
# Completes Stalwart's bootstrap and creates two test users. For the Stalwart container in
# ../docker-compose.yml, on a fresh container (`down -v` first); running it again on a container that
# is already set up does nothing.
#
#   test-servers/stalwart/setup.sh [HTTP_PORT]        # default 8080
#
# Environment:
#   CROFT_COMPOSE  the compose command, so the right project and file are restarted. Default:
#                  docker compose -f <this folder>/../docker-compose.yml
#                  (use `docker compose -p NAME -f ...` when the container was started with -p NAME).
#
# What it does, through Stalwart 1.x's own JMAP management API (registry objects, `x:` methods):
#   1. Bootstrap/set: host name localhost, domain example.org, no ACME certificate, no DKIM keys.
#   2. Restarts the container (the server only leaves bootstrap mode on a restart).
#   3. Account/set: alice@example.org and bob@example.org.
# Everything here is throwaway test data for localhost.
set -euo pipefail

PORT="${1:-8080}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE="${CROFT_COMPOSE:-docker compose -f $HERE/../docker-compose.yml}"
BASE="http://127.0.0.1:$PORT"
RECOVERY="admin:croft-recovery-admin-4d1f9c"   # STALWART_RECOVERY_ADMIN in docker-compose.yml
ALICE="alice@example.org:croft-alice-purple-orbit-7421"
BOB_PW="croft-bob-yellow-lantern-5530"

json() { python3 -I -c "$1"; }

wait_for() { # wait_for CREDENTIAL: until the session answers 200 with it
  for _ in $(seq 1 60); do
    code=$(curl -sS -m 3 -o /dev/null -w '%{http_code}' -u "$1" "$BASE/jmap/session" 2>/dev/null || true)
    [ "$code" = "200" ] && return 0
    sleep 1
  done
  echo "setup.sh: the server did not answer on $BASE" >&2
  return 1
}

call() { # call CREDENTIAL METHODCALLS_JSON
  curl -sS -m 30 -u "$1" -H 'Content-Type: application/json' \
    -d "{\"using\":[\"urn:ietf:params:jmap:core\",\"urn:stalwart:jmap\"],\"methodCalls\":$2}" "$BASE/jmap/"
}

# Already set up?
if [ "$(curl -sS -m 3 -o /dev/null -w '%{http_code}' -u "$ALICE" "$BASE/jmap/session" 2>/dev/null || true)" = "200" ]; then
  echo "setup.sh: already set up"
  exit 0
fi

wait_for "$RECOVERY"
OUT=$(call "$RECOVERY" '[["x:Bootstrap/set",{"accountId":"d333333","update":{"singleton":{"defaultHostname":"localhost","defaultDomain":"example.org","requestTlsCertificate":false,"generateDkimKeys":false}}},"a"]]')
ADMIN=$(printf '%s' "$OUT" | json 'import sys,json
r=json.load(sys.stdin)["methodResponses"][0][1]["updated"]["singleton"]
print(r["username"]+":"+r["secret"])')

$COMPOSE restart stalwart >/dev/null
wait_for "$ADMIN"

PID=$(curl -sS -m 5 -u "$ADMIN" "$BASE/jmap/session" | json 'import sys,json
print(json.load(sys.stdin)["primaryAccounts"]["urn:stalwart:jmap"])')
DOMAIN_ID=$(call "$ADMIN" "[[\"x:Domain/get\",{\"accountId\":\"$PID\",\"ids\":null},\"a\"]]" | json 'import sys,json
print(json.load(sys.stdin)["methodResponses"][0][1]["list"][0]["id"])')

OUT=$(call "$ADMIN" "[[\"x:Account/set\",{\"accountId\":\"$PID\",\"create\":{\"a\":{\"@type\":\"User\",\"name\":\"alice\",\"domainId\":\"$DOMAIN_ID\",\"credentials\":{\"0\":{\"@type\":\"Password\",\"secret\":\"${ALICE#*:}\"}}},\"b\":{\"@type\":\"User\",\"name\":\"bob\",\"domainId\":\"$DOMAIN_ID\",\"credentials\":{\"0\":{\"@type\":\"Password\",\"secret\":\"$BOB_PW\"}}}}},\"c\"]]")
printf '%s' "$OUT" | json 'import sys,json
r=json.load(sys.stdin)["methodResponses"][0][1]
assert "created" in r and not r.get("notCreated"), "could not create the users"'
wait_for "$ALICE"
echo "setup.sh: ready (alice@example.org, bob@example.org)"
