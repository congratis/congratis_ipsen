# Congratis MCP SSH Gateway (Rust)

Standalone MCP server using stdio transport and the system OpenSSH client. It exposes one tool, `exec`, for remote shell commands on the configured Congratis VPS.

## Access model

- SSH connects as a dedicated non-root account, e.g. `congratis-ops`.
- By default, every command is executed as `sudo -n -- /bin/bash -lc <command>`.
- The account therefore needs passwordless sudo for the intended commands. Unrestricted sudo is effectively root access.
- Set `SSH_SUDO=false` only if commands should run as the SSH account itself.

This is intentionally a full-administration gateway, not a sandbox. A command can change configuration, restart services, delete data, or make the VPS unavailable. Only connect it to a trusted MCP client and explicitly authorize production-changing commands.

## Build

Requirements: Rust stable and OpenSSH client.

```sh
cd mcp-ssh-gateway
cargo fmt --check
cargo check --all-targets
cargo build --release
```

## Configuration

Copy `.env.example` to a private environment file and set:

- `SSH_HOST`: VPS hostname or IP
- `SSH_USER`: dedicated Linux account, never root
- `SSH_IDENTITY_FILE`: private key path
- `SSH_KNOWN_HOSTS_FILE`: pinned host key file
- `SSH_SUDO=true`: run remote commands via non-interactive sudo

Run the binary with these variables in its environment. MCP client launch configuration must use the absolute path to the compiled binary. The exact integration method depends on whether the MCP client supports local stdio servers or a remote MCP connector.

## SSH account provisioning

Provisioning is a separate, operator-controlled step. Do not copy a private key into this repository or send it through chat. Use a dedicated key, verify the VPS host fingerprint out of band, pin it in `known_hosts`, and restrict SSH ingress with the firewall.

Example account setup to be reviewed and run by the VPS owner:

```sh
sudo adduser --disabled-password --gecos "" congratis-ops
sudo install -d -m 0700 -o congratis-ops -g congratis-ops /home/congratis-ops/.ssh
# Install the public key into authorized_keys using a secure channel.
sudo visudo -f /etc/sudoers.d/congratis-ops
```

For unrestricted administrative execution, the sudoers entry is equivalent to:

```sudoers
congratis-ops ALL=(ALL:ALL) NOPASSWD: ALL
```

This grants root-equivalent power. Keep the private key tightly controlled and revoke it immediately if compromised. Consider sudo command allowlisting if full root-equivalent access is not actually required.

## Runtime safeguards

- SSH host-key verification is mandatory; no insecure host-key bypass.
- SSH uses batch mode, identity pinning, connection timeout and keepalives.
- Command size, execution time and captured output are bounded.
- Tool arguments are JSON-schema validated and passed as one safely quoted remote shell argument.
- Audit events are written to stderr. They record the target, SSH account, sudo mode, command byte length, outcome, and errors; the command text itself is deliberately not logged to avoid leaking secrets.
- The Gateway does not expose a network listener; stdio is the transport.

No VPS connection, key provisioning, or production command has been performed by this repository change.
