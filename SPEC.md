# `g8`, G8

**Product specification, v0.1**
**Status:** Architected 2026-05-26. Authored by A1 from Phase 0 research synthesis.
**Audience:** Phase 2 builders (B1–B6), Phase 3 integrator (I1), Phase 4 polisher (P1), and any human or agent installing `g8` into a target codebase.

---

## 1. Elevator pitch

`g8` is a lightweight Rust developer tool that gives a planning agent (human or AI) the single missing dimension that plain code search, call graphs, and AGENTS.md files don't cover: **who else is planning to touch this seam right now, and what's already been ruled out**. It works across one repo or many, requires no service to run, and stores its substrate in a `.g8/store.db` SQLite file next to the code it indexes.

You annotate plans, intents, and capabilities in (or alongside) your codebase using a uniform magic-comment grammar that works in every supported language. Agents about to start a feature run `g8 plan new "<title>"` and receive one structured fit-and-conflict report covering overlapping in-flight plans, capability collisions, WIP-cap status per substrate, parked ideas that match, and architectural-boundary violations, all from a single transactional query.

---

## 2. North-star use cases

These are the four use cases v0.1 must serve. They drive every design decision in this spec.

### 2.1 Feature planning into an existing codebase

A developer (or an agent) says "I want to add a streaming-export module." They run:

```bash
g8 plan new "streaming-export" --substrate export-pipeline
```

The CLI runs a composite query against the local `.g8/store.db` and prints a structured report: are there in-flight plans that touch the export pipeline? Any parked ideas that cover this? Any capability annotations whose `produces` or `consumes` shape overlaps? The report ends with a recommendation drawn from a fixed set (`Park`, `Drop`, `Wait`, `Extend`, `Rename`, `Pivot`, `Proceed`). Nothing about the recommendation is fuzzy: it's derived deterministically from gate booleans.

### 2.2 Combining codebases

Two repos are about to merge. The user runs:

```bash
g8 merge --from /path/to/repoB --into .
```

`g8` ATTACHes `repoB/.g8/store.db` as a remote SQLite database, walks Plan × Capability × Intent across both stores, and emits a `Conflict` report categorized by kind (`duplicate_intent`, `capability_name_collision`, `contradictory_decisions`, `governance_violation`). Identical capability names are reported as collisions by default; the user resolves them with explicit links (see §6.3).

### 2.3 Adding to a platform

```bash
g8 plan new "user-export-quota" --space platform
```

A multi-project ConvergenceSpace (see §6) lets a developer query across N project members. The same composite query that drives §2.1 returns results from every member project of the space, so a platform team can see prior art in any service before writing new code.

### 2.4 Multi-feature governance

WIP caps per substrate, idea-park, and stale-promotion are enforced *at planning time*, not at merge time. `g8 plan new` will refuse to create a Plan in `Dispatched` status if the substrate is at its WIP cap; the user must `--park` it, raise the cap explicitly, or wait. This is the substrate-first-bootstrap principle: enforcement ships with v0.1, before any target codebase has been annotated.

---

## 3. Design principles

These principles, in priority order, g8 every decision in this spec and ARCHITECTURE.md. When two principles conflict, the higher-numbered one yields.

1. **Substrate-first bootstrap.** Ship enforcement with v0.1. The window where adoption is zero is the cheapest moment to flip gates on.

2. **Composite queries, not verb chains.** The planner runs one transactional query that returns existing matches + budget + bottlenecks + drift hints + intent overlaps + parked candidates. This is the load-bearing pattern.

3. **Deterministic where determinism is the contract.** Extractor, store, planner, conflict-detector, CLI are 100% deterministic. No LLM calls live inside these crates. LLM-driven plan-drafting is the non-deterministic zone and is out of scope for v0.1; ARCHITECTURE.md §11 documents the seam where it will plug in.

4. **Skills not MCP tools.** `g8-planner` is a callable Rust library and a `clap` subcommand. It is *not* an MCP verb. Agent integration happens via subagent definitions shipped as part of `g8 init` (per `r3-anthropic-collab.md §5.2`).

5. **Multi-agent, not human-bounded.** WIP caps are per-substrate, adaptive, NOT per-developer. Multiple agents can coordinate against the same store.

6. **Tracing is contract, not baggage.** Every store query opens a tracing span. Every CLI command exports a trace on exit.

7. **Live shared state, not async clipboard.** When `.g8/store.db` changes, downstream UIs are notified via `notify`-crate file-watch. The `.g8/INTENT_SUMMARY.md` regeneration is the canonical pull point.

8. **AUDIT-before-enforce.** `g8 init` on a codebase with existing annotations runs a scan first, prints what *would* pass/fail the gates, and asks for explicit `g8 init --enforce` to flip them on.

9. **Lightweight, drop-into-any-repo.** Binary size, install time, and runtime deps matter. No SurrealDB v3. No Codegraph hard dep. SQLite via `rusqlite` bundled + ast-grep CLI shell-out are the load-bearing dependency choices (per R4 §2, R2 §4).

10. **Magic-comment grammar before language idioms.** Annotation surface is uniform across all supported languages in v0.1. Per-language idioms (Rust attributes, TS decorators, Py decorators) are explicitly deferred to v0.2. (per §4 of this spec)

---

## 4. Non-goals (v0.1)

- **No SurrealDB.** Substrate is local SQLite. `g8` keeps its substrate embedded and file-local; no server, no network.
- **No native Rust attribute parser in extractor.** Even Rust source uses the magic-comment grammar in v0.1. The native attribute path is v0.2. (see Decision §4 below for rationale)
- **No LLM calls in core/store/planner/conflict.** Determinism is the contract. (per principle 3)
- **No MCP server.** CLI-first. Subagent definitions (Markdown files) are the agent-integration surface.
- **No web UI.** `g8 status` writes Markdown to stdout and to `.g8/INTENT_SUMMARY.md`. A dashboard is roadmap material.
- **No automatic stale-promotion of plans.** The scanner *proposes* promotions; humans approve.
- **No modification of target codebases during `scan`.** `g8 scan` is read-only; only `g8 plan new` and `g8 init` write to `.g8/`.
- **No cross-process write coordination beyond SQLite's WAL.** Two concurrent `g8` invocations on the same store are safe up to SQLite's `busy_timeout`; beyond that, we error loudly rather than corrupt. (per R4 §6 Risk 2)

---

## 5. Glossary

- **Annotation.** A typed record extracted from source, a `Capability`, `Intent`, `ConvergenceTest`, `Decision`, or `Plan` marker, produced by `g8-extractor` from either inline magic-comments or sidecar Markdown files. All annotations carry a source file path and a line number where applicable. (See ARCHITECTURE.md §4.)

- **Capability.** A typed unit of behavior declared by a `// @g8.capability(...)` annotation. Has a `name` (kebab-case), an optional `substrate`, `consumes`/`produces` lists, a `status` from the six-state lifecycle, and optional `stub`/`since` fields. The "what does this code do at the architectural layer" record.

- **Intent.** A directional statement about what a directory or file is *for*, typically parsed from `AGENTS.md` / `CLAUDE.md` / `*.g8.md` files. Has a `scope` (directory path), a `description`, and an optional `Owner`. Intent lives at the directory level; Capability lives at the symbol level.

- **Plan.** A unit of work tracked through the lifecycle `Idea → Scoped → Dispatched → Blocked → Done | Parked`. Has a `title`, a `substrate`, `touched_capabilities`, `derived_from_intents`, and `governed_by_decisions`. Plans are the principal user-visible artifact: `g8 plan new` creates them.

- **ConvergenceSpace.** A named set of one or more projects. Identified by a name and rooted at a directory. Stored as a row in `convergence_space`. A space is the unit of cross-project query: WIP caps, planner_intent_check results, and `g8 merge` all scope to a space. (See §6 below.)

- **Project.** A single source-controlled directory tree within a space. Identified by `name + root_path`. Plans and Capabilities are project-scoped; Intent and Decision are space-scoped.

- **Substrate.** A named architectural layer (e.g. `auth`, `export-pipeline`, `scoring-strategy`). Substrates carry the per-substrate WIP cap. They are not nodes in a graph, they are string labels that group Plans and Capabilities. A deliberate vocabulary step lets users `g8 substrate add` deliberately rather than letting substrates accumulate ad-hoc.

- **Owner.** A reference to a Claude Code subagent (`agent:` field), a human team (`team:` field), or a contact channel (`contact:` field). Optional. Lives on `Intent`. Used by the planner to direct conflict reports. (per `r3-anthropic-collab.md §5.4`)

- **FitReport.** The strongly-typed return value of `g8-planner::plan_check(draft, store)`. Six fields: `existing_matches`, `budget_status`, `bottlenecks`, `drift_hints`, `intent_overlaps`, `parked_ideas`, plus a derived `decision_input` and a `recommendation` enum.

- **Conflict.** The unit return value of `g8-conflict`. Has a `kind` (`DuplicateIntent | CapabilityNameCollision | ContradictoryDecisions | GovernanceViolation`), a `severity` (`Info | Warn | Error`), and a `Vec<Evidence>` payload.

- **Annotation grammar.** The uniform magic-comment form: `// @g8.<kind>(key1="v1", key2=["a","b"], ...)`. See §4 / Decision Lockdown below and ARCHITECTURE.md §6 for the exact shape per language.

---

## 6. Locked design decisions

These are the consequential decisions A1 has locked. Each is cited against the research that backs it. Builders inherit these without renegotiation.

### Decision 1, Annotation grammar: magic-comment only in v0.1.

**Locked:** v0.1 supports ONE annotation surface across all languages: the magic-comment form

```
// @g8.<kind>(key1 = "value1", key2 = ["a", "b"], ...)
```

Rust-native `#[capability(...)]` attribute parsing is deferred to v0.2.

**Rationale:** The R2 survey (§5.1, §7.2) shows that ast-grep's `#[capability($$$)]` pattern works for direct Rust attribute syntax but breaks on `cfg_attr`-wrapped forms (which require `kind: attribute_item + has: {regex}` workarounds). The R1 patterns table (A2) recommends adapting Rust proc-macros to a magic-comment form for cross-language uniformity. R3 §5.1 recommends ingesting AGENTS.md/CLAUDE.md alongside inline markers via a uniform extractor. Supporting both forms in v0.1 doubles the extractor surface, the parse-failure mode space, and the test matrix for an unclear win.

**Practical effect:** Rust code that wants to be `g8`-annotated writes:

```rust
// @g8.capability(name = "claim-validation",
//                 consumes = ["substrate::Claim"],
//                 produces = ["substrate::ContradictionEvent"],
//                 status = "in_flight",
//                 substrate = "claims")
pub fn validate_claim(c: Claim) -> Result<Outcome> { … }
```

The comment must appear immediately before the symbol it annotates (no blank lines). Multi-line forms wrap with `//` prefix on each line.

### Decision 2, Annotation taxonomy: five kinds + six-status lifecycle

**Locked:** Annotation kinds are `Capability`, `Intent`, `ConvergenceTest`, `Decision`, `Plan`. (`Plan` is promoted to a first-class annotation so `// @g8.plan(...)` markers can live in code.)

Plan lifecycle: `Idea → Scoped → Dispatched → Blocked → Done | Parked`. Stored as the `PlanStatus` enum in `g8-core`.

The capability status lifecycle (`proposed | in_flight | landed | stale | superseded | parked`) is preserved for backward-conceptual-compat but g8 v0.1 only enforces gating on `in_flight` and `landed`.

**Rationale:** This lifecycle is small enough to enforce and rich enough to express park-and-resume, which is the state a coding agent actually needs.

### Decision 3, Multi-project topology: per-project store + space.toml federation, with cross-store ATTACH for queries.

**Locked:**

- **ConvergenceSpace is declared by `.g8/space.toml`** at the root of the space, with members listed as relative or absolute paths to project directories.
- **Each project has its own `.g8/store.db`** (so dropping `g8` into a single repo just works; the space is opt-in via `g8 space add`).
- **The space root itself has a `.g8/store.db`** for space-scoped entities (Decision, Intent, ConvergenceSpace, substrate_budget).
- **Cross-project queries** are run by ATTACHing each member's store as a SQLite database alias at planner-invocation time. (per R4 §6 Risk 5)
- **Cross-project capability identity:** capability names are project-scoped by default. `repoA::auth.login` and `repoB::auth.login` are distinct unless an explicit `g8 link --canonical "repoA::auth.login" --alias "repoB::auth.login"` is recorded.
- **Canonical name format inside annotations:** kebab-case (`claim-validation`); cross-project references in space-scoped queries use `<project>::<name>` notation. Annotations themselves NEVER carry `project::` prefix, projects are inferred at extraction time from `Project.root_path`.

**Rationale:** R4 §6 Risk 5 documents that SQLite's `ATTACH DATABASE` is the cleanest pattern for cross-store reads. R1 §8.1 Q1–Q4 explicitly defer these decisions to the architect; the per-project default + opt-in space is the lightest path that supports the single-repo and multi-repo use cases without forcing one to behave like the other. Forcing a global store (`~/.g8/projects/...`) couples the user's local fs to `g8`'s data model, bad.

**`.g8/space.toml` shape** (exhaustive; ARCHITECTURE.md §10 specifies parser):

```toml
[space]
name = "platform"
description = "Platform monorepo (cross-product space)"

[[members]]
name = "checkout-api"
root = "../checkout-api"        # relative to space.toml

[[members]]
name = "render-kit"
root = "../render-kit"

[substrate_budgets]
# Optional overrides; defaults from g8-core::DEFAULT_WIP_CAP = 3
"export-pipeline" = 5
"auth-layer" = 2

[links]
# Cross-project capability identity declarations
[[links.aliases]]
canonical = "checkout-api::http-fetch"
alias = "render-kit::http-fetch"
```

### Decision 4, AGENTS.md / CLAUDE.md / *.g8.md ingestion.

**Locked:** All three file types are ingested by the same Markdown-section parser in `g8-extractor`. Parser contract:

- **Detection:** walk via the `ignore` crate, match exact filenames `AGENTS.md`, `CLAUDE.md`, and glob `*.g8.md` at any depth.
- **Sectioning:** Headings (`#`, `##`) split the file into sections. Each section becomes one Annotation.
- **Section-to-kind mapping** (case-insensitive heading match; unknown headings produce `AnnotationKind::IntentUnclassified`):

  | Heading regex | AnnotationKind | Stored in table |
  |---|---|---|
  | `^architecture$ \| ^design$ \| ^overview$` | `Intent { kind: ArchitecturalScope }` | `intent` |
  | `^boundaries$ \| ^never do$ \| ^constraints$` | `Intent { kind: Boundary }` | `intent` |
  | `^commands$ \| ^build$ \| ^test$ \| ^testing$` | `Intent { kind: Operational }` (deprioritized in planner queries) | `intent` |
  | `^stack$ \| ^dependencies$` | `Intent { kind: TechStack }` | `intent` |
  | `^owner$ \| ^ownership$` | `Owner` (attached to enclosing Intent) | parsed into `intent.owner_*` columns |
  | `^parked ideas$ \| ^deferred$` | `Plan { status: Parked }` | `plan` (parsed line-by-line, see §6.4 below) |
  | `^active plans$ \| ^in flight$` | (read-only; ignored by extractor, those rows live in `plan`) | n/a |
  | `^decisions$ \| ^architectural decisions$` | `Decision` | `decision` |
  | `^intent$` | `Intent { kind: ExplicitIntent }` | `intent` |
  | anything else | `Intent { kind: Unclassified }` | `intent` |

- **Precedence:** deeper file wins. The OpenAI Codex three-tier model (`r3 §1.2`) is mirrored. A `src/auth/AGENTS.md` overrides the root `AGENTS.md` *for any intent that resolves to a path under `src/auth/`*. The store records every Intent with its `scope_depth` (= directory depth); planner queries use `ORDER BY scope_depth DESC LIMIT 1` to pick the most specific.

- **Owner field shape:**

  ```
  # Owner
  agent: scorer-specialist          # references .claude/agents/scorer-specialist.md
  team: core-ranker                 # human team name
  contact: #checkout-api-core        # email, Slack channel, etc.
  ```

  All three subfields are optional. Stored as `intent.owner_agent`, `intent.owner_team`, `intent.owner_contact`.

- **Parked Ideas parsing:** lines of the form `- <name>: <reason>` become `Plan` rows with `status=Parked`, `title=<name>`, `parked_reason=<reason>`, `space_id=<enclosing space>`. Lines that don't match the pattern are skipped with a warn-level tracing event.

- **`INTENT_SUMMARY.md` regeneration:** `g8 status` (and implicitly `g8 scan`) writes `.g8/INTENT_SUMMARY.md` from a fixed template (ARCHITECTURE.md §10.4). The file has an `<!-- AUTO-GENERATED -->` header; humans MUST NOT edit it; extractor MUST NOT ingest it (extractor adds `.g8/INTENT_SUMMARY.md` to its ignore set explicitly).

- **`g8 init` writes `@.g8/INTENT_SUMMARY.md` import into CLAUDE.md** (creating CLAUDE.md if absent). (per `r3 §5.5`)

**Rationale:** R3's §5.1 and §5.5 recommendations land on a single Markdown-heading scanner; R3 §2.4 confirms the precedence model from the AGENTS.md spec; R3 §6.2 explicitly forbids LLM-generation of these files, so we only ever *read* them.

### Decision 5, StoreConnection trait + rusqlite primary, Turso swap planned.

**Locked:** `g8-store` exposes a `StoreConnection` trait. The v0.1 implementation is `RusqliteStore` (per R4 §2). The trait surface is designed so `TursoStore` can plug in without touching `g8-planner` or `g8-conflict`. Exact trait signatures live in ARCHITECTURE.md §5.

**Rationale:** R4 §2 picks `rusqlite` over Turso decisively (Turso is BETA, async-only, 27–45 MB binary bloat). R4 §6 Risk 4 commits to the trait abstraction precisely so the swap is reversible.

### Decision 6, `planner_intent_check` composite query: six-CTE SQLite shape as defined in R4 §4.

**Locked:** The composite-query SQL in R4 §4 is the v0.1 query (with field renames to match `g8-core` naming). Returns six JSON columns: `existing_matches`, `budget`, `bottlenecks`, `drift_hints`, `intent_overlaps`, `parked_ideas`. Runs as one read transaction. Full SQL in ARCHITECTURE.md §7. Trait method on `StoreConnection`: `fn planner_intent_check(&self, draft: &PlanDraft) -> Result<RawIntentCheckPayload>`. `g8-planner` deserializes the JSON columns into `FitReport`.

**Rationale:** R4 §4 provides the full SQL; R1 D2 commits to the principle ("Cuts latency, eliminates partial-state bugs, makes the decision atomic"). Composite-queries-not-verb-chains is principle 2.

### Decision 7, CLI surface: locked in ARCHITECTURE.md §9.

**Locked:** Full `clap` subcommand tree with all flags is defined in ARCHITECTURE.md §9. Top-level subcommands:
`init | scan | plan {new,list,show,park,promote} | status | check | merge | query | space {add,remove,list} | link | substrate {add,list}`.

Two output modes (uniform across all commands): `--output pretty` (default) and `--output json`. Pretty output is human-readable Markdown; JSON output is stable and versioned with `"g8_version": "0.1.0"` in every payload.

### Decision 8, Crate graph: six crates.

**Locked:** Workspace members:

```
g8-core         (no deps on other workspace crates)
  └── used by all
g8-store        (deps: g8-core)
g8-extractor    (deps: g8-core)
g8-planner      (deps: g8-core, g8-store)
g8-conflict     (deps: g8-core, g8-store)
g8          (deps: all of the above)
```

This is a strict DAG with `g8-core` as the root and `g8` as the only sink. No cycles, no diamonds.

### Decision 9, AUDIT-before-enforce ergonomics for `g8 init`.

**Locked:** `g8 init` runs as follows on a target directory:

1. Create `.g8/` directory.
2. Apply migrations to `.g8/store.db`.
3. Run `g8-extractor` against the cwd in **report-only mode** (no writes to store).
4. Print a summary: "Found N capabilities, M intents, K decisions. Of N capabilities, X would PASS the pairing gate, Y would FAIL because of {missing convergence_test | stub expired | duplicate name}."
5. Write `.g8/config.toml` with `enforcement = "off"` by default.
6. Append `@.g8/INTENT_SUMMARY.md` to `CLAUDE.md`.
7. Drop `.claude/agents/g8-planner.md` subagent definition.
8. Print the next step: "Run `g8 scan` to populate the store. Run `g8 init --enforce` to flip the pairing gate ON."

`g8 init --enforce` (or `g8 config set enforcement on`) flips `.g8/config.toml::enforcement = "on"`. `g8 check` reads this flag and skips gating when off.

`g8 init` is idempotent: re-running it re-applies migrations (no-op if already applied), refreshes the AUDIT report, and leaves `enforcement` untouched.

**Rationale:** The AUDIT-before-enforce pattern (`r1 §6.7, E10`) compose cleanly.

### Decision 10, In-loop validator: ship `g8 check --json` with a stable JSON contract.

**Locked:** `g8 check --json` emits a stable, versioned shape:

```json
{
  "g8_version": "0.1.0",
  "errors": [
    {
      "capability": "claim-validation",
      "file": "crates/core/src/claims.rs",
      "line": 42,
      "reason": "no_matching_convergence_test"
    }
  ],
  "exit_code": 1
}
```

Stable reason vocabulary (snake_case):

- `no_matching_convergence_test`
- `stub_expired`
- `stub_missing_since`
- `duplicate_capability_name`
- `invalid_root`
- `invalid_checker_json` (only ever emitted by callers wrapping `g8 check`)
- `enforcement_disabled` (informational only; exit_code remains 0)

The binary always emits valid JSON on stdout, even on success (`{"errors": [], "exit_code": 0}`). Human-readable text goes to stderr.

Exit codes: `0` on no errors (or enforcement disabled), `1` on errors found, `2` on internal failure (e.g. store unreachable).

**Rationale:** If anyone wires `g8` into a harness, they get a drop-in. Stable contracts are cheap to maintain and worth shipping early.

### Decision 11, Deterministic vs non-deterministic zones (the M1 principle).

**Locked labels:**

| Crate | Zone | Justification |
|---|---|---|
| `g8-core` | DETERMINISTIC | Pure data types + serde; no I/O |
| `g8-extractor` | DETERMINISTIC | Subprocess ast-grep + Markdown parser; identical input → identical output |
| `g8-store` | DETERMINISTIC | SQLite + refinery; queries are functions of the store state |
| `g8-planner` | DETERMINISTIC | Pure function over `g8-store`; FitReport.recommendation derived from booleans by a fixed lookup table |
| `g8-conflict` | DETERMINISTIC | Pure function over two stores; conflict kinds are exhaustively enumerated |
| `g8` | DETERMINISTIC | Argument parsing + dispatch + render; no decision logic |

**Future non-deterministic zone (out of scope v0.1):** an `g8-plan-draft` crate that calls an LLM to *draft* a `PlanDraft` from a free-text user request. ARCHITECTURE.md §11 documents the seam: it produces `PlanDraft`, which is then passed unchanged to `g8-planner::plan_check`. The deterministic crates never know it was LLM-drafted; the LLM never touches the planning logic.

**Rationale:** The value of the protocol is in the deterministic gate, not the drafting layer. Mixing zones inside one crate is the failure mode to avoid.

---

## 7. Plan lifecycle (canonical)

```
                 g8 plan new "..."
                          │
                          ▼
                       ┌──────┐
                       │ Idea │  (created at planner Phase 1; no gate)
                       └──┬───┘
                          │   g8 plan scope <id>
                          ▼
                      ┌────────┐
                      │ Scoped │  (g8-planner has run; FitReport produced)
                      └──┬─────┘
                         │   g8 plan dispatch <id>  ← gate: WIP cap check
                         ▼
                   ┌────────────┐                    ┌────────┐
                   │ Dispatched │ ───────────────────► Blocked │ (g8 plan block <id> --reason ...)
                   └──┬─────────┘                    └──┬─────┘
                      │                                  │ g8 plan unblock <id>
                      │                                  ▼
                      │                              (back to Dispatched)
                      │
                      │   g8 plan complete <id>
                      ▼
                  ┌──────┐
                  │ Done │  (terminal; counts against WIP only until status flips)
                  └──────┘

      ┌─────────┐
      │ Parked  │  ◄── from any state via `g8 plan park <id> --reason ...`
      └────┬────┘
           │ g8 plan promote <id>  ← back to Scoped (re-runs planner_intent_check)
           ▼
        Scoped
```

WIP-cap status counts Plans in `Dispatched` **and** `Blocked`. `Blocked` doesn't free a slot; that's deliberate, blocked work still occupies the substrate's attention surface.

---

## 8. Governance model

Three governance primitives are enforced at planning time, not at merge time.

### 8.1 WIP caps per substrate

Per `Decision 3`, `.g8/space.toml::substrate_budgets` declares the cap; absent overrides, `g8-core::DEFAULT_WIP_CAP = 3` applies. The cap is checked by the planner before a Plan transitions to `Dispatched`. The cap is *per substrate per space*, so two projects in the same space share the cap (this is the multi-project flavor; per-project scoping is deliberately not offered).

v0.1 ships a **static** WIP cap (overridable per-substrate). An adaptive `clamp(2 + stability_bonus, 2, 20)` formula is roadmap material; v0.1 doesn't ship enough query infrastructure (drift counts, failing counts, time-bonuses) to compute `stability_bonus` cheaply. ARCHITECTURE.md §13 sketches the v0.2 upgrade path.

### 8.2 Idea park

A first-class Plan status. `g8 plan new --park` creates a Plan directly in `Parked` status; `g8 plan park <id>` parks an existing Plan. `g8 plan promote <id>` moves it back to `Scoped` (re-runs planner_intent_check; promotion isn't unconditional). `g8 plan list --parked` enumerates them.

Parked Plans:
- Don't count toward WIP cap.
- Don't appear in drift queries.
- DO appear in `planner_intent_check.parked_ideas` (so a new plan can be reminded of related parked ideas).

### 8.3 Stale-promotion proposals

A scanner pass identifies Plans in `Dispatched` for longer than `space.toml::stale_threshold_days` (default 14) and emits proposals in `.g8/INTENT_SUMMARY.md` under a "Stale Promotion Proposals" section. The scanner does NOT auto-promote; humans run `g8 plan park <id>` or `g8 plan complete <id>` based on the proposal.

In v0.1 the scanner runs as part of `g8 scan` (no daemon, no cron). Daemon mode is a future addition.

---

## 9. Adoption path (AUDIT-before-enforce)

The recommended onboarding flow for a real codebase:

1. **`g8 init`** in the repo root. Creates `.g8/`, applies migrations, runs the AUDIT scan, prints the report. Enforcement is OFF.
2. Optionally **`g8 space add`** to federate multiple repos into one ConvergenceSpace (cross-project queries).
3. **`g8 scan`** to populate the store from existing AGENTS.md / CLAUDE.md / `*.g8.md` files. (Zero inline annotations required to start.)
4. **`g8 status`** to see what `g8` thinks about the codebase. Iterate until the report matches mental model.
5. Add inline `// @g8.capability(...)` markers to the most-touched files, one substrate at a time.
6. **`g8 init --enforce`** when ready. Pairing gate is now active.
7. **Wire `g8 check --json`** into CI (`.github/workflows/g8-check.yml`, example shipped with `g8 examples`) and/or pre-commit (`g8 init --install-pre-commit`).
8. **Install the `g8-planner` subagent** by running `g8 init` (already done) and let agents auto-delegate planning checks.

The substrate-first-bootstrap principle (§3.1) means step 1's AUDIT will frequently report `0 capabilities, 0 intents found`, that's the ideal moment to flip enforcement on.

---

## 10. v0.1 scope vs roadmap

### In v0.1 (Phase 2 builders MUST ship)

- All six crates per Decision 8.
- Magic-comment annotation grammar (per Decision 1) for Rust, TypeScript, Python, Go.
- `g8-extractor` via ast-grep CLI shell-out (per R2 §4).
- `g8-store` via rusqlite + refinery (per R4 §2).
- `g8-planner::plan_check` returning the full `FitReport`.
- `g8-conflict::compare_spaces` returning `Vec<Conflict>`.
- `g8` with the full subcommand tree per Decision 7.
- AGENTS.md / CLAUDE.md / `*.g8.md` ingestion (per Decision 4).
- `g8 check --json` with the JSON contract per Decision 10.
- `.g8/INTENT_SUMMARY.md` auto-regeneration on `g8 status` and `g8 scan`.
- One end-to-end trace exported on every CLI invocation (principle 6).
- Subagent definition `.claude/agents/g8-planner.md` shipped by `g8 init`.
- Phase 3 smoke against checkout-api, render-kit, docs-hub.

### Roadmap (NOT v0.1)

| Item | Why deferred | Where the seam lives |
|---|---|---|
| Native Rust attribute extraction (`#[capability(...)]`) | Doubles extractor surface; cfg_attr gotcha (R2 §7.2); v0.1 magic-comment grammar covers the use case | `g8-extractor::lang::rust_attribute` module (stub) |
| Adaptive WIP cap (`clamp(2 + stability_bonus, ...)`) | Needs drift/failing/time queries we don't ship in v0.1 | `g8-store::adaptive_budget_inputs` query (stub) |
| LLM-drafted plans (`g8-plan-draft` crate) | Out of zone; v0.1 is deterministic | `PlanDraft` struct is the in-edge |
| Dreaming-inspired cross-session consolidation (`g8 learn`, `g8 consolidate`) | Needs session-transcript ingestion infra | New `g8-history` crate |
| Daemon mode for stale scanner | v0.1 scanner is on-demand; daemon adds packaging complexity | `g8 daemon` subcommand placeholder |
| Web UI for `g8 status` | CLI is sufficient for v0.1; the "live shared state" principle is satisfied by INTENT_SUMMARY.md + notify | Future `g8-server` crate |
| MCP server for `g8` verbs | Skills-not-MCP-tools principle (4); subagent ships instead | n/a |
| Pre-commit hook installer | Easy add but not load-bearing | `g8 init --install-pre-commit` |
| Turso swap | rusqlite v0.1, Turso when it ships sync API | `StoreConnection` trait |
| Codegraph optional adapter | "No Codegraph hard dep" (P9) | Cargo feature flag `codegraph` |
| Cross-language `g8 link` aliasing in queries | v0.1 records aliases; query layer applies them | `g8-store::resolve_capability_aliases` |
| Adaptive budget formulation per substrate | See above | n/a |
| Conflict-resolution suggestion engine | Out of zone (non-deterministic) | n/a |

---

## 11. Success and failure criteria for v0.1

### Success looks like

- `g8 init` works on checkout-api, render-kit, and docs-hub in under 5 seconds each.
- `g8 scan` produces zero false-error reports on real code (warnings are OK; errors must be true errors).
- `g8 plan new "test-feature"` returns a structured `FitReport` in under 100ms on a small repo.
- `g8 merge --from render-kit --into docs-hub` runs without panic; emits a `Vec<Conflict>` JSON report.
- `cargo test --workspace` passes.
- One end-to-end trace is exported per CLI invocation.
- The `g8 check --json` contract is stable and versioned, so a harness can wire in-loop validation in front of `g8` and rely on the shape.
- A new contributor can read SPEC.md + ARCHITECTURE.md in < 30 minutes and write a new annotation kind without asking questions.

### Failure looks like

- `g8 init` on a 50k-LOC repo takes > 30 seconds (extractor too slow).
- `g8 plan new` returns an empty `FitReport` for a feature that clearly overlaps with an existing in-flight Plan (composite query broken).
- A SQLite schema migration in a v0.2 release silently corrupts a v0.1 store (refinery bypassed).
- `g8 scan` writes to source files (extractor is read-only by contract; this would be a hard breach).
- Two concurrent `g8 scan` invocations corrupt the store (busy_timeout misconfigured or omitted).
- The `g8-planner` crate gains an LLM dep (zone violation).
- A `cfg_attr`-wrapped magic-comment-equivalent annotation gets gated.

---

## 12. Open micro-decisions delegated to builders

A1 has locked the consequential decisions. The following are explicitly delegated to builder discretion (with the named builder owning the choice):

- **B1:** Whether `Plan.meta` is `serde_json::Value` or a typed `PlanMeta` struct with a free-form `extra: HashMap<String, Value>` field. (Recommended: `serde_json::Value` for v0.1, typed in v0.2.)
- **B3:** Choice of `nanoid` vs `uuid` for IDs. (Recommended: `nanoid` per R4 §8 Cargo snippet, 21-char URL-safe IDs are sufficient.)
- **B4:** Whether `--output pretty` writes to stdout or to a pager (`less`-style). (Recommended: stdout; user pipes to `less` if they want.)
- **B6:** Whether `Conflict::evidence` is `Vec<String>` (human-readable) or `Vec<Evidence>` (typed). (Recommended: typed for v0.1 so JSON consumers can structure it.)

Everything else is locked. Builders who think a locked decision is wrong should raise it explicitly with A1 before deviating.

---

**End of SPEC.md.** See ARCHITECTURE.md for the technical realization.
