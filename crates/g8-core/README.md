# g8-core

Canonical data model for the [G8](../../README.md).

This crate is the root of the g8 crate DAG. Every other workspace crate
depends on it. It owns all shared types and contains zero I/O, no tokio,
no rusqlite, no filesystem access.

## Role in the workspace

```
g8-core  <──  g8-extractor
          <──  g8-store
          <──  g8-planner
          <──  g8-conflict
          <──  g8
```

`g8-core` has no workspace siblings as dependencies; it is the foundation
everything else builds on.

## Primary public types

| Type | Purpose |
|---|---|
| `Annotation`, `AnnotationKind`, `AnnotationValue` | Parsed `@g8.<kind>(...)` annotation |
| `RawAnnotation`, `SourceLocation` | Pre-parse wire form + file location |
| `Capability`, `CapabilityStatus` | Named capability annotation |
| `Intent`, `IntentKind`, `IntentSourceKind` | Architectural intent annotation |
| `Plan`, `PlanStatus` | Feature plan (Idea / InFlight / Parked / Done) |
| `Decision`, `DecisionStatus` | Architectural decision record |
| `PlanDraft` | Input to the planner |
| `FitReport`, `Recommendation` | Planner output |
| `Conflict`, `ConflictKind`, `Evidence`, `Severity` | Conflict-detection output |
| `ConvergenceSpace`, `Project` | Top-level domain aggregates |
| `SubstrateBudget` | WIP-cap configuration per substrate |
| `G8Error` | Top-level error enum |
| `EpochMillis` | Millisecond-epoch timestamp newtype |

## Typed IDs

All entity IDs are nanoid-backed newtypes in `g8_core::ids`:
`SpaceId`, `ProjectId`, `PlanId`, `CapabilityId`, `IntentId`, `DecisionId`, `AnnotationId`.

## Free functions

```rust
let ms: EpochMillis = g8_core::now_millis();
```

## Further reading

- Root README: [../../README.md](../../README.md)
- Spec: [../../SPEC.md](../../SPEC.md)
- Architecture: [../../ARCHITECTURE.md](../../ARCHITECTURE.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
