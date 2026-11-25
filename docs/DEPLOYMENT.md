# Production deployment — Congratis

- Repository: `congratis/congratis_ipsen` (private).
- Production branch: `main`.
- Host OS: Ubuntu 26.04 LTS (Resolute), amd64 or arm64.
- Domain: `congratis.online`.
- Stack: Rust/Axum + RethinkDB 2.4.4 + Nginx in Docker Compose.

Only Nginx publishes host ports 80/443. The Rust app and RethinkDB stay on private Docker networks.

## 1. VPS prerequisites

- Ubuntu 26.04 LTS and supported CPU architecture.
- DNS A record for `congratis.online` pointing to the VPS; `www` CNAME to the apex.
- Inbound TCP 22 restricted to the administrator's IP, TCP 80 and TCP 443.
- Persistent disk space for Docker images, database data, backups and certificates.

The target VPS was previously measured at roughly 1.6 GiB RAM with no swap. Add 2 GiB swap before building Rust release images. First inspect:

```bash
free -h
swapon --show
df -h /
```

If there is no existing swap and `/swapfile` does not exist:

```bash
sudo fallocate -l 2G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
swapon --show
```

Do not overwrite an existing swapfile; inspect it first. Use the VPS provider firewall as the primary inbound firewall. Do not publish ports 8080, 28015, 29015 or the RethinkDB admin UI.

## 2. Configure private GitHub access

The repository is private. Configure a read-only deploy key for the Linux account that will run Git commands:

```bash
ssh-keygen -t ed25519 -C 'congratis-vps-readonly' -f ~/.ssh/congratis_github -N ''
cat ~/.ssh/congratis_github.pub
```

Add the public key in repository **Settings → Deploy keys** with read-only access. Configure SSH:

```sshconfig
Host github.com
  HostName github.com
  User git
  IdentityFile ~/.ssh/congratis_github
  IdentitiesOnly yes
```

Verify with `ssh -T git@github.com`. Never copy the private key into the repository or share it.

For a first checkout:

```bash
sudo mkdir -p /opt/congratis
sudo chown "$USER":"$(id -gn)" /opt/congratis
git clone git@github.com:congratis/congratis_ipsen.git /opt/congratis
cd /opt/congratis
git switch main
```

If `/opt/congratis` already contains a checkout, do not clone over it. Inspect Git status and remotes, preserve local changes, then fetch and switch to `main`.

## 3. Install Docker

Run from the repository directory:

```bash
cd /opt/congratis
sudo bash deploy/scripts/install-docker-ubuntu.sh
docker version
docker compose version
systemctl is-active docker
```

The installer checks Ubuntu 26.04 (Resolute) and supported architecture, installs Docker Engine, Buildx and Compose from Docker's signed APT repository, and preserves an existing `/etc/docker/daemon.json`.

## 4. Prepare production environment

Create `.env` only if it does not already exist:

```bash
cd /opt/congratis
cp -n .env.example .env
chmod 600 .env
```

Set `RETHINKDB_PASSWORD` (long random secret), `DOMAIN=congratis.online`, a real `LETSENCRYPT_EMAIL`, `PUBLIC_BASE_URL=https://congratis.online`, `ADMIN_USERNAME`, `ADMIN_PASSWORD_HASH` and `COOKIE_SECURE=true`.

Generate the Argon2id hash using the built image:

```bash
docker compose run --rm -e ADMIN_PASSWORD='your-long-password' app hash-admin-password
```

Put the resulting hash into `ADMIN_PASSWORD_HASH` in `.env`. Do not put the plaintext password there. Never print or share `.env`.

## 5. Verify DNS and firewall before TLS

Required records: `congratis.online A` → VPS IPv4; `www.congratis.online CNAME` → `congratis.online`.

```bash
dig @ns8-l2.nic.ru congratis.online A +norecurse
dig @ns4-l2.nic.ru congratis.online A +norecurse
dig @1.1.1.1 congratis.online A
dig @8.8.8.8 congratis.online A
```

Do not request a Let's Encrypt certificate until public DNS resolves to this VPS and inbound TCP 80 is reachable.

## 6. Build and start the HTTP stack

```bash
cd /opt/congratis
docker compose config --quiet
docker compose up -d --build
docker compose ps
docker compose logs --tail=100 rethinkdb app nginx
curl -fsS http://congratis.online/api/health
```

Nginx starts with the HTTP-only bootstrap template when no certificate exists. The health endpoint checks the app's RethinkDB connection; do not continue until it returns HTTP 200.

## 7. Issue the first TLS certificate with Let's Encrypt (optional)

If using Let's Encrypt, after public DNS and HTTP health are confirmed:

```bash
bash deploy/scripts/issue-certificate.sh
curl -fsS https://congratis.online/api/health
```

Certbot uses the shared ACME webroot. Nginx switches to HTTPS after certificate files exist.

## 8. Install a GlobalSign certificate

The Nginx service supports either Let's Encrypt (default) or a manually managed GlobalSign certificate. For the GlobalSign certificate, the server certificate must be concatenated with the intermediate certificate in `fullchain.pem`. Do not append the root certificate. Keep the private key out of Git and outside the repository.

On the VPS, create the certificate directory and install the files with restrictive permissions:

```bash
sudo install -d -o root -g root -m 700 /etc/congratis/tls/live/congratis.online
sudo install -o root -g root -m 644 www_congratis_online_2027_06_11.crt /etc/congratis/tls/live/congratis.online/server.crt
sudo sh -c 'cat /etc/congratis/tls/live/congratis.online/server.crt /path/to/intermediate.crt > /etc/congratis/tls/live/congratis.online/fullchain.pem'
sudo install -o root -g root -m 600 /path/to/private.key /etc/congratis/tls/live/congratis.online/privkey.pem
```

Before switching, verify that the private key matches the certificate by comparing the SHA-256 hashes of their public keys:

```bash
openssl x509 -in /etc/congratis/tls/live/congratis.online/server.crt -pubkey -noout | openssl pkey -pubin -outform DER | sha256sum
sudo openssl pkey -in /etc/congratis/tls/live/congratis.online/privkey.pem -pubout -outform DER | sha256sum
```

The two hashes must be identical. Validate the chain and both hostnames:

```bash
openssl verify -purpose sslserver -verify_hostname congratis.online -CAfile /path/to/root.crt -untrusted /path/to/intermediate.crt /etc/congratis/tls/live/congratis.online/server.crt
openssl verify -purpose sslserver -verify_hostname www.congratis.online -CAfile /path/to/root.crt -untrusted /path/to/intermediate.crt /etc/congratis/tls/live/congratis.online/server.crt
```

Only after both checks succeed, set `TLS_PROVIDER=globalsign` in `.env`, then recreate Nginx:

```bash
cd /opt/congratis
docker compose up -d --build --no-deps nginx
docker compose exec nginx nginx -t
curl -fsS https://congratis.online/api/health
curl -fsS https://www.congratis.online/api/health
```

The certificate covers both `congratis.online` and `www.congratis.online`. Nginx reads it from the host directory mounted read-only at `/etc/globalsign`. The Let's Encrypt volume remains intact; set `TLS_PROVIDER=letsencrypt` to switch back.

GlobalSign DV Free certificates are not renewed by Certbot. Arrange a replacement certificate before 11 June 2027, install its matching key and chain in the same paths, then reload Nginx with `docker compose exec nginx nginx -s reload`. The existing Certbot timer, if enabled, only renews Let's Encrypt certificates.

## 9. Create the campaign and distribute invitations

Open `https://congratis.online`, choose administrator login and authenticate using `ADMIN_USERNAME` and the password whose Argon2id hash is in `.env`.

Create the campaign in the admin panel with 1–35 participants. Save every one-time invitation link securely and distribute it only to its intended recipient. Raw tokens are shown once and cannot be recovered from the database.

**Choose only one provisioning method.** If the admin panel creates the campaign, do not run `generate-tokens`. The CLI is an alternative for a fresh empty database:

```bash
docker compose exec app generate-tokens 20
```

It accepts a count from 1 to 35 and prints raw tokens once. Do not run it after campaign creation or as part of routine deployment.

The CI-only script `deploy/scripts/smoke-test-api.sh` creates invitations and submits test data. It requires a fresh disposable database and must not be run against production data.

## 10. Certificate renewal

For Let's Encrypt only:

```bash
sudo cp deploy/systemd/congratis-certbot-renew.service /etc/systemd/system/
sudo cp deploy/systemd/congratis-certbot-renew.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now congratis-certbot-renew.timer
systemctl list-timers congratis-certbot-renew.timer
```

The service expects the repository at `/opt/congratis`.

## 11. Updates and operations

Review Git status first. Preserve local configuration and never overwrite `.env`.

```bash
cd /opt/congratis
git status --short --branch
git fetch origin
git switch main
git pull --ff-only origin main
docker compose config --quiet
docker compose up -d --build --remove-orphans
docker compose ps
curl -fsS https://congratis.online/api/health
```

Useful checks: `docker compose logs -f app nginx rethinkdb`, `docker compose exec nginx nginx -t`, `docker compose ps`.

RethinkDB data is stored in the named `rethinkdb_data` volume. Back up the database regularly using RethinkDB dump/restore tools and test restoring to a separate temporary instance. Securely back up `.env` and the Let's Encrypt volume.

**Never run `docker compose down -v` on production.** It deletes persistent database and certificate volumes. A Docker restart is not a backup, and a single VPS is not highly available.
