#!/usr/bin/env bash
set -Eeuo pipefail

# Install Docker Engine and the Compose v2 plugin from Docker's official
# Ubuntu apt repository. Safe to run more than once.
if [[ "${EUID}" -ne 0 ]]; then
  echo "Run this script as root: sudo bash deploy/scripts/install-docker-ubuntu.sh" >&2
  exit 1
fi

if [[ ! -r /etc/os-release ]]; then
  echo "Cannot identify the operating system (/etc/os-release is missing)." >&2
  exit 1
fi

# shellcheck disable=SC1091
. /etc/os-release
CODENAME="${UBUNTU_CODENAME:-${VERSION_CODENAME:-}}"
ARCH="$(dpkg --print-architecture)"

if [[ "${ID:-}" != "ubuntu" || "${VERSION_ID:-}" != "26.04" || "${CODENAME}" != "resolute" ]]; then
  echo "This installer targets Ubuntu 26.04 LTS (resolute); detected ${PRETTY_NAME:-unknown} (${CODENAME:-unknown})." >&2
  exit 1
fi

case "${ARCH}" in
  amd64|arm64) ;;
  *)
    echo "Unsupported architecture for this deployment: ${ARCH}" >&2
    exit 1
    ;;
esac

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl

install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc

cat > /etc/apt/sources.list.d/docker.sources <<EOF
Types: deb
URIs: https://download.docker.com/linux/ubuntu
Suites: ${CODENAME}
Components: stable
Architectures: ${ARCH}
Signed-By: /etc/apt/keyrings/docker.asc
EOF

apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

# Use Docker's rotating local log driver only when the host has no existing
# daemon configuration. Never overwrite an administrator's daemon.json.
install -d -m 0755 /etc/docker
if [[ ! -e /etc/docker/daemon.json ]]; then
  cat > /etc/docker/daemon.json <<'EOF'
{
  "log-driver": "local"
}
EOF
  chmod 0644 /etc/docker/daemon.json
else
  echo "Keeping existing /etc/docker/daemon.json unchanged."
fi

systemctl enable --now docker
systemctl is-active --quiet docker

docker version
docker compose version
echo "Docker Engine and Compose plugin are ready."
