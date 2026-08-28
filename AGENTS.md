# G8: Agent Onboarding Guide

This file is for Claude, Codex, and any other AI agent working in this repository.
Read it before writing code.

---

## What this codebase does

`g8` is a CLI tool that coordinates plans across
projects by storing capabilities, intents, decisions, and plans in a local SQLite
database (`.g8/store.db`). The primary entry point is `g8`; the load-bearing
logic lives in `g8-planner` and `g8-store`.

See `SPEC.md` for the product specification and `ARCHITECTURE.md` for the technical
realization. Both are binding for v0.1.

---

## Single CLI binary entry point

There is exactly one binary: `g8` in `crates/g8`. Everything else is a
library crate. Never add a second binary. The crate graph is a strict DAG with
`g8-core` as the root and `g8` as the only sink.

```
g8
  -> g8-planner, g8-conflict, g8-store, g8-extractor
       -> g8-core
```

---

## Deterministic vs non-deterministic zones (the M1 principle)

**All six crates in v0.1 are DETERMINISTIC.** No LLM calls, no randomness, no
network calls inside core logic.

| Crate | Zone | Justification |
|---|---|---|
| `g8-core` | DETERMINISTIC | Pure data types + serde; no I/O |
| `g8-extractor` | DETERMINISTIC | ast-grep subprocess + Markdown parser |
| `g8-store` | DETERMINISTIC | SQLite + refinery; queries are functions of state |
| `g8-planner` | DETERMINISTIC | Pure function over the store |
| `g8-conflict` | DETERMINISTIC | Pure function over two stores |
| `g8` | DETERMINISTIC | Argument parsing + dispatch + render |

If you are adding a feature that requires LLM calls (e.g. draft-from-free-text),
it belongs in a new `g8-plan-draft` crate (roadmap) that produces a `PlanDraft`
struct. The deterministic crates never know where the draft came from.

Mixing zones inside one crate is the principal failure mode to avoid.

---

## Composite queries, not verb chains

When querying the store for planner decisions, write **one composite SurrealQL or
SQLite query** with LET/CTE bindings that returns all needed data in one
transaction. Do NOT chain multiple `StoreConnection` calls.

The canonical example is `planner_intent_check` in `crates/g8-store/src/store_impl.rs`
,  it returns six JSON columns in a single read transaction. This is the load-bearing
pattern (SPEC principle 2).

---

## Which agent is appropriate for which crate

| Crate | Appropriate agent |
|---|---|
| `g8-core` | `architect` (type design) or `engineer` (implementation) |
| `g8-extractor` | `engineer` (parser work, ast-grep rules) |
| `g8-store` | `engineer` (SQL, migrations), needs careful schema validation |
| `g8-planner` | `engineer` (post-processing logic), pure Rust, no SQL |
| `g8-conflict` | `engineer` (detection rules) |
| `g8` | `engineer` (subcommand wiring, render); `validator` for acceptance tests |

The `Algorithm` agent is appropriate when designing the recommendation lookup table
(`g8_core::plan::recommend`) or the composite-query SQL shape.

### g8-planner subagent

If `~/.codex/agents/g8-planner.md` exists (written by `g8 init`), it
is a baked subagent definition for the `g8-planner` crate. Use it when a task
requires structuring a `PlanDraft` and interpreting a `FitReport`.

---

## Annotation grammar (v0.1)

The magic-comment form is uniform across all languages:

```
// @g8.capability(name = "...", status = "in_flight", substrate = "...")
// @g8.convergence_test(for_capability = "...", scenario = "...")
```

Each annotation is **single-line in v0.1**. Multi-line continuation (wrapping
with `//` on each line) works via the space-join in `strip_comment_prefixes`, but
error messages will refer to the flat joined string. Per-line error positions are a
v0.2 improvement.

Do not write Rust `#[capability(...)]` proc-macro attributes, those are deferred
to v0.2. The magic-comment form is the only surface in v0.1.

---

## Store schema + migrations

Migrations live at `crates/g8-store/src/migrations/V1__init.sql`. Applied via
`refinery` on first store open. To add a column in a future migration:

1. Add `V2__<name>.sql` to the migrations directory.
2. Never modify `V1__init.sql` after it has been applied to any store.
3. The `RusqliteStore::migrate()` call is idempotent; refinery tracks applied versions.

---

## Build and test

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Tests live in:
- `crates/g8/tests/cli.rs`, 30 unit tests (integration-style, in-process)
- `crates/g8/tests/smoke_targets.rs`, 18 smoke tests (real codebases)
- `crates/g8-core/`, 76 unit + doctests
- `crates/g8-store/`, 22 unit + doctests
- `crates/g8-extractor/`, 13 unit tests

Baseline: 161 tests, 0 failures. Do not break them.

---

## Output contract (stable, locked)

`g8 check --json` always emits valid JSON on stdout even on success:

```json
{"g8_version":"0.1.0","errors":[],"exit_code":0,"enforcement_disabled":true}
```

When enforcement is on and errors exist:

```json
{
  "g8_version": "0.1.0",
  "errors": [{"capability":"...","file":"...","line":42,"reason":"no_matching_convergence_test"}],
  "exit_code": 1
}
```

Exit codes: `0` = clean/disabled, `1` = errors, `2` = internal failure.
Human-readable text always goes to stderr; JSON always goes to stdout.

---

## What to avoid

- Do NOT add LLM calls inside `g8-core`, `g8-store`, `g8-planner`, `g8-conflict`,
  or `g8-extractor`. These are the deterministic zone.
- Do NOT add a second binary target.
- Do NOT add SurrealDB as a dependency. Storage is SQLite via rusqlite (SPEC §4).
- Do NOT modify `specs/smoke-evidence/SMOKE_REPORT.md`, it is a frozen artifact.
- Do NOT modify `research/` files or `specs/o-*.md` files.
- Do NOT write native Rust `#[capability(...)]` attribute parsing, that is v0.2.
- Do NOT use `pip` or `npm`, this is a pure Rust workspace.

---

## Key files

| File | Purpose |
|---|---|
| `SPEC.md` | Product specification (binding) |
| `ARCHITECTURE.md` | Technical realization (binding) |
| `crates/g8-core/src/annotation.rs` | Grammar parser (`parse_annotation_grammar`) |
| `crates/g8-store/src/migrations/V1__init.sql` | Schema DDL |
| `crates/g8-store/src/store_impl.rs` | All composite queries (e.g. `planner_intent_check`) |
| `crates/g8/src/render/json.rs` | Stable JSON output contracts |
| `examples/` | Realistic annotated code for `g8 scan` |
