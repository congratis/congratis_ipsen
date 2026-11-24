# Security notes

## Threat boundary

The MCP caller is untrusted input. Tool names and validated scalar arguments are the only caller-controlled inputs that reach the command dispatcher. No arbitrary command, remote path, environment variable, SSH option, or host is accepted from a tool call.

## Deployment requirements

1. Run the Gateway on a trusted machine and bind it only through the MCP transport chosen by the operator.
2. Store the SSH private key outside the repository; mount it read-only with restrictive permissions.
3. Pin the VPS host key in a managed known_hosts file. Never use `StrictHostKeyChecking=no`.
4. Use a dedicated non-root SSH account. Never reuse the VPS administrator's personal key.
5. Do not expose a raw TCP listener without authenticated transport and network restrictions.
6. Keep audit logs private; command output and service logs may contain secrets or personal data.
7. Rotate and revoke the SSH key immediately if the Gateway host or MCP client is compromised.

## Docker privilege

The Docker daemon socket and membership in the docker group generally grant root-equivalent control of the host. A dedicated Linux username alone does not make Docker access low privilege. Prefer a root-owned, narrowly scoped helper for the four fixed diagnostics, or accept and document the Docker privilege boundary explicitly.

## Not implemented

- Arbitrary shell or file access
- Writes, edits, uploads, restarts, compose up/down, or deployments
- Browser-accessible HTTP endpoint
- Multi-user authentication or approval workflow
- Centralized tamper-resistant audit storage
- Automatic key provisioning or rotation
