FROM nginx:1.30.8-alpine
RUN apk add --no-cache gettext && rm -f /etc/nginx/conf.d/default.conf
COPY frontend /usr/share/nginx/html
COPY deploy/nginx/*.template /etc/nginx/templates/
COPY deploy/nginx/entrypoint.sh /usr/local/bin/congratis-nginx-entrypoint
RUN chmod 0755 /usr/local/bin/congratis-nginx-entrypoint
ENTRYPOINT ["/usr/local/bin/congratis-nginx-entrypoint"]
