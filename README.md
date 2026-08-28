# G8

**Pronounced "gate."** Machine-checked obligations for agentic software
development: a structural gate over the compiler, run in pre-commit or CI.

You write each requirement your project must keep as an obligation in
`specs/obligations-v0.1.json`. `g8 check --enforce` verifies every one and fails
the build with the drift named. An agent can refactor, rename, and add all it
likes; it cannot silently relax a rule you ratified.

One binary, one `.g8/` directory next to the code. No service, no LLM calls in
the core, no shell strings in the obligations file.

```
$ g8 check --enforce
[unratified_spec_change] specs/obligations-v0.1.json
    locked  sha256:49e825a4...  (g8.lock)
    current sha256:7c1d02b9...
[failed] OBL-EVENTS-01  rust_enum_shape  crates/core/src/event.rs::EventType
    missing variant: RUN_FINISHED
exit 1
```

---

## Install

```bash
cargo install g8
```

From source:

```bash
git clone https://github.com/MikeSchirtzinger/g8
cd g8
cargo install --path crates/g8
```

`ast-grep` is a runtime dependency for the `ast_grep_*` backends and for
`g8 scan`. `rg` (ripgrep) is required by `rg_match_count`.

```bash
cargo install ast-grep ripgrep   # or: brew install ast-grep ripgrep
```

---

## Set up G8 in a project

Every project that wants a gate does these five steps once. They take about
ten minutes, everything they produce is committed, and the whole sequence
below was run against the `g8` binary before this README was written.

### 1. Initialise

```bash
cd my-project
g8 init --install-pre-commit
```

This creates `.g8/config.toml` and `.g8/store.db`, and installs a
`.git/hooks/pre-commit` that runs `g8 check`. Enforcement starts **off** so you
can audit before you gate. `.g8/store.db` is ignored by git; commit
`.g8/config.toml`.

### 2. Write your first obligations

Create `specs/obligations-v0.1.json`. Each obligation names one requirement and
chooses one checker backend from the closed menu (11 backends; see
[Backends](#backends)). Start small: two or three rules you would be angry to
see broken.

```json
{
  "obligations": [
    {
      "id": "DEPS-NO-OPENSSL-01",
      "title": "No openssl anywhere in the resolved dependency graph",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "cargo_metadata_no_dep",
            "args": {
              "denied": [{ "kind": "exact", "value": "openssl" }],
              "scope_crates": [],
              "build_config": "default"
            }
          }
        ]
      },
      "signal": { "gate": "spec_conformance", "wiring": "repository_artifact", "advisory": false }
    },
    {
      "id": "EVENTS-SHAPE-01",
      "title": "The wire-format event enum is exactly the documented set",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rust_enum_shape",
            "args": {
              "file": "src/event.rs",
              "enum_name": "EventType",
              "expected_variants": ["RUN_STARTED", "RUN_FINISHED", "TEXT_DELTA"],
              "expected_serde_rename_all": "SCREAMING_SNAKE_CASE"
            }
          }
        ]
      },
      "signal": { "gate": "spec_conformance", "wiring": "repository_artifact", "advisory": false }
    },
    {
      "id": "DOCS-ROADMAP-01",
      "title": "The roadmap still marks Phase 2 as active",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "^## Phase 2 .* ACTIVE$",
              "glob": ["docs/roadmap.md"],
              "expected": { "kind": "exactly", "value": 1 }
            }
          }
        ]
      },
      "signal": { "gate": "spec_conformance", "wiring": "repository_artifact", "advisory": true }
    }
  ]
}
```

Rules of thumb:

- Prefer a **structural** backend (`rust_enum_shape`, `cargo_metadata_*`,
  `ast_grep_*`) over a textual one (`rg_match_count`). Text matches give false
  confidence.
- `advisory: true` makes a finding visible without blocking. Use it while you
  tune a rule, then flip it.
- An obligation with no `checker` reports `Unknown` and never gates. That is
  the honest state for a requirement you have not yet found a check for.
- A requirement no backend can express uses `"mode": "attestation"` and gets
  hash-pinned evidence (step 4).

### 3. Ratify

```bash
g8 ratify
```

This writes `g8.lock`, a SHA-256 pin of the obligations file. From now on,
any edit to `specs/obligations-v0.1.json` that is not followed by a fresh
`g8 ratify` makes `check --enforce` fail with `unratified_spec_change`. A gate
cannot be loosened quietly. Commit `g8.lock`.

### 4. Attest the evidence

A typed check passing is `checked`. Under enforcement a gate must be
**trusted**: `verified` (a behavioral backend that actually ran your binary:
`fixture_integration_test`, `byte_diff_twice`, `g8_check_contract`) or
`asserted` (a fresh attestation from a human). So for each structural gate,
pin the files it reads:

```bash
g8 attest DEPS-NO-OPENSSL-01 --files Cargo.toml --evidence "Cargo.toml: no openssl dependency"
g8 attest EVENTS-SHAPE-01   --files src/event.rs --evidence "src/event.rs: three documented variants"
```

The file hashes go into `specs/attestations-v0.1.json`. Commit it. The same
command hash-pins evidence for `"mode": "attestation"` obligations, the ones
no backend can express (a receipt, a generated report).

What happens next is the point:

- an agent adds a variant: `unaccounted_drift`, the check itself fails, exit 1
- an agent edits a pinned file in a way the check does not see (a comment,
  a reordering): `stale_attestation_pin`, exit 1, a human re-attests after
  reading the change
- nobody touches it: the pin stays fresh indefinitely; staleness is content
  based, never wall-clock

Advisory obligations need no attestation; they are visible, never blocking.

### 5. Turn enforcement on and wire CI

```bash
g8 check --enforce        # exit 0 now; exit 1 on any non-advisory failure
g8 init --enforce         # make it the default in .g8/config.toml
```

Exit codes: `0` clean (or enforcement off), `1` a gate failed, `2` internal
error. `g8 check --json` emits the same result as a stable JSON payload.

GitHub Actions:

```yaml
- uses: dtolnay/rust-toolchain@stable
- run: cargo install g8 ast-grep ripgrep --locked
- run: g8 check --enforce
```

Pre-commit was installed in step 1. If you skipped it:

```bash
g8 init --install-pre-commit
```

### What to commit

| Path | Commit? | Why |
|---|---|---|
| `.g8/config.toml` | yes | project name, enforcement mode |
| `.g8/store.db` | no | local scan cache; rebuilt by `g8 scan` |
| `specs/obligations-v0.1.json` | yes | the requirements |
| `g8.lock` | yes | the ratified hash |
| `specs/attestations-v0.1.json` | yes | pinned evidence |

### Rigor floor

Under `--enforce`, a passing gate must also be **trusted**: `Verified` from a
run-backed check, or `Asserted` from a fresh attestation. A bare "checked" pass
reports `insufficient_rigor` and fails. Green means something.

---

## Day to day

```bash
g8 check                  # audit; always exit 0 unless enforcement is on
g8 check --enforce        # gate
g8 check --json           # machine-readable result
g8 ratify                 # after any deliberate obligations edit
g8 attest <ID> --files …  # after evidence legitimately changed
```

When an agent trips a gate, the output names the obligation, the backend, the
file, and the reason. The fix is one of three things: revert the drift, change
the rule and re-ratify, or re-attest the new evidence. Editing `g8.lock` or the
attestation sidecar by hand defeats the model and is the one thing not to do.

---

## Backends

`CheckerBackend` is a closed enum. You pick from it; you cannot add a
`shell_command` variant, so an obligations file can never smuggle arbitrary
execution.

| Tier | Backend | Checks |
|---|---|---|
| Structural | `rust_enum_shape` | a Rust enum's variant set and `rename_all` |
| Structural | `cargo_metadata_no_dep` | deny-list against the resolved dep graph, feature-aware |
| Structural | `cargo_metadata_dep_graph` | required internal edges, bin counts, optional forbid-extra |
| Structural | `ast_grep_no_match` | a forbidden AST shape is absent |
| Structural | `ast_grep_match_count` | a required AST shape appears N times |
| Structural | `builtin_algorithm` | vocabulary drift and other built-in predicates |
| Behavioral | `fixture_integration_test` | seed files, run a CLI, assert outcomes, in a scratch dir |
| Behavioral | `byte_diff_twice` | two runs of a command produce byte-identical output |
| Behavioral | `g8_check_contract` | `g8 check --json` itself conforms to its contract |
| Receipt | `receipt_query` | structural queries over an action receipt (`check --receipt`) |
| Textual | `rg_match_count` | regex line count over a glob |

Plus `"mode": "attestation"` for human evidence.

Shared arg shapes: **Count** `{"kind":"zero|exactly|at_least","value":N}`,
**DepMatcher** `{"kind":"exact|regex","value":"…"}`, **Lang**
`"rust|typescript|python|go"`. Full per-backend args: `skills/g8/BackendReference.md`.

---

## Standing agents: the pre-act gate

A session agent borrows your authority for one task. A standing agent holds
durable authority and acts on its own. For those, the same gate moves one step
earlier:

```bash
g8 check --receipt run-0042.json --enforce
```

Only `action_receipt`-wired obligations run, against the proposed run's JSON
receipt: provenance required on every publish, target domain allow-list, spend
cap, rate cap. The run does not stand until the gate clears. Worked example
with clean and violating receipts: `examples/standing-agent/`.

---

## Planning and multi-project coordination

G8 also carries the coordination layer it started as: annotations in code and
in `AGENTS.md` / `CLAUDE.md` become capabilities, intents, and decisions in
`.g8/store.db`; `g8 plan new` reports overlap, WIP-cap status, and parked
ideas before an agent starts a feature; `g8 merge` detects collisions across
repos; `g8 serve` (build feature `serve`) shows the space as a live graph.

```bash
g8 scan                                       # populate the store
g8 plan new "streaming-export" --substrate "http-ingestion"
g8 merge --from ../other-repo --into .
g8 link --canonical "repoA::http-fetch" --alias "repoB::http-fetch"
```

Annotation grammar (Rust, TypeScript, Python, Go; `@govern.` is still accepted):

```
// @g8.capability(name = "http-fetch", status = "in_flight", substrate = "network")
// @g8.convergence_test(for_capability = "http-fetch", scenario = "timeout")
// @g8.intent(description = "owns outbound HTTP", substrate = "network")
// @g8.decision(title = "use rusqlite", status = "accepted")
// @g8.plan(title = "streaming-export", status = "Idea", substrate = "export-pipeline")
```

Federation across repos uses `.g8/space.toml`; see `SPEC.md` and
`ARCHITECTURE.md` for the full planner, conflict, and dashboard surface.

---

## Crates

| Crate | Role |
|---|---|
| `g8` | the binary: argument parsing, dispatch, rendering |
| `g8-obligations` | the 11 checker backends, attestation, ratification |
| `g8-core` | data types, annotation grammar, check contract (no I/O) |
| `g8-extractor` | ast-grep shell-out and Markdown sidecar scanner |
| `g8-store` | SQLite substrate via rusqlite + refinery |
| `g8-planner` | composite queries and FitReport derivation |
| `g8-conflict` | cross-store overlap detection |

Strict DAG, everything deterministic. Storage is one `.g8/store.db` per
project; cross-project queries use SQLite `ATTACH`.

### Migrating from `govern`

The tool was previously published under the working name `govern`. Existing
projects keep working: a `.govern/` directory is found where `.g8/` would be,
`govern_version` is accepted wherever `g8_version` is read, and `@govern.`
annotations still parse. New projects should use `.g8/`, `g8.lock`, and `@g8.`.

---

## For agents

`skills/g8/SKILL.md` is a drop-in skill for Claude Code and compatible agents.
It routes on the words **g8**, **gate**, and **govern**, and covers authoring an
obligation, running the check, ratifying, and attesting.

---

## Further reading

- `SPEC.md`: design principles, glossary, locked decisions
- `ARCHITECTURE.md`: crate APIs, store schema, query SQL
- `specs/obligation-checker-contract.md`: the `check --json` contract and the receipt addendum
- `examples/`: annotated Rust service, TypeScript frontend, multi-repo space, standing agent

---

## License

MIT OR Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
