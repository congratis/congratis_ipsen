#!/usr/bin/env bash
set -Eeuo pipefail

APP_DIR="${CONGRATIS_DIR:-/opt/congratis}"
cd "$APP_DIR"

docker compose run --rm certbot renew \
  --webroot --webroot-path /var/www/certbot --quiet

# Reload Nginx after successful or no-op renewal; the running config remains valid.
docker compose exec -T nginx nginx -s reload
