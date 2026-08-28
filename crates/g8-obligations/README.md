# g8-obligations

Typed obligation-checker backends for the [G8](../../README.md).

Executes the 27-obligation self-audit (`specs/obligations-v0.1.json`) that keeps
g8's own repository honest against its own SPEC. Every obligation resolves to
a member of a **closed Rust enum** of checker backends with structured, typed
arguments, there is no `shell_command: String` field anywhere in this crate,
per `specs/obligation-checker-contract.md` §0.

## Role in the workspace

```
g8-core, g8-store  ──►  g8-obligations  ──►  g8
```

`g8-obligations` does not depend on `g8` (that edge runs the other way:
`g8` wires `run_obligations` into `g8 check`). Backends that need to
exercise real CLI behavior (`FixtureIntegrationTest`, `ByteDiffTwice`) reach it
by subprocess-spawning `std::env::current_exe()`, the same subprocess-boundary
trick `g8-extractor` uses for `ast-grep`, just spawning itself.

## Primary public types

| Type | Purpose |
|---|---|
| `CheckerBackend` | Closed 10-variant enum; the only executable checker shape |
| `ObligationArtifact` / `load_artifact` | Deserializes `specs/obligations-v0.1.json`'s `checker` field |
| `run_obligations` | The one entry point: never panics, one `ObligationResult` per obligation |
| `ObligationStatus` / `TrustLevel` | `{passed,failed,error,unknown}` × `{verified,checked,asserted}` |

## Runtime dependencies

`cargo`, `ast-grep`, and `rg` must be on `PATH` for the backends that shell out
to them (`CargoMetadataNoDep`/`CargoMetadataDepGraph`, `AstGrepNoMatch`/
`AstGrepMatchCount`, `RgMatchCount` respectively). A missing binary surfaces as
that one obligation's `status = error`, never a panic and never a silent pass.

## Further reading

- Binding contract: [../../specs/obligation-checker-contract.md](../../specs/obligation-checker-contract.md)
- Root README: [../../README.md](../../README.md)

## License

Licensed under either of Apache License 2.0 or MIT License at your option.
