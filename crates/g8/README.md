# g8

Command-line interface for the [G8](../../README.md).

This crate builds the `g8` binary. It contains zero domain logic, argument
parsing, context construction, dispatch to subcommand handlers in `cmd/`, and
rendering to stdout. All computation happens in the library crates.

## Role in the workspace

```
g8-core      ──►
g8-extractor ──►
g8-store     ──►  g8  →  `g8` binary
g8-planner   ──►
g8-conflict  ──►
```

`g8` is the top of the crate DAG. It wires everything together and
exposes the user-facing `g8` command.

## Install

```bash
# From crates.io (once published)
cargo install g8

# From source
cargo build --release -p g8
# binary at: target/release/g8
```

## Subcommands

| Command | Purpose |
|---|---|
| `g8 init` | Initialise `.g8/` in the project, optionally install pre-commit hook |
| `g8 scan [path]` | Extract annotations + sidecar files, populate store |
| `g8 plan new "<title>"` | Run the planner, get a `FitReport` + `Recommendation` |
| `g8 plan list` | List plans (filter by status, substrate, etc.) |
| `g8 plan promote <id>` | Promote a Parked plan to InFlight |
| `g8 merge --from <path>` | Detect conflicts when combining two stores |
| `g8 check` | Pairing-gate CI check (exit 1 on errors) |
| `g8 substrate add <name>` | Register a substrate with optional WIP cap |
| `g8 link` | Resolve capability alias across spaces |
| `g8 space add` | Add a member to a multi-project convergence space |

## Output modes

All commands accept `--json` (machine-readable JSON) or default to
human-friendly tables/text. Traces go to stderr; stdout stays clean for
structured output.

## Runtime dependency

`g8 scan` requires `ast-grep` on `PATH`:

```bash
cargo install ast-grep   # or: brew install ast-grep
```

## Further reading

- Root README: [../../README.md](../../README.md)
- Architecture §12 (CLI tracing): [../../ARCHITECTURE.md](../../ARCHITECTURE.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
