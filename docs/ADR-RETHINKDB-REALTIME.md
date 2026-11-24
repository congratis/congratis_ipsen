# ADR: RethinkDB and real-time updates

Status: CI green on 2026-11-24 (Rust, real RethinkDB integration, Docker, Compose, Nginx HTTP/TLS and reverse-proxy smoke); Ubuntu 26.04 LTS Docker installer and deployment guide added; VPS smoke test pending
Date: 2026-11-24

## Decision

Replace PostgreSQL/SQLx with a self-hosted RethinkDB instance in Docker Compose. Keep the Rust/Axum application. Run Nginx in its own container as the public edge proxy. Browser updates use Server-Sent Events (SSE), driven by a single RethinkDB Changefeed consumer in the Rust service.

## Rust driver assessment

The native Rust driver is the `reql` crate (0.11.2), from `rethinkdb/rethinkdb-rs`. It implements asynchronous ReQL queries and Changefeeds. It is not one of the four official language drivers. The upstream Rust repository's latest pushed commit is 2023-07-24, so the driver must be pinned and covered by integration tests before production use.

A separate unofficial `unreql` project exists, but its repository is small and its own documentation describes it as unofficial. The similarly named `gothinkdb` crate targets GoThinkDB, a RethinkDB-compatible Go implementation; it is not the driver choice for the official RethinkDB server.

## Data model and atomic invitation claim

RethinkDB guarantees atomic writes per document, not across multiple documents. The old PostgreSQL model split participants and greetings across tables and relied on a SQL transaction. That model must not be ported literally.

Use one document per ornament in a table named `ornaments`. The ornament number is the primary key (`id`), so the database itself prevents two documents from owning the same ornament. The SHA-256 token hash is a secondary index:

```json
{
  "id": 1,
  "ornament_number": 1,
  "token_hash": "<sha256-token-hash>",
  "used": false,
  "display_name": null,
  "message": null,
  "created_at": 1795500000,
  "used_at": null
}
```

RethinkDB secondary indexes are not unique constraints. Do not describe `token_hash` as a unique index. Tokens are generated from 256 random bits; the generator rejects duplicate hashes within its batch and refuses to run against a non-empty invitation table. The primary key guarantees unique ornament assignment.

Redemption locates the invitation with `get_all(token_hash, index: "token_hash").limit(1)` and applies one conditional ReQL update function to the resulting single-document selection. The function checks the current `used` value and, only when false, writes `used = true`, the greeting fields, and `used_at`. The check and mutation are evaluated as one atomic single-document write. Never split this into a read-then-write sequence. The `limit(1)` is required: do not update an unbounded filtered selection.

Treat a write as successful only when the ReQL write result reports exactly one replaced document and zero errors. An already-used or unknown token must produce a conflict (zero replacements); inspect `errors` and `first_error` because ReQL may report per-document write errors in the result rather than raising an exception. Add a concurrent integration test proving that two simultaneous submissions for the same token result in exactly one success and one conflict.

The generator creates one document for each ornament number and stores only the hash. It emits the raw token only inside the personal URL; it must not print a separate raw-token CSV column. RethinkDB does not provide a multi-document transaction, so invitation batch creation must be treated as a one-time provisioning operation and failures must be surfaced rather than silently retried against a partially populated table.

## Real-time delivery

- One dedicated RethinkDB session owns the long-lived Changefeed. The Rust driver currently marks a session as occupied while a Changefeed is active, so ordinary CRUD must use a separate session.
- Subscribe to `ornaments.changes()` with initial values and state notifications.
- A Tokio background task consumes the feed and publishes normalized events through `tokio::sync::broadcast`.
- Axum exposes `GET /api/events` as SSE.
- The browser uses `EventSource` and updates only the affected ornament.
- On initial load and every SSE reconnect, the browser fetches a fresh `GET /api/greetings` snapshot. Changefeeds are unidirectional and do not guarantee delivery; SSE reconnection alone is not a durable event log.
- If the feed disconnects or overflows, reconnect with bounded exponential backoff and emit tracing logs. Operational metrics/alerts remain a deployment follow-up.

## Container topology

- `nginx`: public ports 80/443; serves the frontend and proxies `/api/*` to the Rust service.
- `app`: Rust/Axum, private Docker network only; no host-published port.
- `rethinkdb`: official RethinkDB image, persistent named volume, private data network only; no host-published ports and no public admin UI.
- `certbot`: obtains/renews Let's Encrypt certificates using a shared ACME webroot and certificate volume.

RethinkDB starts with `--initial-password`; use a long random secret from the VPS environment. The web admin UI should be disabled. Never publish ports 28015, 29015 or 8080 on the VPS.

## Required implementation gates

1. Pin the official RethinkDB image to a tested 2.4.x tag and pin `reql = 0.11.2`.
2. Keep the real-server atomic invitation race test as a release gate: exactly one of two concurrent claims must replace the document.
3. Keep real-server tests for idempotent initialization, insert/read, claim and Changefeed delivery.
4. Add automated coverage for feed reconnect/overflow recovery and browser snapshot reconciliation.
5. Keep SQLx/PostgreSQL dependencies, migrations and Supabase-specific configuration out of the application build.
6. Keep invitation provisioning one-time and verify duplicate ornament numbers and duplicate token hashes are rejected.
7. Validate Nginx configuration, TLS bootstrap, SSE buffering and certificate renewal in CI.
8. Complete the Ubuntu 24.04 VPS smoke test, including HTTPS, health, invitation submission, live browser update, restart recovery and backup/restore before production release.

## Operational caveats

RethinkDB Changefeeds are not a durable message queue: they can squash changes, can overflow their queue, and do not acknowledge delivery to browser clients. The application must treat the database snapshot as source of truth and use feeds only for low-latency notification.

A single-node RethinkDB deployment on one VPS is not highly available. Persist `/data`, back up regularly with tested restore procedures, monitor disk and memory, and do not assume Docker restart is a backup strategy.
