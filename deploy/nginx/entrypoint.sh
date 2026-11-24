#!/bin/sh
set -eu

: "${DOMAIN:?DOMAIN must be set in the environment}"

if [ -s "/etc/letsencrypt/live/${DOMAIN}/fullchain.pem" ] && [ -s "/etc/letsencrypt/live/${DOMAIN}/privkey.pem" ]; then
    template="/etc/nginx/templates/https.conf.template"
    CERT_ROOT="/etc/letsencrypt"
elif [ "${NGINX_TEST_ONLY:-0}" = "1" ] && [ "${NGINX_FORCE_HTTPS:-0}" = "1" ]; then
    CERT_ROOT="/tmp/test-certs"
    mkdir -p "$CERT_ROOT/live/$DOMAIN"
    openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "$CERT_ROOT/live/$DOMAIN/privkey.pem" \
        -out "$CERT_ROOT/live/$DOMAIN/fullchain.pem" \
        -subj "/CN=$DOMAIN" -days 1
    template="/etc/nginx/templates/https.conf.template"
else
    template="/etc/nginx/templates/http.conf.template"
    CERT_ROOT="/etc/letsencrypt"
fi

envsubst '${DOMAIN} ${CERT_ROOT}' < "$template" > /etc/nginx/conf.d/default.conf
nginx -t

if [ "${NGINX_TEST_ONLY:-0}" = "1" ]; then
    exit 0
fi

exec nginx -g "daemon off;"
