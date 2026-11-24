#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "$0")/../.."
set -a
# shellcheck disable=SC1091
. ./.env
set +a

: "${DOMAIN:?Set DOMAIN in .env}"
: "${LETSENCRYPT_EMAIL:?Set LETSENCRYPT_EMAIL in .env}"

docker compose -f docker-compose.rethinkdb.yml run --rm certbot certonly \
  --webroot --webroot-path /var/www/certbot \
  --email "$LETSENCRYPT_EMAIL" --agree-tos --no-eff-email \
  -d "$DOMAIN"

# Nginx switches from bootstrap HTTP to HTTPS after the certificate exists.
docker compose -f docker-compose.rethinkdb.yml restart nginx
