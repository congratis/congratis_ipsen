#!/usr/bin/env bash
set -Eeuo pipefail

APP_DIR="${CONGRATIS_DIR:-/opt/congratis}"
cd "$APP_DIR"

docker compose -f docker-compose.rethinkdb.yml run --rm certbot renew \
  --webroot --webroot-path /var/www/certbot --quiet

# Reload Nginx after successful or no-op renewal; the running config remains valid.
docker compose -f docker-compose.rethinkdb.yml exec -T nginx nginx -s reload
