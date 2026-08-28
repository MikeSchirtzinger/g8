# g8-conflict

Cross-project intent/capability overlap detection for the
[G8](../../README.md).

Provides `ConflictDetector` with two entry points for finding collisions when
combining codebases or auditing a single store for internal contradictions.

## Role in the workspace

```
g8-core  ──►  g8-conflict  ──►  g8
```

`g8-conflict` is purely computational, no I/O, no async. It reads data from
the store via the `ConflictStore` trait and produces `Vec<Conflict>` ordered by
descending severity.

## Primary public types

| Type | Purpose |
|---|---|
| `ConflictDetector` | Stateless entry point for all conflict-detection operations |
| `ConflictStore` | Trait: minimal store API required for conflict detection |
| `ConflictError` | Error enum wrapping store errors |

Key types from `g8-core`:

| Type | Purpose |
|---|---|
| `Conflict` | A detected collision with kind, severity, and evidence |
| `ConflictKind` | `DuplicateIntent \| CapabilityNameCollision \| ContradictoryDecisions \| GovernanceViolation` |
| `Severity` | `Info \| Warn \| Error` |
| `Evidence` | Typed evidence variant (capability ref, intent ref, note) |

## Conflict kinds

| Kind | Trigger | Severity |
|---|---|---|
| `DuplicateIntent` | Same `kind + scope_path`, different `description` | Warn |
| `CapabilityNameCollision` | Same name in both spaces, no alias | Error |
| `CapabilityNameCollision` | Same name, alias resolves it | Info |
| `ContradictoryDecisions` | Same title, both accepted, different body | Error |
| `GovernanceViolation` | Plan substrate forbidden by a Boundary intent | Error |

## Entry points

```rust
use g8_conflict::ConflictDetector;

// Compare two stores (local + remote-attached via SQLite ATTACH)
let conflicts = ConflictDetector::compare_spaces(&local, &remote)?;

// Detect conflicts within one store
let conflicts = ConflictDetector::detect_intra_space(&store)?;
```

## Further reading

- Root README: [../../README.md](../../README.md)
- Architecture §6 (conflict detection): [../../ARCHITECTURE.md](../../ARCHITECTURE.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
