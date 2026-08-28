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
  -> g8-obligations, g8-planner, g8-conflict, g8-store, g8-extractor
       -> g8-core
g8-obligations -> g8-core, g8-store
g8-conflict    -> g8-core, g8-store   (trait-level: generic over StoreConnection)
```

The authoritative edge set is OBL-D8-01's `required_edges` in
`specs/obligations-v0.1.json` (14 edges), enforced by `g8 check`'s self-audit.

---

## Deterministic vs non-deterministic zones (the M1 principle)

**All seven crates in v0.1 are DETERMINISTIC.** No LLM calls, no randomness, no
network calls inside core logic. IDs are content-derived (SHA-256, see
`g8-core/src/ids.rs`, Q4 ruling), never random; the one sanctioned randomness
is `g8`'s opt-in `serve` feature (token minting), which is off by default
and outside the deterministic zone.

| Crate | Zone | Justification |
|---|---|---|
| `g8-core` | DETERMINISTIC | Pure data types + serde; content-derived IDs; no I/O |
| `g8-extractor` | DETERMINISTIC | ast-grep subprocess + Markdown parser |
| `g8-store` | DETERMINISTIC | SQLite + refinery; queries are functions of state |
| `g8-planner` | DETERMINISTIC | Pure function over the store |
| `g8-conflict` | DETERMINISTIC | Pure function over two stores |
| `g8-obligations` | DETERMINISTIC | Typed checker backends; subprocess calls to cargo/ast-grep/rg only |
| `g8` | DETERMINISTIC | Argument parsing + dispatch + render (`serve` feature opt-in excepted) |

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

If `~/.claude/agents/g8-planner.md` exists (written by `g8 init`), it
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
# serve feature config (off by default, exercised separately):
cargo test -p g8 --features serve --test serve
cargo clippy -p g8 --features serve --all-targets -- -D warnings
```

**Release gate (not in the default run):** the full 27-obligation self-audit
against the real artifact is `#[ignore]`d because OBL-D7-01 triggers real
cargo builds of both feature configs. Run it explicitly before any release
claim, and CI runs it as the named `obligations` job:

```bash
cargo test -p g8 --test obligations_check -- --ignored --nocapture
```

Tests live in:
- `crates/g8/tests/cli.rs`, integration-style CLI tests
- `crates/g8/tests/smoke_targets.rs`, smoke tests (real codebases)
- `crates/g8/tests/obligations_check.rs`, self-audit wiring + the
  ignored real-artifact end-to-end gate
- `crates/g8/tests/serve.rs`, serve feature-gating + hardening
  (see its header for what it deliberately does and does not cover)
- `crates/g8-obligations/`, checker backends (largest suite)
- `crates/g8-core/`, `crates/g8-store/`, `crates/g8-extractor/`,
  `crates/g8-planner/`, `crates/g8-conflict/`, unit + doctests + crate
  integration tests

Baseline: 0 failures across `cargo test --workspace`, the serve test, and
the ignored obligations gate. Do not break them.

---

## Output contract (stable, locked)

`g8 check --json` always emits valid JSON on stdout even on success:

```json
{"g8_version":"0.1.0","errors":[],"exit_code":0,"enforcement_disabled":true,"obligations":[]}
```

`obligations` is the 27-obligation self-audit (`specs/obligation-checker-contract.md`)
and is populated on EVERY `g8 check` run in an initialized project, including
when enforcement is off (audit checks always run; enforcement affects the EXIT
CODE only). Qualifier (for honesty, not a behavior change): `check` requires an
initialized `.g8/`, an uninitialized tree exits with the internal-failure path
before the self-audit ever loads, so "every run" means "every run that gets past
`require_init`". Each entry:

```json
{
  "id": "OBL-P9-01",
  "status": "passed",
  "trust": "checked",
  "evidence": {
    "backend": "cargo_metadata_no_dep",
    "summary": "...",
    "duration_ms": 42,
    "detail": {},
    "checks_run": [{"backend": "cargo_metadata_no_dep", "status": "passed", "detail": {}, "duration_ms": 42}]
  }
}
```

`status` is one of `passed | failed | error | unknown`; `trust` is one of
`verified | checked | asserted` (`null` iff `status = unknown`). The artifact is
discovered at `<project_root>/specs/obligations-v0.1.json` (`project_root` = the
`.g8/` store's parent directory). If absent, every `g8`-adopting project except
g8's own repository, today, `obligations` is `[]` and a sibling `obligations_note`
string explains why; this is NOT an error and never affects `exit_code` by itself.

When enforcement is on and errors or blocking obligations exist:

```json
{
  "g8_version": "0.1.0",
  "errors": [{"capability":"...","file":"...","line":42,"reason":"no_matching_convergence_test"}],
  "exit_code": 1,
  "obligations": [{"id":"OBL-D11-02","status":"failed","trust":"checked","evidence":{"...":"..."}}]
}
```

`errors`/`exit_code` keep their pre-existing enforcement-gated behavior exactly
(`errors` is only ever populated when enforcement is on); `enforcement_disabled` is
present only when enforcement is off (key absent entirely when on, unchanged shape,
unchanged appearance rule).

Exit codes: `0` = clean/disabled, `1` = `errors` non-empty OR a non-advisory obligation
resolves to `failed`/`error`, `2` = internal failure (store unreachable, malformed
config, unrelated to any one obligation). Obligations whose artifact-declared
`signal.advisory` is `true` never affect exit code regardless of status; `status =
unknown` never affects it either.

Human-readable text always goes to stderr; JSON always goes to stdout.

---

## What to avoid

- Do NOT add LLM calls inside `g8-core`, `g8-store`, `g8-planner`, `g8-conflict`,
  `g8-extractor`, or `g8-obligations`. These are the deterministic zone.
- Do NOT add random or wall-clock-derived IDs. IDs are content-derived
  (`g8_core::ids`, Q4 ruling); the only sanctioned randomness is the opt-in
  `serve` feature's token minting in `g8`.
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
