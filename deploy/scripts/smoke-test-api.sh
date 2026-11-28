#!/usr/bin/env bash
set -Eeuo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1}"
WORK_DIR="$(mktemp -d)"
COOKIE_JAR="$WORK_DIR/cookies.txt"
BODY="$WORK_DIR/body.json"
HEADERS="$WORK_DIR/headers.txt"
SSE_BODY="$WORK_DIR/events.txt"
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

echo "Creating disposable one-time invitations..."
invitation="$(docker compose exec -T app generate-tokens 20 | awk -F, '$1 == 1 { line = $0 } END { print line }')"
IFS=, read -r ornament token <<< "$invitation"
[[ "$ornament" == "1" && "${#token}" == "64" ]]

echo "Exchanging invitation for a participant session..."
payload="$(jq -cn --arg token "$token" '{token:$token}')"
request 200 --request POST "$BASE_URL/api/auth/participant/exchange" \
  --header 'Content-Type: application/json' --data "$payload" \
  --cookie-jar "$COOKIE_JAR"
csrf="$(jq -er '.csrf_token' "$BODY")"

echo "Checking participant role restrictions..."
request 403 "$BASE_URL/api/admin/participants" --cookie "$COOKIE_JAR"

echo "Checking CSRF enforcement..."
request 403 --request POST "$BASE_URL/api/greetings" \
  --header 'Content-Type: application/json' --cookie "$COOKIE_JAR" \
  --data '{"surname":"CI","first_name":"Participant","message":"missing CSRF must be rejected"}'

echo "Opening authenticated SSE stream..."
curl --silent --show-error --no-buffer --max-time 8 \
  --cookie "$COOKIE_JAR" "$BASE_URL/api/events" > "$SSE_BODY" &
SSE_PID=$!
sleep 1

echo "Submitting a greeting through the real API..."
greeting='{"surname":"CI","first_name":"Participant","message":"Real HTTP and RethinkDB integration works"}'
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

echo "Verifying live Changefeed-to-SSE delivery..."
set +e
wait "$SSE_PID"
set -e
grep -Fq 'event: greeting' "$SSE_BODY"
grep -Fq '"display_name":"CI Participant"' "$SSE_BODY"
grep -Fq '"message":"Real HTTP and RethinkDB integration works"' "$SSE_BODY"

echo "Revoking the participant session..."
request 204 --request POST "$BASE_URL/api/auth/logout" \
  --header "X-CSRF-Token: $csrf" --cookie "$COOKIE_JAR"
request 401 "$BASE_URL/api/me" --cookie "$COOKIE_JAR"
request 401 "$BASE_URL/api/presence" \
  --header 'Connection: Upgrade' --header 'Upgrade: websocket' \
  --header 'Sec-WebSocket-Version: 13' \
  --header 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ=='

admin_password='CI-Only-Admin-Password-2026'
echo "Authenticating administrator and checking management endpoints..."
admin_payload="$(jq -cn --arg username admin --arg password "$admin_password" '{username:$username,password:$password}')"
request 200 --request POST "$BASE_URL/api/auth/admin/login" \
  --header 'Content-Type: application/json' --data "$admin_payload" \
  --cookie-jar "$COOKIE_JAR"
admin_csrf="$(jq -er '.csrf_token' "$BODY")"

echo "Checking authenticated WebSocket upgrade through Nginx..."
set +e
curl --silent --show-error --http1.1 --max-time 2 \
  --dump-header "$HEADERS" --output /dev/null \
  --header 'Connection: Upgrade' --header 'Upgrade: websocket' \
  --header 'Sec-WebSocket-Version: 13' \
  --header 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  --cookie "$COOKIE_JAR" "$BASE_URL/api/presence"
ws_curl_status=$?
set -e
if ! grep -Eiq '^HTTP/1\.1 101' "$HEADERS"; then
  echo "Expected authenticated WebSocket HTTP 101 upgrade (curl exit: $ws_curl_status)" >&2
  cat "$HEADERS" >&2
  exit 1
fi

request 200 "$BASE_URL/api/admin/participants" --cookie "$COOKIE_JAR"
jq -e 'length == 20 and .[0].ornament_number == 1 and .[0].submitted == true' "$BODY" >/dev/null

echo "Reissuing and revoking an unused invitation..."
request 200 --request POST "$BASE_URL/api/admin/invitations/2/reissue" \
  --header "X-CSRF-Token: $admin_csrf" --cookie "$COOKIE_JAR"
jq -e '.ornament_number == 2 and (.token | length == 64)' "$BODY" >/dev/null
request 204 --request POST "$BASE_URL/api/admin/invitations/2/revoke" \
  --header "X-CSRF-Token: $admin_csrf" --cookie "$COOKIE_JAR"

echo "Backend HTTP, admin and live SSE integration tests passed."
