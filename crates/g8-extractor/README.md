# g8-extractor

Annotation extractor for the [G8](../../README.md).

Detects `// @g8.<kind>(...)` magic-comment annotations across Rust, TypeScript,
Python, and Go source files via the `ast-grep` CLI shell-out. Also parses
`AGENTS.md`, `CLAUDE.md`, and `*.g8.md` sidecar Markdown files into typed
annotations without requiring any inline comments.

## Role in the workspace

```
g8-core  ──►  g8-extractor  ──►  g8
```

`g8-extractor` sits between the raw filesystem and the store: it reads source
and Markdown files, produces `Vec<Annotation>`, and hands them to `g8-store`
for persistence.

## Runtime dependency

The `ast-grep` binary (`sg`) must be on `PATH`:

```bash
cargo install ast-grep   # or: brew install ast-grep
```

The extractor returns `ExtractorError::BinaryNotFound` if the binary is absent
at construction time.

## Primary public types

| Type | Purpose |
|---|---|
| `Extractor` | Main entry point; constructed via `Extractor::new()` or `Extractor::with_binary()` |
| `ScanResult` | Output of `Extractor::scan_dir`: `Vec<Annotation>`, warnings, stats |
| `ExtractorError` | Error variants: `BinaryNotFound`, `AstGrepFailed`, `ParseError`, `IoError` |

## Supported annotation kinds

`capability`, `convergence_test`, `intent`, `decision`, `plan`, all sharing the
uniform `// @g8.<kind>(...)` grammar. Python files accept `# @g8.<kind>(...)`.

## Sidecar Markdown mapping

| Heading | Intent kind |
|---|---|
| `# Architecture` / `# Overview` | `ArchitecturalScope` |
| `# Boundaries` / `# Never do` | `Boundary` |
| `# Stack` / `# Dependencies` | `TechStack` |
| `# Commands` / `# Testing` | `Operational` |
| `# Decisions` | `Decision` |
| `# Parked ideas` | `Plan (status=Parked)` |

## Further reading

- Root README: [../../README.md](../../README.md)
- Spec §4 (annotation grammar): [../../SPEC.md](../../SPEC.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
