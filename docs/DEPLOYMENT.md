# Production deployment (Ubuntu 26.04 LTS)

This guide deploys Congratis (RethinkDB + Rust/Axum + Nginx) on Ubuntu 26.04 LTS, codename `resolute`. The application and database run in containers; Nginx is the only service publishing host ports (80/443).

## 1. VPS prerequisites

- Ubuntu 26.04 LTS (`resolute`) on `amd64` or `arm64`.
- A DNS A record for `congratis.ru` pointing to this VPS. Add an AAAA record only if IPv6 is configured, routed and filtered correctly.
- Inbound TCP 22 restricted to your administration IP, and TCP 80/443 reachable from the Internet. Prefer the VPS provider's network firewall/security group.
- At least 2 GB RAM and 20 GB free disk space recommended for the initial build, images, database and backups.

Docker Engine officially supports Ubuntu 26.04 LTS. Install it from Docker's signed APT repository using the included script:

```sh
cd /opt
git clone https://github.com/congratis/congratis_ipsen.git congratis
cd /opt/congratis
git switch feat/rethinkdb-sse
bash deploy/scripts/install-docker-ubuntu.sh
```

The installer checks the OS release and CPU architecture, configures Docker's official APT signing key and repository, installs Engine + Buildx + Compose v2, and enables the Docker service. It does not add users to the `docker` group or overwrite an existing `/etc/docker/daemon.json`.

Check the installation:

```sh
docker version
docker compose version
systemctl is-active docker
```

**Firewall note:** Docker-published ports can bypass host UFW rules. Use the provider firewall to expose only SSH (restricted to your IP), HTTP 80 and HTTPS 443. Do not publish 8080, 28015, 29015 or the RethinkDB admin UI. If you use a host firewall, validate the effective Docker forwarding rules separately; do not assume a UFW deny rule protects a published container port.

## 2. Configure Congratis

Clone only once. If the repository is already present, update it instead of cloning again:

```sh
mkdir -p /opt/congratis
cd /opt/congratis
git fetch origin
git switch feat/rethinkdb-sse
git pull --ff-only origin feat/rethinkdb-sse
if [ ! -f .env ]; then cp .env.example .env; fi
```

Edit `.env` and set:

- `RETHINKDB_PASSWORD`: a long random secret (generate with `openssl rand -hex 32`).
- `DOMAIN=congratis.ru`.
- `LETSENCRYPT_EMAIL`: a monitored email address.
- `PUBLIC_BASE_URL=https://congratis.ru`.

Generate the password without printing it to shell history:

```sh
openssl rand -hex 32
```

Put the value in `.env`, restrict its permissions, and never commit it:

```sh
chmod 600 .env
```

Before starting Certbot, confirm public DNS resolves to this server:

```sh
getent ahostsv4 congratis.ru
curl -4 https://ifconfig.me
```

The IPv4 address returned by DNS must be `46.243.187.13` for the current VPS. If DNS does not resolve or points elsewhere, stop here and fix the A record at the DNS provider. Do not request a Let's Encrypt certificate yet.

Validate and start the stack:

```sh
docker compose config --quiet
docker compose up -d --build
docker compose ps
docker compose logs --tail=100 rethinkdb app nginx
```

At first, Nginx serves the HTTP bootstrap configuration. Check `http://congratis.ru` and `http://congratis.ru/api/health` after DNS is correct.

## 3. Issue the first TLS certificate

Only after the public A record points to this VPS and inbound TCP 80 is reachable:

```sh
bash deploy/scripts/issue-certificate.sh
curl -fsS https://congratis.ru/api/health
```

Certbot uses the shared ACME webroot. Nginx restarts and selects its HTTPS configuration when the certificate files exist. HTTP redirects to HTTPS while ACME challenge files remain reachable.

## 4. Verify one-time invitations and live updates

Do not generate the real 20 invitation links until the smoke test is complete. First test with a disposable invitation in a non-production test database or a dedicated test deployment. The production invitation generator is intentionally one-shot and refuses to populate an already initialized table.

For the production event after smoke testing:

```sh
docker compose exec app generate-tokens 20
```

Store the output securely and distribute each personal URL only to its intended recipient. The database stores only SHA-256 token hashes. Never run the generator again against a populated table.

## 5. Certificate renewal

Install the systemd renewal service and timer:

```sh
sudo cp deploy/systemd/congratis-certbot-renew.service /etc/systemd/system/
sudo cp deploy/systemd/congratis-certbot-renew.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now congratis-certbot-renew.timer
systemctl list-timers congratis-certbot-renew.timer
```

The timer checks daily. The script runs Certbot renewal and reloads Nginx. The service expects the deployment at `/opt/congratis`.

## 6. Operations and recovery

- Health: `GET /api/health` (checks RethinkDB connectivity).
- Snapshot: `GET /api/greetings`.
- Live events: `GET /api/events` (SSE).
- Logs: `docker compose logs -f app nginx rethinkdb`.
- Nginx config: `docker compose exec nginx nginx -t`.
- Update: `git pull --ff-only` then `docker compose up -d --build`.

RethinkDB persists in the named `rethinkdb_data` volume. Back up regularly with RethinkDB's dump/restore tools and test restoring to a separate temporary database/host. Back up the Let's Encrypt volume and `.env` securely as well. A Docker restart is not a backup; a single VPS is not highly available.

Never run `docker compose down -v` on production: it deletes the database and certificate volumes. Do not delete Docker volumes during routine updates.

## 7. Ubuntu 26.04-specific checks

Ubuntu 26.04 uses the `resolute` codename. The installer deliberately fails closed on another release instead of silently configuring an unverified repository. Docker Engine uses the host's cgroup v2/systemd environment; this stack does not require legacy cgroup v1. After installation, verify:

```sh
docker info --format '{{.CgroupVersion}} {{.CgroupDriver}}'
```

Expected output should report cgroup version `2`. If Docker reports a different cgroup version or fails to start, stop deployment and inspect `journalctl -u docker -n 100 --no-pager`.
