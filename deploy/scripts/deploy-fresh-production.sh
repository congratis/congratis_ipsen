#!/usr/bin/env bash
set -Eeuo pipefail

# Destructive first deployment for the production Congratis stack.
# Keeps the Docker named volume and RethinkDB schema, but deletes all
# application rows from ornaments, auth_sessions and campaigns.

ROOT="/opt/congratis"
DOMAIN="congratis.online"
DATABASE="congratis"

cd "$ROOT"

if [[ ! -d .git || ! -f docker-compose.yml || ! -f .env ]]; then
  echo "Expected Git checkout, docker-compose.yml and existing .env under $ROOT." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Working tree is not clean. Commit or safely preserve local changes before deploying." >&2
  git status --short
  exit 1
fi

if [[ "$(git remote get-url origin)" != "git@github.com:congratis/congratis_ipsen.git" ]]; then
  echo "Unexpected origin remote. Refusing to deploy from this checkout." >&2
  exit 1
fi

if ! grep -Eq '^DOMAIN=["]?congratis[.]online["]?$' .env \
  || ! grep -Eq '^TLS_MODE=["]?globalsign["]?$' .env \
  || ! grep -Eq '^PUBLIC_BASE_URL=["]?https://congratis[.]online["]?$' .env \
  || ! grep -Eq '^COOKIE_SECURE=["]?true["]?$' .env \
  || ! grep -Eq '^RETHINKDB_DB=["]?congratis["]?$' .env; then
  echo ".env must target congratis.online with GlobalSign TLS, secure cookies and RETHINKDB_DB=congratis." >&2
  exit 1
fi

sudo test -s "/etc/congratis/tls/live/$DOMAIN/fullchain.pem"
sudo test -s "/etc/congratis/tls/live/$DOMAIN/privkey.pem"

echo "This operation deploys main and irreversibly clears all rows in:"
echo "  $DATABASE.ornaments"
echo "  $DATABASE.auth_sessions"
echo "  $DATABASE.campaigns"
echo "The RethinkDB database, tables, indexes and Docker volume remain in place."
echo "The app will recreate its protected legacy-default system campaign on startup."
echo "No database backup will be made by this script."
echo
read -r -p "Type DEPLOY FRESH $DOMAIN to continue: " CONFIRM
if [[ "$CONFIRM" != "DEPLOY FRESH $DOMAIN" ]]; then
  echo "Cancelled. No deployment or database changes were made."
  exit 0
fi

git fetch origin
git switch main
git pull --ff-only origin main

docker compose config --quiet
docker compose up -d --build --pull never --remove-orphans

echo "Waiting for RethinkDB and the HTTPS reverse proxy..."
for attempt in $(seq 1 60); do
  db_id="$(docker compose ps -q rethinkdb)"
  if [[ -n "$db_id" ]] && [[ "$(docker inspect --format '{{.State.Health.Status}}' "$db_id" 2>/dev/null || true)" == "healthy" ]]; then
    break
  fi
  if [[ "$attempt" -eq 60 ]]; then
    echo "RethinkDB did not become healthy." >&2
    docker compose ps
    docker compose logs --tail=100 rethinkdb
    exit 1
  fi
  sleep 5
done

docker compose exec -T nginx nginx -t

echo "Clearing the old application data. The reset helper asks for a second confirmation."
bash scripts/reset-production-db.sh "$DATABASE"

echo "Waiting for the application to restart and recreate its system campaign..."
for attempt in $(seq 1 30); do
  if curl -fsS --max-time 5 "https://$DOMAIN/api/health" >/dev/null; then
    break
  fi
  if [[ "$attempt" -eq 30 ]]; then
    echo "Application health check failed after reset." >&2
    docker compose ps
    docker compose logs --tail=150 app nginx rethinkdb
    exit 1
  fi
  sleep 5
done

curl -fsS --max-time 10 "https://www.$DOMAIN/api/health" >/dev/null
docker compose ps
echo
echo "Fresh deployment completed. Database application rows were cleared and HTTPS health checks passed."
