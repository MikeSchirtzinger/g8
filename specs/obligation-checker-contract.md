# Obligation Checker Contract, v1

**Board:** o-g8-defuse-and-wire-20260702, Task T2 (amended under T2b)
**Status:** BINDING for T3 (backends-builder), T4 (artifact-annotator), T5 (check-integrator), T6 (prose-handler)
**Author:** contract-architect

## Amendment log

**Amendment 1 (T2b, response to annotator/T4 feedback while implementing §1):**

1. **§1.2 schema addition, additive/backward-compatible only**, two new `Assertion` variants (`StdoutContainsAll`, `StdoutNotContains`) and a new `binary: BinarySource` field on `CliInvocation` (defaults to the existing `CurrentExe` behavior via `#[serde(default)]`, so every one of the other 26 obligations' existing args are untouched, **no re-work needed for anything T3 has already built against §1 except the one new `CargoRun` variant, used only by D7-01**).
2. **D7-01 remapped** (§2 row 14, §2.1), the original `FixtureIntegrationTest` design assumed one running binary could observe both the default and `--features serve` builds via `current_exe()`; it cannot (a single compiled binary only knows its own feature set). Fixed by using the new `BinarySource::CargoRun` to explicitly invoke both build configs, mirroring the exact verification method T1's own acceptance criteria already use (`cargo run -q -p g8 -- --help` / `--features serve`).
3. **D9-01 decomposition ratified** (§2 row 16), annotator's 4-independent-`FixtureIntegrationTest` split is correct and required **zero schema changes**: my original single-sequence mapping wrongly used the stdout-only `JsonField` assertion to inspect `.g8/config.toml` (a file, not a step's stdout). Splitting into 4 short, independent setups sidesteps the need for any before/after-step anchoring on file assertions entirely, ratified as-is.
4. **D3-02 adjudicated: checker is correct, code is not** (§2 row 8, new §2.2), `g8 space add` (`crates/g8/src/cmd/space.rs:22-62`) verified by direct read to only call `store.upsert_project`; it never writes `.g8/space.toml`, contradicting SPEC.md Decision 3's locked text. The obligation's checker mapping is **unchanged**, this is a new, disclosed live-code-vs-SPEC drift for Mike's pile, not a checker bug.

**Erratum 1 (post-T2b, serde wire-shape corrections; `crates/g8-obligations/src/backend.rs` is now ground truth for §1.2's exact Rust shapes, verified directly, not taken on report):**

Backends-builder found real inconsistencies between my §1.2 sketches and (a) my own §6 worked JSON example and (b) the real, already-annotated `specs/obligations-v0.1.json`. These fall into three distinct categories, not the single "tuple variants became struct variants" story a quick read of the fix might suggest:

1. **Missing serde attribute, no variant restructuring:** `DepMatcher` and `BuildConfig` never carried a `#[serde(...)]` attribute in §1.2, silently defaulting to Rust's ordinary externally-tagged form, which does not match §6's own `{"kind":"exact","value":"surrealdb"}` / `"build_config":"default"` shapes shown a few sections later in the same document. Fixed by adding `#[serde(tag = "kind", content = "value", rename_all = "snake_case")]` to `DepMatcher` and `#[serde(rename_all = "snake_case")]` to `BuildConfig`. `DepMatcher`'s variants remain the original tuple variants (`Exact(String)`, `Regex(String)`), adjacent tagging (`tag` + `content`) represents tuple variants natively, so nothing needed restructuring here.
2. **Genuine tuple-variant / internal-tagging incompatibility, real restructuring, confirmed unavoidable:** `CountExpectation::Exactly(u32)` / `AtLeast(u32)` and `Assertion::FileExists(String)` (plus my own T2b addition, `BinarySource`) needed `#[serde(tag = "kind")]` internal tagging to match the real artifact's wire format, and serde's internal tagging cannot represent a bare-scalar tuple variant, the tag has to merge into a JSON object, and a lone `u32`/`String` isn't one. `Exactly{value: u32}` / `AtLeast{value: u32}` / `FileExists{path: String}` is the standard serde workaround: one named field, identical information. Confirmed independently this is a real, load-bearing serde constraint (not a style choice T3 could have avoided while keeping internal tagging), not escalating.
3. **A type I referenced but never defined, found while verifying this erratum, not reported by T3:** `AstGrepLang` (used by `AstGrepArgs`/`AstGrepCountArgs`) appeared only in a trailing comment in §1.2 (`// Rust | TypeScript | Python | Go`), never as an actual `pub enum`. `backend.rs` defines it properly (4 variants, `rename_all = "snake_case"`, values cross-checked against `g8-extractor/rules/g8-all-languages.yaml`'s `language:` keys and `ast-grep`'s accepted `--lang` values). Recorded here so this doc doesn't silently under-specify something a T5/T6 reader would reasonably expect spelled out.

Also checked directly (not assumed): `crates/g8-obligations/src/result.rs` implements §3's `ObligationStatus`/`TrustLevel`/`ObligationResult`/`Evidence`/`SubCheckResult` field-for-field with no discrepancies, and the §3.3 aggregation rule is a pure function with a passing unit test per edge case this doc describes in prose. No erratum needed for §3.

None of the above changes any obligation's mapping (§2), any status/trust semantics (§3), or the closed 10-backend enum's membership (§1.1), these are wire-representation corrections only, each confirmed semantically identical to what it replaced.

**Erratum 2 (T2c, three defects surfaced by the first real 27-obligation run; full detail and evidence in new §2.3):**

1. **D2-01 casing convention** (§2 row 6), ruled checker-side: `RustEnumShape` must normalize raw PascalCase Rust identifiers through the enum's `expected_serde_rename_all` transform before comparing to `expected_variants`, which correctly stays wire-format (T4's artifact-side authoring needs no change). Fix lands in T3b.
2. **D8-01 dependency edges** (§2 row 15), two independent rulings: (a) `required_edges` genuinely needs 3 new tuples for `g8-obligations`, which postdates the original 11-edge list, artifact fix, T4c. (b) The separately-reported `g8-conflict → g8-store` missing edge is **not** an artifact bug, it's a real gap, matching both SPEC.md Decision 8 and `g8-conflict`'s own code comments describing (in future tense) an unfinished Phase-2-parallelism bootstrap step. Checker stands; third live-drift finding for Mike's pile, distinct in character from OQ-08/D3-02 (self-disclosed intentional bootstrap state, not silent). Includes an actionable D5-01-interaction note for whoever implements the fix.
3. **D6-01 target path** (§2 row 12), `crates/g8-store/src/queries.rs` never existed; corrected to `store_impl.rs` (confirmed: `planner_intent_check` and all six named columns live there). Artifact fix, T4c. The separately-reported ast-grep generic-fn bug (`plan_check<S>`, not `plan_check(store: &dyn StoreConnection)`) is registered with its concrete shape for T3b but not fixed here, per the task's own scoping.

**Erratum 3 (T2d, six items from T6's fidelity review; full detail and evidence in new §2.4; all six ruled artifact-side, T4d, zero checker-side work needed):**

1. **D6-01(b) pattern hardened** (§2 row 12), bare word search over-matched 34 vs 6 in the real file *and* was gameable (matches struct fields/CTE names/deserialization sites, none of which are "the query's declared columns"). Anchored to the `) AS <name>` SELECT-clause-alias syntax instead, empirically verified exactly 6/6 matches, structurally excludes every prior false-positive source. Residual false-pass gap (structural, not behavioral, proof) disclosed, not silently closed. **Revised in T2d-2** (below) after T6's adversarial probe defeated this single anchor too, now three independent anchors (struct-fields + CTE-declarations + column-aliases), not one; see §2.4 item 1's rewritten discussion for why two isn't enough either.
2. **D4-01 narrowed** (§2 row 9), `g8 scan --output json` only exposes aggregate counts, confirmed directly; per-heading assertions were never observable through it. Narrowed to an `inserted_intents` count plus an `g8 query "intents --path=..."` step for the deeper-file-wins check specifically.
3. **D4-02 fixture bug, not a drift finding** (§2 row 10), `status.rs` correctly writes the header (confirmed by direct read); the fixture's setup was missing a prerequisite `init` step, so `status` either errored or hit its own "no space yet" early return. Added `init` to setup, no live-drift finding here.
4. **D6-02 re-grounded** (§2 row 13), confirmed directly against the real SQL that `drift_hints` means stale-Dispatched-plan detection, not unresolved capability wiring as the obligation's own (self-hedged, "not confirmed") inference guessed. Useful resolved uncertainty for the record. The true positive case isn't cheaply fixturable (day-granularity staleness, no time-injection primitive in this schema), narrowed to an honest negative-case assertion (`drift_hints: []` for a fresh, non-stale fixture) rather than forcing a slow or fabricated test.
5. **P6-01(c) root-caused empirically** (§2 row 2), built and ran the real binary. `status` produces zero stderr lines regardless of trace mode (unlogged `#[instrument]`-only spans, no `with_span_events` configured, vacuous-truth bug in the "not all JSON" direction). Switched to `scan --quiet -vv` on a 1-annotation fixture, which also required `--quiet` to suppress an unconditional non-tracing `eprintln!` banner that was contaminating both trace-format modes. Verified end-to-end: 1 clean line per mode, 100%/0% JSON as expected. Checker logic itself (`StderrFormat` in `exec/fixture.rs`) confirmed correct on direct read, no checker-side fix needed.
6. **G1-01 allowlist ratified** (§2 row 27), all six proposed additions confirmed by direct grep to be real, current `g8-core` types (not `g8-store`, despite how ARCHITECTURE.md's section layout might read at a skim) following the established Canonical+Suffix pattern. Added without reservation.

**T2d-2 (same task, second pass, T6's fidelity re-run adversarially defeated item 1's single-anchor fix above before T4d finished applying it):** §2 row 12 revised again, D6-01(b) is now three `RgMatchCount` anchors (struct-field declarations, CTE declarations, and the original column-alias pattern), not one. Two of the three anchors came from a review round's recommendations (one candidate rejected, needs a struct-name-scoping ast-grep capability this crate doesn't have, correctly identified as new-checker-work rather than an artifact fix, so not adopted); I independently re-verified all three against the real file (6/6/6) before ruling, and kept all three rather than the two-anchor synthesis proposed to me, two alone still misses six correctly-named-but-unreferenced CTEs paired with a wrong actual output, which the third (output-alias) anchor specifically catches. Zero new checker capability required either way. Items 2–6 of Erratum 3 are unchanged by this pass.

**T2e (empirical-only closure pass, T7 re-run showed D4-01/D4-02/P6-01 still failing post-T4d; full evidence in new §2.5):**

1. **P6-01(c)**, T2d's fix was directionally right (`scan --quiet -vv`, right env var, right flag placement, all re-confirmed live) but empirically incomplete: the encoded row scans an *empty* fixture, so the debug-level event it needs never fires (vacuous-truth bug, recurring one layer under the first one T2d found). Fixed with a new `seed_files` primitive (§1.2) that writes a real annotation before setup runs. Verified live: 1 line/mode, 100%/0% JSON.
2. **D4-01**, the count (`inserted_intents: 2`) and content (`data[0].description`) assertions were already correct, verified against a real two-level fixture built this pass. The actual bug: `scope_path` is stored as an *absolute* path (confirmed via direct SQLite query), so a relative `--path=src/auth` query can never match. Fixed with `seed_files` for the fixture content plus a new `{{fixture_dir}}` runtime-path substitution (§1.2). Verified live end-to-end.
3. **D4-02**, **my T2d diagnosis was wrong**, not just incomplete. `[init, status]` was already correct and already worked when re-tested directly. The real bug: `expected_prefix: "<!-- AUTO-GENERATED -->"` closes the HTML comment immediately, but the real header has a timestamp between "GENERATED" and "-->", so the literal string was never a true prefix, confirmed with a direct `str.startswith()` check (`False` before the fix, `True` after truncating to `"<!-- AUTO-GENERATED"`). Pure string-literal fix, nothing to do with the setup sequence.
4. **D11-02/`tempfile`**, for the record, no fix: confirmed `tempfile` is dev-deps-only in `g8-store/Cargo.toml`, and confirmed `CargoMetadataNoDep` does not filter by dependency kind (matching the obligation's own original kind-blind `jq` check). Flagged as a genuine open scoping question for whoever rules on Q4, D11-02 may never cleanly pass post-fix if dev-dependency edges stay in scope, and that's a real decision, not an oversight to quietly patch.

Two new, additive-only schema elements from this pass: `FixtureTestArgs.seed_files: Vec<SeedFile>` and the `{{fixture_dir}}` substitution on `CliInvocation.args` (§1.2), both are new, small checker-side work (T3-family), contradicting my earlier T2d claim that all six of that round's fixes needed zero checker changes for P6-01/D4-01 specifically; that claim is superseded by what this pass found.

**Erratum 4 (Q-rulings implementation pass, 2026-07-02, full detail and evidence in new §2.6):**

1. **D11-04 semantics corrected, this contract misread the obligation** (§2 row 21, §9). `conv-d11-04`'s own binding prose is *"run the same `g8 check --json` (or `g8 query`) command twice against an identical, >=2-element fixture store"*, ONE store, TWO reads, array-ordering divergence as the failure condition (the DEF-11 HashMap hazard its `rule.params` names). §2 row 21's "deliberately mint-SENSITIVE" two-independent-branches encoding came from the obligation's *title*, not its prose, and §9 point 1's claim that "the two branches' filesystem paths are never smuggled into the compared output" was falsified by T5's later `obligations_note` (which embedded each branch's absolute artifact path, the actual first-divergence byte of every recorded FAIL). Re-encoded to one seeded fixture (new additive `ByteDiffArgs.seed_files`, same T2e precedent as `FixtureTestArgs`), setup once, compare `query capabilities --output json` twice, `normalize_paths: []`, zero masking. The two-branch shape is additionally shown structurally unsatisfiable as a byte-identity claim (fresh stores legitimately embed their own absolute `scope_path`s and wall-clock timestamps), which corroborates the same-store reading. Product-side: `obligations_note` now names the artifact path repo-relative only.
2. **Q4 ruled and executed (D11-02)**, decision-gate option 1: IDs are now content-derived (SHA-256 over kind-tagged, unit-separated canonical parts; 21-char URL-safe output, nanoid-compatible shape; `new()`/`Default` deleted so every mint site declares identity content; pre-Q4 nanoid IDs stay valid via `from_string`, no migration). Rider 2 ruled: **dev-dependencies are OUT of deny-list scope**, `CargoMetadataNoDep` walks normal+build edges only and discloses `dependency_kinds_in_scope` in evidence on every run; `CargoMetadataDepGraph` deliberately stays kind-blind (no shipped/dev carve-out was ruled for D8's edge-set shape). Serve keeps nanoid (feature-gated sanctioned randomness, per the Q4 rider).
3. **Q5/Q6 executed**, `space add`/`remove` now write/maintain `.g8/space.toml` (D3-02; checker row unchanged since T2b, now genuinely passes) and `g8-conflict -> g8-store` exists as a real Cargo edge via the generic `ConflictAdapter` moved into `g8_conflict::store_adapter` (D8-01; D5-01-safe exactly per §2.3's interaction note).
4. **New disclosed finding, NOT fixed here: OBL-D11-01 false-pass hole.** Its rg pattern is the fully-qualified literal `std::time::SystemTime::now\(\)`, but `g8-core/src/time.rs:45` calls `SystemTime::now()` through a `use std::time::{SystemTime, ...}` import, structurally invisible to the pattern, so D11-01 passes while wall-clock minting demonstrably exists in the deterministic zone (`now_millis`, feeding every `created_at`/`updated_at`). Same class as D6-01's anchor-gaming history. Deliberately surfaced for a ruling rather than silently patched, because the *product intent* is genuinely ambiguous: the staleness features (drift_hints, `stale_threshold_days`) are BUILT on wall-clock timestamps, so the honest fix is either (a) widen the pattern + encode an explicit, disclosed `time.rs` exemption, or (b) amend the obligation's scope to name timestamp minting as sanctioned, either is a Mike decision, not a checker's.

Post-Erratum-4 state, verified by the real-artifact end-to-end run (ignored test tier): **27 passed / 0 failed / 0 error / 0 unknown**, enforcement exit-code arithmetic consistent in both modes.

---

## 0. Non-negotiable constraint

> **g8 must NEVER execute raw shell strings from JSON.**

Every obligation resolves to a member of a **closed Rust enum** of checker backends with **structured, typed arguments**. There is no `shell_command: String` field anywhere in this design, and none may be added later without a new binding ruling. Where a real external tool is invoked (`cargo`, `ast-grep`, `rg`), it is invoked the way `g8-extractor` already invokes `ast-grep` (§3.2 of ARCHITECTURE.md): a `std::process::Command` built from a fixed program name plus an argument *vector* assembled from typed fields, never a formatted string handed to a shell.

---

## 1. Checker schema

### 1.1 Backend enum

Ten backends. The five named in the task brief (`cargo_metadata_no_dep`, `ast_grep_no_match`, `rg_match_count`, `g8_check_contract`, `byte_diff_twice`) plus five more that the 27 obligations actually require once you try to map every one of them (derived below in §2, every addition is justified by at least one obligation that cannot be honestly expressed with the starter five).

```rust
// crates/g8-obligations/src/backend.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", content = "args", rename_all = "snake_case")]
pub enum CheckerBackend {
    /// Deny-list check against `cargo metadata`'s resolved dependency graph.
    /// Respects Cargo feature-gating (optional/non-default deps are correctly
    /// excluded), the only backend in this enum that does.
    CargoMetadataNoDep(CargoMetadataNoDepArgs),

    /// Positive specification of the workspace-internal dependency-edge set
    /// plus `[[bin]]` target count. Distinct from the deny-list shape above:
    /// this asserts the graph equals an exact edge set, not just "excludes X".
    CargoMetadataDepGraph(CargoMetadataDepGraphArgs),

    /// Structural AST pattern(s) that must NOT match anywhere in `glob`.
    /// Operates on source TEXT/AST, does **not** respect `#[cfg(...)]`
    /// gating (see §2 callout "cfg-blindness"). Supports an optional
    /// capture-predicate for patterns like D3-01's `$NAME` substring check.
    AstGrepNoMatch(AstGrepArgs),

    /// Structural AST pattern whose match COUNT (optionally scoped to inside
    /// one named function) must equal/meet an expectation. Generalizes
    /// `AstGrepNoMatch` for "must appear exactly once" shapes (D6-01).
    AstGrepMatchCount(AstGrepCountArgs),

    /// Regex line-count/presence check via the `rg` binary (respects
    /// .gitignore by default, same semantics as running `rg` by hand).
    RgMatchCount(RgArgs),

    /// Extract one Rust enum's variant list (+ optional `#[serde(rename_all
    /// = ...)]`) via ast-grep capture and compare against an expected shape.
    /// Same cfg-blindness caveat as AstGrepNoMatch.
    RustEnumShape(EnumShapeArgs),

    /// Run a sequence of `g8` CLI invocations against an isolated scratch
    /// fixture and assert structured outcomes (files, JSON fields, hashes,
    /// stderr format). The general-purpose "real integration test" backend.
    FixtureIntegrationTest(FixtureTestArgs),

    /// Run the same read-only comparison twice against a controlled fixture
    /// and byte-compare stdout, after masking only the declared timing
    /// fields. See §9 for the full determinism boundary.
    ByteDiffTwice(ByteDiffArgs),

    /// Validate the SHAPE of `g8 check --json`'s own output contract
    /// **in-process**, never by shelling out to `g8 check`. See §8.
    G8CheckContract(CheckContractArgs),

    /// A named, hand-reviewed deterministic algorithm implemented as a real
    /// Rust function in `g8-obligations`, for constraints that are
    /// genuinely bespoke logic rather than a pattern/metadata query (e.g.
    /// G1-01's Levenshtein/prefix vocabulary-drift predicate, ported
    /// verbatim from graph-tool per the obligation's own text).
    /// Closed: `name` is itself an enum, not a free string, this is NOT an
    /// escape hatch for arbitrary code, it is a registry of exactly the
    /// named algorithms this contract enumerates.
    BuiltinAlgorithm(BuiltinAlgorithmArgs),
}
```

Why these five and not fewer: `CargoMetadataDepGraph` is not a deny-list (D8-01 needs the *exact* edge set, order-independent-equal, plus a bin-target count, a different assertion shape than "these names are absent"). `AstGrepMatchCount` is needed because D6-01 asserts "exactly one call inside this function," which `NoMatch`/`Presence` cannot express. `RustEnumShape` is needed because D2-01/D7-01 assert a variant *set* plus a serde casing attribute, which is an extraction-then-compare, not a single pattern match. `FixtureIntegrationTest` absorbs every obligation whose convergence test is "run one or more CLI commands against a scratch dir and inspect the result" (D3-02, D4-01, D4-02, D6-02, D7-01 post-T1, D9-01, N7-01, N8-01), folding these into one generic backend keeps the enum closed instead of growing a bespoke variant per obligation. `BuiltinAlgorithm` is the one deliberate, narrow escape hatch for logic that is genuinely an algorithm and not a query; it is closed by an inner enum (`AlgorithmName`), so "no raw shell strings" is preserved and the set of algorithms is still auditable at a glance.

### 1.2 Structured argument types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DepMatcher {
    Exact(String),
    Regex(String),   // e.g. "(?i)mcp" for OBL-N4-01
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoMetadataNoDepArgs {
    pub denied: Vec<DepMatcher>,
    /// Which workspace crates' resolved dep trees to inspect. Empty = all.
    pub scope_crates: Vec<String>,
    /// Which Cargo feature set to resolve against. `Default` = no
    /// `--features` flag (this is what makes the backend cfg-aware: an
    /// `optional = true` dep with no default feature enabling it will
    /// correctly NOT appear in a `Default`-resolved graph).
    pub build_config: BuildConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildConfig {
    Default,
    Features(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoMetadataDepGraphArgs {
    pub required_edges: Vec<(String, String)>,   // (from_crate, to_crate)
    pub forbid_extra_edges: bool,
    pub expected_bin_targets: Vec<(String, u32)>, // (crate, count)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstGrepArgs {
    pub patterns: Vec<String>,          // ast-grep pattern syntax, one or more
    pub lang: AstGrepLang,              // Rust | TypeScript | Python | Go
    pub glob: Vec<String>,
    pub exclude_glob: Vec<String>,
    /// Optional: after a structural match, inspect one captured metavariable
    /// and only count it as a hit if the predicate is satisfied. Used by
    /// D3-01 (`$NAME` must not contain "::").
    pub capture_predicate: Option<CapturePredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturePredicate {
    pub capture: String,               // e.g. "NAME"
    pub forbidden_substring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstGrepCountArgs {
    pub pattern: String,
    pub lang: AstGrepLang,
    pub glob: Vec<String>,
    /// Restrict the match to inside one named function/impl block, e.g.
    /// `Some(FnScope { function: "plan_check" })` for D6-01.
    pub scope: Option<FnScope>,
    pub expected: CountExpectation,
    /// If set, every match's named capture must equal this literal (D6-01:
    /// the one call inside plan_check must be `planner_intent_check`).
    pub capture_equals: Option<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnScope { pub function: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CountExpectation { Zero, Exactly(u32), AtLeast(u32) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgArgs {
    pub pattern: String,      // regex, passed as a single argv element to `rg`
    pub glob: Vec<String>,
    pub expected: CountExpectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumShapeArgs {
    pub file: String,
    pub enum_name: String,
    pub expected_variants: Vec<String>,      // order-independent set compare
    pub expected_serde_rename_all: Option<String>,
}

/// **Amended (T2b).** `binary` defaults to `CurrentExe` when omitted from the
/// artifact JSON (`#[serde(default)]`), every existing `CliInvocation` usage
/// across the other 26 obligations is unaffected by this addition.
///
/// **Amended again (T2e):** any element of `args` containing the literal
/// substring `{{fixture_dir}}` has it replaced with the fixture directory's
/// own absolute, canonicalized path before spawning. Needed because
/// `g8-store`'s `scope_path` column is stored as an absolute filesystem
/// path (confirmed by direct query against a real store, `list_intents_at_path`'s
/// `?2 LIKE scope_path || '%'` match requires the query path to be in the
/// SAME representation), so `g8 query "intents --path=..."` against a
/// scratch fixture can only match with an absolute path, one only known at
/// runtime, never something a static artifact JSON could hardcode portably.
/// Applies only to `args`; `env` values are not substituted (no current
/// obligation needs it there).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliInvocation {
    pub args: Vec<String>,             // argv, e.g. ["init"], ["space","add","../member"]
    pub env: BTreeMap<String, String>, // additive to the controlled base env, §9
    #[serde(default)]
    pub binary: BinarySource,
}

/// **New (T2b).** Which `g8` binary a `CliInvocation` runs against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum BinarySource {
    /// Subprocess-spawn the currently-running `g8` binary via
    /// `std::env::current_exe()` (§4). Fast, no rebuild, but reflects
    /// whatever Cargo feature set THIS binary happens to be compiled with.
    /// Correct default for every obligation except D7-01.
    #[default]
    CurrentExe,
    /// Explicit `cargo run -q -p <package> [--features <features>] --
    /// <args>`. Slower (may trigger a real (re)compile of a different
    /// feature combination) but deterministically exercises a build config
    /// independent of whatever `g8 check` itself was compiled as, the
    /// only way a single running process can observe a *different* build's
    /// behavior. Used only where an obligation must compare across Cargo
    /// feature configurations (today: only D7-01, §2 row 14). This mirrors,
    /// verbatim, the verification method T1's own acceptance criteria
    /// already specify (`cargo run -q -p g8 [--features serve] --
    /// --help`), not a new verification philosophy, just reflected into
    /// the checker.
    CargoRun { package: String, features: Vec<String> },
}

/// **Amended (T2e).** `seed_files` are written to the scratch fixture
/// directory *before* `setup[0]` runs. Added because several obligations
/// (D4-01, P6-01) need a real file to exist for `scan`/`query` to observe
/// anything meaningful, and nothing in the pre-T2e schema could express
/// "place this content at this path", `setup` is CLI invocations only.
/// `#[serde(default)]`, every pre-T2e obligation's args omit this field and
/// are unaffected (empty seed set, unchanged behavior).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureTestArgs {
    #[serde(default)]
    pub seed_files: Vec<SeedFile>,
    pub setup: Vec<CliInvocation>,
    pub assertions: Vec<Assertion>,
}

/// **New (T2e).** One file to write into the fixture directory before setup
/// runs. `path` is relative to the fixture root (parent directories created
/// as needed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Assertion {
    FileExists(String),
    FileHashUnchanged { path: String, before_step: usize, after_step: usize },
    FileHeaderEquals { path: String, expected_prefix: String },
    FileContains { path: String, needle: String },
    JsonField { from_step: usize, path: String, equals: serde_json::Value },
    JsonPathNonEmpty { from_step: usize, path: String },
    ExitCode { from_step: usize, equals: i32 },
    StderrFormat { from_step: usize, json_lines: bool },
    /// **New (T2b).** Plain-text stdout must contain every needle (as a
    /// word-boundary substring match). Added because `--help` output is
    /// plain text, not JSON, nothing in the pre-amendment `Assertion` enum
    /// could inspect it (annotator/T4 finding on D7-01).
    StdoutContainsAll { from_step: usize, needles: Vec<String> },
    /// **New (T2b).** Companion negative-match assertion, same rationale.
    StdoutNotContains { from_step: usize, needle: String },
    NoAnnotationSourceFile { basename: String },
    SecondConnectionErrsAfterTimeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteDiffArgs {
    /// Run once per branch, independently, BEFORE the comparison command.
    /// Empty `setup` = both branches read the same pre-existing fixture
    /// (mint-safe). Non-empty `setup` = each branch independently
    /// bootstraps its own fixture (mint-SENSITIVE, see §9).
    pub setup: Vec<CliInvocation>,
    pub compare: CliInvocation,
    /// JSON paths whose values are masked to `"<normalized>"` before
    /// byte-comparison. Restricted allowlist, see §9. NEVER used to mask
    /// ordering or content, only declared timing fields.
    pub normalize_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheckContractScenario { CleanStore, KnownBadCapabilityFixture }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckContractArgs {
    pub scenarios: Vec<CheckContractScenario>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name", content = "params", rename_all = "snake_case")]
pub enum BuiltinAlgorithmArgs {
    VocabularyDrift {
        canonical_terms: Vec<String>,
        allowlist: Vec<String>,
        scope_glob: Vec<String>,
    },
}
```

### 1.3 Expected-outcome shape

Every backend's "expected outcome" is folded into its own args (`CountExpectation`, `denied` being empty of matches, `required_edges` equality, assertions all holding) rather than a separate generic field, a uniform `expected: Value` would just re-open the shell-string problem one level up (arbitrary JSON standing in for arbitrary logic). Each backend's *executor* returns a `SubCheckResult` (§3) whose `status` is computed by that backend's own, fixed comparison rule.

---

## 2. Complete mapping table (27 of 27)

Ground truth for "what each check does" is `o-g8-decision-gate-20260702.md`'s baseline verification table, cross-checked against `obligations-v0.1.json`'s own `convergence_test.check` text. Every row below is TYPED, see the end-of-table note on why zero rows resolve to `attestation` or `prose_only` in this design, and §10 for the honestly-disclosed approximations that make some of these typed-not-prose calls defensible rather than glossed-over.

| # | Obligation | Backend(s) | Concrete args (abbreviated) | One-line justification |
|---|---|---|---|---|
| 1 | OBL-P5-01 | `RgMatchCount` | pattern `(?i)\b(developer_id\|dev_id\|agent_id\|user_id)\b`, glob `[g8-store/src/queries.rs, g8-store/src/migrations/*.sql]`, expected `Zero` | WIP-cap partition-key check is a column-name text search; heuristic (disclosed §10), not exhaustive semantics. |
| 2 | OBL-P6-01 | `RgMatchCount` ×2 + `FixtureIntegrationTest` **(c fixed for real, T2e, needs `seed_files`)** | (a)/(b) unchanged. (c) `seed_files: [{path:"src/lib.rs", content:"// @g8.capability(name = \"test-cap\", status = \"in_flight\", substrate = \"test\")\npub fn foo() {}\n"}]`, setup `[init, scan --no-watch --quiet -vv (env G8_TRACE_JSON=1), scan --no-watch --quiet -vv (env {})]`, assertions `[StderrFormat{from_step:1, json_lines:true}, StderrFormat{from_step:2, json_lines:false}]` | (a)/(b) unchanged. **(c)'s T2d fix was right about the command/flags but incomplete**, verified live in T2e: the *encoded* row (no `seed_files`, scanning an empty scratch dir) produces **zero stderr lines in both modes**, the exact vacuous-truth bug, one level removed, since an empty dir has nothing for `scan`'s debug-level ast-grep-match event to fire on. Re-verified `-vv` placement (`scan --no-watch --quiet -vv`, verbose token *after* the subcommand) and the env var name (`G8_TRACE_JSON`, confirmed against `main.rs:115`) both already matched reality, reviewer's suspicion of an argument-order/wrong-var-name bug did not hold up; the actual gap was upstream of both, that no annotation existed to observe. With `seed_files` writing one real annotation before setup runs: **verified live, exactly 1 non-empty stderr line per mode, 100%/0% JSON respectively.** |
| 3 | OBL-P9-01 | `CargoMetadataNoDep` | denied `[Exact("surrealdb"), Exact("graph-tool")]`, scope = all 6 crates, `Default` | Textbook deny-list; exactly what the backend exists for. |
| 4 | OBL-D1-01 | `RgMatchCount` ×2 | (a) glob `[g8-extractor/src/lib.rs]`, expected `Zero`; (b) glob `[examples/rust-service/src/lib.rs]`, expected `Exactly(8)` | Obligation's own text mandates `rg`, not ast-grep (ast-grep demonstrably breaks on Rust `//` comment nodes here). |
| 5 | OBL-D1-02 | `AstGrepNoMatch` | pattern `#[capability($$$ARGS)]`, glob `[g8-extractor/src/**]` | Direct negative structural match, no cfg concerns (this pattern shouldn't exist at all in v0.1). |
| 6 | OBL-D2-01 | `RustEnumShape` ×3 **(convention fixed, T2c, checker-side)** | `AnnotationKind`@annotation.rs, `expected_variants:[capability,intent,convergence_test,decision,plan]`, `expected_serde_rename_all:"snake_case"`; `PlanStatus`@plan.rs, `expected_variants:[Idea,Scoped,Dispatched,Blocked,Done,Parked]`, `expected_serde_rename_all:"PascalCase"` (identity transform, Rust idents are already PascalCase); `CapabilityStatus`@model.rs, `expected_variants:[proposed,in_flight,landed,stale,superseded,parked]`, `expected_serde_rename_all:"snake_case"` | Locked enum shapes, exact file:line grounding already given inside the obligation's own `rule.params.*_source`. **`expected_variants` is wire-format** (matches the artifact's own pre-existing `rule.params` values, authored before this contract existed), the checker must extract raw PascalCase Rust identifiers via ast-grep, then apply the `expected_serde_rename_all`-named casing transform to each before comparing, not compare raw identifiers directly. See §2.3 (T2c erratum). |
| 7 | OBL-D3-01 | `AstGrepNoMatch` | pattern `// @g8.capability(name = "$NAME", $$$REST)`, `capture_predicate{capture:"NAME", forbidden_substring:"::"}` | Structural match + captured-metavariable predicate, exactly what `capture_predicate` exists for. |
| 8 | OBL-D3-02 | `FixtureIntegrationTest` **(adjudicated unchanged, T2b, see §2.2)** | setup `[init, space add <member>]`, assertions `[FileExists(.g8/store.db), FileExists(.g8/space.toml), FileContains(space.toml, member-name)]` | Filesystem-shape check across two commands, canonical integration-test shape, and matches SPEC.md Decision 3's locked text exactly. **Will currently FAIL against the real repo**: `space_add` (`cmd/space.rs:22-62`) never writes `.g8/space.toml`, verified by direct read, not assumption. The checker is correct; this is a new live-code drift, not a mapping error, not remapped. |
| 9 | OBL-D4-01 | `FixtureIntegrationTest` **(fixed for real, T2e, needs `seed_files` + `{{fixture_dir}}`)** | `seed_files: [{path:"AGENTS.md", content:"# Boundaries\n\nRoot-level boundary text.\n"}, {path:"src/auth/AGENTS.md", content:"# Boundaries\n\nAuth-specific boundary text.\n"}]`; setup `[init, scan --output json, query "intents --path={{fixture_dir}}/src/auth" --output json]`; assertions `[JsonField{from_step:1, path:"$.inserted_intents", equals:2}, JsonPathNonEmpty{from_step:2, path:"$.data"}, JsonField{from_step:2, path:"$.data[0].description", equals:"Auth-specific boundary text."}]` | **T2d's `inserted_intents=2` and `data[0].description` values were already correct**, verified live against a real two-level fixture built this pass: `scan --output json` on exactly this tree reports `inserted_intents: 2`, matching T4d's encoding precisely. **The actual, T7-reported bug: `g8-store` stores `scope_path` as an absolute filesystem path** (confirmed via direct SQLite query, `scope_path = '/private/tmp/.../t2e-d401-fixture/src/auth'`, not `'src/auth'`), and `list_intents_at_path`'s match (`?2 LIKE scope_path \|\| '%'`) requires the query argument in the same representation, a relative `--path=src/auth` can never match, returning `data: []` regardless of how correct the fixture or the counts are. Needs the new `{{fixture_dir}}` substitution (§1.2) since the scratch directory's absolute path is only known at runtime. **Verified live end-to-end with the absolute path substituted manually: `data[0]` is the auth-specific intent (scope_depth 11, ordered before the root one at scope_depth 9), exactly the assertion's expectation.** |
| 10 | OBL-D4-02 | `FixtureIntegrationTest` **(fixed for real, T2e, my T2d diagnosis was wrong)** | setup `[init, status]` (unchanged, this was never the bug), assertions `[FileHeaderEquals{path:".g8/INTENT_SUMMARY.md", expected_prefix:"<!-- AUTO-GENERATED"}` (was `"<!-- AUTO-GENERATED -->"`, closing the comment immediately)`, NoAnnotationSourceFile{basename:"INTENT_SUMMARY.md"}]` | **My T2d "missing init" diagnosis was empirically wrong**, `[init, status]` was already the encoded setup since T4c, exactly as reviewer reported, and it correctly produces the file with the real header. **The actual bug: a literal-string mismatch in `expected_prefix` itself.** The real header is `<!-- AUTO-GENERATED by g8 v0.1.0 at {timestamp} -->` (`status.rs:144-146`), a variable-length `by g8 v0.1.0 at {timestamp}` segment sits between "GENERATED" and the closing `-->`. `expected_prefix: "<!-- AUTO-GENERATED -->"` (closing the comment immediately, no gap) can therefore never be a true prefix of the real content, confirmed with a direct Python `str.startswith()` check against the real generated file: `False`. Truncating to `"<!-- AUTO-GENERATED"` (dropping the closing `-->`, since the timestamp makes anything past that point non-constant) **verified `True`** against the same real file. The code was correct the whole time; the assertion string was the only thing wrong. Not a live-drift finding. |
| 11 | OBL-D5-01 | `AstGrepNoMatch` | pattern `RusqliteStore`, glob `[g8-planner/src/**, g8-conflict/src/**]` | Direct negative structural match, as literally specified. |
| 12 | OBL-D6-01 | `AstGrepMatchCount` + `RgMatchCount` ×3 **(target fixed T2c, false-pass hole closed T2d-2, both artifact-side)** | (a) pattern `store.$METHOD($$$ARGS)`, `scope:{function:"plan_check"}`, glob `[g8-planner/src/lib.rs]`, `Exactly(1)`, `capture_equals:("METHOD","planner_intent_check")`; (b1) struct fields `pub\s+(existing_matches\|budget\|bottlenecks\|drift_hints\|intent_overlaps\|parked_ideas):\s*serde_json::Value`, glob `[g8-store/src/store_impl.rs]`, `Exactly(6)`; (b2) CTE declarations `^\s*(existing_matches\|budget\|bottlenecks\|drift_hints\|intent_overlaps\|parked_ideas)\s+AS`, same glob, `Exactly(6)`; (b3) final-`SELECT` column aliases `\)\s*AS\s+(existing_matches\|budget\|bottlenecks\|drift_hints\|intent_overlaps\|parked_ideas)\b`, same glob, `Exactly(6)` | `plan_check` is one function, ast-grep's function-scoping makes "exactly one call, and it's the right one" fully mechanical. **(a)'s glob is unchanged/correct**; its scope-matching has a separate, real ast-grep generic-function bug (T3b's, §2.3), not re-scoped here. **(b) is now three independent anchors, not one**, T6's adversarial probe proved a single anchor (any one of struct-fields, CTE-names, or column-aliases alone) can false-pass on "the right names exist somewhere, no real composite query behind them." All three independently verified against the real file: **exactly 6/6/6, zero false positives, each.** Anchoring all three closes the hole T6 proved without any new checker capability (all three are `RgMatchCount`, already-existing), see §2.4 for exactly which fake-SQL construction each pair alone still misses and why the third closes it. |
| 13 | OBL-D6-02 | `FixtureIntegrationTest` **(re-grounded, T2d, artifact-side)** | setup `[init, substrate add <sub> --stale-days 3, plan new "..." --substrate <sub> --dispatch, plan new "..." --json]`; assertions **`[JsonField{from_step:3, path:"$.drift_hints", equals:[]}]`** (was `JsonPathNonEmpty` on an unresolved-consumes fixture) | `advisory:true` affects exit-code wiring (§5), not checker trust. **Re-grounded against the real SQL**, not the obligation's original inference: `store_impl.rs`'s real `drift_hints` CTE computes **stale-Dispatched-plan detection** (`days_stale = (now - updated_at)/86400000`, filtered `status NOT IN (Done,Parked)` and `> stale_threshold_days`), it has nothing to do with unresolved `consumes`/`produces` wiring, which is what the obligation's own `rule.params.inference` (honestly labeled "NOT a verbatim SPEC statement... NOT confirmed anywhere in the evidence") guessed. A fixture built around an unresolved-consumes Capability was *always* going to show empty `drift_hints`, regardless of code correctness, not a bug, a wrong hypothesis. See §2.4 for why the *positive* case (an actually-stale plan) isn't practically fixturable here and what this assertion proves instead. |
| 14 | OBL-D7-01 | `FixtureIntegrationTest` **(amended, T2b)** | setup: step0 `CliInvocation{binary: CargoRun{package:"g8", features:[]}, args:["--help"]}`; step1 `CliInvocation{binary: CargoRun{package:"g8", features:["serve"]}, args:["--help"]}`. assertions: `StdoutContainsAll{from_step:0, needles:[init,scan,plan,status,check,merge,query,space,link,substrate]}`, `StdoutNotContains{from_step:0, needle:"serve"}`, `StdoutContainsAll{from_step:1, needles:[the same 10, plus serve]}` | **Not** `RustEnumShape` and **not** `current_exe()`-based: the `Command` enum's `Serve` variant stays in *source* even after T1 cfg-gates it (ast-grep is cfg-blind, §2.1), and one running binary only knows its own feature set. `CargoRun` explicitly rebuilds/runs each config, mirroring T1's own acceptance-criteria verification method exactly. **Disclosed cost:** the only obligation in the set that can trigger a real (re)compile, accepted, not optimized away, per T2b. |
| 15 | OBL-D8-01 | `CargoMetadataDepGraph` **(edges updated, T2c, artifact-side)** | `required_edges` = the original 11 edges from ARCH §1 / this obligation's `rule.params`, **plus 3 new**: `(g8, g8-obligations)`, `(g8-obligations, g8-core)`, `(g8-obligations, g8-store)`, 14 total; `forbid_extra_edges:true`; `expected_bin_targets:[("g8",1)]` (unchanged, `g8-obligations` is a lib crate, adds no bin target) | Exact positive edge-set + bin-count assertion, the shape this backend exists for. **`g8-obligations` postdates the original 11-edge list** (it didn't exist when D8-01 was compiled or when this contract's §4 first proposed it), confirmed against the real workspace: `crates/g8-obligations/Cargo.toml` depends on `g8-core` + `g8-store`, root `Cargo.toml` lists it as a member, `g8` depends on it. See §2.3 (T2c erratum) for the separate `g8-conflict → g8-store` adjudication (unchanged, was already in the original 11, stays required, currently genuinely missing from the real graph). |
| 16 | OBL-D9-01 | 4× `FixtureIntegrationTest` **(decomposed, T2b, ratified per annotator/T4)** | (a) setup `[init]`, assert `FileContains(.g8/config.toml, "enforcement = \"off\"")`; (b) setup `[init, check --json]`, assert `ExitCode{from_step:1, equals:0}` + `JsonField{from_step:1, path:"$.enforcement_disabled", equals:true}`; (c) setup `[init, init]`, assert `FileContains(.g8/config.toml, "enforcement = \"off\"")` (second init doesn't flip it); (d) setup `[init, init --enforce]`, assert `FileContains(.g8/config.toml, "enforcement = \"on\"")` | My original single-sequence mapping wrongly reused the stdout-only `JsonField` assertion to inspect `.g8/config.toml`, a **file**, not a step's stdout, `JsonField`/`ExitCode`/`StderrFormat` are anchored to a setup step's own CLI output; nothing in v1's `Assertion` enum could anchor a file assertion to an *intermediate* point in a multi-step sequence. Decomposing into 4 short, independent fixtures (each ending exactly where its own assertion needs to look) sidesteps the need for that anchoring entirely, **zero schema changes required**, confirmed by re-deriving it independently in T2b before ratifying. `FileContains` on TOML output is a disclosed substring heuristic (§10), grounded against `ctx.rs::write_config`'s real `toml::to_string_pretty` output shape. |
| 17 | OBL-D10-01 | `G8CheckContract` | `scenarios: [CleanStore, KnownBadCapabilityFixture]` | Validates the very contract this document extends; **must** stay in-process, see §8. |
| 18 | OBL-D11-01 | `RgMatchCount` ×2 | (a) `std::time::SystemTime::now\(\)`, glob `[crates/*/src/**]`, `Zero`; (b) `chrono::(Local\|Utc)::now\(\)`, glob `[crates/*/src/**]`, `Zero` | Wall-clock/RNG literal patterns are mechanical; the "async fn combined with a network import" clause is **not** independently checked, it is logically subsumed by OBL-D11-02 (no network crate is even a dependency, so no code path, async or sync, can reach one). Documented narrowing, not silent. |
| 19 | OBL-D11-02 | `CargoMetadataNoDep` | denied = the 10-crate list verbatim, scope = all 6 crates, `Default` | Textbook deny-list. **RESOLVED (Erratum 4 / §2.6):** Q4 ruled, IDs are content-derived (nanoid removed from `g8-core`/`g8-store`; serve's feature-gated usage stays per the rider) and dev-dependencies are OUT of deny-list scope (backend now walks normal+build edges only, disclosing `dependency_kinds_in_scope` in evidence). Passes for real: `cargo tree -i getrandom` reaches the workspace only via `tempfile [dev-dependencies]`. |
| 20 | OBL-D11-03 | `AstGrepNoMatch` | patterns `[Uuid::new_v4(), rand::random, rand::thread_rng()]`, glob `[crates/*/src/**]` | Direct negative structural match over 3 literal patterns, as specified. |
| 21 | OBL-D11-04 | `ByteDiffTwice` **(re-encoded, Erratum 4, the per-branch shape was a misreading)** | `seed_files`: one `src/lib.rs` with 2 capability annotations; setup (ONCE, one fixture) `[init, scan]`; compare `query capabilities --output json` TWICE against that same store; `normalize_paths: []` | `conv-d11-04`'s prose: same command twice against an identical, >=2-element fixture store, ONE store, TWO reads; ordering divergence = FAIL (DEF-11 HashMap hazard, fresh SipHash seed per process). The former two-branch shape compared two *different* inputs (own paths/timestamps per store) and failed on tempdir noise in `obligations_note`, not signal, see §2.6. **Passes for real now, with zero normalization.** |
| 22 | OBL-N4-01 | `CargoMetadataNoDep` + `AstGrepNoMatch` | (a) denied `[Regex("(?i)mcp")]`, scope = all 6, `Default`; (b) pattern `$SOCKET.incoming()`, glob `[g8/src/**]` | Two of the obligation's three documented checks retained; the third (`Command::Serve` pattern) is **dropped** from the active mapping, see §2.1, it now always matches the legitimate (non-MCP) `serve` dashboard variant and is a stale false-positive generator post-OQ-08. |
| 23 | OBL-N5-01 | `CargoMetadataNoDep` | denied `[axum, actix-web, warp, rocket, tower-http, tiny_http]`, scope = all 6, **`Default`** build config | `cargo_metadata`'s feature-awareness (§2.1) is exactly what makes this obligation meaningful again post-T1: once `tiny_http` is `optional = true` + default-off, this check on a `Default`-resolved graph correctly flips FAIL→PASS. The `tiny_http::Server::$METHOD` ast-grep sub-check is **dropped**, same cfg-blindness problem as #22/#14. |
| 24 | OBL-N6-01 | `AstGrepNoMatch` | pattern `$STORE.update_plan_status($ID, PlanStatus::$ANY)`, glob `[g8/src/cmd/scan.rs]` | Was bucketed "prose" only because the obligation's own convergence_test prose says "call graph reachability," which ast-grep can't do generically, but its own `rule.params` already narrows this to a literal, glob-scoped pattern check, which IS mechanical. Direct-call-only; transitive reachability is a disclosed gap (§10), not silently claimed. |
| 25 | OBL-N7-01 | `FixtureIntegrationTest` | setup `[<hash before>, scan, <hash after>]`, assertions `[FileHashUnchanged{path:.g8/store.db, before_step:0, after_step:2}]` | Direct hash-before/after check, exactly as specified. |
| 26 | OBL-N8-01 | `RgMatchCount` ×2 + `FixtureIntegrationTest` | (a) `journal_mode\s*=\s*WAL`, `AtLeast(1)`; (b) `busy_timeout`, `AtLeast(1)`; (c) assertions `[SecondConnectionErrsAfterTimeout]` | Pragma-presence grep + the genuine concurrency behavioral test the obligation itself describes. |
| 27 | OBL-G1-01 | `BuiltinAlgorithm` **(allowlist extended, T2d, artifact-side)** | `VocabularyDrift{canonical_terms: [10 terms], allowlist: [12 types, the original 6 (`PlanStatus`, `CapabilityStatus`, `AnnotationKind`, `IntentKind`, `DecisionStatus`, `PlanDraft`) plus 6 ratified this pass (`AnnotationValue`, `ConflictKind`, `IntentOverlap`, `IntentSourceKind`, `PlanIntentRelation`, `SubstrateBudget`)], scope_glob: [g8-core/src/**]}` | **Mandatory per Mike's ruling** ("no reason to stay prose"). Reuses the real graph-tool predicate per the obligation's own `rule.params`. OQ-10 (new-vs-baseline diff) unresolved, v1 default is "check all current names," not just ones new since some baseline (§10). **All 6 new entries ratified**, each independently confirmed as a real, currently-defined `g8-core` type following the existing Canonical+Suffix precedent (`AnnotationValue`@annotation.rs:94, `ConflictKind`@conflict.rs:33, `IntentOverlap`@plan.rs:214, `IntentSourceKind`@model.rs:221, `PlanIntentRelation`@plan.rs:365, `SubstrateBudget`@substrate.rs:13, all genuinely in `g8-core`, matching G1-01's own scope, not `g8-store` as ARCHITECTURE.md's §3.3 layout might suggest at a skim). |

**Tally: 27 typed / 0 attestation / 0 prose_only.**

This is a deliberate result, not an assumption papered over: every one of the 27, including the 3 the baseline table calls PROSE (D6-01, N6-01, G1-01) and the 3 the plan flags as having "splittable judgment-call parts" (D4-01, D11-01, P5-01), turned out to have a mechanical, honestly-scoped resolution once the actual `rule.params` (not just the prose `convergence_test.check` summary) was read closely. §10 lists every place this required narrowing the obligation's literal scope, so a reviewer can independently judge whether the narrowing is acceptable. The attestation mechanism (§7) is still fully specified and load-bearing infrastructure, it is the fallback if T3 finds one of these genuinely infeasible to implement as specified, and it remains available for any obligation added to the artifact in the future.

### 2.1 Cross-cutting finding: three checks are stale post-OQ-08

`ast-grep`/`RustEnumShape` operate on source **text/AST**. They are blind to `#[cfg(feature = "...")]` gating, a `#[cfg(feature = "serve")] Serve(ServeArgs)` variant is exactly as visible to an ast-grep pattern as an unconditional one, because cfg-stripping happens at the compiler's macro-expansion stage, which ast-grep never runs. Three obligations were originally written assuming `serve` was simply absent from the source (the pre-ruling world where Q1 might have resolved to "delete"); now that Mike ruled **feature-gate + harden** (`o-g8-defuse-and-wire-20260702.md` OQ-08), all three need to check *compiled-artifact behavior*, not source presence:

- **D7-01** (#14 above): remapped from a would-be `RustEnumShape` to `FixtureIntegrationTest`. **Amended again in T2b**: the first pass of this remap still assumed a single running binary could observe both build configs via `current_exe()`, it cannot, since `current_exe()` only ever returns the path to whatever process is currently executing, which was compiled with exactly one feature set. The T2b fix uses the new `BinarySource::CargoRun` (§1.2) to explicitly `cargo run` each config, independent of how the checker itself was built. This is the one obligation in the entire set that can trigger a real compile as a side effect of `g8 check`, a genuine, disclosed cost, not something to silently optimize away by weakening the check.
- **N4-01** (#22): the `Command::Serve` sub-check dropped; the behavioral `$SOCKET.incoming()` check (already added per the artifact's own DEF-14 fix, for the same underlying reason) is the one retained.
- **N5-01** (#23): the `tiny_http::Server::$METHOD` sub-check dropped in favor of a `Default`-build-config `CargoMetadataNoDep`, which, unlike ast-grep, correctly reads Cargo's own feature resolution.

**General rule for T3/T4/T6:** prefer `CargoMetadataNoDep`/`CargoMetadataDepGraph` or `FixtureIntegrationTest` (compiled-binary behavior) over `AstGrepNoMatch`/`AstGrepMatchCount`/`RustEnumShape` for any obligation whose truth depends on which Cargo feature set is active. Static AST backends remain correct and cheap for everything that must be unconditionally true regardless of feature flags. **Note the finer distinction D7-01 forced out:** even among behavioral backends, `current_exe()` only ever tells you about *this* build, reaching a genuinely *different* feature combination requires `BinarySource::CargoRun`, not just "any `FixtureIntegrationTest`."

### 2.2 New live-drift finding: OBL-D3-02 (T2b adjudication)

**Question posed:** does `g8 space add` writing only a store row, never `.g8/space.toml`, mean the checker (§2 row 8) is wrong, or the code is?

**Answer: the code is.** Evidence, read directly rather than taken on the annotator's word:

- `crates/g8/src/cmd/space.rs:22-62` (`space_add`) does exactly one piece of persistence, `store.upsert_project(&project)` at line 47, then prints a confirmation message. No reference anywhere in the function (or the file) to `space.toml`, `SpaceToml`, or any `toml::to_string*` call.
- SPEC.md Decision 3 (locked, non-negotiable per CLAUDE.md's "Builders inherit these without renegotiation"): *"ConvergenceSpace is declared by `.g8/space.toml`... members listed as relative or absolute paths."* ARCHITECTURE.md §10.2 reproduces the exact TOML shape, `[[members]]` blocks included.
- OBL-D3-02 itself (compiled well before this defuse-and-wire session, in `o-g8-obligations-20260701.md`'s original pass) already encoded this expectation: *"run `g8 space add <member>`, assert -f .g8/space.toml at space root and the member path is listed in it."* This is not a new interpretation invented for this contract, it is what the obligation has said since it was first compiled.
- The codebase already has the exact TOML-writing infrastructure this would need (`ctx.rs::write_config`, used by `init.rs:82` for `.g8/config.toml`), `space_add` simply never calls an equivalent for `space.toml`. This reads as an incomplete implementation, not a deliberate, differently-designed alternative (no comment, no `// TODO`, no alternate persistence path visible anywhere in `cmd/space.rs`).

**Disposition:** OBL-D3-02's mapping in §2 is **unchanged**, the checker stays exactly as specified, and it will honestly report `status: failed` against the real repo the first time it runs. Weakening the check to match the current gap would repeat the exact failure mode Mike's rulings on this board exist to stop (the `g8 serve` v0.1-scope drift, OQ-08, was found the same way: locked docs vs. real code, docs treated as ground truth, not silently reconciled downward). This is now a second, independent live-drift finding of the same shape, flagged here for reviewer to add to Mike's pile, not resolved unilaterally as part of this contract amendment (out of T2b's scope, which is the checker contract, not `cmd/space.rs`).

### 2.3 T2c erratum: three defects from the first real 27-obligation run

Adjudicated against real code and the real artifact, not on report. Three unrelated root causes, three separate rulings.

**(1) OBL-D2-01: checker/artifact casing-convention mismatch, ruled checker-side, no artifact change.**

`RustEnumShape` (§1.2) extracts a Rust enum's variant identifiers via ast-grep, which necessarily returns them exactly as written in source, always PascalCase, because that is the only casing the Rust compiler accepts for enum variants, *independent of* any `#[serde(rename_all = ...)]` attribute (the attribute only changes the *serialized wire form*, never the source-level identifier). T4 authored `expected_variants` as **wire-format** strings, correctly, since that is what the obligation's own pre-existing `rule.params` already say (`CapabilityStatus`'s locked list is `["proposed","in_flight",...]`, snake_case, straight from the original o-g8-obligations compile, T4 didn't invent this, it re-used the artifact's own authoritative values). T3's first-pass checker compared the raw PascalCase extraction directly against that snake_case list, so it could never match, regardless of whether the real code is correct.

**Ruling:** the checker must normalize. Algorithm: (a) extract raw variant identifiers via ast-grep; (b) extract the enum's actual `#[serde(rename_all = "...")]` attribute string, if present, and compare it against `expected_serde_rename_all`, a mismatch here is a `Failed` in its own right (this is what lets the check catch someone silently changing or deleting the attribute, which raw-identifier comparison alone could never detect, direct evidence the wire-format convention is the semantically correct one, not just the convenient one: SPEC Decision 2 states the two enums' differing casings are "by design," i.e. the casing itself is part of the locked contract); (c) apply the transform named by `expected_serde_rename_all` to each raw identifier (`snake_case`: `InFlight`→`in_flight`; `PascalCase`: identity); (d) order-independent set-compare the transformed names against `expected_variants`. Fix lands in T3b. §2 row 6 amended above with the exact per-enum `expected_serde_rename_all` values, verified against ARCHITECTURE.md §3.1's real attribute lines (`AnnotationKind`/`CapabilityStatus`: `snake_case`; `PlanStatus`: `PascalCase`, an identity transform present for explicitness). **No artifact change**, T4's `expected_variants` were already correct.

**(2) OBL-D8-01: `g8-obligations` edges are a real gap (artifact-side fix); `g8-conflict → g8-store` is a real, separate finding (not an artifact bug).**

*Missing `g8-obligations` edges:* confirmed directly, `crates/g8-obligations/Cargo.toml` depends on `g8-core` and `g8-store`; root `Cargo.toml` lists `g8-obligations` as a workspace member and `g8` depends on it. None of these 3 edges were in the original 11 (that list, and ARCH §1's diagram, predate this crate's existence, it's this contract's own §4 that proposed it). Ruled: add `(g8, g8-obligations)`, `(g8-obligations, g8-core)`, `(g8-obligations, g8-store)` to `required_edges`. Fix lands in T4c (data-only change to §2 row 15's `checker.checks[0].args.required_edges`, no checker-logic change).

*`g8-conflict → g8-store` missing_edge:* this edge was already in the original 11 (SPEC.md Decision 8, locked: *"g8-conflict (deps: g8-core, g8-store)"*), and it genuinely does not exist in the real graph, confirmed: `crates/g8-conflict/Cargo.toml`'s `[dependencies]` lists only `g8-core`, `serde`, `serde_json`, `thiserror`, `tracing`. But this is not a silent oversight the way D3-02 was, `crates/g8-conflict/src/store_trait.rs:1-9` documents it explicitly: `g8-conflict` defines its own narrower `ConflictStore` trait instead of depending on `g8_store::StoreConnection`, and says why: *"1. `g8-conflict` compiles while `g8-store` is still being developed (Phase 2 parallelism). 2. Future adapters are thin: `impl ConflictStore for RusqliteStore` delegates to the corresponding `StoreConnection` methods."* Reason 1 describes a bootstrap state; reason 2, written in future tense, describes the follow-up that was never executed. **Ruling: the checker is correct, this is a real, currently-missing edge, matching both the locked SPEC and the code's own documented intent, not an artifact bug. `required_edges` is unchanged for this edge (no fix needed, it was already right).** This is a third live-code-vs-docs finding for Mike's pile, but a different shade from OQ-08/D3-02: those were silent; this one is self-disclosed as an intentional, temporary, not-yet-completed bootstrap measure. Flagging for reviewer, not resolving here (same reasoning as §2.2: out of the contract's scope to touch `g8-conflict` source).

One actionable note for whoever picks this up, found while reading `store_trait.rs`/`lib.rs` in full to ground this ruling: the reason-2 comment's literal suggestion (`impl ConflictStore for RusqliteStore`) would name the concrete `RusqliteStore` type inside `g8-conflict`'s own source, which is exactly what **OBL-D5-01** (§2 row 11, already in this mapping) forbids in that crate. `g8-planner::plan_check<S>(store: &S, ...)` (see item 3 below) already shows the established, D5-01-safe pattern in this codebase: implement generically over `S: StoreConnection` (or `&dyn StoreConnection`), never concretely over `RusqliteStore`. Worth relaying so the eventual fix doesn't trade one honest FAIL for another.

**(3) OBL-D6-01: wrong target path, `queries.rs` doesn't exist; corrected to `store_impl.rs`. Artifact-side fix; ast-grep generic-fn bug is separately real but is T3b's, not re-scoped here.**

Confirmed directly: `crates/g8-store/src/` contains exactly `lib.rs`, `store_impl.rs`, and `migrations/mod.rs`, no `queries.rs`. `planner_intent_check` (trait decl + the actual composite-query implementation, six named CTEs and all) lives entirely in `store_impl.rs` (trait at line 255, impl at line 1872, the six column names at lines 1889–2054). `CLAUDE.md`'s own `queries.rs` reference (in its composite-queries example) is stale for the identical reason, ARCHITECTURE.md §2's planned file layout named a separate `queries.rs`, and the real implementation consolidated it into `store_impl.rs` instead, without either doc being updated. Unlike the OQ-08/D3-02/g8-conflict findings, this has zero externally-observable behavioral consequence (it's which literal `.rs` file a private implementation detail lives in, not a locked product property), noting it plainly rather than escalating it with the same weight.

**Ruling:** §2 row 12's sub-check (b) glob corrected from `[g8-store/src/queries.rs]` to `[g8-store/src/store_impl.rs]`; sub-check (a)'s glob (`g8-planner/src/lib.rs`) is unchanged and confirmed correct, `plan_check` really is there (line 145). Separately, but *not* part of this ruling (explicitly out of scope, T3b's): `plan_check` is declared `pub fn plan_check<S>(store: &S, draft: &PlanDraft) -> ...`, generic over `S`, not `plan_check(store: &dyn StoreConnection, ...)` as ARCHITECTURE.md §3.4 itself is quoted (in the function's own doc comment) as specifying. Registering this as the concrete shape of the "ast-grep scope-finder generic-fn bug" mentioned in the task brief, for T3b's benefit, without attempting to fix the ast-grep matching logic myself.

### 2.4 T2d erratum: D6-01 anchoring, four fixture defects, G1-01 allowlist

Six items, all adjudicated against real code and the real compiled binary, several run empirically (built `g8`, executed it against scratch fixtures), not just read. Full detail here; §2 rows 2, 9, 10, 12, 13, 27 carry the corresponding short rulings.

**(1) D6-01(b): over-matching, the false-pass hole, and its closure, revised in a second pass (T2d-2) after T6's adversarial probe defeated my first fix.** T6 first found the bare word-boundary alternation over-matches badly, confirmed directly: word-bounded `rg -c` for the six names against the real `store_impl.rs` returns **34** hits, not 6 (each name legitimately recurs as a CTE declaration, a `FROM`-clause reference, a struct field, and a deserialization site, none of which are "the composite query's declared return columns"). My first fix anchored to the final `SELECT`'s column-alias syntax (`) AS <name>`), precise (exactly 6/6, verified), but **T6's adversarial probe then proved even that single anchor can false-pass**: nothing stops someone from writing syntactically-valid, correctly-aliased-but-semantically-empty SQL (e.g. `(SELECT NULL) AS existing_matches, ...`) that satisfies the alias pattern without any real composite query behind it. One anchor, however well-chosen, is still one place to fake.

A second review round proposed two alternative single/paired anchors: struct-field declarations on `RawIntentCheckPayload` (`pub existing_matches: serde_json::Value`, etc.) and CTE declarations (`existing_matches AS (`, name *before* `AS`, distinct from my column-alias pattern which has the name *after*). One candidate in that round (ast-grep scoped to the struct's name) was rejected outright, `g8-obligations` has no struct-name-scoping capability today (verified: zero `struct_name`-shaped support in the backend), so choosing it would mean new checker work mid-flight, not just an artifact edit, contradicting the "artifact-side only" bias for this fix. The remaining two (struct-fields, CTE-declarations) were verified independently, both exactly 6/6 against the real file, no phantom-file errors this time.

**Ruling: keep all three anchors, not just the proposed pair.** I verified all three (struct-fields, CTE-declarations, and my original column-aliases) independently: **6/6/6, zero false positives across the board.** The reasoning for keeping three rather than settling on the recommended two: struct-fields + CTE-declarations together still has a real gap, six correctly-named CTEs that are never actually referenced by the final `SELECT` (dead/unused CTEs; SQLite doesn't error on these) would satisfy *both* of those checks while the query's *actual returned output* is wrong or missing. My column-alias anchor is specifically anchored to the final `SELECT`'s output naming, not the intermediate CTEs, so it's the one anchor that would catch exactly that scenario. All three together require an adversary to fake three independent, physically-separated locations (the Rust struct definition, the CTEs' declarations, and the final SELECT's output aliases) simultaneously and consistently, a meaningfully narrower target than any one or two of them, at zero additional checker cost (all three are `RgMatchCount`, the existing backend; no new capability). Combined with sub-check (a) (exactly one call to a method literally named `planner_intent_check`), this is the strongest structural claim achievable without upgrading to a `FixtureIntegrationTest` that seeds real overlapping data and inspects the *returned* `FitReport`, the genuine verified-tier closure, not attempted here, disclosed as the residual gap (§10) exactly like this contract's other checked-vs-verified limits: no static check can fully prove the SQL *executes* correctly, only that it's *declared* correctly, however many independent places that declaration is checked.

**(2) D4-01: `g8 scan --output json` cannot carry the assertions the original row implied.** Checked `render_scan_result`/`cmd/scan.rs` directly: scan's JSON output is `{g8_version, status, inserted_capabilities, inserted_intents, inserted_decisions, files_scanned, duration_ms}`, aggregate counts only, no per-annotation or per-heading classification detail. My original row 9 ("per-heading `JsonField` checks") assumed a level of detail that command never exposed, for any narrowing to have needed to happen. **Ruling:** narrow to what's actually observable, (a) `inserted_intents` count via `scan --output json`, proving the classification table fires broadly rather than silently dropping headings; (b) deeper-file-wins via `g8 query "intents --path=<deep>"` (confirmed real DSL form + JSON shape directly in `cmd/query.rs`), asserting the *returned* intent's content is the deeper file's, not the shallower root's, this is the part that actually needs a query, since scan's own JSON can't carry it. Artifact-side, T4d.

**(3) D4-02: not a live-drift finding, the fixture skipped a prerequisite.** Read `status.rs::regenerate_intent_summary` directly: it unconditionally writes the `<!-- AUTO-GENERATED -->` header at lines 144–146, the code is correct, full stop. The missing header is explained entirely by the fixture: `status` alone (my original row 10's whole setup) either fails `require_init` outright (no `.g8/` yet) or, if `.g8/` exists but no space has been created, hits `regenerate_intent_summary`'s own early return (`None => return Ok(())` at line 74, "no space yet, skip"), either path means the file is never written, header or otherwise. **Ruling:** add `init` as a prerequisite setup step (`[init, status]`). This is squarely a fixture-completeness bug on my/T4's side from the original mapping, not a fourth entry for Mike's pile, unlike D3-02/g8-conflict, there is no gap between what the code does and what SPEC/ARCH require.

**(4) D6-02: the obligation's own "inferred, not confirmed" caveat was flagged for a reason, the inference was wrong, and now we know the right answer.** Read the real `drift_hints` CTE directly in `store_impl.rs` (not ARCHITECTURE.md's copy, the actual implementation): it computes **stale-Dispatched-plan detection**, `days_stale = (now_ms - updated_at) / 86400000`, filtered to `status NOT IN ('Done','Parked')` and `days_stale > stale_threshold_days`. This has nothing to do with "declared `consumes`/`produces` with no resolved producer," which is what OBL-D6-02's `rule.params.inference` guessed (explicitly hedged in its own text: *"NOT a verbatim SPEC statement... NOT confirmed anywhere in the evidence gathered for this compile"*). A fixture built around an unresolved-`consumes` Capability was never going to populate `drift_hints`, regardless of whether the code is right, the fixture was testing for the wrong phenomenon. This is genuinely useful new information, independent of the fixture-defect framing: **`drift_hints` = stale-plan detection, confirmed, not capability-wiring drift.** Worth relaying to reviewer/Mike as a resolved uncertainty, not a bug.

Practical consequence for the fixture: the *positive* case (a plan that has actually gone stale) is not cheaply fixturable through `CliInvocation`-only setup. `stale_threshold_days` and the day-granularity `/86400000` integer division mean nothing short of real multi-day wall-clock elapse trips it, there is no CLI flag to backdate a Plan's `updated_at`, and this contract's `FixtureIntegrationTest` schema has no raw-DB-write primitive to inject one (deliberately, see §0's non-negotiable constraint's spirit of no uncontrolled state mutation). **Ruling:** narrow the assertion to what's honestly achievable fast and deterministically, a freshly-dispatched plan under a real `substrate_budget` (`stale-days 3`) shows `$.drift_hints` as `[]` (confirms the field is present, correctly typed, and correctly reports "nothing stale" rather than erroring or omitting the key). This proves structural/negative correctness, not the positive case; recorded here as an honestly-disclosed gap rather than a forced, slow (multi-day) or fabricated test. Artifact-side, T4d.

**(5) P6-01(c): root cause empirically isolated by running the real binary, not a checker bug, a fixture-scenario bug (two distinct causes).** Built `g8` and ran it directly against scratch fixtures rather than reasoning about it in the abstract:

- `status --no-regen` (the original setup) produces **zero stderr lines**, in *both* `G8_TRACE_JSON` states, confirmed by direct execution. Root cause: `status`'s entire call path (its own handler plus every `StoreConnection` method it calls) relies solely on `#[instrument]` spans, and `main.rs::init_tracing` never calls `.with_span_events(...)`, so tracing-subscriber's `fmt()` layer never actually prints span enter/exit, regardless of verbosity or JSON mode. With zero lines, `[].iter().all(predicate)` is vacuously `true` for *any* predicate, which makes the "should NOT be all-JSON" assertion (`json_lines: false`, expecting `!all_json`) spuriously **fail**, since `all_json` is vacuously true even with nothing to check. This is a real, confirmed edge case in the original test design, not a T3 implementation defect, `crates/g8-obligations/src/exec/fixture.rs`'s actual `StderrFormat` logic (read directly) is correct and already filters blank lines; it was simply never fed anything.
- Switching to `scan` (which *does* have a real `debug!`-level tracing event, in `g8-extractor` on each ast-grep match) does produce output once given `-vv` and a fixture with a real annotation, but by default it *also* emits `cmd/scan.rs:27`'s unconditional `eprintln!("Scanning {}...", ...)`, a plain-text, non-tracing line that appears in **both** trace-format modes (it isn't gated by `G8_TRACE_JSON` at all, only by `--quiet`), breaking the "all lines JSON" assertion even in correctly-configured JSON mode.

**Ruling, verified empirically end-to-end:** sub-check (c)'s setup becomes `scan --no-watch --quiet` (suppresses the banner) with global `-vv` (guarantees the debug-level span event fires), against a fixture containing one real `// @g8.capability(...)` annotation, run twice (`G8_TRACE_JSON=1` and unset). Confirmed against the real compiled binary: exactly 1 non-empty stderr line in each mode, 100% valid JSON with the env var set, 0% without. Both `StderrFormat` assertions now have real, non-vacuous signal to evaluate. Artifact-side only (T4d), the checker's `StderrFormat` logic itself needed no change, confirmed correct by direct code read.

**(6) G1-01 allowlist: all six ratified, all confirmed to actually live in `g8-core`.** Grepped each type's real definition directly rather than trusting ARCHITECTURE.md's section layout (which groups some of these under g8-store's *usage* of them, as trait-method parameter types, easy to misread as meaning they're *defined* there): `AnnotationValue` (`g8-core/src/annotation.rs:94`), `ConflictKind` (`g8-core/src/conflict.rs:33`), `IntentOverlap` (`g8-core/src/plan.rs:214`), `IntentSourceKind` (`g8-core/src/model.rs:221`), `PlanIntentRelation` (`g8-core/src/plan.rs:365`), `SubstrateBudget` (`g8-core/src/substrate.rs:13`), every one genuinely in `g8-core`, matching G1-01's own `scope.paths: [crates/g8-core/src/**]`. Each follows the same Canonical+Suffix shape as the original 6-entry allowlist (`Conflict`+`Kind`, `Intent`+`Overlap`, `Intent`+`SourceKind`, `Plan`+`IntentRelation`, `Substrate`+`Budget`, `Annotation`+`Value`) against G1-01's 10 canonical terms. Ratified without reservation.

### 2.5 T2e: empirical-only closure of the three still-failing fixture rows

Rules of engagement for this pass, per reviewer: run the real binary against each candidate fixture until it actually passes, then amend with the evidence. All three did fail against T7's independent run; none of the three failed for the reason my T2d ruling assumed. Built `g8` fresh (clean build) before any of this, then iterated against real scratch fixtures, every claim below has a command transcript behind it, not a re-reading of source.

**(1) OBL-P6-01(c): the T2d fix was directionally right and empirically incomplete.** Re-tested the exact encoded row (`scan --no-watch --quiet -vv`, both trace modes) against an **empty** scratch fixture, zero setup step in the artifact ever puts a real annotation there, since `setup` is CLI invocations only and nothing in the pre-T2e schema could express "write this file first." Result: **zero non-empty stderr lines in both modes**, live-confirmed, the identical vacuous-truth failure mode T2d diagnosed for `status`, recurring one layer down for `scan` once the file that would trigger its debug-level span event is missing. Also directly re-checked the two things reviewer specifically asked me to diff: `-vv` placed *after* "scan" (rather than before, as my original T2d test happened to run it) parses identically, confirmed live, no argument-order bug; and `G8_TRACE_JSON` is exactly the variable `main.rs:115` reads, no invented/misspelled var. Both were fine; the actual gap was one level upstream of either. **Fix:** added `seed_files` (§1.2, new), writes a real one-line `// @g8.capability(...)` annotation into the fixture before `setup[0]` runs. Re-ran the full corrected sequence live: **exactly 1 non-empty stderr line per mode, 100% valid JSON with `G8_TRACE_JSON=1`, 0% without.** Passes.

**(2) OBL-D4-01: the count and content assertions were already right; the query's path representation was the only real bug.** Built the actual two-level `AGENTS.md` fixture T2d's row only ever described in prose (root `# Boundaries` / `Root-level boundary text.`, nested `src/auth/AGENTS.md` with `# Boundaries` / `Auth-specific boundary text.`) and ran `init` + `scan --output json` against it for real: **`inserted_intents: 2`**, exactly T4d's encoded value, no change needed there. The break is in the query step: direct SQLite inspection of the resulting store shows `intent.scope_path` is stored as an **absolute filesystem path** (`/private/tmp/.../t2e-d401-fixture/src/auth`, not `src/auth`), and `list_intents_at_path`'s SQL (`?2 LIKE scope_path || '%'`, `store_impl.rs:1451`) requires the query argument in that same representation, a relative `--path=src/auth` structurally cannot match, returning `data: []` regardless of how correct the fixture and the other assertions are. Re-ran the query with the fixture's real absolute path substituted by hand: **`data[0]` is the auth intent** (`scope_depth: 11`, "Auth-specific boundary text."), **`data[1]` is the root one** (`scope_depth: 9`), `ORDER BY scope_depth DESC` puts the deeper, more-specific match first, exactly what T4d's `data[0].description` assertion already expected. **Fix:** `seed_files` for the two `AGENTS.md` files (same new primitive as P6-01) plus the new `{{fixture_dir}}` substitution (§1.2) so the query step can reference the fixture's own runtime-only absolute path portably. Passes with both substituted live.

**(3) OBL-D4-02: my T2d diagnosis was wrong, full stop, re-diagnosed from scratch.** The reviewer reported the exact encoded setup (`[init, status]`, present since T4c) still fails, contradicting my T2d "missing `init`" theory. Re-ran that *exact* sequence directly: it works, `.g8/INTENT_SUMMARY.md` exists, and to a human eye starts with `<!-- AUTO-GENERATED`. So why does the strict assertion fail? Checked the literal `expected_prefix` value against the literal real file content with `str.startswith()`, not an eyeball read: `"<!-- AUTO-GENERATED by g8 v0.1.0 at 2026-07-02T...Z -->".startswith("<!-- AUTO-GENERATED -->")` → **`False`**. The real header has a variable `by g8 v0.1.0 at {timestamp}` segment between "GENERATED" and the closing `-->` (`status.rs:144-146`); the encoded `expected_prefix` closes the comment immediately, with no such gap, so it is never actually a prefix of the real string, a pure literal-string bug in the assertion value, unrelated to the setup sequence, which was correct all along. **Fix:** truncate `expected_prefix` to `"<!-- AUTO-GENERATED"` (drop the closing `-->`, since nothing after "GENERATED" is constant). Re-checked: `.startswith("<!-- AUTO-GENERATED")` → **`True`**. Passes. My T2d ruling is superseded by this one, not merely refined, the diagnosis, not just the fix, was wrong.

**(4) OBL-D11-02 / `tempfile`, for the record, no fix, a real open scoping question.** *(Resolved by Erratum 4 / §2.6: dev-deps ruled OUT of deny-list scope.)* Confirmed directly: `crates/g8-store/Cargo.toml` lists `tempfile` under `[dev-dependencies]` only, not `[dependencies]`. Confirmed directly: `exec/cargo_metadata.rs`'s `no_dep` walks `cargo metadata`'s resolved `resolve` graph with no dependency-kind filtering anywhere in it, it does not distinguish `dev`/`build`/`normal` dependency edges, matching the *original* obligation's own literal convergence-test text (`cargo metadata ... | jq '.packages[].dependencies[].name'`, which is likewise kind-blind). This means `tempfile`, and anything it transitively pulls (commonly `fastrand`/`getrandom`-shaped crates, for generating unique temp names), is walked and matched against the deny-list exactly as if it were a production dependency, even though dev-dependencies never ship in the release binary and don't affect any end-user-observed determinism. **This is a real, load-bearing scoping question, not a bug**: is "the deterministic zone" (P3/D11's actual concern, per SPEC.md) about the *production build* specifically, or literally *anything reachable from any `cargo metadata` invocation regardless of purpose*? Under the current (unfiltered) semantics, D11-02 can structurally never reach a clean PASS post-Q4-fix if any workspace crate's dev-dependencies transitively touch a denied name, worth surfacing to whoever rules on Q4, since it changes what "fixed" even means for this obligation. Not resolved here; §2 row 19's mapping is unchanged.

### 2.6 Erratum 4: the Q-rulings implementation pass (2026-07-02)

Executed against Mike's rulings relayed via the independent post-board review (whose findings drove the sequencing: *"fix D11-04's checker/artifact first, then decide D11-02's ID and dev-dep scope; D3 and D8 are real code/spec work"*). Every claim below was verified by running the real binary, per the empirical-only rule T2e established.

**(1) D11-04, the noise adjudication.** The reviewer's finding, confirmed by direct read of both artifact and checker: the encoded args ran `[init, scan]` against fresh *empty* scratch dirs (no seed content, nothing in `ByteDiffArgs` could express it), and compared `check --json` output whose `obligations_note` embedded each branch's absolute artifact-discovery path, so the recorded FAIL diverged first at a tempdir path, not at any ordering or ID byte. Meanwhile the obligation's own `convergence_test.check` prose (binding, never edited, quoted in full in the Erratum-4 log entry) specifies the same-command-twice-against-one-populated-store shape. Ruling: this contract's §2 row 21 and §9's "deliberately mint-SENSITIVE" paragraph are **superseded**, the mint-sensitivity story belonged to Q4/D11-02 (where it is now resolved at the root by content-derived IDs), not to D11-04's read-path-ordering test. `ByteDiffTwice` reimplemented as: one scratch fixture → `seed_files` (new, `#[serde(default)]`, additive) → `setup` once → `compare` twice, run 1's process exiting before run 2 spawns → byte-diff, masking rules unchanged. Artifact re-encoded: two real capability annotations seeded (`>=2`-element store per the prose), compare = `query capabilities --output json`, `normalize_paths: []`. **Verified live: 2 elements, two reads byte-identical with zero normalization.** Product-side companion fix: `obligations_note` names `specs/obligations-v0.1.json` relative to the project root, never absolute, restoring §9 point 1's (previously falsified) path-hygiene claim for every future consumer.

**(2) D11-02, Q4 ruled: content-derived IDs; dev-deps out of scope.** Product side: `g8_core::ids` derives every ID as `SHA-256(kind ␟ part₁ ␟ …)` mapped to 21 chars of the same URL-safe alphabet nanoid used; `new()`/`Default` deleted (compile-driven conversion of every mint site: spaces/projects from root-path+name, plans from space+title+existing-count, scan-synced rows from project/space+name/heading/title+file+line, location keeps same-name duplicates distinct, and rescans of an unchanged tree now mint byte-identical rows); tests use a `#[doc(hidden)]` deterministic-counter constructor; serve keeps explicit nanoid randomness (Q4 rider). nanoid removed from `g8-core` and `g8-store` (the store's dependency was dead, zero source references). Checker side: `CargoMetadataNoDep` parses `resolve.nodes[].deps[].dep_kinds` and walks normal+build edges only, disclosing `dependency_kinds_in_scope: ["normal","build"]` in every evidence detail; two pinned regression tests (dev-only `tempfile` no longer matches; the full rand-family list passes across all six crates). **Verified independently of the checker: `cargo tree -i getrandom` reaches workspace crates only via `tempfile [dev-dependencies]`; `cargo tree -i nanoid` matches nothing in the default graph; the serve feature builds and retains nanoid.**

**(3) D3-02 / D8-01, the two live-drift code fixes.** Checker rows unchanged (both adjudications, §2.2, §2.3, stand); the code moved to the SPEC. `space add` writes/updates `.g8/space.toml` via a `toml::Value` round-trip (unmodeled sections survive; member `root` recorded as given, relative stays relative; `space remove` keeps the file in sync), verified live against the exact encoded fixture. The `g8-conflict -> g8-store` edge exists as a normal Cargo dependency: `g8`'s `ConflictAdapter` moved to `g8_conflict::store_adapter`, still generic over `StoreConnection`, `RusqliteStore` is named nowhere in `g8-conflict/src/**` (the concrete binding lives in `tests/`, outside D5-01's glob, which is scope-honest: the adapter under test is the generic one).

**(4) New finding for the pile: D11-01 false-passes.** `rg 'std::time::SystemTime::now\(\)'` cannot match `time.rs:45`'s import-form `SystemTime::now()`, a checker-fidelity hole of exactly the D6-01 class, found while converting mint sites. Disclosed, not patched: wall-clock timestamps are load-bearing product surface (staleness detection), so whether the fix is pattern-widening-plus-exemption or obligation-scope amendment is Mike's ruling to make. Until ruled, D11-01's PASS overstates what was checked; its evidence cannot yet say so because the pattern itself is the blind spot.

**Post-pass state (real-artifact end-to-end, both enforcement modes): 27 passed / 0 failed / 0 error / 0 unknown.** For the record: all four former FAILs turned green by making the *code or the checker honest*, never by weakening an assertion, D3-02/D8-01 by code moving to SPEC, D11-02 by removing the violation and ruling the scope, D11-04 by making the checker measure what its obligation actually says.

---

## 3. Status / trust semantics

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus { Passed, Failed, Error, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel { Verified, Checked, Asserted }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationResult {
    pub id: String,
    pub status: ObligationStatus,
    /// `None` **iff** `status == Unknown`. Every other status has a trust.
    pub trust: Option<TrustLevel>,
    pub evidence: Evidence,
}

/// Rolled-up, top-level view. `backend`/`trust`-tier here are the ones that
/// determined the OVERALL `ObligationResult` per the §3.3 aggregation rule
/// (i.e. the erroring/failing sub-check if any, else the highest-trust one).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub backend: CheckerBackendName,   // the enum tag, e.g. "cargo_metadata_no_dep"
    pub summary: String,               // one-line human-readable outcome
    pub detail: serde_json::Value,     // backend-specific structured detail
    pub duration_ms: u64,
    /// One entry per element of the obligation's `checks: Vec<CheckerBackend>`
    /// (§6), full transparency for composite/multi-check obligations, even
    /// though `status`/`trust` above are already the aggregated view.
    pub checks_run: Vec<SubCheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCheckResult {
    pub backend: CheckerBackendName,
    pub status: ObligationStatus,      // this one sub-check's own outcome
    pub detail: serde_json::Value,
    pub duration_ms: u64,
}

/// The `#[serde(tag = "backend", ...)]` string from `CheckerBackend` (§1.1),
/// carried alone here since `Evidence`/`SubCheckResult` report which backend
/// ran without needing to repeat its full args.
pub type CheckerBackendName = String;
```

### 3.1 Status, what happened this run

- **Passed**, the checker(s) executed to completion and every sub-check's actual outcome matched its expected outcome.
- **Failed**, the checker(s) executed to completion and at least one sub-check's actual outcome did **not** match (a genuine, actionable constraint violation), or, for an attested obligation, the attestation record itself declares a failure. `passed`/`failed` is a claim about the *code*.
- **Error**, the checker(s) could **not** reach a conclusive result: the `ast-grep`/`rg`/`cargo` binary is missing, a subprocess crashed, a fixture directory couldn't be created, args referenced a nonexistent file/glob, or execution timed out. `error` is a claim about the *checker*, not the code, an obligation must never silently read as "passed" because its checker happened to crash.
- **Unknown**, no checker is configured for this obligation (mapping = `prose_only`) and no non-stale attestation exists. Per §2's tally this applies to none of the 27 today, but the runner must produce it gracefully, never panic, for any future obligation added without a `checker` field.

### 3.2 Trust, how much the status is worth

Trust is a **static** property of *how* an obligation is evaluated (fixed by its mapping-table entry), not a runtime measurement:

| Backend family | Trust tier |
|---|---|
| `CargoMetadataNoDep`, `CargoMetadataDepGraph`, `AstGrepNoMatch`, `AstGrepMatchCount`, `RgMatchCount`, `RustEnumShape`, `BuiltinAlgorithm` | **Checked**, static/structural inspection of source or build metadata; no real runtime behavior exercised. |
| `FixtureIntegrationTest`, `ByteDiffTwice`, `G8CheckContract` | **Verified**, the backend actually runs real `g8` code (subprocess or in-process) against a constructed fixture and observes real behavior. |
| Attestation record (§7), no `checker` configured | **Asserted**, a human or agent claims the obligation holds, with recorded evidence, but no checker reproduced it *this run*. |
| No checker, no attestation | **N/A** (`status = Unknown`, `trust = None`) | |

If an obligation's `checks` array (§6) mixes tiers, overall trust is the **highest** tier present among sub-checks that did not `Error` (Verified > Checked), verified evidence subsumes a checked claim about the same code. `advisory: true` on an obligation's `signal` (D6-02, G1-01) is an orthogonal axis, it changes exit-code wiring (§5), never trust.

### 3.3 Aggregation rule for multi-check obligations

Given `checks: Vec<CheckerBackend>` (one obligation's full list of configured checkers, §6) with N ≥ 1 entries:

```
status = Error   if any sub-check returned Error   (broken evidence taints the whole claim)
       else Failed  if any sub-check returned Failed
       else Passed  (all sub-checks passed)

trust  = max(tier) over sub-checks that did NOT error
         (falls back to the nominal tier of the highest-tier configured
          backend if literally every sub-check errored, still meaningful
          for filtering even though status = Error)
```

---

## 4. Crate placement

**Recommendation confirmed: new lib crate `crates/g8-obligations`.** Deterministic zone (no LLM, no network, no unseeded randomness, subprocess calls to `cargo`/`ast-grep`/`rg` are the same class of dependency `g8-extractor` already has on `ast-grep`, per SPEC Decision 11 / ARCH §3.2). DAG position:

```
g8
  -> g8-obligations, g8-planner, g8-conflict, g8-store, g8-extractor
       -> g8-core
g8-obligations -> g8-core, g8-store   (NOT g8, NOT g8-planner/-conflict/-extractor)
```

`g8-obligations` depends on `g8-core` (shared types, including the relocated check-contract types, §8) and `g8-store` (needed by `G8CheckContract` and `FixtureIntegrationTest`'s in-process branches to build a real `RusqliteStore::open_in_memory()`). It does **not** depend on `g8`, that edge already exists in the other direction (`g8` will depend on `g8-obligations` to wire the runner into `check.rs`) and a reverse edge would be a literal cycle. It does not need `g8-planner`/`g8-conflict`/`g8-extractor` as Cargo dependencies: backends that need multi-command CLI behavior (`FixtureIntegrationTest`, `ByteDiffTwice`) reach it by **subprocess-spawning the compiled `g8` binary**, found at runtime via `std::env::current_exe()`, since this code executes *as part of* the already-running `g8` process, the same subprocess-boundary trick `g8-extractor` uses for `ast-grep`, just spawning itself instead of a third-party tool.

**Testing-strategy note (verified empirically, not assumed):** `g8-obligations`'s own unit tests cannot rely on a `g8` dev-dependency to get `CARGO_BIN_EXE_g8` at compile time, I built a 2-crate scratch workspace reproducing this exact shape (lib crate dev-depending on the bin crate that depends on it) and confirmed Cargo silently fails to define the `CARGO_BIN_EXE_*` variable in that configuration, even when the bin crate also exposes a lib target. This is a compile-time concern only (`current_exe()` at *production* runtime is unaffected, it's an OS syscall, not a Cargo mechanism). Recommendation for T3: unit-test each backend's *logic* in isolation inside `g8-obligations` (small scratch fixtures + a real `ast-grep`/`rg` subprocess, no `g8` binary needed); push true "does `g8 check` correctly wire and run obligations end-to-end" testing into `g8`'s own test suite (`crates/g8/tests/cli.rs`, T5/T3v's natural home), where `env!("CARGO_BIN_EXE_g8")` works natively because it's the same package building its own binary.

### 4.1 Public API surface

```rust
// crates/g8-obligations/src/lib.rs

pub struct ObligationArtifact { /* deserialized specs/obligations-v0.1.json,
                                    including the new `checker` field (§6) */ }

pub fn load_artifact(path: &Path) -> Result<ObligationArtifact, ObligationsError>;

/// The one entry point. Never panics; a single backend crashing surfaces as
/// that one obligation's `status = Error`, not a propagated panic, every
/// backend invocation is isolated (caught, not just `?`-propagated).
pub fn run_obligations(
    artifact: &ObligationArtifact,
    workspace_root: &Path,
) -> Vec<ObligationResult>;
```

`run_obligations` opens tracing span `obligations.run_all` (fields: `artifact_version`, `obligations_count`, `duration_ms`), per SPEC Principle 6 / the P6-01 span table, this is a **new** crate not covered by ARCHITECTURE.md §12's existing 9-span inventory; that table needs a 10th row. No task in the current plan has ARCHITECTURE.md in its writable scope (T5 is explicitly limited to "CLAUDE.md output-contract section only"), flagging this as an open gap for Mike/reviewer rather than silently leaving it inconsistent or silently expanding my own writable scope to fix it.

---

## 5. `g8 check --json` contract extension

Backward compatible: `g8_version`, `errors`, `exit_code`, `enforcement_disabled` are **unchanged in shape and unchanged in when they appear**. One new top-level key.

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
  "exit_code": 0,
  "enforcement_disabled": true,
  "obligations": [
    {
      "id": "OBL-P9-01",
      "status": "passed",
      "trust": "checked",
      "evidence": {
        "backend": "cargo_metadata_no_dep",
        "summary": "no denied crates found in resolved dependency graph",
        "duration_ms": 42,
        "detail": { "denied_checked": ["surrealdb", "graph-tool"] },
        "checks_run": [
          {
            "backend": "cargo_metadata_no_dep",
            "status": "passed",
            "detail": { "denied_checked": ["surrealdb", "graph-tool"] },
            "duration_ms": 42
          }
        ]
      }
    },
    {
      "id": "OBL-D11-02",
      "status": "failed",
      "trust": "checked",
      "evidence": {
        "backend": "cargo_metadata_no_dep",
        "summary": "denied crate 'getrandom' reachable via nanoid in g8-core, g8-store",
        "duration_ms": 51,
        "detail": { "matched": ["getrandom", "rand_core"], "via": "nanoid" },
        "checks_run": [
          {
            "backend": "cargo_metadata_no_dep",
            "status": "failed",
            "detail": { "matched": ["getrandom", "rand_core"], "via": "nanoid" },
            "duration_ms": 51
          }
        ]
      }
    }
  ]
}
```

`errors[]` above is shown as the existing `PairingError` JSON shape, byte-for-byte unchanged from today's `g8 check --json`.

`errors` and `exit_code` keep their **existing** enforcement-gated behavior exactly as today (T5 acceptance criterion: "existing 4 pairing checks unchanged"). `obligations` is populated **unconditionally**, including when `enforcement_disabled: true`, this is the literal ruling: *"audit checks run even when enforcement is off; findings always emitted in JSON; enforcement affects EXIT CODE only."* This asymmetry (one array enforcement-gated, one always-on) is intentional, not an oversight, it is what makes `g8 check` usable as an AUDIT tool before flipping enforcement on (Decision 9), now extended to the 27 compiled obligations, not just the 4 pairing conditions.

### 5.1 Exit-code rule

```
when enforcement is OFF:
    exit_code = 0                         // unchanged from today
    enforcement_disabled = true           // unchanged from today
    obligations[] fully populated with real statuses  // NEW

when enforcement is ON:
    exit_code = 1  if  errors.len() > 0                                    // unchanged
                   OR  exists o in obligations where
                         o.status in {Failed, Error}
                         AND artifact[o.id].signal.advisory != true        // NEW
                = 0  otherwise
    (exit_code = 2 remains reserved for the CLI's own internal failures , 
     store unreachable, malformed config, unrelated to any one obligation)
```

`advisory: true` obligations (currently D6-02, G1-01) never affect exit code, matching their existing `signal.advisory` marking, they can fail loudly in the `obligations` array without blocking CI. `status = Unknown` never affects exit code either (a documentation gap should not block a gate that hasn't been configured yet).

---

## 6. Artifact evolution: inline `checker` field

**Recommendation: inline, sibling to `convergence_test`, inside each obligation object in `specs/obligations-v0.1.json`.** Not a sidecar file for the checker mapping itself (sidecar chosen instead for *attestations*, §7, different tradeoff, see there).

Rationale: the artifact already mixes a semi-structured `rule` field (provenance/intent) with prose fields (`convergence_test.check`, `descends_from.why`) in one object per obligation, this is the established pattern (CLAUDE.md's own annotation grammar shows structured and prose fields coexisting for different purposes). Inlining `checker` keeps the prose check, the compiled `rule`, and the now-executable `checker` co-located, so a reviewer auditing whether a typed checker *faithfully represents* its prose intent (exactly the audit Mike's rulings are about) reads one object, not two files kept in sync by convention. A sidecar keyed by obligation ID would require T4 and any future editor to maintain referential integrity across two files with no schema-enforced link, inline, the link is structural.

**Exact shape added per obligation (new key, siblings unchanged):**

```json
{
  "id": "OBL-P9-01",
  "descends_from": { "...": "... UNTOUCHED ..." },
  "also_stated_by": ["..."],
  "rule": { "...": "... UNTOUCHED ..." },
  "scope": { "...": "... UNTOUCHED ..." },
  "convergence_test": {
    "id": "conv-p9-01",
    "check": "cargo metadata --format-version1 | jq '.packages[].dependencies[].name'; assert 'surrealdb' and 'graph-tool' are absent...",
    "fails_when": "... UNTOUCHED, this is documentation, not the executable spec ..."
  },
  "checker": {
    "mode": "typed",
    "checks": [
      {
        "backend": "cargo_metadata_no_dep",
        "args": {
          "denied": [{"kind": "exact", "value": "surrealdb"}, {"kind": "exact", "value": "graph-tool"}],
          "scope_crates": [],
          "build_config": "default"
        }
      }
    ]
  },
  "signal": { "...": "... UNTOUCHED ..." }
}
```

Rust-side, `checker` deserializes to a closed, mutually-exclusive enum, this is what makes the `mode` tag in the JSON below meaningful rather than three independently-optional fields that could contradict each other:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ObligationChecker {
    Typed { checks: Vec<CheckerBackend> },       // §2's mapping, all 27, today
    Attestation { attestation_id: String },       // unused today, §7
    ProseOnly,                                    // unused today
}
```

**Rule for T4 (binding):** `convergence_test.check` and `convergence_test.fails_when` are **never edited** when adding `checker`. They remain the human-readable prose specification the `checker` field is a compiled, executable translation *of*, exactly parallel to how `rule` is already a compiled translation of `descends_from.why`. If a future obligation's prose and its `checker` visibly diverge (as happened with N4-01/N5-01's stale sub-checks, §2.1), the prose stays as the historical record and the divergence gets called out in a `checker`-adjacent note, not resolved by silently rewriting the prose. Obligations with no typed checker in v1 (none today) get `"checker": {"mode": "prose_only"}` or `"checker": {"mode": "attestation", "attestation_id": "..."}` instead of a `checks` array, the `mode` tag makes the three states (`typed` implied by presence of `checks`, `attestation`, `prose_only`) mutually exclusive and machine-readable without a fourth file.

---

## 7. Attestation record format

Unused by the v1 mapping (§2's tally is 27/0/0) but fully specified: it is the load-bearing fallback the moment T3 finds one of the "typed-but-approximated" obligations (§10) genuinely infeasible, and it is what T6's plan text explicitly names as one of the two resolution paths.

**Storage: sidecar file `specs/attestations-v0.1.json`**, not a store table. Justification: the obligations artifact this attests *about* already lives in `specs/`, not in any project's `.g8/store.db`, these are constraints on *g8's own repository*, evaluated by `g8 check` running against g8's own source tree, which may not even have `.g8/` initialized yet (the baseline audit hit exactly this: OBL-D10-01 was `BLOCKED` because no `.g8/store.db` existed). A sidecar avoids that bootstrapping dependency entirely, stays git-diffable/human-auditable (matches "local audit engine" posture), and is append-friendly by construction (a flat JSON array of records, new entries are additive, matching the instruction to design evidence records as append-friendly without building any identity/lease/distributed machinery now).

```json
{
  "g8_version": "0.1.0",
  "attestations": [
    {
      "id": "ATT-2026-07-02-001",
      "obligation_id": "OBL-N6-01",
      "attested_by": { "kind": "agent", "name": "prose-handler" },
      "attested_at": "2026-07-02T00:00:00Z",
      "claim": "passed",
      "evidence_summary": "manually traced scan.rs's call graph; no plan-status-mutating call reachable outside plan_promote as of this commit",
      "pinned_content_hash": "sha256:<hash of the exact file(s) inspected, e.g. crates/g8/src/cmd/scan.rs>",
      "expires_policy": "stale_on_hash_mismatch"
    }
  ]
}
```

**Consumption:** the runner (`run_obligations`) checks, for any obligation whose `checker.mode == "attestation"`, whether `specs/attestations-v0.1.json` has a record with matching `obligation_id`; if found and not stale, `status = attestation.claim`, `trust = Asserted`, `evidence = { backend: "attestation", summary: attestation.evidence_summary, detail: { attested_by, attested_at, attestation_id } }`. If no matching record, `status = Unknown`, `trust = None`.

**Staleness policy: content-hash pinning**, per the explicit instruction to propose something checkable rather than a time-based expiry (which would be an arbitrary, un-auditable number). Each attestation pins the SHA-256 of the exact file(s) it inspected (`pinned_content_hash`). At runtime, the checker re-hashes those same paths (relative to `workspace_root`); a mismatch means the attested-about file changed since the human/agent looked at it, and the attestation is stale: `status = Unknown` (not silently `Passed`, and not `Failed` either, a stale attestation is a documentation gap, not a proven violation) with `evidence.summary` explaining the hash mismatch so a human knows re-attestation is needed. This is cheap (one hash comparison, already the same primitive N7-01's own checker uses), needs no server/clock, and degrades safely, an attestation can only ever go stale toward `Unknown`, never silently keep asserting `Passed` against changed code.

---

## 8. `g8_check_contract` recursion hazard, resolution

**The hazard:** OBL-D10-01 validates the *shape* of `g8 check --json`'s own output. If its checker were implemented as "shell out to `g8 check --json` again," that nested invocation would itself run the obligations runner, which includes OBL-D10-01 again, at minimum one wasteful, fragile extra subprocess per invocation; at worst (if the nested invocation targets the same store file) a self-lock, and in every case a conceptually wrong "the thing under test invokes itself to test itself."

**Resolution, mandatory in-process design, no subprocess spawn for this one backend:**

1. Move the `CheckPayload` / `CheckError` / reason-vocabulary type definitions that currently live in `crates/g8/src/render/json.rs` into `g8-core` (e.g. a new `g8_core::contract` module: `CheckPayload`, `CheckErrorItem`, `PairingReason`, plus the `obligations` array's own `ObligationResult`/`Evidence` types from §3, which already need to live somewhere both `g8-obligations` and `g8` can see). `g8`'s `render/json.rs` becomes a thin wrapper: construct `g8_core::contract::CheckPayload`, `println!("{}", serde_json::to_string_pretty(&payload)?)`. This is a small, precisely-scoped refactor for T5 (already in T5's writable scope: `crates/g8/src/render/`), not something I am doing here.
2. `G8CheckContract`'s implementation constructs **fixture instances** of `g8_core::contract::CheckPayload` directly in Rust, for `CleanStore`: an empty `errors: vec![]`; for `KnownBadCapabilityFixture`: build a real `RusqliteStore::open_in_memory()` (an allowed dependency, §4), insert one hand-crafted `Capability` with no matching `ConvergenceTest`, call the real `StoreConnection::pairing_check` on it to get a real `PairingError`, then construct `CheckPayload` from it, then asserts the resulting `serde_json::to_value(&payload)` has the locked shape (required keys present with correct types, `errors[].reason` drawn from the closed vocabulary, `exit_code` consistent with `errors.is_empty()`).
3. Because both the *real* `g8 check` invocation and the *obligation checker* construct the identical `g8_core::contract::CheckPayload` Rust type and serialize it the identical way, they cannot silently drift apart the way two independently-hand-written JSON shapes could, the obligation-checker is validating the type's own `Serialize` impl and the assembly logic around it, not re-implementing a parallel, potentially-stale copy of the contract.
4. **Nothing about this backend ever calls `std::process::Command::new(current_exe())`.** This is the one backend in the entire enum for which subprocess spawning is explicitly forbidden by this contract, precisely because it is the one obligation whose subject matter *is* `g8 check`'s own output.

As a secondary, defense-in-depth property (not the primary safety mechanism, since it is more fragile): any backend that *does* legitimately subprocess-spawn `g8` against a scratch fixture (`FixtureIntegrationTest`, `ByteDiffTwice`) targets a fixture directory that has no `specs/obligations-v0.1.json` of its own, `run_obligations` must therefore treat "artifact file not found at the target root" as a normal, empty `obligations: []` result, never an error, so even if a future backend accidentally re-invoked `g8 check` against an isolated fixture, that nested call would find nothing to recurse on. This does not excuse `G8CheckContract` from the in-process rule above; it only prevents the *other* subprocess-based backends from becoming a second, accidental instance of the same hazard.

---

## 9. `byte_diff_twice` determinism boundary

**What is controlled, precisely, so the double-run comparison is meaningful:**

1. **Temp/fixture paths.** The comparison command (`compare`) always runs against a fixture directory that is either (a) built exactly once and read twice (`setup` empty, see below), or (b) built independently per branch in two *separate* scratch tempdirs created via the same mechanism `tempfile` already gives the workspace (a test-time dependency per root `Cargo.toml`). Either way, the two branches' filesystem paths are never smuggled into the compared output, none of `errors[]`/`obligations[]`'s fields are path-shaped in a way that would embed a tempdir's random suffix (`file` fields are workspace-relative, not absolute, consistent with `meta.determinism_note`'s existing "all paths repo-relative" rule elsewhere in the artifact).
2. **Env vars.** The controlled base environment for both runs is fixed and identical: `G8_TRACE_JSON` unset (tracing goes to stderr only regardless, but pinned for cleanliness), `TZ=UTC`, `LANG=C`, `NO_COLOR=1`. `CliInvocation.env` (§1.2) may only *add* to this base, never leave it implicit, both branches get the literal same base map.
3. **Timestamps.** `g8 check --json`'s payload carries no wall-clock "checked at" field by design (§5's shape has none), the only inherently-volatile numeric field anywhere in scope is `duration_ms` (already present in tracing spans and in `Evidence.duration_ms`, §3). **`normalize_paths` is a narrow, explicit allowlist restricted to timing fields** (`ByteDiffArgs.normalize_paths`, e.g. `"$..duration_ms"`), masked to a fixed placeholder before comparison. This is the *only* category of field ever normalized.
4. **Array ordering is never normalized.** This is the entire point of the backend (it exists to catch exactly the class of bug OBL-D11-04 targets, `HashMap`/`HashSet` iteration-order non-determinism, per that obligation's own `rule.params`). Sorting arrays before comparison, or masking anything beyond the timing allowlist, would silently defeat the check it is supposed to be. Forbidden.
5. **Sequencing.** Run 1 completes fully (process exits) before run 2 starts, no concurrency, no filesystem-watch race (`--no-watch` implied/forced for any `scan` in `setup`).

**Mint-safe vs. mint-sensitive, SUPERSEDED by Erratum 4/§2.6.** The paragraph below is retained as the historical record of a misreading: D11-04's own prose specifies the same-store-read-twice shape (what this section called "mint-safe"), and the "deliberately mint-sensitive" choice compared two different inputs, failing on environment noise. Mint determinism is now guaranteed at the root by Q4's content-derived IDs, not tested by this backend. Original text:

**Mint-safe vs. mint-sensitive, and why D11-04 is deliberately the sensitive one:** `ByteDiffArgs.setup` (§1.2) is the knob. Empty `setup` = both branches share one pre-built, already-populated fixture store, queried twice, reading an already-persisted nanoid string twice returns the identical string both times (nanoid is only non-deterministic at *mint* time, not at *read* time), so this shape is **immune** to the nanoid issue and would pass today. Non-empty `setup` = each branch independently re-runs `init`+`scan` (or whatever the obligation specifies) in its own fresh fixture, which mints fresh IDs each time, this shape is **exactly** what "byte-identical **store**" (D11-04's own name) means, and it is deliberately mapped this way in §2 row 21 (independent `setup` per branch), matching the decision-gate doc's explicit claim that "random ID minting... dooms OBL-D11-04's byte-identical-store test." This is the honest, non-papered-over FAIL the task brief calls for: the backend is fully specified and will run correctly the moment Q4 is ruled and IDs become content-derived (or a store-seeded counter), until then, `run_obligations` reports `OBL-D11-04: status=failed, trust=verified, evidence.detail` naming the specific byte offset/field where the two branches diverge (nanoid-generated ID columns), not a vague "differs."

---

## 10. Known approximations and disclosed gaps

In the spirit of this artifact's own culture of disclosure (`known_limitation`, `tokio_note`, `known_live_violation` fields already present throughout `obligations-v0.1.json`), every place in §2 where a typed backend is an honest **approximation** of the obligation's literal prose, rather than a logically-complete verification of it:

- **P5-01**, column-name regex is a heuristic (`developer_id`/`dev_id`/`agent_id`/`user_id`); it cannot prove no per-developer scoping exists under an unanticipated column name. Narrower than "prove semantically," matches what a grep-based check can honestly claim.
- **P6-01(a)**, `AtLeast(30)` instrumented-span threshold approximates "every non-exempt `pub fn` is instrumented"; it does not per-function-correlate exemptions (open/open_in_memory/migrate). A precise per-fn `AstGrepMatchCount` variant is a plausible v2 refinement, not required for v1.
- **D11-01**, the "async fn combined with a network import" clause is not independently checked; it is asserted to be logically subsumed by D11-02's Cargo-level deny-list (no network crate dependency ⇒ no code path can reach one). This is a documented narrowing of scope, not a silent drop.
- **N4-01, N5-01**, the `Command::Serve` and `tiny_http::Server::$METHOD` sub-checks are dropped from the active mapping (§2.1); both are structurally guaranteed to match now that `serve` is a legitimate, cfg-gated feature, making them permanent false-positive generators rather than useful signal.
- **N6-01**, direct literal-call detection only, scoped to `scan.rs`; a transitive call (e.g. `scan.rs` calling a helper elsewhere that itself calls `update_plan_status`) is not caught. Same class of disclosed gap as this artifact's own DEF-15 (vendored/indirect code is undetectable by construction).
- **D11-02 / D11-04**, *(originally: both honestly FAIL pending Q4)*, resolved by Erratum 4/§2.6: Q4 ruled and executed (content-derived IDs, dev-deps out of scope), D11-04 re-encoded to its own prose's same-store shape. Both pass for real. D11-04's residual disclosed gap: it covers the `query` render path; the `plan`/FitReport composite-query render path is a plausible v2 second sub-check.
- **G1-01**, OQ-10 (what counts as "new" since a baseline, for diffing) is unresolved. v1 default: check **all current** type/struct/enum names in `g8-core` against the canonical-term predicate on every run, not just ones introduced since some commit/baseline, accepting mild redundant-but-harmless re-checking of already-approved names (the `allowlist` absorbs known-good collisions) rather than inventing a baseline-tracking mechanism this repo (not under version control) can't currently support cheaply.

None of these gaps are hidden inside a `Passed` result, each surfaces in `evidence.summary`/`evidence.detail` at runtime so a human reading `g8 check --json`'s `obligations` array sees the caveat, not just a green checkmark.

---

## 11. Handoff summary

- **T3** (backends-builder): implement `crates/g8-obligations` against §1 (types) and §4 (crate placement/API/testing strategy). The 10-backend enum is closed, if a backend genuinely doesn't fit, escalate rather than adding a shell-string escape hatch. **Amendment 1 (T2b):** `BinarySource`/`CargoRun` and the two `StdoutContains*` assertions are new since your first read of §1, additive only, nothing else in §1 changed shape.
- **T4** (artifact-annotator): apply §2's table verbatim as the `checker` field per §6's exact shape; leave `convergence_test`/`rule` untouched; use `"checker": {"mode": "prose_only"}` / `{"mode": "attestation", ...}` for anything this contract didn't resolve (none today, but the shape must round-trip cleanly if T6 later needs it). **Amendment 1 (T2b):** re-encode OBL-D7-01 (row 14) and OBL-D9-01 (row 16, now 4 independent `checks` entries) per their amended rows; OBL-D3-02 (row 8) is unchanged from your original encoding, the drift is in the code, not your annotation. **T4d (post-T6, Erratum 3 / §2.4):** six rows need re-encoding, P6-01 (row 2, sub-check c), D4-01 (row 9), D4-02 (row 10), D6-01 (row 12, sub-check b pattern only), D6-02 (row 13), G1-01 (row 27, allowlist). **T4f (post-T7, §2.5):** three of those rows need re-encoding *again* with real, empirically-verified fixes, P6-01/D4-01 both need the new `seed_files` field populated (exact content given in their rows), D4-01's query step needs `{{fixture_dir}}` substituted into its `--path=` argument, D4-02 needs only its `expected_prefix` string corrected (setup unchanged).
- **T5** (check-integrator): wire `run_obligations` into `check.rs` per §5's exit-code rule; execute the `g8-core::contract` type relocation described in §8 as part of this work (small, in T5's existing writable scope); delete `invalid_root` per the existing plan (orthogonal to this contract). Budget for D7-01's `CargoRun` cost (§2.1) when this lands in any fast/hot-path invocation of `g8 check` (pre-commit hook, tight local loop), not a blocker, just don't be surprised by it.
- **T6** (prose-handler): §7's attestation mechanism is ready to use if any of §10's approximations prove infeasible in practice; §2.1's cross-cutting finding (three stale checks) and §10's gap list are the first places to look if T3 needs backends re-scoped. §2.2 (OBL-D3-02) is a live-drift finding for reviewer/Mike, not something for you to resolve by weakening a checker.

---

## 12. Addendum: backend #11 `receipt_query` and `action_receipt` wiring (o-g8-receipts-20260721)

**Status:** BINDING, implemented. 
This addendum is the one binding ruling §1.1 anticipated ("if an obligation
genuinely doesn't fit one of these [ten], the answer is to escalate for a
new binding ruling"), it adds exactly one structural variant. The ten-backend
enum described in §1 is otherwise unchanged; every existing obligation's
mapping, every status/trust rule in §3, and the repo-mode floor code in §5.1
are untouched.

### 12.1 Wiring: `action_receipt`

`Signal.wiring` (§6; `crates/g8-obligations/src/artifact.rs`) was a free
string no code read before this addendum. `"action_receipt"` is now its
first real consumer, declaring an obligation's subject is a *parsed action
receipt*, not the repository, a second choke point alongside `check`'s
existing pre-commit one, this one pre-act (the tool-call boundary).

`crates/g8-obligations/src/lib.rs`'s `run_obligations` gained a third
parameter, `receipt: Option<&ReceiptSubject>`, and a scope-filtering pass
(`run_one_scoped`) ahead of the existing per-obligation dispatch:

| `receipt` | `wiring == "action_receipt"` | Behavior |
|---|---|---|
| `None` (repo mode) | `false` (repo-wired) | Executed exactly as before this addendum, §3/§5.1 floor code is untouched, byte-for-byte. |
| `None` (repo mode) | `true` (receipt-wired) | **Not executed.** Surfaces as `status: "unknown"`, `trust: null`, `evidence.detail.scope: "action_receipt"`. Exempt from `g8`'s enforcement classification (a dedicated check keyed on that `scope` marker, not incidental Unknown-status behavior), a green repo stays green regardless of how many receipt-wired obligations exist. |
| `Some(subject)` (receipt mode) | `true` (receipt-wired) | Executed against `subject`. |
| `Some(subject)` (receipt mode) | `false` (repo-wired) | **Excluded entirely**, not represented in the returned `Vec<ObligationResult>` at all (distinct from the repo-mode case above, which DOES produce a placeholder; here there is nothing to place, repo mode is not this run's choke point). |

**Subject-binding validation** (either direction): a receipt-wired
obligation's `checks` may only contain `receipt_query`; `receipt_query` may
not appear in a repo-wired obligation's `checks`. Checked once per
obligation, before dispatching any of its checks (`subject_binding_violation`
in `lib.rs`). A violation is `status: "error"`, `trust: "checked"`, a claim
about the artifact's own authoring, gates like any other `Error` per the
existing §3.3 aggregation rule (nothing new there: `Error` already taints
the whole result and already gates in both repo mode's and this addendum's
own enforcement classifier).

### 12.2 Backend #11: `receipt_query`, args, ops, and the missing-path table

```rust
// crates/g8-obligations/src/backend.rs
CheckerBackend::ReceiptQuery(ReceiptQueryArgs)
```

Static trust tier: **`Checked`** (§3.2's own table, "static/structural
inspection... no real runtime behavior exercised" is exactly what this
backend does; see §12.3 for why `Checked` still clears the gate in receipt
mode). `name()` → `"receipt_query"`.

```jsonc
{ "backend": "receipt_query",
  "args": {
    "select": "$.actions[*]",
    "where": [{ "path": "$.kind", "op": "eq", "value": "publish" }],
    "aggregate": { "kind": "count" },
    "expected": { "kind": "zero" } } }
```

- **`select`** resolves against the parsed receipt using the exact same
  `$.`-prefixed dot/bracket-index dialect `FixtureIntegrationTest`'s
  `JsonField`/`JsonPathNonEmpty` assertions already use (`resolve_json_path`
  et al., factored out of `exec/fixture.rs` into `exec/json_path.rs`, no
  behavior change, a pure relocation, verified by carrying its existing test
  coverage over unchanged), plus one addition: a trailing `[*]` explodes an
  array at that path into one candidate per element (stripped before handing
  the rest to the shared resolver, the base dialect has no wildcard-index
  concept and never gained one). Without `[*]`, the resolved node (array or
  not) is the sole candidate. A path that does not resolve at all, with or
  without `[*]`, yields zero candidates, never an error. **Disclosed
  extension beyond the base dialect's existing behavior:** if `[*]` is
  present but the resolved node is *not* actually an array (a receipt shaped
  differently than expected), the resolved node itself becomes the sole
  candidate rather than erroring, a deliberately defensive fallback, not a
  silent behavior a reader would have to discover by testing.
- **`where`** (`Vec<WhereClause>`, wire key `"where"`) is ANDed; each clause's
  `path` resolves independently against every candidate. Ops: `eq`, `ne`,
  `matches` (regex on a string candidate value; `value` is the pattern),
  `in`/`not_in` (`value` a JSON array; membership by `serde_json::Value`'s
  own structural equality, arrays/objects compare deep, for free), `exists`,
  `absent`, `gt`/`gte`/`lt`/`lte` (numeric).

  **Missing-path semantics, the fail-closed table, exactly as designed:**

  | Op | Missing path (`resolve_json_path` → `None`) | Rationale |
  |---|---|---|
  | `eq`, `in`, `matches`, `exists`, `gt`, `gte`, `lt`, `lte` | **fails** (`false`) | "positive" ops, a candidate that doesn't have the field cannot satisfy a claim *about* the field. |
  | `ne`, `not_in`, `absent` | **satisfies** (`true`) | "negative" ops, implemented as the exact logical negation of their positive counterpart, which is what makes them resolve `true` on a missing path, not a second rule to keep in sync. Canonical case: "count of actions whose target is `not_in` the allowlist must be zero" has to count a targetless action as outside the allowlist, or an agent could omit the field to slip through. |

  `exists`/`absent` are about path *presence*, not JSON-`null`-ness, a
  candidate carrying `"field": null` counts as present for `exists` (and
  absent's negation, so `false`), the same as any other resolved value.

  **Config vs. candidate-data errors, a disclosed design decision beyond
  what the design spec's own worked examples spell out:** every op except
  `exists`/`absent` requires a `value` in the clause's own args. A *missing*
  `value` (or the wrong JSON type for the op, e.g. `in`/`not_in` with a
  non-array `value`, `matches` with a non-string pattern, a numeric
  comparison with a non-numeric `value`) is a **config** error, this
  obligation's own args are malformed, and is `ObligationStatus::Error`,
  independent of any particular candidate. By contrast, a *candidate*'s
  resolved value being the wrong type for the op (e.g. `matches` against a
  number, a numeric comparison against a string) fails closed (`false`) the
  same as a missing path, extending the missing-path philosophy to
  "present but unusable" rather than erroring on receipt content the
  obligation author doesn't control. This line (config args vs. receipt
  content) is the same line `sum`'s own loud rule (below) draws, just in the
  opposite direction, where the caller's own args are always closed for
  wrongness, and receipt content is fail-closed for a positive claim but
  never blocking for the checker itself.

- **`aggregate`**, `{"kind":"count"}` (candidates surviving `where`) or
  `{"kind":"sum","path":"$..."}` (numeric sum of `path` resolved against each
  surviving candidate). **Sums are loud:** a missing or non-numeric value at
  any candidate that passed `where` is `ObligationStatus::Error`, never
  silently skipped ("a spend cap that silently skips unparseable amounts is
  a hole", verbatim from the design spec). Scoped to the post-`where` set
  only: a candidate `where` already excluded (e.g. a `publish` action with no
  `amount` at all, when the clause is scoped to `kind == spend`) is
  irrelevant to the sum and never trips this rule, only a candidate the sum
  actually needs to add is held to the numeric bar.
- **`expected`**, `zero | exactly | at_least | at_most`, `{"kind", "value":
  f64}` mirroring `CountExpectation`'s shape (§1.2), widened to `f64` since
  `sum` aggregates are not necessarily integral.

### 12.3 CLI: `g8 check --receipt <path>` and the receipt-mode rigor floor

`crates/g8/src/cli.rs`'s `CheckArgs` gained `--receipt <PATH>`.
`crates/g8/src/cmd/check.rs::run` routes to a dedicated
`run_receipt_mode` before the store is even opened, the capability pairing
check is skipped entirely (`errors` is always `[]` in receipt-mode JSON;
different choke point, and "speed matters pre-act" per the design spec).
Ratification is still enforced exactly as in repo mode: an edited-since-
ratification spec still fails `unratified_spec_change` under `--receipt
--enforce` (an agent cannot loosen its own leash mid-run by editing its own
obligations).

The receipt file is parsed and SHA-256-hashed **once** per invocation
(`load_receipt_subject`, hashing before parsing so a merely-malformed-JSON
file, readable, just not valid JSON, still yields a real hash on its
error path: "even a malformed receipt's hash is meaningful evidence of
exactly what bad input was rejected"). `g8 check --json`'s stable
contract (CLAUDE.md's "Output contract" section, §5 above) gains one new,
purely additive top-level key, present **only** for `--receipt` invocations
(omitted entirely otherwise, same "unchanged shape, unchanged appearance
rule" §5 already documents for `enforcement_disabled`/`obligations_note`):

```jsonc
"receipt": { "path": "receipts/clean-run.json", "sha256": "sha256:...", "scope": "action_receipt" }
```

Two new `EnforcementClassification` values (`crates/g8/src/render/json.rs`):

- **`invalid_receipt`**, the `--receipt` file was missing or not valid
  JSON. Exit `1` under `--enforce`, exit `0` without (same audit-vs-enforce
  asymmetry as every other finding in this system), **never** exit `2`, bad
  input is a refusal, not the CLI's own internal failure.
- **`receipt_violation`**, a gating `receipt_query` obligation resolved
  `Failed`/`Error` against the checked receipt. Deliberately **not**
  `unaccounted_drift`: that classification means the repository diverged;
  this means a proposed *act* violated the contract, a different kind of
  finding even though both gate the same way.

**Receipt-mode rigor floor (design spec §4):** the enforcement floor for a
receipt gate is `Passed` **+ trust tier ≥ `Checked`**, i.e., every tier in
this system (`Checked`/`Asserted`/`Verified`) clears it, because the
decision record's rigor derives from *(ratified spec hash × inline subject
hash)*, pinned by construction at evaluation time, not from the backend's
own nominal tier. Concretely: `classify_enforcement_failures_receipt` has no
`insufficient_rigor`/`stale_attestation_pin` classification at all, a
`Passed` receipt-wired obligation never fails the floor regardless of trust.
**Repo mode's own floor (`meets_rigor_floor` in `check.rs`, requiring
`Verified`/`Asserted`) is completely untouched**, a plain `Checked`-tier
repo-wired obligation (e.g. a bare `rg_match_count` with no attestation)
still needs an attestation to clear repo mode's gate, exactly as before this
addendum (integration-test-verified: `receipt_check.rs`'s AC3 test attests
its one repo-wired fixture obligation for precisely this reason, matching
`obligations_check.rs`'s pre-existing `test_checked_gate_without_attestation_is_insufficient_rigor`).

### 12.4 Disclosed gap, not fixed here: `g8 attest` does not mint ad-hoc IDs

The design spec's own text (§ "Rigor ruling"): *"after a run, `g8 attest
STANDING-RUN-<id> --files runs/<id>.json` pins the executed receipt."* Tried
live against `examples/standing-agent/`: `g8 attest STANDING-RUN-0007
--files receipts/clean-run.json` fails , 
`crates/g8/src/cmd/attest.rs` requires `obligation_id` to already
name an obligation present in `specs/obligations-v0.1.json`; it pins
evidence for an existing obligation, it does not mint a new one on demand.
The design spec's prose reads as if an arbitrary, freshly-chosen ID works
standalone; it does not, today. Not fixed here, `attest.rs` is explicitly
out of this addendum's writable scope (reused, not modified), and the fix
(if wanted) is a product decision, either `attest` gains an
allow-unknown-id mode, or the convention becomes "add a
`checker.mode: attestation` placeholder obligation for the run-ledger ID
first." `examples/standing-agent/README.md`'s walkthrough uses a real,
verified working form instead (`g8 attest OBL-PROVENANCE-01 --files
receipts/clean-run.json`, attesting the receipt as evidence for one of the
artifact's own existing obligations).

### 12.5 Validation

`crates/g8-obligations/src/exec/receipt.rs`'s inline unit tests cover
every op (including every missing-path table entry), both `aggregate` kinds
(including the loud non-numeric-sum error, and confirming a `where`-excluded
candidate's own missing/non-numeric value never trips it), all four
`expected` kinds, `[*]` vs. single-node `select`, and empty-select-counts-
zero. `crates/g8/tests/receipt_check.rs` covers acceptance criteria
1–7 and 9 end-to-end through the real CLI binary; criterion 10 (the example
walkthrough) is validated by running its documented commands directly
against a real build, not by a `#[test]`.

---

**End of obligation-checker-contract.md.**
