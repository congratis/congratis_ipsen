#!/usr/bin/env bash
set -Eeuo pipefail

REPO_URL="${REPO_URL:-git@github.com:congratis/congratis_ipsen.git}"
APP_DIR="${APP_DIR:-/opt/congratis}"
WEB_ROOT="/var/www/congratis"

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run as root: sudo bash scripts/deploy-ubuntu-24.04.sh" >&2
  exit 1
fi
if [[ "$(. /etc/os-release && echo "$VERSION_ID")" != "24.04" ]]; then
  echo "This installer targets Ubuntu 24.04 LTS." >&2
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl git rsync nginx certbot python3-certbot-nginx docker.io docker-compose-v2
systemctl enable --now docker
systemctl enable --now nginx

if [[ ! -d "$APP_DIR/.git" ]]; then
  install -d -m 0755 "$(dirname "$APP_DIR")"
  git clone "$REPO_URL" "$APP_DIR"
else
  git -C "$APP_DIR" pull --ff-only
fi

if [[ ! -f "$APP_DIR/.env" ]]; then
  install -m 0600 "$APP_DIR/.env.example" "$APP_DIR/.env"
  echo "Created $APP_DIR/.env. Set DATABASE_URL, then run this script again." >&2
  exit 2
fi
chmod 0600 "$APP_DIR/.env"

install -d -m 0755 "$WEB_ROOT/frontend"
rsync -a --delete "$APP_DIR/frontend/" "$WEB_ROOT/frontend/"
install -m 0644 "$APP_DIR/infra/nginx/congratis-rate-limit.conf" /etc/nginx/conf.d/congratis-rate-limit.conf
install -m 0644 "$APP_DIR/infra/nginx/congratis.ru.conf" /etc/nginx/sites-available/congratis.ru
ln -sfn /etc/nginx/sites-available/congratis.ru /etc/nginx/sites-enabled/congratis.ru
rm -f /etc/nginx/sites-enabled/default

nginx -t
systemctl reload nginx
cd "$APP_DIR"
docker compose up -d --build --remove-orphans

echo
echo "Application deployed. Verify A records for congratis.ru and www.congratis.ru."
echo "Then enable HTTPS:"
echo "  certbot --nginx --redirect -d congratis.ru -d www.congratis.ru"
echo "Local health check: curl -fsS http://127.0.0.1:8080/api/health"
echo "Logs: cd $APP_DIR && docker compose logs -f app"
