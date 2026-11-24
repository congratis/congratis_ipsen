# Congratis MCP SSH Gateway

A small MCP server that exposes a deliberately narrow, read-only set of Congratis VPS diagnostics over SSH.

**This is not a general-purpose terminal.** It does not accept shell commands, paths, Docker arguments, or arbitrary hostnames from MCP callers. Every remote command is defined in source code. Mutating operations and deployment are intentionally out of scope for this first version.

## Exposed tools

| Tool | Purpose |
|---|---|
| `host_overview` | Uptime, memory and CPU count |
| `disk_usage` | Usage for `/` and `/opt` |
| `compose_status` | Compose status in `/opt/congratis` |
| `service_logs` | Last 1-200 lines from `app`, `nginx` or `rethinkdb` |

## Requirements

- Node.js 20+
- OpenSSH client
- A dedicated, non-root Linux account on the VPS
- A dedicated SSH key and a pinned `known_hosts` entry

## Configuration

Copy `.env.example` to a private environment file and set:

- `SSH_HOST`: VPS DNS name or IP
- `SSH_USER`: dedicated account (not root)
- `SSH_IDENTITY_FILE`: private key path, mounted read-only
- `SSH_KNOWN_HOSTS_FILE`: pinned host key file, mounted read-only

Do not disable host-key checking. Do not put private keys, passwords, or tokens in this repository or MCP tool arguments.

Run:

```sh
cd mcp-ssh-gateway
npm run check
npm start
```

The server uses MCP stdio transport. Configure the MCP client to launch `node /absolute/path/to/mcp-ssh-gateway/src/server.mjs` with the environment variables above. Exact client configuration depends on the MCP client and its support for local stdio servers or remote MCP connectors.

## SSH account hardening

Use a dedicated account such as `congratis-ops`, with:

- no interactive password authentication;
- no root login and no unrestricted sudo;
- a dedicated key restricted in `authorized_keys` where practical;
- access only to the minimum Docker operations required by this diagnostics phase;
- firewall rules that allow SSH only from the Gateway host/network;
- host-key pinning and key rotation procedures.

**Important Docker caveat:** membership in the `docker` group is effectively root-equivalent on a conventional Linux host. If the account needs Docker access for these diagnostics, treat that credential as privileged. For stronger isolation, expose a narrowly scoped root-owned helper with a fixed command allowlist instead of granting general Docker-group access.

## Security properties and limits

- No arbitrary command execution.
- No caller-controlled shell fragments; service names are allowlisted and line counts are numeric and bounded.
- SSH uses `BatchMode`, `IdentitiesOnly`, `StrictHostKeyChecking=yes`, a configured known-hosts file, and connection timeouts.
- Command execution has a timeout and bounded stdout.
- Audit events go to stderr; optionally set `AUDIT_FILE` to append JSON Lines to a local file.
- Tool output can contain sensitive operational data. Only connect this server to a trusted MCP client and protect its logs.

This first implementation is a diagnostic prototype, not a production security certification. It has not been connected to a VPS and has not been used to run commands on production.
