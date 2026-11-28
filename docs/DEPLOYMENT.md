# Production deployment — Congratis

- Repository: `congratis/congratis_ipsen` (private).
- Production branch: `main`.
- Host OS: Ubuntu 26.04 LTS (Resolute), amd64 or arm64.
- Domain: `congratis.online`.
- Stack: Rust/Axum + RethinkDB 2.4.4 + Nginx in Docker Compose.
- Production TLS: manually managed GlobalSign certificate.

Only Nginx publishes host ports 80/443. The Rust app and RethinkDB stay on private Docker networks. Certbot and ACME are not part of this deployment.

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

Set the following production values in `.env`:

- `DOMAIN=congratis.online`
- `TLS_MODE=globalsign`
- `PUBLIC_BASE_URL=https://congratis.online`
- `COOKIE_SECURE=true`
- a long random `RETHINKDB_PASSWORD`
- `ADMIN_USERNAME` and `ADMIN_PASSWORD_HASH`

Do not add a certificate email or any Certbot settings. Never print or share `.env`.

For an existing production checkout that still has the old `TLS_PROVIDER=globalsign` setting, migrate the variable before recreating Nginx:

```bash
cd /opt/congratis
sed -i '/^TLS_PROVIDER=/d; /^TLS_MODE=/d' .env
printf '\nTLS_MODE=globalsign\n' >> .env
grep -n '^TLS_MODE=' .env
```

The output must show exactly `TLS_MODE=globalsign`. This edits only the TLS mode setting and does not print secrets.

Generate the Argon2id hash using the built image:

```bash
docker compose run --rm -e ADMIN_PASSWORD='your-long-password' app hash-admin-password
```

Put the resulting hash into `ADMIN_PASSWORD_HASH` in `.env`. Do not put the plaintext password there.

## 5. Install and verify the GlobalSign certificate

The production certificate and private key are stored outside the repository on the VPS:

```text
/etc/congratis/tls/live/congratis.online/fullchain.pem
/etc/congratis/tls/live/congratis.online/privkey.pem
```

The `fullchain.pem` must contain the leaf certificate followed by the intermediate certificate. Do not append the root certificate. Keep the private key out of Git and outside the repository.

The certificate must cover both `congratis.online` and `www.congratis.online`. Verify the installed files and key pair before starting or recreating Nginx:

```bash
sudo test -s /etc/congratis/tls/live/congratis.online/fullchain.pem
sudo test -s /etc/congratis/tls/live/congratis.online/privkey.pem
sudo openssl x509 -in /etc/congratis/tls/live/congratis.online/fullchain.pem -noout -subject -issuer -dates -ext subjectAltName
openssl x509 -in /etc/congratis/tls/live/congratis.online/fullchain.pem -pubkey -noout | openssl pkey -pubin -outform DER | sha256sum
sudo openssl pkey -in /etc/congratis/tls/live/congratis.online/privkey.pem -pubout -outform DER | sha256sum
```

The two SHA-256 hashes must be identical. Restrict private-key permissions:

```bash
sudo chown root:root /etc/congratis/tls/live/congratis.online/privkey.pem
sudo chmod 600 /etc/congratis/tls/live/congratis.online/privkey.pem
```

Compose mounts `/etc/congratis/tls` read-only into Nginx at `/etc/globalsign`. Nginx reads the certificate from that mount. GlobalSign DV Free certificates are renewed manually; arrange and install a replacement before the current certificate expires, then validate and reload Nginx. No automatic certificate-renewal service is installed by this project.

## 6. Build and start the stack

First validate the Compose configuration:

```bash
cd /opt/congratis
docker compose config --quiet
```

Build and start the stack:

```bash
docker compose up -d --build --pull never --remove-orphans
docker compose ps
docker compose logs --tail=100 rethinkdb app nginx
curl -fsS https://congratis.online/api/health
curl -fsS https://www.congratis.online/api/health
```

The Nginx container must report healthy and both HTTPS health checks must return HTTP 200. Check the active Nginx configuration with:

```bash
docker compose exec nginx nginx -t
```

For local development without TLS, use `TLS_MODE=http` and `COOKIE_SECURE=false`. Production must use `TLS_MODE=globalsign` and `COOKIE_SECURE=true`.

## 7. Fresh production deployment (destructive)

Use this only when intentionally replacing the current campaign data with a clean start. It deploys `main`, then permanently deletes every record from:

- `congratis.ornaments` — invitations, participant records and greetings.
- `congratis.auth_sessions` — all participant and administrator sessions.
- `congratis.campaigns` — all campaign records.

The database, tables, indexes and Docker named volume are retained. The app recreates its protected `legacy-default` system campaign when it starts again. The script does not create a database backup. One-time invitation tokens and submitted greetings are irrecoverable after this operation.

Before running it, verify that `.env`, the GlobalSign certificate and private key are correct and that the Git checkout is the expected production checkout. The script refuses a dirty working tree and requires two separate typed confirmations.

From the VPS:

```bash
cd /opt/congratis
bash deploy/scripts/deploy-fresh-production.sh
```

The first confirmation must be `DEPLOY FRESH congratis.online`; the reset helper then separately requires `CLEAR congratis`. If either confirmation is not exact, the corresponding operation is cancelled. The helper stops the app while deleting records, then starts it again. It does not run `docker compose down -v`, remove the RethinkDB volume, or delete TLS files.

After deployment, verify the admin login and create a new campaign in the admin panel. Save each one-time invitation link when it is displayed.

## 8. Create the campaign and distribute invitations

Open `https://congratis.online`, choose administrator login and authenticate using `ADMIN_USERNAME` and the password whose Argon2id hash is in `.env`.

Create the campaign in the admin panel with 1–35 participants. Save every one-time invitation link securely and distribute it only to its intended recipient. Raw tokens are shown once and cannot be recovered from the database.

**Choose only one provisioning method.** If the admin panel creates the campaign, do not run `generate-tokens`. The CLI is an alternative for a fresh empty database:

```bash
docker compose exec app generate-tokens 20
```

It accepts a count from 1 to 35 and prints raw tokens once. Do not run it after campaign creation or as part of routine deployment.

The CI-only script `deploy/scripts/smoke-test-api.sh` creates invitations and submits test data. It requires a fresh disposable database and must not be run against production data.

## 9. Updates and operations

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

Docker Compose reuses locally available base images and build cache by default. `--pull never` prevents Compose from pulling service images; the Docker build also reuses the locally cached base-image layers. Do not use `--no-cache` for a routine rebuild.

Useful checks: `docker compose logs -f app nginx rethinkdb`, `docker compose exec nginx nginx -t`, `docker compose ps`.

RethinkDB data is stored in the named `rethinkdb_data` volume. Back up the database regularly using RethinkDB dump/restore tools and test restoring to a separate temporary instance. Securely back up `.env` and the GlobalSign certificate/private key.

**Never run `docker compose down -v` on production.** It deletes persistent volumes. A Docker restart is not a backup, and a single VPS is not highly available.

## 10. Remove legacy Certbot units (one-time migration)

This project no longer deploys or uses Certbot. If the old systemd timer was installed on the VPS, disable it and remove its unit files:

```bash
sudo systemctl disable --now congratis-certbot-renew.timer
sudo rm -f /etc/systemd/system/congratis-certbot-renew.timer
sudo rm -f /etc/systemd/system/congratis-certbot-renew.service
sudo systemctl daemon-reload
sudo systemctl reset-failed
```

Run these commands only if those legacy units exist. Do not remove the GlobalSign certificate directory. The old `letsencrypt` and `acme_webroot` Docker volumes are not referenced by the current Compose project; leave them untouched until the new deployment is verified and you have explicitly confirmed they are no longer needed.
