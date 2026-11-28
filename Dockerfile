FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler nodejs npm && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
WORKDIR /app
COPY Cargo.toml ./
COPY Cargo.lock ./
COPY src ./src
RUN cargo build --release --bins
COPY frontend ./frontend
COPY web ./web
WORKDIR /app/web
RUN npm ci --no-audit --no-fund \
    && npm run build \
    && cp index.html dist/index.html \
    && cp style.css dist/style.css \
    && cp ../frontend/christmas-tree.webp dist/christmas-tree.webp \
    && npm run finalize:assets

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/christmas-greetings /usr/local/bin/christmas-greetings
COPY --from=builder /app/target/release/generate-tokens /usr/local/bin/generate-tokens
COPY --from=builder /app/target/release/rethinkdb-probe /usr/local/bin/rethinkdb-probe
COPY --from=builder /app/target/release/rethinkdb-reset /usr/local/bin/rethinkdb-reset
COPY --from=builder /app/target/release/hash-admin-password /usr/local/bin/hash-admin-password
COPY frontend ./frontend
COPY --from=builder /app/web/dist ./frontend
ENV HOST=0.0.0.0 PORT=8080 RUST_LOG=info
EXPOSE 8080
CMD ["christmas-greetings"]
