# ADR: RethinkDB and real-time updates

Status: Accepted for implementation, pending driver integration tests
Date: 2026-11-24

## Decision

Replace PostgreSQL/SQLx with a self-hosted RethinkDB instance in Docker Compose. Keep the Rust/Axum application. Run Nginx in its own container as the public edge proxy. Browser updates use Server-Sent Events (SSE), driven by a single RethinkDB Changefeed consumer in the Rust service.

## Rust driver assessment

The native Rust driver is the `reql` crate (0.11.2), from `rethinkdb/rethinkdb-rs`. It implements asynchronous ReQL queries and Changefeeds. It is not one of the four official language drivers. The upstream Rust repository's latest pushed commit is 2023-07-24, so the driver must be pinned and covered by integration tests before production use.

A separate unofficial `unreql` project exists, but its repository is small and its own documentation describes it as unofficial. The similarly named `gothinkdb` crate targets GoThinkDB, a RethinkDB-compatible Go implementation; it is not the driver choice for the official RethinkDB server.

## Data model and atomic invitation claim

RethinkDB guarantees atomic writes per document, not across multiple documents. The old PostgreSQL model split participants and greetings across tables and relied on a SQL transaction. That model must not be ported literally.

Use one document per ornament in a table named `ornaments`. The SHA-256 token hash is the **primary key** (`id`); `ornament_number` is a secondary unique index:

```json
{
  "id": "<sha256-token-hash>",
  "ornament_number": 1,
  "used": false,
  "display_name": null,
  "message": null,
  "created_at": 1795500000,
  "used_at": null
}
```

This key choice is intentional: invitation redemption can address exactly one document with `get(token_hash)`, avoiding a lookup through a secondary index followed by a separate write. The redemption query must use a single-document conditional ReQL `update` function (or equivalent `replace`) that checks the current document's `used` state and, only when false, sets `used = true`, writes the greeting fields, and records `used_at`. The conditional check and mutation must be part of the same atomic single-document write. Do not perform a read-then-write sequence, and do not use a filtered multi-document selection as the atomicity boundary.

Treat a write as successful only when the ReQL write result reports exactly one replaced document and zero errors; inspect `errors` and `first_error` because ReQL may report per-document write errors in the result rather than raising an exception. Add a concurrent integration test proving that two simultaneous submissions for the same token result in exactly one success and one conflict.

The generator creates one document for each ornament and stores only the hash as the primary key. The unique ornament index prevents duplicate ornament assignments.

The invitation generator must not print the raw token in a separate CSV column if the personal URL already contains it. Keep the raw token only in the one-time invitation output; persist only its SHA-256 hash.

## Real-time delivery

- One dedicated RethinkDB session owns the long-lived Changefeed. The Rust driver currently marks a session as occupied while a Changefeed is active, so ordinary CRUD must use a separate session.
- Subscribe to `ornaments.changes()` with initial values and state notifications.
- A Tokio background task consumes the feed and publishes normalized events through `tokio::sync::broadcast`.
- Axum exposes `GET /api/events` as SSE.
- The browser uses `EventSource` and updates only the affected ornament.
- On initial load and every SSE reconnect, the browser fetches a fresh `GET /api/greetings` snapshot. Changefeeds are unidirectional and do not guarantee delivery; SSE reconnection alone is not a durable event log.
- If the feed disconnects, reconnect with bounded exponential backoff and emit health/metrics signals.

## Container topology

- `nginx`: public ports 80/443; serves the frontend and proxies `/api/*` to the Rust service.
- `app`: Rust/Axum, private Docker network only; no host-published port.
- `rethinkdb`: official RethinkDB image, persistent named volume, private data network only; no host-published ports and no public admin UI.
- `certbot`: obtains/renews Let's Encrypt certificates using a shared ACME webroot and certificate volume.

RethinkDB starts with `--initial-password`; use a long random secret from the VPS environment. The web admin UI should be disabled. Never publish ports 28015, 29015 or 8080 on the VPS.

## Required implementation gates

1. Pin the official RethinkDB image to a tested 2.4.x tag and pin `reql = 0.11.2`.
2. Confirm the Rust driver's conditional update API can express the atomic `get(token_hash).update(function)` query; keep the integration race test as the release gate.
2. Install `protobuf-compiler` in the Rust builder if required by the driver build.
3. Add integration tests against a real RethinkDB service in CI:
   - initialize database and tables idempotently;
   - insert and read ornament documents;
   - claim one invitation successfully;
   - race two submissions for the same invitation and prove exactly one succeeds;
   - verify a Changefeed sees a committed greeting;
   - verify feed reconnect and snapshot recovery.
4. Replace SQLx migrations and SQL queries; remove PostgreSQL/SQLx dependencies and Supabase-specific environment variables.
5. Update token generator to use RethinkDB and the one-document-per-ornament model.
6. Add SSE frontend integration and browser reconnect behavior.
7. Only after all gates pass, switch production Compose to the new RethinkDB topology.

## Operational caveats

RethinkDB Changefeeds are not a durable message queue: they can squash changes, can overflow their queue, and do not acknowledge delivery to browser clients. The application must treat the database snapshot as source of truth and use feeds only for low-latency notification.

A single-node RethinkDB deployment on one VPS is not highly available. Persist `/data`, back up regularly with tested restore procedures, monitor disk and memory, and do not assume Docker restart is a backup strategy.
