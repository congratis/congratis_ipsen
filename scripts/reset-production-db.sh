#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DB_NAME="${1:-congratis}"
if [[ ! "$DB_NAME" =~ ^[A-Za-z0-9_-]+$ ]]; then
  echo "Invalid database name: $DB_NAME" >&2
  exit 1
fi

docker compose config --quiet
echo "Building the existing Rust application image with the reset utility..."
docker compose build app

echo
echo "This will permanently delete every record from:"
echo "  $DB_NAME.ornaments"
echo "  $DB_NAME.auth_sessions"
echo "Tables, indexes, database and Docker volume will be preserved."
read -r -p "Type CLEAR $DB_NAME to continue: " CONFIRM
if [[ "$CONFIRM" != "CLEAR $DB_NAME" ]]; then
  echo "Cancelled. No data was changed."
  exit 0
fi

echo "Stopping the app to prevent concurrent writes..."
docker compose stop app

restart_app() {
  docker compose start app >/dev/null
}
trap restart_app EXIT

docker compose --profile tools run --rm rethinkdb-reset --confirm-database "$DB_NAME"

docker compose start app
trap - EXIT

echo
echo "Reset completed. Current containers:"
docker compose ps
