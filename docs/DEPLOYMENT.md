# Production deployment (Ubuntu 24.04 LTS)

This guide deploys the RethinkDB + Rust/Axum + Nginx stack from the `feat/rethinkdb-sse` branch. Nginx is the only public container. The application and database have no host-published ports.

## 1. VPS prerequisites

- Ubuntu 24.04 LTS.
- A DNS A record for `DOMAIN` pointing to the VPS IPv4 address. Configure an AAAA record only if IPv6 is actually routed and firewalled.
- Docker Engine and the Docker Compose plugin.
- Firewall allows inbound TCP 80 and 443; SSH remains restricted to administrators.

Do not expose ports 8080, 28015, 29015 or the RethinkDB admin UI to the Internet.

## 2. Configure the application

Clone the repository to `/opt/congratis` and check out the intended release branch:

```sh
sudo mkdir -p /opt/congratis
sudo chown "$USER":"$USER" /opt/congratis
git clone https://github.com/congratis/congratis_ipsen.git /opt/congratis
cd /opt/congratis
git switch feat/rethinkdb-sse
cp .env.example .env
```

Edit `.env`:

- `RETHINKDB_PASSWORD`: long random secret (generate with `openssl rand -hex 32`).
- `DOMAIN`: the real public hostname.
- `LETSENCRYPT_EMAIL`: monitored email address for certificate notices.
- `PUBLIC_BASE_URL`: `https://` plus the public hostname.

Keep `.env` and the generated invitation output outside Git. The application uses the RethinkDB admin account with the configured password; access is restricted to the private Docker data network.

Validate the Compose file and start the stack:

```sh
docker compose config --quiet
docker compose up -d --build
docker compose ps
docker compose logs -f rethinkdb app nginx
```

Before a certificate exists, the Nginx entrypoint selects the bootstrap HTTP configuration. Confirm that DNS resolves to this VPS and that `http://DOMAIN` is reachable.

## 3. Issue the first TLS certificate

Run:

```sh
bash deploy/scripts/issue-certificate.sh
```

The script uses the shared ACME webroot, obtains a Let's Encrypt certificate, then restarts Nginx. On restart, Nginx detects the certificate and loads the HTTPS configuration. Verify:

```sh
curl -fsS "https://YOUR_DOMAIN/api/health"
```

The HTTP listener then redirects normal traffic to HTTPS while continuing to serve ACME challenge files.

## 4. Create one-time invitations

After the app and database are healthy, generate the 20 invitations once:

```sh
docker compose exec app generate-tokens 20
```

The command prints each ornament number and personal URL. Save the output in a protected location and distribute each URL only to its intended recipient. The database stores only the SHA-256 token hash; raw tokens cannot be recovered.

Do not run the generator again against a populated table. RethinkDB does not provide a multi-document transaction, so initial invitation creation is a one-time provisioning operation.

## 5. Certificate renewal

Install the included systemd unit and timer:

```sh
sudo cp deploy/systemd/congratis-certbot-renew.service /etc/systemd/system/
sudo cp deploy/systemd/congratis-certbot-renew.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now congratis-certbot-renew.timer
systemctl list-timers congratis-certbot-renew.timer
```

The timer runs a daily renewal check. The script reloads Nginx after Certbot completes. The deployment path is `/opt/congratis`, matching the included systemd unit.

## 6. Operations

- Health: `GET /api/health`.
- Snapshot: `GET /api/greetings`.
- Live events: `GET /api/events` (SSE).
- Logs: `docker compose logs -f app nginx rethinkdb`.
- Rebuild after an update: `git pull` then `docker compose up -d --build`.
- Check Nginx configuration: `docker compose exec nginx nginx -t`.

Nginx serves the static frontend and proxies API calls to Axum. Buffering is disabled for `/api/events`; this is required for low-latency SSE delivery. The browser fetches a fresh greeting snapshot after the EventSource connection opens and after each reconnect.

RethinkDB data persists in the named `rethinkdb_data` volume. Back it up using a consistent RethinkDB backup procedure and regularly test restores. A Docker restart is not a backup. A single VPS/node is not highly available.

Never run `docker compose down -v` on production: it deletes the persistent database and certificate volumes.
