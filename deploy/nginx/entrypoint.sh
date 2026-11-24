#!/bin/sh
set -eu

: "${DOMAIN:?DOMAIN must be set in the environment}"

if [ -s "/etc/letsencrypt/live/${DOMAIN}/fullchain.pem" ] && [ -s "/etc/letsencrypt/live/${DOMAIN}/privkey.pem" ]; then
    template="/etc/nginx/templates/https.conf.template"
else
    template="/etc/nginx/templates/http.conf.template"
fi

envsubst '${DOMAIN}' < "$template" > /etc/nginx/conf.d/default.conf
nginx -t

if [ "${NGINX_TEST_ONLY:-0}" = "1" ]; then
    exit 0
fi

exec nginx -g "daemon off;"
