#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "$0")/../.."

# DOMAIN and LETSENCRYPT_EMAIL are simple .env values; do not source the
# whole file because it also contains secrets such as RETHINKDB_PASSWORD.
DOMAIN="$(sed -n 's/^DOMAIN=//p' .env | head -n1)"
LETSENCRYPT_EMAIL="$(sed -n 's/^LETSENCRYPT_EMAIL=//p' .env | head -n1)"
: "${DOMAIN:?Set DOMAIN in .env}"
: "${LETSENCRYPT_EMAIL:?Set LETSENCRYPT_EMAIL in .env}"

docker compose run --rm certbot certonly \
  --webroot --webroot-path /var/www/certbot \
  --email "$LETSENCRYPT_EMAIL" --agree-tos --no-eff-email \
  -d "$DOMAIN" -d "www.$DOMAIN"

# Nginx switches from bootstrap HTTP to HTTPS after the certificate exists.
docker compose restart nginx
