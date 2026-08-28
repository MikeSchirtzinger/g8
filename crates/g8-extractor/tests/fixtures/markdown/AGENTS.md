# Project Overview

A sample project for testing G8 sidecar parsing.

# Architecture

- `src/fetcher/` — HTTP fetch and retry logic; owns the NewsSource abstraction
- `src/scorer/` — scoring models; seam: ScoringStrategy trait
- `src/ranker/` — rank aggregation and output; depends on scorer

# Boundaries

Always: run `cargo clippy` before committing
Ask first: changing the ScoringStrategy trait signature (downstream impact)
Never: add tokio::spawn inside src/scorer/ (scoring must be sync and deterministic)

# Stack

- Language: Rust 1.78 (edition 2021)
- Async runtime: tokio 1.x
- Storage: rusqlite 0.31

# Commands

```bash
cargo build
cargo test
cargo clippy
```

# Owner

agent: scorer-specialist
team: core-ranker
contact: #checkout-api-core

# Parked Ideas

- config-hot-reload: requires Arc<RwLock<Config>>, deferred pending benchmarks
- ml-scoring: requires Python FFI, blocked on cargo-pyo3 stability

# Decisions

Use rusqlite for local SQLite storage (decision locked in SPEC §5).
