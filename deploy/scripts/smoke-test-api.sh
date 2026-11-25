#!/usr/bin/env bash
set -Eeuo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1}"
WORK_DIR="$(mktemp -d)"
COOKIE_JAR="$WORK_DIR/cookies.txt"
BODY="$WORK_DIR/body.json"
HEADERS="$WORK_DIR/headers.txt"
trap 'rm -rf "$WORK_DIR"' EXIT

request() {
  local expected="$1"
  shift
  local actual
  actual="$(curl --silent --show-error --output "$BODY" --dump-header "$HEADERS" --write-out '%{http_code}' "$@")"
  if [[ "$actual" != "$expected" ]]; then
    echo "Expected HTTP $expected, received $actual" >&2
    cat "$BODY" >&2
    exit 1
  fi
}

echo "Checking unauthenticated API access..."
request 401 "$BASE_URL/api/me"

echo "Creating a disposable one-time invitation..."
invitation="$(docker compose exec -T app generate-tokens 1 | tail -n 1)"
IFS=, read -r ornament token <<< "$invitation"
[[ "$ornament" == "1" && "${#token}" == "64" ]]

echo "Exchanging invitation for a participant session..."
payload="$(jq -cn --arg token "$token" '{token:$token}')"
request 200 --request POST "$BASE_URL/api/auth/participant/exchange" \
  --header 'Content-Type: application/json' --data "$payload" \
  --cookie-jar "$COOKIE_JAR"
csrf="$(jq -er '.csrf_token' "$BODY")"

echo "Checking CSRF enforcement..."
request 403 --request POST "$BASE_URL/api/greetings" \
  --header 'Content-Type: application/json' --cookie "$COOKIE_JAR" \
  --data '{"display_name":"CI","message":"missing CSRF must be rejected"}'

echo "Submitting a greeting through the real API..."
greeting='{"display_name":"CI Participant","message":"Real HTTP and RethinkDB integration works"}'
request 201 --request POST "$BASE_URL/api/greetings" \
  --header 'Content-Type: application/json' --header "X-CSRF-Token: $csrf" \
  --cookie "$COOKIE_JAR" --data "$greeting"

echo "Checking duplicate submission protection..."
request 409 --request POST "$BASE_URL/api/greetings" \
  --header 'Content-Type: application/json' --header "X-CSRF-Token: $csrf" \
  --cookie "$COOKIE_JAR" --data "$greeting"

echo "Verifying persisted greeting snapshot..."
request 200 --get "$BASE_URL/api/greetings" --cookie "$COOKIE_JAR"
jq -e 'length == 1 and .[0].ornament_number == 1 and .[0].display_name == "CI Participant" and .[0].message == "Real HTTP and RethinkDB integration works"' "$BODY" >/dev/null

echo "Revoking the participant session..."
request 204 --request POST "$BASE_URL/api/auth/logout" \
  --header "X-CSRF-Token: $csrf" --cookie "$COOKIE_JAR"
request 401 "$BASE_URL/api/me" --cookie "$COOKIE_JAR"

echo "Backend HTTP integration smoke test passed."
