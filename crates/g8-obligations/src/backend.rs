//! Checker backend schema — the closed-by-default enum (11 variants as of
//! the o-g8-receipts-20260721 contract addendum) + structured arguments.
//!
//! Verbatim transcription of `specs/obligation-checker-contract.md` §1, with
//! two gaps the contract left implicit filled in here (documented at each
//! site): `AstGrepLang` was referenced but never defined, and `DepMatcher` /
//! `BuildConfig` need explicit serde tag attributes to produce the exact JSON
//! shape shown in the contract's own §6 worked example
//! (`{"kind":"exact","value":"surrealdb"}`, `"build_config":"default"`) —
//! their Rust sketch in §1.2 omits the attribute that shape requires.
//!
//! # Non-negotiable constraint (contract §0)
//!
//! There is no `shell_command: String` field anywhere in this module, and
//! none may be added without a new binding ruling. Every backend that shells
//! out builds a `std::process::Command` from a fixed program name plus a
//! typed-field argument *vector* — never a formatted string handed to a shell.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── The closed backend enum ──────────────────────────────────────────────────

/// Eleven backends (ten from the original contract, plus `ReceiptQuery` —
/// contract addendum o-g8-receipts-20260721, backend #11). Closed: if an
/// obligation genuinely doesn't fit one of these, the answer is to escalate
/// for a new binding ruling, never to add a raw shell-string escape hatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", content = "args", rename_all = "snake_case")]
pub enum CheckerBackend {
    /// Deny-list check against `cargo metadata`'s resolved dependency graph.
    /// Respects Cargo feature-gating (optional/non-default deps are correctly
    /// excluded) — the only backend in this enum that does.
    CargoMetadataNoDep(CargoMetadataNoDepArgs),

    /// Positive specification of the workspace-internal dependency-edge set
    /// plus `[[bin]]` target count.
    CargoMetadataDepGraph(CargoMetadataDepGraphArgs),

    /// Structural AST pattern(s) that must NOT match anywhere in `glob`.
    /// Operates on source TEXT/AST — does **not** respect `#[cfg(...)]`
    /// gating (contract §2.1 "cfg-blindness").
    AstGrepNoMatch(AstGrepArgs),

    /// Structural AST pattern whose match COUNT (optionally scoped to inside
    /// one named function) must equal/meet an expectation.
    AstGrepMatchCount(AstGrepCountArgs),

    /// Regex line-count/presence check via the `rg` binary (respects
    /// .gitignore by default, same semantics as running `rg` by hand).
    RgMatchCount(RgArgs),

    /// Extract one Rust enum's variant list (+ optional `#[serde(rename_all
    /// = ...)]`) and compare against an expected shape. Same cfg-blindness
    /// caveat as `AstGrepNoMatch`.
    RustEnumShape(EnumShapeArgs),

    /// Run a sequence of `g8` CLI invocations against an isolated scratch
    /// fixture and assert structured outcomes (files, JSON fields, hashes,
    /// stderr format). The general-purpose "real integration test" backend.
    FixtureIntegrationTest(FixtureTestArgs),

    /// Run the same read-only comparison twice against a controlled fixture
    /// and byte-compare stdout, after masking only the declared timing
    /// fields. See contract §9 for the full determinism boundary.
    ByteDiffTwice(ByteDiffArgs),

    /// Validate the SHAPE of `g8 check --json`'s own output contract
    /// **in-process** — never by shelling out to `g8 check`. See contract §8.
    G8CheckContract(CheckContractArgs),

    /// A named, hand-reviewed deterministic algorithm implemented as a real
    /// Rust function in this crate, for constraints that are genuinely
    /// bespoke logic rather than a pattern/metadata query. Closed: `name` is
    /// itself an enum, not a free string — not an escape hatch for arbitrary
    /// code, a registry of exactly the named algorithms this contract
    /// enumerates.
    BuiltinAlgorithm(BuiltinAlgorithmArgs),

    /// #11 (contract addendum, o-g8-receipts-20260721). A general
    /// structural evaluator over a *parsed action receipt* — NOT the
    /// repository. Only valid on an obligation whose `signal.wiring ==
    /// "action_receipt"`; subject binding (this backend may only appear
    /// there, and no other backend may appear on a receipt-wired obligation)
    /// is enforced by the runner (`crate::run_obligations`'s scope-filtering
    /// pass), not by this enum. Static trust tier `Checked` — honest, this
    /// is structural analysis, not run-backed behavior (see the addendum's
    /// receipt-mode rigor ruling for why `Checked` still clears the gate
    /// floor in receipt mode specifically).
    ReceiptQuery(ReceiptQueryArgs),
}

impl CheckerBackend {
    /// The `#[serde(tag = "backend", ...)]` string this variant serializes
    /// under. Single source of truth shared by `Evidence::backend` /
    /// `SubCheckResult::backend` and [`trust_tier`](Self::trust_tier) so the
    /// two never drift apart.
    pub fn name(&self) -> &'static str {
        match self {
            CheckerBackend::CargoMetadataNoDep(_) => "cargo_metadata_no_dep",
            CheckerBackend::CargoMetadataDepGraph(_) => "cargo_metadata_dep_graph",
            CheckerBackend::AstGrepNoMatch(_) => "ast_grep_no_match",
            CheckerBackend::AstGrepMatchCount(_) => "ast_grep_match_count",
            CheckerBackend::RgMatchCount(_) => "rg_match_count",
            CheckerBackend::RustEnumShape(_) => "rust_enum_shape",
            CheckerBackend::FixtureIntegrationTest(_) => "fixture_integration_test",
            CheckerBackend::ByteDiffTwice(_) => "byte_diff_twice",
            CheckerBackend::G8CheckContract(_) => "g8_check_contract",
            CheckerBackend::BuiltinAlgorithm(_) => "builtin_algorithm",
            CheckerBackend::ReceiptQuery(_) => "receipt_query",
        }
    }

    /// Static trust tier for this backend family, per contract §3.2's table.
    pub fn trust_tier(&self) -> crate::result::TrustLevel {
        use crate::result::TrustLevel;
        match self {
            CheckerBackend::FixtureIntegrationTest(_)
            | CheckerBackend::ByteDiffTwice(_)
            | CheckerBackend::G8CheckContract(_) => TrustLevel::Verified,
            _ => TrustLevel::Checked,
        }
    }
}

/// Resolve a trust tier from a backend's serialized name string. Used during
/// multi-check aggregation (contract §3.3) where only the string survives in
/// `SubCheckResult`. Must stay exhaustive over [`CheckerBackend::name`]'s
/// output; a stale/unknown name defaults to `Checked` (the conservative,
/// lower tier) rather than panicking.
pub(crate) fn trust_tier_for_name(name: &str) -> crate::result::TrustLevel {
    use crate::result::TrustLevel;
    match name {
        "fixture_integration_test" | "byte_diff_twice" | "g8_check_contract" => {
            TrustLevel::Verified
        }
        "attestation" => TrustLevel::Asserted,
        _ => TrustLevel::Checked,
    }
}

// ── §1.2 structured argument types ───────────────────────────────────────────

/// A single crate-name matcher.
///
/// Explicit `kind`/`value` tagging (not the contract sketch's implied default
/// derive) — required to produce contract §6's own worked example shape:
/// `{"kind": "exact", "value": "surrealdb"}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DepMatcher {
    Exact(String),
    /// e.g. `"(?i)mcp"` for OBL-N4-01.
    Regex(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoMetadataNoDepArgs {
    pub denied: Vec<DepMatcher>,
    /// Which workspace crates' resolved dep trees to inspect. Empty = all.
    pub scope_crates: Vec<String>,
    /// Which Cargo feature set to resolve against.
    pub build_config: BuildConfig,
}

/// `rename_all = "snake_case"` — required to produce contract §6's own
/// worked example shape: `"build_config": "default"` (the contract's §1.2
/// sketch shows no attribute, which would default to `"Default"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildConfig {
    /// No `--features` flag. This is what makes the backend cfg-aware: an
    /// `optional = true` dep with no default feature enabling it correctly
    /// does NOT appear in a `Default`-resolved graph.
    Default,
    Features(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoMetadataDepGraphArgs {
    /// (from_crate, to_crate) — workspace-internal edges only.
    pub required_edges: Vec<(String, String)>,
    pub forbid_extra_edges: bool,
    /// (crate, expected `[[bin]]` target count).
    pub expected_bin_targets: Vec<(String, u32)>,
}

/// Language selector for ast-grep-backed backends.
///
/// Not given an explicit definition in contract §1.2 (only referenced via
/// `pub lang: AstGrepLang, // Rust | TypeScript | Python | Go`) — defined
/// here per the four values that comment enumerates. Serialized values match
/// `g8-extractor/rules/g8-all-languages.yaml`'s own `language:` keys
/// (`rust`, `typescript`, `python`, `go`), verified against the installed
/// `ast-grep` binary's accepted `--lang` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstGrepLang {
    Rust,
    TypeScript,
    Python,
    Go,
}

impl AstGrepLang {
    /// The exact `--lang` value ast-grep's CLI expects.
    pub fn as_ast_grep_arg(self) -> &'static str {
        match self {
            AstGrepLang::Rust => "rust",
            AstGrepLang::TypeScript => "typescript",
            AstGrepLang::Python => "python",
            AstGrepLang::Go => "go",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstGrepArgs {
    /// ast-grep pattern syntax, one or more independent patterns (each
    /// checked; any match anywhere is a hit).
    pub patterns: Vec<String>,
    pub lang: AstGrepLang,
    pub glob: Vec<String>,
    #[serde(default)]
    pub exclude_glob: Vec<String>,
    /// Optional: after a structural match, inspect one captured metavariable
    /// and only count it as a hit if the predicate is satisfied.
    #[serde(default)]
    pub capture_predicate: Option<CapturePredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturePredicate {
    /// e.g. `"NAME"` for a `$NAME` metavariable.
    pub capture: String,
    pub forbidden_substring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstGrepCountArgs {
    pub pattern: String,
    pub lang: AstGrepLang,
    pub glob: Vec<String>,
    /// Restrict the match to inside one named function/impl block.
    #[serde(default)]
    pub scope: Option<FnScope>,
    pub expected: CountExpectation,
    /// If set, every match's named capture must equal this literal.
    #[serde(default)]
    pub capture_equals: Option<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnScope {
    pub function: String,
}

/// Wire shape verified against the real `specs/obligations-v0.1.json`
/// (ground truth per T4/T4b): `{"kind":"zero"}`, `{"kind":"exactly","value":30}`,
/// `{"kind":"at_least","value":1}` — internal tagging with each non-unit
/// variant's payload flattened under a single named field (`value`), not
/// the tuple-variant shape the contract's own §1.2 sketch shows. Pure
/// `#[serde(tag = "kind")]` cannot represent a bare-`u32` tuple variant
/// (internal tagging requires map-shaped content), so `Exactly`/`AtLeast`
/// are struct variants here, not `Exactly(u32)`/`AtLeast(u32)`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CountExpectation {
    Zero,
    Exactly { value: u32 },
    AtLeast { value: u32 },
}

impl CountExpectation {
    pub fn is_met_by(self, actual: u32) -> bool {
        match self {
            CountExpectation::Zero => actual == 0,
            CountExpectation::Exactly { value } => actual == value,
            CountExpectation::AtLeast { value } => actual >= value,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgArgs {
    /// Regex, passed as a single argv element to `rg` — never interpolated
    /// into a shell string.
    pub pattern: String,
    pub glob: Vec<String>,
    pub expected: CountExpectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumShapeArgs {
    pub file: String,
    pub enum_name: String,
    /// Order-independent set compare.
    pub expected_variants: Vec<String>,
    pub expected_serde_rename_all: Option<String>,
}

/// **Amended (T2b).** `binary` defaults to [`BinarySource::CurrentExe`] when
/// omitted from the artifact JSON (`#[serde(default)]`) — every
/// `CliInvocation` across the other 26 obligations predates this field and
/// deserializes unaffected. `Default` derived so existing Rust-level
/// construction sites can opt in via `..Default::default()` rather than
/// naming `binary` explicitly everywhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CliInvocation {
    /// argv, e.g. `["init"]`, `["space", "add", "../member"]`.
    pub args: Vec<String>,
    /// Additive to the controlled base env (contract §9) — never leaves it
    /// implicit.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub binary: BinarySource,
}

/// **New (T2b).** Which `g8` binary a [`CliInvocation`] runs against.
///
/// Wire shape verified against the real artifact:
/// `{"kind":"cargo_run","package":"g8","features":[]}` — internal
/// tagging, `CargoRun`'s fields flattened as siblings of `"kind"`.
/// `CurrentExe` never actually appears in the 26 pre-amendment obligations
/// (they omit `binary` entirely and rely on this `Default`), but is still
/// given a `{"kind":"current_exe"}` wire shape for round-trip completeness.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BinarySource {
    /// Subprocess-spawn the currently-running `g8` binary via
    /// `std::env::current_exe()`. Fast — no rebuild — but reflects whatever
    /// Cargo feature set THIS binary happens to be compiled with. Correct
    /// default for every obligation except D7-01.
    #[default]
    CurrentExe,
    /// Explicit `cargo run -q -p <package> [--features <features>] --
    /// <args>`. Slower (may trigger a real (re)compile of a different
    /// feature combination) but deterministically exercises a build config
    /// independent of whatever `g8 check` itself was compiled as. Mirrors,
    /// verbatim, T1's own acceptance-criteria verification method
    /// (`cargo run -q -p g8 [--features serve] -- --help`).
    CargoRun {
        package: String,
        features: Vec<String>,
    },
}

/// **Amended (T2e).** `seed_files` are written to the scratch fixture
/// directory *before* `setup[0]` runs. Added because several obligations
/// (D4-01, P6-01) need a real file to exist for `scan`/`query` to observe
/// anything meaningful, and nothing in the pre-T2e schema could express
/// "place this content at this path" — `setup` is CLI invocations only.
/// `#[serde(default)]` — every pre-T2e obligation's args omit this field and
/// are unaffected (empty seed set, unchanged behavior).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixtureTestArgs {
    #[serde(default)]
    pub seed_files: Vec<SeedFile>,
    pub setup: Vec<CliInvocation>,
    pub assertions: Vec<Assertion>,
}

/// **New (T2e).** One file to write into the fixture directory before setup
/// runs. `path` is relative to the fixture root (parent directories created
/// as needed). Plain struct, no `kind` tag needed (unlike `Assertion`/
/// `BinarySource`) — `SeedFile` has no variants to disambiguate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedFile {
    pub path: String,
    pub content: String,
}

/// Wire shape verified against the real `specs/obligations-v0.1.json`:
/// `{"kind":"file_exists","path":"..."}`, `{"kind":"exit_code","from_step":1,
/// "equals":0}`, `{"kind":"second_connection_errs_after_timeout"}` —
/// internal tagging (`tag = "kind"`, no `content`), every variant's payload
/// flattened as siblings of `"kind"`. This is why `FileExists` is a
/// single-named-field struct variant (`{ path: String }`) here rather than
/// the tuple shape `FileExists(String)` the contract's §1.2 sketch shows —
/// pure internal tagging cannot represent a bare-`String` tuple variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assertion {
    FileExists {
        path: String,
    },
    FileHashUnchanged {
        path: String,
        before_step: usize,
        after_step: usize,
    },
    FileHeaderEquals {
        path: String,
        expected_prefix: String,
    },
    FileContains {
        path: String,
        needle: String,
    },
    JsonField {
        from_step: usize,
        path: String,
        equals: serde_json::Value,
    },
    JsonPathNonEmpty {
        from_step: usize,
        path: String,
    },
    ExitCode {
        from_step: usize,
        equals: i32,
    },
    StderrFormat {
        from_step: usize,
        json_lines: bool,
    },
    /// **New (T2b).** Plain-text stdout must contain every needle, each
    /// matched as a whole word (not a bare substring) — added because
    /// `--help` output is plain clap text, not JSON; nothing in the
    /// pre-amendment `Assertion` enum could inspect it.
    StdoutContainsAll {
        from_step: usize,
        needles: Vec<String>,
    },
    /// **New (T2b).** Companion negative-match assertion, same rationale.
    StdoutNotContains {
        from_step: usize,
        needle: String,
    },
    NoAnnotationSourceFile {
        basename: String,
    },
    SecondConnectionErrsAfterTimeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteDiffArgs {
    /// **Amended (Erratum 4, Q-rulings pass).** Files written into the ONE
    /// scratch fixture before `setup` runs — same primitive and rationale
    /// as `FixtureTestArgs::seed_files` (T2e): OBL-D11-04's own prose
    /// demands a populated, ">=2-element fixture store", which CLI
    /// invocations alone cannot conjure from an empty directory.
    /// `#[serde(default)]` — absent in pre-Erratum-4 artifacts, unchanged
    /// behavior when empty.
    #[serde(default)]
    pub seed_files: Vec<SeedFile>,
    /// Run ONCE against one scratch fixture, BEFORE `compare` runs twice.
    ///
    /// **Amended (Erratum 4):** OBL-D11-04's binding prose is "run the same
    /// command twice against an identical fixture store" — ONE store, TWO
    /// reads. The previous per-branch shape (independent bootstrap per
    /// side) was a contract-level misreading: it compared two DIFFERENT
    /// inputs (each fresh store legitimately embeds its own absolute
    /// scope paths and wall-clock timestamps), so its failures were
    /// environment noise, not the HashMap-iteration-order signal the
    /// obligation targets. See contract §2.6.
    ///
    /// Empty `setup` + empty `seed_files` = `compare` runs twice against
    /// `workspace_root` itself (nothing bootstrapped).
    pub setup: Vec<CliInvocation>,
    pub compare: CliInvocation,
    /// JSON paths whose values are masked before byte-comparison. Supported
    /// syntax: `$..<key>` (recursive-descent-by-key — the only form contract
    /// §9's own example uses, e.g. `"$..duration_ms"`). Restricted allowlist
    /// — NEVER used to mask ordering or content, only declared timing
    /// fields; an unsupported path syntax is a checker `Error`, not a
    /// silently-ignored no-op.
    pub normalize_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckContractScenario {
    CleanStore,
    KnownBadCapabilityFixture,
}

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

// ── ReceiptQuery args — contract addendum (o-g8-receipts-20260721) ──────

/// `receipt_query` backend #11's arguments (addendum §2). Selects candidates
/// out of the parsed receipt, filters them through `where` (ANDed), computes
/// an aggregate over the surviving candidates, and compares it to `expected`.
///
/// Wire shape:
/// ```jsonc
/// { "select": "$.actions[*]",
///   "where": [{ "path": "$.kind", "op": "eq", "value": "publish" }],
///   "aggregate": { "kind": "count" },
///   "expected": { "kind": "zero" } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptQueryArgs {
    /// Path into the parsed receipt, in the same `$.`-prefixed dot/bracket
    /// dialect [`crate::exec`]'s `FixtureIntegrationTest` assertions already
    /// use, plus one addition: a trailing `[*]` explodes an array at that
    /// path into one candidate per element. Without `[*]`, the resolved node
    /// itself (array or not) is the sole candidate. A path that does not
    /// resolve at all yields zero candidates.
    pub select: String,
    /// ANDed predicates; each clause's `path` resolves against every
    /// candidate independently. `[*]` is not meaningful here — only `select`
    /// produces multiple candidates. Empty = every candidate matches (no
    /// filtering).
    #[serde(default, rename = "where")]
    pub where_clauses: Vec<WhereClause>,
    pub aggregate: ReceiptAggregate,
    pub expected: ReceiptExpectation,
}

/// One `where`-clause predicate, evaluated against a single candidate.
///
/// **Missing-path semantics are fail-closed for the canonical deny
/// patterns** (addendum): a candidate whose `path` does not resolve
/// *satisfies* the negative ops (`Ne`, `NotIn`, `Absent`) and *fails* every
/// positive op (`Eq`, `In`, `Matches`, `Exists`, and the four numeric
/// comparisons). Rationale: "count of actions whose target is `not_in` the
/// allowlist must be zero" has to count a targetless action as outside the
/// allowlist, not let it slip through unnoticed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhereClause {
    pub path: String,
    pub op: WhereOp,
    /// Required by every op except `Exists`/`Absent`. Missing or the wrong
    /// JSON type for the op it's paired with is a **config** error (this
    /// obligation's own args are malformed) — surfaces as
    /// `ObligationStatus::Error`, never silently treated as a per-candidate
    /// outcome.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhereOp {
    Eq,
    Ne,
    /// Regex match on a string candidate value; `value` is the pattern. A
    /// non-string candidate value fails closed (does not match) rather than
    /// erroring — only a malformed pattern/non-string `value` is a config
    /// error.
    Matches,
    /// `value` is a JSON array; membership test by deep (structural) equality.
    In,
    NotIn,
    /// Path resolves to *something* (including JSON `null`).
    Exists,
    /// Path does not resolve at all.
    Absent,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReceiptAggregate {
    /// Number of candidates that passed every `where` clause.
    Count,
    /// Sum of `path` resolved against each passing candidate. **Loud**: a
    /// missing or non-numeric value at any passing candidate is
    /// `ObligationStatus::Error`, never silently skipped — addendum: "a
    /// spend cap that silently skips unparseable amounts is a hole."
    Sum { path: String },
}

/// Mirrors `CountExpectation`'s `{kind, value}` shape (§1.2 of the original
/// contract), with `value` widened to `f64` since `Sum` aggregates (e.g.
/// dollar amounts) are not necessarily integral.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReceiptExpectation {
    Zero,
    Exactly { value: f64 },
    AtLeast { value: f64 },
    AtMost { value: f64 },
}

impl ReceiptExpectation {
    pub fn is_met_by(self, actual: f64) -> bool {
        match self {
            ReceiptExpectation::Zero => actual == 0.0,
            ReceiptExpectation::Exactly { value } => actual == value,
            ReceiptExpectation::AtLeast { value } => actual >= value,
            ReceiptExpectation::AtMost { value } => actual <= value,
        }
    }
}
