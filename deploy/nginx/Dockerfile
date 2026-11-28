FROM nginx:1.30.8-alpine
RUN apk add --no-cache gettext openssl && rm -f /etc/nginx/conf.d/default.conf
COPY deploy/nginx/*.template /etc/nginx/templates/
COPY deploy/nginx/entrypoint.sh /usr/local/bin/congratis-nginx-entrypoint
RUN chmod 0755 /usr/local/bin/congratis-nginx-entrypoint
ENTRYPOINT ["/usr/local/bin/congratis-nginx-entrypoint"]
