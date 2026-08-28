# g8-planner

Composite-query orchestration and deterministic recommendation derivation for
the [G8](../../README.md).

This crate is a **pure deterministic transformer** over `StoreConnection`. It
calls exactly one store method, `planner_intent_check`, and post-processes
the result into a structured `FitReport` with a deterministic `Recommendation`.
No LLM calls, no async runtime, no HTTP, no direct file I/O.

## Role in the workspace

```
g8-core  ──►  g8-planner  ──►  g8
g8-store ──►  g8-planner
```

`g8-planner` is the decision layer: given a `PlanDraft` describing a proposed
feature, it answers "should this proceed?" using only in-store data.

## Primary public types

| Type | Purpose |
|---|---|
| `Planner` | Stateless entry point; use `Planner::plan_check(store, draft)` |
| `PlannerError` | Error variants wrapping store / core errors |

Key types consumed and returned are from `g8-core`:

| Type | Purpose |
|---|---|
| `PlanDraft` | Input: title, substrate, touched capabilities/paths |
| `FitReport` | Output: existing matches, budget, bottlenecks, drift hints, parked ideas, recommendation |
| `Recommendation` | Deterministic verdict: `Proceed \| Park \| Wait \| Extend \| Drop \| Pivot` |

## Usage

```rust
use g8_planner::Planner;
use g8_core::plan::PlanDraft;
use g8_store::RusqliteStore;

let mut store = RusqliteStore::open_in_memory()?;
store.migrate()?;

let draft = PlanDraft {
    title: "streaming-export".into(),
    substrate: Some("export-pipeline".into()),
    ..Default::default()
};
let report = Planner::plan_check(&store, &draft)?;
assert_eq!(report.recommendation, g8_core::Recommendation::Proceed);
```

## Determinism guarantee

Same `PlanDraft` + same store state → same `FitReport` and `Recommendation`
every time. No randomness, no external calls.

## Further reading

- Root README: [../../README.md](../../README.md)
- Architecture §5 (planner): [../../ARCHITECTURE.md](../../ARCHITECTURE.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
