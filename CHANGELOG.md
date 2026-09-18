# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [0.1.2] - 2026-09-18

All seven crates move to one workspace version from this release on.
0.1.1 was a CLI-only republish that dropped hardcoded smoke-test paths from
the published `g8` crate; the library crates stayed at 0.1.0.

### Changed

- **Renamed to G8** (pronounced "gate"). Crates `govern-*` are now `g8-*` and
  the CLI crate and binary are `g8`. The project directory is `.g8/`, the
  ratification lock is `g8.lock`, JSON payloads carry `g8_version`, env vars
  use the `G8_` prefix, and annotations are `@g8.<kind>(...)`. Legacy inputs
  keep working: a `.govern/` directory is found where `.g8/` would be,
  `govern_version` is accepted on read, and `@govern.` annotations still parse.
- README rewritten around per-project onboarding: init, first obligations,
  ratify, attest, enforce, CI.

### Fixed

- Stores created by govern 0.1.0 could not be opened: the rename edited
  `V1__init.sql` in place, so refinery saw a different checksum for an
  already-applied migration and refused every such store ("running store
  migrations"). V1 is restored byte-for-byte and pinned by a test against
  the checksum govern 0.1.0 recorded; the `govern_sidecar` to `g8_sidecar`
  rename now lives in `V2__g8_sidecar.sql`, which rebuilds `intent` and
  rewrites existing rows. `migrate()` switches foreign keys off around the
  run and runs `PRAGMA foreign_key_check` afterwards. A store that has taken
  V2 is no longer readable by govern 0.1.0.
- Multi-file attestation pins recorded by `govern attest` read as stale under
  g8: the rename changed the hash domain separator. Verification accepts the
  govern separator as well and reports `pin_scheme` (`g8-v1` or `govern-v1`)
  in the attestation detail. New pins are written with the g8 separator only.
- `govern.lock` is read where `g8.lock` is absent, so a project ratified
  before the rename is still ratified. `g8.lock` wins when both exist.
- `.govern/config.toml` written by `govern init` uses a `[govern]` table;
  it is now read as `[g8]`.
- `g8 check` printed only the outermost error label on an internal failure
  (for example "running store migrations" without the refinery cause). The
  full error chain is printed and carried in the JSON `obligations_note`.
  The same truncation is fixed in the context, status, scan, and init
  warnings.
- `g8 ... | head` panicked with "failed printing to stdout: Broken pipe"
  (exit 101). SIGPIPE is restored to its default disposition for every
  command except `serve`, so a closed pipe ends the process quietly.
- The CLI round-trip test pinned `g8_version` to `0.1.0` and failed since
  the 0.1.1 bump; it now compares against the crate version.
- Plan ids that begin with `-` (the id alphabet includes it) were rejected
  by the `plan` subcommands as unknown flags. Positional ids now accept
  hyphen-leading values.

### Added

- **`g8 serve`**, a dependency-light live web dashboard for the convergence
  space (synchronous `tiny_http`, no async runtime; isolated to `g8`).
  Renders plans (colored by status), capabilities, intents, decisions, and the
  substrates that group them as an interactive force-directed graph. Endpoints:
  `GET /` (embedded SPA), `/api/graph`, `/api/scene` (AgentViz scene export),
  `/events` (Server-Sent Events; pushes on any `.g8/store.db` change via a
  `notify` file-watch), `/api/version`, and `POST` action routes
  (`/api/plan`, `/api/plan/<id>/<action>`, `/api/substrate`) that mutate the
  store through the typed `g8-store` API (lifecycle state machine enforced).
  Live shared state: a CLI edit and a dashboard click both reflect in every open
  tab within a frame.
- **AgentViz integration**, `GET /api/scene` emits a valid AgentViz scene
  (`graph_explorer`); the dashboard's "Open in AgentViz" button deep-links to
  the explorer's `g8-live` preset, which pulls the live scene over HTTP.
- **Action receipts, `g8 check --receipt <path>`**, a second choke
  point alongside `check`'s existing pre-commit gate, this one **pre-act**:
  checks a *proposed or executed agent action receipt* (any JSON document)
  instead of the repository (contract addendum:
  `specs/obligation-checker-contract.md` §12). New backend
  #11, `receipt_query`, a structural evaluator (`select`/`where`/`aggregate`/
  `expected`) over the parsed receipt, reusing the existing `FixtureIntegrationTest`
  JSON-path dialect (now factored into `g8-obligations::exec::json_path`)
  plus a `[*]` array-explode addition. New `Signal.wiring = "action_receipt"`
  value: receipt-wired obligations are exempt from repo mode's gate
  classification (`status: unknown`, `detail.scope: "action_receipt"`) and
  only run under `--receipt`; repo-wired obligations are skipped entirely in
  receipt mode. Ratification is still enforced in receipt mode. New
  enforcement classifications `receipt_violation` and `invalid_receipt`
  (malformed/missing `--receipt` file → exit 1 under `--enforce`, 0 without,
  never exit 2). Worked example: `examples/standing-agent/`.

### Fixed

- **WIP-cap miscount**, the `planner_intent_check` budget CTE counted only
  `Dispatched` plans, so a `Blocked` plan silently freed a WIP slot, contrary
  to SPEC §7 ("Blocked doesn't free a slot"). WIP now counts `Dispatched` **and**
  `Blocked`, matching the documented intent and the `bottlenecks` CTE.
- **`attach_remote_store` URI escaping**, a remote `--from` path containing
  `?`, `#`, `%`, or a quote could be truncated at the URI query separator or
  break out of the SQL string literal. The path is now percent-encoded and the
  full `file:` URI is bound as a parameter; the attach alias is validated as a
  bare identifier.

## [0.1.0] - 2026-05-27

### Added

- **`g8-core`**, data types, magic-comment grammar parser (`@g8.<kind>(...)`),
  `Recommendation` enum with deterministic lookup table, status-transition
  validation, and all shared enums (`PlanStatus`, `CapabilityStatus`,
  `IntentKind`, `AnnotationKind`, `ConflictKind`, `Severity`).
- **`g8-extractor`**, `Extractor::scan_dir` with ast-grep CLI shell-out for
  Rust/TypeScript/Python/Go source files plus a native Markdown-section parser
  for `AGENTS.md`, `CLAUDE.md`, and `*.g8.md` sidecar files. Conditional /
  `cfg_attr` / tests-dir annotation flags.
- **`g8-store`**, `StoreConnection` trait + `RusqliteStore` implementation
  backed by rusqlite (bundled SQLite) and refinery migrations. Includes the
  six-CTE `planner_intent_check` composite query returning a full `FitReport`
  payload in a single read transaction, plus `pairing_check`,
  `stale_dispatched_plans`, and cross-project `attach_remote_store` /
  `detach_remote_store` via SQLite `ATTACH DATABASE`.
- **`g8-planner`**, `Planner::plan_check` orchestration: calls one store
  method, deserializes six JSON payloads, assembles `FitReport` with a
  deterministic `Recommendation` (`Proceed | Park | Wait | Extend | Drop | Pivot`).
  No LLM calls; no async.
- **`g8-conflict`**, `ConflictDetector::compare_spaces` and
  `detect_intra_space` with four conflict kinds: `DuplicateIntent`,
  `CapabilityNameCollision`, `ContradictoryDecisions`, `GovernanceViolation`.
  Alias resolution downgrades name collisions to `Info` severity.
- **`g8`**, `g8` binary (clap derive). Subcommands: `init`, `scan`,
  `plan`, `check`, `merge`, `status`, `substrate`, `link`, `config`. Stable
  JSON output contract (`g8_version`, `errors`, `exit_code`). Pre-commit hook
  install via `g8 init --install-pre-commit`. Two output modes: pretty
  (Markdown to stdout) and `--json`.
- **Multi-project spaces**, `.g8/space.toml` federation; per-substrate WIP
  caps; idea park (`Plan` status `Parked`); stale-promotion proposals.
- **AGENTS.md / CLAUDE.md ingestion**, `g8 scan` extracts intents from
  Markdown headings without requiring inline annotations; `g8 init` appends
  `@.g8/INTENT_SUMMARY.md` import to `CLAUDE.md`.
- **218 tests** passing across all six crates on Rust 1.78 (including 9 P0-regression tests covering composite-query budget materialisation, dispatch gating, status-transition validation, structured error output, capability link persistence, root-scope intent listing, and governance-classification preservation across scans).

### Notes

This is the initial public release. The `g8-plan-draft` LLM integration crate
and native Rust attribute extraction are deferred to v0.2.

[Unreleased]: https://github.com/MikeSchirtzinger/g8/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MikeSchirtzinger/g8/releases/tag/v0.1.0
