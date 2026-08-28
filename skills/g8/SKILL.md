---
name: g8
description: >
  G8 (pronounced "gate", formerly govern) turns a project's requirements into
  machine-checked obligations that hold through agentic development: a structural
  type-checker and drift-alarm layered over the compiler, gated in pre-commit or CI.
  TRIGGER on any of the words g8, gate, or govern used as a tool name. USE WHEN
  setting up g8 in a project (init, first obligations, ratify, pre-commit, CI),
  pointing an agent at a g8-enabled repo, authoring or editing
  `specs/obligations-v0.1.json`, choosing a checker backend, running
  `g8 check`/`ratify`/`attest`, wiring a g8 gate into CI, or asking how to make a
  requirement actually catch drift. Covers per-project onboarding, the obligations
  schema, all 11 checker backends, the ratify + attest trust model, action-receipt
  pre-act gates for standing agents, advisory-vs-gate enforcement, and the
  decoupling rules.
---

# g8: make your requirements hold through agentic development

Per-project setup (init, first obligations, ratify, attest, enforce, CI, and the
deliberate-break proof) is the README section "Set up G8 in a project". Do that
first on any repo without a `g8.lock`.

`g8` is a coordination + verification layer for repos built with AI agents. One
binary, one `.g8/store.db` next to the code, no service, no LLM calls in the core.
Two things it does:

1. **Pre-flight coordination**, `g8 plan new "<title>"` reports overlapping
   in-flight plans, capability collisions, WIP-cap status, parked ideas, and boundary
   violations before an agent starts work.
2. **Requirement enforcement (this skill's focus)**, you write your project's
   requirements as **obligations** in `specs/obligations-v0.1.json`; `g8 check`
   verifies every one and, under `--enforce`, fails the build (exit 1) with the drift
   *named and attributed*. This is how you keep an agent from silently breaking an
   invariant across dozens of edits.

> Mental model in one line: **g8 is a programmable, ratifiable type-checker for
> whatever invariants you declare**, enum shapes, dependency edges, AST patterns,
> CLI behavior, documented facts. The compiler checks Rust's rules; g8 checks
> *yours*.

---

## The two enums people confuse

There are two different enums in play. Getting this right is the whole game:

- **`CheckerBackend`** (g8's, closed, 11 variants), the fixed menu of *check
  mechanisms*. You do **not** extend this; you **pick from** it. Closed by design:
  there is no `shell_command` escape hatch, so an obligation can never smuggle
  arbitrary execution.
- **Your domain enum** (e.g. an `EventType`, a status enum, a config discriminant) , 
  the thing you actually want pinned. The `rust_enum_shape` backend reads *your* enum
  and fails the instant a variant is added, removed, renamed, or its `serde rename_all`
  changes.

You don't "log requirements into a Rust enum." You **write each requirement as an
obligation** in `specs/obligations-v0.1.json`, and each obligation *chooses a backend*
to enforce it.

---

## Lifecycle (the loop an agent runs)

```
g8 init            # scaffold .g8/, optionally install the pre-commit hook
  │
  ├─ author obligations in specs/obligations-v0.1.json   # one per requirement
  │
g8 ratify          # SHA-256-pin the obligations file into g8.lock
  │                    #   → any later edit to the spec without re-ratify = caught
  │
g8 attest <ID> --files ...   # pin the files each structural gate reads (checked -> asserted),
  │                            #   and the proof for evidence/prose obligations
  │
g8 check [--enforce]         # verify everything; --enforce → exit 1 on drift
  │
└─ gate it: pre-commit hook + a CI step running `g8 check --enforce`
```

Re-run `g8 check --enforce` **before and after** any substantive change during
agentic development. When you *add a requirement*, add an obligation and re-ratify.
When you *change the contract itself*, re-ratify (g8 will otherwise flag
`unratified_spec_change`).

---

## The teeth ladder, pick the strongest backend that fits

The single most important skill: **prefer a structural backend that models the actual
invariant over a textual one that merely pattern-matches text.** A textual check
(`rg_match_count`) gives *false confidence*, reword a doc and it flips. A structural
check (`rust_enum_shape`, `cargo_metadata_*`, `ast_grep_*`) is hard to fool because it
inspects the parsed artifact.

Ranked, strongest first:

| Tier | Backends | Use for |
|---|---|---|
| **Structural code facts** | `rust_enum_shape`, `cargo_metadata_no_dep`, `cargo_metadata_dep_graph`, `ast_grep_no_match`, `ast_grep_match_count`, `builtin_algorithm` | Enum/variant completeness, dependency-graph rules (feature-aware), forbidden/required AST shapes, vocabulary drift. **Default here.** |
| **Behavioral (sandboxed CLI)** | `fixture_integration_test`, `byte_diff_twice`, `g8_check_contract` | Invariants about a *CLI's* behavior: seed files → run argv → assert exit code / stdout / file hashes; or determinism (same input → byte-identical output). Runs `g8` by default, or `cargo run -p <pkg>`. **Not a `cargo test` runner**, it drives a binary, not your crate's unit tests. |
| **Receipt (pre-act)** | `receipt_query` | Structural queries over an *action receipt* (a JSON log of an agent run) under `g8 check --receipt`, provenance, allowlists, spend/rate caps. Only valid in `action_receipt`-wired obligations. |
| **Textual (last resort)** | `rg_match_count` | "This marker exists / is absent in this file." Presence/absence of a documented fact. Brittle, use only when no structural backend models the requirement. |
| **Human evidence** | *(mode: `attestation`)* | Requirements no mechanism can check. Hash-pin the proof with `g8 attest`; it auto-goes-stale when the evidence file changes. Beats a weak `rg` check that pretends to verify. |

**Decision rule:** if a structural backend can express the requirement, use it. If not,
use a behavioral one. If neither, prefer an *attestation* over a textual check , 
false confidence is worse than an honest "this is human-attested."

---

## Obligation anatomy

`specs/obligations-v0.1.json` deserializes to `{ meta?, obligations[], conflicts?, open_questions? }`.
Unknown top-level/obligation fields are ignored (you may keep `descends_from`, `rule`,
`scope`, prose, etc. for humans). Each obligation:

```jsonc
{
  "id": "AREA-THING-01",              // required, stable, unique
  "checker": {                         // omit → status Unknown (prose-only, never a gate)
    "mode": "typed",                   // "typed" | "attestation" | "prose_only"
    "checks": [ { "backend": "...", "args": { ... } } ]   // ALL must pass
  },
  "signal": {                          // all fields optional
    "gate": "spec_conformance",        // free-text label for the gate family
    "wiring": "repository_artifact",   // "action_receipt" → subject is a receipt, not the repo
    "advisory": false                  // false = blocking gate; true = visible-but-non-blocking
  }
}
```

- `checker.mode: "typed"` → run the `checks[]` (every check must pass).
- `checker.mode: "attestation"` → `{ "attestation_id": "ATT-..." }`; passes only if a
  fresh, hash-matching attestation exists.
- `checker.mode: "prose_only"` → documented, never mechanically enforced (status Unknown).
- No `checker` at all → also Unknown. **An obligation with no real checker is not a
  gate**, don't mistake a prose obligation for enforcement.
- `signal.advisory: true` → failures are *shown in every run but never change the exit
  code*. Use for aspirational/known-debt items you refuse to hide.

Every check object is `{ "backend": "<snake_case>", "args": { <that backend's struct> } }`.

Shared arg shapes:
- **Count** (`expected`): `{"kind":"zero"}` · `{"kind":"exactly","value":N}` · `{"kind":"at_least","value":N}`
- **DepMatcher**: `{"kind":"exact","value":"surrealdb"}` · `{"kind":"regex","value":"(?i)mcp"}`
- **AstGrepLang / BuildConfig**: `"rust"|"typescript"|"python"|"go"` · `"default"` or `{"features":["x"]}`

---

## The 11 backends (valid `args` for each)

```jsonc
// 1. rust_enum_shape, pin a Rust enum's variant SET + rename_all.
//    expected_variants are WIRE-FORMAT names (post rename_all), order-independent.
{ "backend": "rust_enum_shape", "args": {
    "file": "crates/ag-ui-core/src/event.rs", "enum_name": "EventType",
    "expected_variants": ["TEXT_MESSAGE_START", "RUN_STARTED", "..."],
    "expected_serde_rename_all": "SCREAMING_SNAKE_CASE" } }   // null if the enum has none

// 2. cargo_metadata_no_dep, deny-list against the RESOLVED dep graph (feature-aware).
{ "backend": "cargo_metadata_no_dep", "args": {
    "denied": [{"kind":"exact","value":"surrealdb"}],
    "scope_crates": [], "build_config": "default" } }

// 3. cargo_metadata_dep_graph, required internal edges + [[bin]] counts; forbid extras.
{ "backend": "cargo_metadata_dep_graph", "args": {
    "required_edges": [["g8-store","g8-core"]],
    "forbid_extra_edges": true, "expected_bin_targets": [["g8",1]] } }

// 4. ast_grep_no_match, AST patterns that must NOT appear.
{ "backend": "ast_grep_no_match", "args": {
    "patterns": ["#[capability($$$ARGS)]"], "lang": "rust",
    "glob": ["crates/x/src/**"], "exclude_glob": [], "capture_predicate": null } }

// 5. ast_grep_match_count, AST match count, optionally scoped to one function.
{ "backend": "ast_grep_match_count", "args": {
    "pattern": "store.$METHOD($$$ARGS)", "lang": "rust", "glob": ["src/lib.rs"],
    "scope": {"function": "plan_check"}, "expected": {"kind":"exactly","value":1},
    "capture_equals": ["METHOD","planner_intent_check"] } }

// 6. rg_match_count, regex LINE count (respects .gitignore). Textual → last resort.
{ "backend": "rg_match_count", "args": {
    "pattern": "^### Phase 2 .* ACTIVE$", "glob": ["docs/roadmap.md"],
    "expected": {"kind":"exactly","value":1} } }

// 7. fixture_integration_test, seed files → run CLI argv steps → assert outcomes.
//    Drives `g8` (default) or cargo_run a package. NOT your crate's cargo test.
{ "backend": "fixture_integration_test", "args": {
    "seed_files": [{"path":"src/lib.rs","content":"// @g8.capability(...)\n"}],
    "setup": [ {"args":["init"],"env":{}}, {"args":["scan","--no-watch"],"env":{}} ],
    "assertions": [ {"kind":"exit_code","from_step":1,"equals":0},
                    {"kind":"file_exists","path":".g8/store.db"} ] } }

// 8. byte_diff_twice, run setup once, run `compare` twice, byte-compare stdout
//    after masking declared timing fields ($..key). Determinism probe.
{ "backend": "byte_diff_twice", "args": {
    "seed_files": [], "setup": [{"args":["init"],"env":{}}],
    "compare": {"args":["query","capabilities","--output","json"],"env":{}},
    "normalize_paths": ["$..duration_ms"] } }

// 9. g8_check_contract, validate the SHAPE of `g8 check --json` in-process.
{ "backend": "g8_check_contract", "args": {
    "scenarios": ["clean_store","known_bad_capability_fixture"] } }

// 10. builtin_algorithm, named, hand-reviewed deterministic function (closed set).
{ "backend": "builtin_algorithm", "args": {
    "name": "vocabulary_drift",
    "params": { "canonical_terms": ["Annotation","Capability"],
                "allowlist": ["PlanStatus"], "scope_glob": ["crates/core/src/**"] } } }

// 11. receipt_query, structural query over an ACTION RECEIPT (receipt mode only;
//     the obligation must carry signal.wiring = "action_receipt"). See §receipts.
{ "backend": "receipt_query", "args": {
    "select": "$.actions[*]",                       // [*] explodes an array into candidates
    "where": [ {"path":"$.kind","op":"eq","value":"publish"},
               {"path":"$.source_ref","op":"absent"} ],   // ANDed; ops: eq ne matches in
    "aggregate": {"kind":"count"},                  //   not_in exists absent gt gte lt lte
    "expected": {"kind":"zero"} } }                 // zero|exactly|at_least|at_most
```

Full `fixture_integration_test` assertion kinds: `file_exists`, `file_hash_unchanged`,
`file_header_equals`, `file_contains`, `json_field`, `json_path_non_empty`, `exit_code`,
`stderr_format`, `stdout_contains_all`, `stdout_not_contains`, `no_annotation_source_file`,
`second_connection_errs_after_timeout`.

---

## Worked example, giving one requirement real teeth

**Requirement:** "The `EventType` enum must always be exactly the AG-UI spec's event
set, serialized as `SCREAMING_SNAKE_CASE`." An agent doing a spec catch-up could add,
drop, or misname a variant across a big diff, a textual grep wouldn't reliably catch it.

Wrong (textual, brittle): an `rg_match_count` that "`TEXT_MESSAGE_START` appears
somewhere." Passes even if three other variants were deleted.

Right (structural, `rust_enum_shape`): declare the full variant set once. Now any
add/remove/rename, or a `rename_all` change, fails `g8 check` with the exact enum
named:

```json
{ "id": "AGUI-EVENTTYPE-SHAPE-01",
  "checker": { "mode": "typed", "checks": [ { "backend": "rust_enum_shape", "args": {
    "file": "crates/ag-ui-core/src/event.rs", "enum_name": "EventType",
    "expected_variants": [ "TEXT_MESSAGE_START", "...all 38..." ],
    "expected_serde_rename_all": "SCREAMING_SNAKE_CASE" } } ] },
  "signal": { "gate": "spec_conformance", "advisory": false } }
```

Then **`g8 ratify`** so the contract itself is hash-pinned, and gate it in CI.
Pair it with an `rg_match_count` guard that forbids per-variant `serde(rename = ...)`
overrides (which would silently break the wire contract), and one that asserts the
conformance test file still exists (so the safety net can't be quietly deleted).
Structural where it counts; textual only for "this artifact is present."

---

## Trust, ratify, and attest, why a green check is meaningful

A passing check is only worth something if a failing one is possible and *attributable*.
g8 gives every gate a trust tier and two tamper-evident locks:

- **`g8 ratify`** hashes `specs/obligations-v0.1.json` into `g8.lock`
  (`{g8_version, spec_path, spec_sha256}`). If an agent later edits the spec to
  *relax a gate* without re-ratifying, `check --enforce` fails with
  **`unratified_spec_change`**, printing the locked vs current hash. You cannot weaken
  the contract silently.
- **`g8 attest <ID> --files <paths> [--evidence <pointer>]`** hash-pins evidence
  for an obligation into `specs/attestations-v0.1.json`. If any attested file changes
  after attestation, the pin goes stale and the gate fails with
  **`stale_attestation_pin`**, naming the attestation and the file. Staleness is
  content-based, never wall-clock.
- **Rigor floor:** under `--enforce`, a gate obligation must be `Passed` *and* trusted
  (`Verified` from a run-backed check, or `Asserted` from a fresh attestation). A merely
  "checked" pass without attestation is `insufficient_rigor` for a gate.

So the two ways to drift are both caught and *named*: edit attested evidence →
`stale_attestation_pin`; edit the spec to loosen a gate → `unratified_spec_change`.

---

## Enforcement, advisory, exit codes

- `g8 check`, always runs the full self-audit and emits JSON. **Exit 0 always**
  when enforcement is off (prints advisories).
- `g8 check --enforce`, invocation-local enforcement (does *not* rewrite
  `config.toml`). **Exit 1** if any *gate* (non-advisory) obligation fails; **exit 0**
  if clean; **exit 2** on an internal error (e.g. store won't open).
- `advisory: true` obligations surface their failures in every run but never change the
  exit code, the honest way to keep known debt visible instead of hidden.
- `g8 init --enforce` bakes enforcement into `.g8/config.toml`;
  `g8 init --install-pre-commit` drops a hook that runs `g8 check`.

---

## Action receipts, the pre-act gate for standing agents

`g8 check --receipt <run.json> [--enforce]` points the same machinery at a JSON
log of an agent's proposed/executed actions, the choke point moves from pre-commit to
**pre-act** (worked example:
`examples/standing-agent/`).

- Obligations opt in with `signal.wiring: "action_receipt"`; only those run in receipt
  mode (repo-wired obligations and the pairing check are skipped), and in repo mode
  they surface as Unknown/scoped, never gating. Receipt-wired obligations may only use
  `receipt_query`, and vice versa, a mismatch is a loud Error.
- Ratification is still enforced: an agent editing its own obligations mid-run hits
  `unratified_spec_change`, it cannot weaken its own leash.
- Failed receipt gates classify as **`receipt_violation`**; a missing/malformed receipt
  is **`invalid_receipt`** (a refusal under `--enforce`, exit 1, never exit 2). The
  JSON output embeds the receipt's SHA-256: the check output *is* the decision record
  (batch hash × ratified contract hash × verdict).
- Rigor: `receipt_query` is tier Checked; in receipt mode `Passed` + Checked clears the
  floor (the subject is content-hashed inline at evaluation, see contract addendum).
  Post-hoc, pin an executed receipt into the ledger with `g8 attest <existing-OBL-id>
  --files runs/<id>.json` (attest pins evidence for an existing obligation; it does not
  mint new IDs).
- `where` missing-path semantics are fail-closed for deny patterns: positive ops
  (`eq/in/matches/exists/gt/gte/lt/lte`) fail on a missing path; negative ops
  (`ne/not_in/absent`) are satisfied, so "domain `not_in` allowlist must be zero"
  counts an action with *no* domain as violating. `sum` over a missing/non-numeric
  value on a matched candidate is a loud Error, never a silent skip.

---

## Decoupling, do NOT wire g8 into the build

g8 is a *development-coordination* layer, not a runtime dependency. Everything it
writes into a repo is inert:

- `g8.lock`, `specs/*.json`, `.g8/config.toml`, `.g8/store.db`,
  `.g8/INTENT_SUMMARY.md`, plain JSON/TOML/SQLite/Markdown. No Cargo manifest
  references them. **Never add a `build.rs`, a crate dep, or runtime code that calls the
  g8 CLI or reads its store.**
- The only executable g8 installs is the pre-commit hook, and it **self-skips when
  g8 isn't on PATH** (`command -v g8 … || exit 0`), so contributors without
  g8 still `git commit`, and `cargo build` / `cargo test` pass g8-free.
- There is **no shell-command backend**, the 11-variant `CheckerBackend` enum is closed;
  the answer to "my check doesn't fit" is to escalate for a new binding, not to shell out.

This is what lets you dogfood g8 as *your* maintainer ratchet in a repo you also ship
publicly: the g8 instance + "maintained with g8" attribution travel with the
repo, but every contributor's build stays independent of g8.

---

## The agentic-development operating procedure

When pointed at a g8-enabled repo, an agent should:

1. **Read the contract first.** Open `specs/obligations-v0.1.json` and
   `.g8/INTENT_SUMMARY.md` (auto-generated: in-flight plans, WIP caps, boundary
   alerts, top intents). Run `g8 check` to see current status.
2. **Before touching a substrate,** `g8 plan new "<title>" --touches <substrate>` to
   surface collisions and parked ideas.
3. **When adding a requirement,** add an obligation with the *strongest fitting backend*
   (teeth ladder), then `g8 ratify`.
4. **After a change,** run `g8 check --enforce`. If a gate fails, fix the code, do
   not relax the obligation. If the requirement genuinely changed, edit the spec
   deliberately and re-ratify (expect `unratified_spec_change` until you do).
5. **For claims no backend can check,** attest evidence (`g8 attest`) rather than
   faking a check.
6. **Never** couple runtime code to g8, and never add a backend/shell escape hatch.

---

## Anti-patterns

- **Textual check where a structural one exists**, `rg_match_count` on an enum name
  instead of `rust_enum_shape`. False confidence.
- **Prose obligation mistaken for a gate**, no `checker`, or `mode: prose_only`, yields
  status Unknown; it enforces nothing.
- **Editing the spec without re-ratifying**, silently intended as tightening, reads as
  `unratified_spec_change`. Always `g8 ratify` after a deliberate contract change.
- **Attesting then editing the evidence**, the pin goes stale and the gate fails; re-run
  `g8 attest` after the evidence legitimately changes.
- **Wiring g8 into `build.rs`/runtime**, forbidden; breaks the "builds without
  g8" guarantee and turns a coordination tool into a dependency.
- **Using `fixture_integration_test` as a unit-test runner**, it drives a CLI in a
  sandbox, not your crate's `cargo test`. Keep `cargo test` as the portable layer.

---

## Command cheatsheet

```
g8 init [--enforce] [--install-pre-commit] [--name <n>]   # scaffold
g8 scan [PATH] [--no-watch] [--report-only]               # extract annotations → store
g8 status [--no-regen] [--intra-conflicts]                # regenerate INTENT_SUMMARY
g8 plan new "<title>" --touches <substrate> [--park]      # pre-flight coordination
g8 check [--enforce] [--json] [--project <p>]             # verify obligations (exit 0/1/2)
g8 check --receipt <run.json> [--enforce]                 # pre-act gate over an action receipt
g8 ratify                                                 # hash-pin the spec → g8.lock
g8 attest <ID> --files <paths...> [--evidence <ptr>]      # hash-pin evidence
g8 query "capabilities|intents|drift|parked ..."         # ad-hoc store query
g8 space add <PATH> [--name n] | link | substrate add    # federation / boundaries
```

Global flags: `--output pretty|json`, `--store <path>`, `--space <id>`, `-v/-vv/-vvv`,
`--quiet`, `--no-color`.
