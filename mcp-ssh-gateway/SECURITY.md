# Security model

This service intentionally grants arbitrary remote command execution. With `SSH_SUDO=true`, the configured SSH identity can exercise root-equivalent privileges through sudo. Treat the Gateway process, its environment, SSH key, MCP client, and audit output as privileged administrative assets.

## Mandatory controls

1. Dedicated SSH account, never root.
2. Dedicated key, not an operator's personal key.
3. Strict host-key checking with a pinned known_hosts file.
4. SSH ingress limited by firewall to the Gateway host/network.
5. Private key mounted/readable only by the Gateway process.
6. MCP stdio process exposed only to a trusted local client; do not publish an unauthenticated TCP endpoint.
7. Keep audit logs private and rotate them; command text may include secrets.
8. Revoke the key immediately if the client or Gateway host is compromised.

## Operational risk

Arbitrary shell commands can modify or destroy production data, disable security controls, exfiltrate secrets, or make the VPS unavailable. Unrestricted sudo is root-equivalent. Require explicit operator authorization for production-changing commands and verify target host/environment before execution.

## Implemented limits

- Command length cap
- Execution timeout
- Captured output cap
- Strict SSH host-key verification
- JSON-RPC/MCP stdio protocol
- Audit events to stderr

These limits reduce accidental abuse but do not sandbox the shell command or make the account least-privileged.
