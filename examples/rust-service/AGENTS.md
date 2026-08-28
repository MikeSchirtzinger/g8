# Architecture

This crate implements the HTTP ingestion layer for the platform. It owns the
request lifecycle from TLS termination through deserialization and validation,
handing off to the downstream scorer crate via an async channel.

# Boundaries

- Never hold a database connection in request-handler scope; connections are
  pooled at the service level.
- Never log PII (IP addresses, user tokens) at INFO level or above.
- Never block the async executor; offload CPU work to `spawn_blocking`.

# Stack

- Rust, tokio, axum, tower
- SQLite (via rusqlite) for local state; no external DB required in dev
- tracing + tracing-subscriber for structured logging

# Owner

agent: http-ingestion-specialist
team: platform-core
contact: #platform-eng
