#!/bin/sh
set -eu

: "${DOMAIN:?DOMAIN must be set in the environment}"
TLS_MODE="${TLS_MODE:-http}"

if [ "${NGINX_TEST_ONLY:-0}" = "1" ] && [ "${NGINX_FORCE_HTTPS:-0}" = "1" ]; then
    CERT_ROOT="/tmp/test-certs"
    mkdir -p "$CERT_ROOT/live/$DOMAIN"
    openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "$CERT_ROOT/live/$DOMAIN/privkey.pem" \
        -out "$CERT_ROOT/live/$DOMAIN/fullchain.pem" \
        -subj "/CN=$DOMAIN" -days 1
    template="/etc/nginx/templates/https.conf.template"
else
    case "$TLS_MODE" in
        http)
            template="/etc/nginx/templates/http.conf.template"
            CERT_ROOT="/tmp/no-tls"
            ;;
        globalsign)
            CERT_ROOT="/etc/globalsign"
            if [ ! -s "$CERT_ROOT/live/$DOMAIN/fullchain.pem" ] || [ ! -s "$CERT_ROOT/live/$DOMAIN/privkey.pem" ]; then
                echo "GlobalSign TLS selected, but certificate or private key is missing under $CERT_ROOT/live/$DOMAIN" >&2
                exit 1
            fi
            template="/etc/nginx/templates/https.conf.template"
            ;;
        *)
            echo "Unsupported TLS_MODE: $TLS_MODE (expected http or globalsign)" >&2
            exit 1
            ;;
    esac
fi

export CERT_ROOT
envsubst '${DOMAIN} ${CERT_ROOT}' < "$template" > /etc/nginx/conf.d/default.conf
nginx -t

if [ "${NGINX_TEST_ONLY:-0}" = "1" ]; then
    exit 0
fi

exec nginx -g "daemon off;"
