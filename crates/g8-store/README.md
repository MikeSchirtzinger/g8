# g8-store

Embedded SQLite persistence layer for the [G8](../../README.md).

Provides the `StoreConnection` trait and its `RusqliteStore` implementation.
All schema migrations are embedded in the binary via `refinery`, no external
migration tooling required, no service to run.

## Role in the workspace

```
g8-core  ──►  g8-store  ──►  g8-planner
                          ──►  g8
```

`g8-store` is the single source of truth for persisted G8 data. The
`planner_intent_check` composite query, the central decision primitive, runs
entirely inside the store as one transactional SQLite query.

## Opening a store

```rust
use g8_store::RusqliteStore;
use std::path::Path;

// On-disk store (production)
let mut store = RusqliteStore::open(Path::new(".g8/store.db"))?;
store.migrate()?;

// In-memory store (tests)
let mut store = RusqliteStore::open_in_memory()?;
store.migrate()?;
```

## Primary public types

| Type | Purpose |
|---|---|
| `RusqliteStore` | Concrete store backed by rusqlite (bundled SQLite) |
| `StoreConnection` | Trait: the full store API surface |
| `StoreError` | Error enum for store operations |
| `ScanResult` | Return type of `apply_scan_report` |
| `PlanFilter` | Filter struct for `list_plans` |
| `WatchHandle` | RAII handle for filesystem-watch subscriptions |
| `PairingError`, `PairingErrorReason` | Pairing-gate validation errors |
| `StalePlanCandidate` | Plans eligible for stale-promotion |
| `RawIntentCheckPayload` | JSON payload from `planner_intent_check` |

## Storage model

One `.g8/store.db` file per project. Cross-project queries use SQLite's
`ATTACH DATABASE`. No external service, no LLM calls inside the store layer.

## Further reading

- Root README: [../../README.md](../../README.md)
- Architecture §2 (store schema): [../../ARCHITECTURE.md](../../ARCHITECTURE.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
