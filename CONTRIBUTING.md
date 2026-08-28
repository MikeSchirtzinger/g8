# Contributing to g8

Thank you for your interest in contributing.

## Workspace Structure

Six crates in a strict dependency DAG:

| Crate | Role |
|-------|------|
| `g8-core` | Data types, grammar parser, recommendation logic, no I/O |
| `g8-extractor` | ast-grep shell-out + Markdown sidecar scanner |
| `g8-store` | SQLite substrate (rusqlite + refinery); composite planner query |
| `g8-planner` | `FitReport` derivation, calls one store method, pure transformation |
| `g8-conflict` | Cross-store overlap detection |
| `g8` | `g8` binary: clap subcommand tree, rendering, dispatch |

`g8-core` has no internal dependencies. `g8` depends on all others.
Do not introduce dependency cycles.

## Prerequisites

- **Rust 1.78 or later** (workspace MSRV). Install via [rustup](https://rustup.rs).
- **ast-grep**, required by `g8 scan` at runtime:
  ```bash
  cargo install ast-grep   # or: brew install ast-grep
  ```

## Running Tests

```bash
cargo test --workspace
```

218 tests must stay green. Do not submit a PR that reduces the test count
without a clear explanation.

## Lint

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

All warnings are treated as errors. Fix clippy findings before submitting.

## Format

```bash
cargo fmt --all
```

Check only (used in CI):

```bash
cargo fmt --all -- --check
```

## Commit Style

This repository uses **Conventional Commits** where practical
(`feat:`, `fix:`, `chore:`, `docs:`, `test:`), but plain descriptive
messages are also accepted. Keep the subject line under 72 characters.

## Pre-commit Hook

Install the g8 pre-commit hook (runs `g8 check` before each commit):

```bash
g8 init --install-pre-commit
```

This writes `.git/hooks/pre-commit` in the target project. The hook is
advisory, enforcement is off by default until you run `g8 init --enforce`.

## Pull Requests

- Keep PRs focused. One feature or fix per PR.
- Add or update tests for any changed behaviour.
- Do not modify `Cargo.lock` manually; let `cargo` manage it.
- Do not add new workspace dependencies without discussion.

## Code of Conduct

All contributors are expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
