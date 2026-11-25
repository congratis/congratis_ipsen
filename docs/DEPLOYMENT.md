# Production deployment — Congratis

Target host: Ubuntu 26.04 LTS (resolute).
Domain: congratis.online.
Application stack: Rust/Axum + RethinkDB 2.4.4 + Nginx in Docker Compose.

Only Nginx publishes host ports 80/443. The Rust app and RethinkDB stay on private Docker networks.

## 1. VPS prerequisites

- Ubuntu 26.04 LTS, amd64 or arm64.
- DNS A record for congratis.online pointing to the VPS IPv4 address.
- Inbound TCP 22 (restricted to your administration IP), TCP 80 and TCP 443.
- Persistent disk space for Docker images, database data and backups.
- This VPS currently has about 1.6 GiB RAM and no swap. Add 2 GiB swap before building Rust release images to reduce the risk of an out-of-memory build. Swap is a safety net, not a replacement for RAM.

Use the VPS provider firewall/security group as the primary inbound firewall. Docker-published ports can bypass ordinary UFW filtering depending on the ruleset. Do not publish ports 8080, 28015, 29015 or the RethinkDB admin UI.

## 2. Install Docker

The repository includes an installer that explicitly checks for Ubuntu 26.04 (resolute) and supported CPU architecture:

    cd /opt/congratis
    sudo bash deploy/scripts/install-docker-ubuntu.sh
    docker version
    docker compose version
    systemctl is-active docker

The script installs Docker Engine, Buildx and the Compose plugin from Docker's signed Ubuntu APT repository. It preserves an existing /etc/docker/daemon.json.

## 3. Prepare the application

Run commands from the repository directory:

    cd /opt/congratis
    git fetch origin
    git switch feat/dominator-spa
    git pull --ff-only origin feat/dominator-spa
    cp -n .env.example .env
    chmod 600 .env

Edit .env and set:

- RETHINKDB_PASSWORD: generate a long random secret with openssl rand -hex 32; put it in the file without committing it.
- DOMAIN=congratis.online.
- LETSENCRYPT_EMAIL: a real email address that you monitor.
- PUBLIC_BASE_URL=https://congratis.online.

The application connects to RethinkDB using the Docker service hostname rethinkdb, not 127.0.0.1. Compose also explicitly sets this hostname for the app and probe containers.

Never print or share the contents of .env. The file is ignored by Git.

## 4. Verify DNS before TLS

The authoritative NIC.RU nameservers should return:

- congratis.online A 46.243.187.13
- www.congratis.online CNAME congratis.online

Check authoritative answers:

    dig @ns8-l2.nic.ru congratis.online A +norecurse
    dig @ns4-l2.nic.ru congratis.online A +norecurse
    dig @ns8-l2.nic.ru www.congratis.online A +norecurse

Then check public recursive resolvers from a network that can reach them:

    dig @1.1.1.1 congratis.online A
    dig @8.8.8.8 congratis.online A

Do not request a Let's Encrypt certificate until public DNS resolves to this VPS and inbound TCP 80 is reachable. If recursive DNS is unavailable from the VPS, test from another network; do not treat a timeout as proof that the DNS record is wrong.

## 5. Validate and start the HTTP stack

    cd /opt/congratis
    docker compose config --quiet
    docker compose up -d --build
    docker compose ps
    docker compose logs --tail=100 rethinkdb app nginx

Nginx starts with the HTTP-only bootstrap template when no certificate exists. After DNS is correct, test:

    curl -fsS http://congratis.online/api/health

The health endpoint checks the application's connection to RethinkDB.

## 6. Issue the first TLS certificate

Only after the HTTP health endpoint is reachable from the public Internet:

    bash deploy/scripts/issue-certificate.sh
    curl -fsS https://congratis.online/api/health

Certbot uses the shared ACME webroot. Nginx restarts and selects the HTTPS template after the certificate files exist. The HTTP challenge path remains available while other HTTP requests redirect to HTTPS.

## 7. Provision invitations

The invitation generator is a one-time operation. Do not run it until the site has passed its smoke tests and you are ready to distribute the 20 personal links.

    docker compose exec app generate-tokens 20

Store the output securely and distribute each personal URL only to its intended recipient. The database stores token hashes, not raw tokens. The generator refuses to initialize a non-empty table; do not rerun it as part of routine deployment.

## 8. Certificate renewal

Install the included systemd service and timer:

    sudo cp deploy/systemd/congratis-certbot-renew.service /etc/systemd/system/
    sudo cp deploy/systemd/congratis-certbot-renew.timer /etc/systemd/system/
    sudo systemctl daemon-reload
    sudo systemctl enable --now congratis-certbot-renew.timer
    systemctl list-timers congratis-certbot-renew.timer

The service expects the repository at /opt/congratis.

## 9. Operations

    # Status and logs
    docker compose ps
    docker compose logs -f app nginx rethinkdb

    # Health and Nginx configuration
    curl -fsS https://congratis.online/api/health
    docker compose exec nginx nginx -t

    # Update after reviewing the Git changes
    git pull --ff-only origin feat/dominator-spa
    docker compose up -d --build

RethinkDB data is stored in the named rethinkdb_data volume. Back up the database regularly using RethinkDB dump/restore tools and test restoring to a separate temporary instance. Securely back up .env and the Let's Encrypt volume as well.

**Never run docker compose down -v on production.** It deletes the persistent database and certificate volumes. A Docker restart is not a backup, and a single VPS is not highly available.
