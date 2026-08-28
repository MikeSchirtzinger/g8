# `g8`, Architecture

**Companion to SPEC.md. v0.1, locked 2026-05-26 by A1.**

This document specifies the technical realization: crate graph, public APIs, data model, store schema, query contracts, CLI surface, file layout, tracing, and the seams between deterministic and (future) non-deterministic zones. Builders B1–B6 work to this contract.

Every section that follows is **binding** for v0.1 unless explicitly marked "builder discretion" or "roadmap."

---

## 1. Crate dependency graph

```
                       g8  (binary "g8")
                        │
            ┌───────────┼───────────────────────┐
            │           │           │           │
            ▼           ▼           ▼           ▼
       g8-planner  g8-conflict  g8-store  g8-extractor
            │           │           │           │
            └─────┬─────┴─────┬─────┘           │
                  ▼           ▼                  │
              g8-core ◄──────┴──────────────────┘
```

Strict DAG. `g8-core` has no workspace-crate deps. `g8` is the only binary.

Dep flow rules:
- `g8-core` re-exports nothing from outside the workspace; it owns the canonical types.
- `g8-extractor` returns `Vec<g8_core::Annotation>`, never touches the store directly.
- `g8-store` consumes `Vec<g8_core::Annotation>` (via `apply_scan()`) and produces query results in `g8-core` types.
- `g8-planner` consumes `&dyn StoreConnection` and returns `g8_core::FitReport`.
- `g8-conflict` consumes two `&dyn StoreConnection` (local + remote-attached) and returns `Vec<g8_core::Conflict>`.
- `g8` orchestrates the others; contains zero domain logic.

---

## 2. Workspace layout

```
g8/
├── Cargo.toml                     # workspace skeleton (A1)
├── rust-toolchain.toml            # channel = "stable" (A1)
├── .gitignore                     # (A1)
├── SPEC.md                        # (A1)
├── ARCHITECTURE.md                # (A1, this file)
├── README.md                      # (P1)
├── CLAUDE.md                      # (P1)
├── demo.sh                        # (P1)
├── crates/
│   ├── g8-core/                  # (B1)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── g8-extractor/             # (B2)
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   ├── rules/
│   │   │   └── g8-all-languages.yaml   # ast-grep rules per R2 §5
│   │   └── tests/fixtures/
│   ├── g8-store/                 # (B3)
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── store_impl.rs       # StoreConnection trait + RusqliteStore + composite
│   │   │   │                       #   queries (as-built: planner_intent_check and the
│   │   │   │                       #   rest live here, not in a separate queries.rs , 
│   │   │   │                       #   see CLAUDE.md "Composite queries")
│   │   │   └── migrations/
│   │   │       ├── mod.rs          # embed_migrations!
│   │   │       └── V1__init.sql
│   ├── g8-planner/               # (B5)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── g8-conflict/              # (B6)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── g8/                   # (B4)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── cmd/                 # one module per subcommand
│           │   ├── init.rs
│           │   ├── scan.rs
│           │   ├── plan.rs
│           │   ├── status.rs
│           │   ├── check.rs
│           │   ├── merge.rs
│           │   ├── query.rs
│           │   ├── space.rs
│           │   ├── link.rs
│           │   └── substrate.rs
│           └── render/              # pretty vs JSON output formatters
│               ├── pretty.rs
│               └── json.rs
├── examples/                       # (P1)
└── .github/
    └── workflows/
        └── ci.yml                  # (P1)
```

---

## 3. Crate-by-crate public API surface

### 3.1 `g8-core` (owns: data types)

**Purpose:** every other crate consumes these types; they are the lingua franca.

**No I/O.** No `tokio`, no `rusqlite`, no `process`. Pure data + serde + derives.

**Cargo deps:** `serde`, `serde_json`, `thiserror`, `chrono`, `nanoid`.

**Public module tree:**

```rust
// crates/g8-core/src/lib.rs
pub mod ids;          // Newtypes: SpaceId, ProjectId, CapabilityId, IntentId, PlanId, DecisionId, AnnotationId
pub mod model;        // ConvergenceSpace, Project, Capability, Intent, Plan, Decision, Annotation
pub mod annotation;   // Annotation, AnnotationKind, SourceLocation, RawAnnotation grammar shapes
pub mod plan;         // PlanStatus, PlanDraft, FitReport, Recommendation
pub mod conflict;     // Conflict, ConflictKind, Severity, Evidence
pub mod substrate;    // SubstrateName, SubstrateBudget
pub mod owner;        // Owner, OwnerAgent, OwnerTeam, OwnerContact
pub mod error;        // G8Error (the top-level error enum)
pub mod time;         // EpochMillis newtype + helpers
```

**Key types, exact field-level shapes:**

```rust
// ───────────────── IDs ─────────────────
// All IDs are 21-char nanoid strings, URL-safe alphabet. The wrapper enforces
// the format; serde_json (de)serializes as plain strings.
pub struct SpaceId(String);
pub struct ProjectId(String);
pub struct CapabilityId(String);
pub struct IntentId(String);
pub struct PlanId(String);
pub struct DecisionId(String);
pub struct AnnotationId(String);

impl SpaceId {
    pub fn new() -> Self;                       // generates nanoid
    pub fn from_string(s: String) -> Result<Self, G8Error>;
    pub fn as_str(&self) -> &str;
}
// (similar impl block for each ID newtype)

// ───────────────── ConvergenceSpace ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvergenceSpace {
    pub id:           SpaceId,
    pub name:         String,
    pub root_path:    PathBuf,
    pub created_at:   EpochMillis,
    pub updated_at:   EpochMillis,
    pub members:      Vec<ProjectId>,        // populated by store loader
    pub meta:         Option<serde_json::Value>,
}

// ───────────────── Project ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub id:           ProjectId,
    pub space_id:     SpaceId,
    pub name:         String,                 // human label; not unique by itself
    pub description:  Option<String>,
    pub root_path:    PathBuf,                // absolute, canonicalized
    pub language:     Option<String>,         // "rust" | "ts" | "py" | "go" | "mixed" | unknown
    pub created_at:   EpochMillis,
    pub updated_at:   EpochMillis,
    pub meta:         Option<serde_json::Value>,
}

// ───────────────── Capability ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub id:               CapabilityId,
    pub project_id:       ProjectId,
    pub name:             String,             // kebab-case; project-scoped (Decision 3)
    pub description:      Option<String>,
    pub substrate:        Option<String>,     // SubstrateName as a string
    pub consumes:         Vec<String>,        // free-form substrate-typed identifiers
    pub produces:         Vec<String>,
    pub status:           CapabilityStatus,
    pub stub:             bool,
    pub stub_since:       Option<chrono::NaiveDate>,
    pub owner:            Option<Owner>,
    pub source:           SourceLocation,
    pub annotation_raw:   String,
    pub conditional:      bool,               // true if extractor flagged as cfg_attr-like / dev_only
    pub created_at:       EpochMillis,
    pub updated_at:       EpochMillis,
    pub meta:             Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Proposed,
    InFlight,
    Landed,
    Stale,
    Superseded,
    Parked,
}

// ───────────────── Intent ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Intent {
    pub id:           IntentId,
    pub space_id:     SpaceId,
    pub project_id:   Option<ProjectId>,      // None when intent attaches to space root
    pub kind:         IntentKind,
    pub heading:      String,
    pub description:  String,
    pub scope_path:   PathBuf,                // directory the intent applies to
    pub scope_depth:  u32,                    // directory depth from space root; precedence key
    pub substrate:    Option<String>,
    pub owner:        Option<Owner>,
    pub source:       SourceLocation,
    pub source_kind:  IntentSourceKind,
    pub created_at:   EpochMillis,
    pub updated_at:   EpochMillis,
    pub meta:         Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    ArchitecturalScope,     // # Architecture
    Boundary,               // # Boundaries / # Never do
    Operational,            // # Commands (deprioritized in planner)
    TechStack,              // # Stack
    ExplicitIntent,         // # Intent (g8-native)
    Unclassified,           // anything else
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSourceKind {
    AgentsMd,
    ClaudeMd,
    G8Sidecar,
    InlineComment,
    Manual,
}

// ───────────────── Plan ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub id:                  PlanId,
    pub space_id:            SpaceId,
    pub project_id:          Option<ProjectId>,
    pub title:               String,
    pub description:         Option<String>,
    pub status:              PlanStatus,
    pub substrate:           Option<String>,
    pub touched_capabilities: Vec<CapabilityId>,
    pub derived_from_intents: Vec<IntentId>,
    pub governed_by_decisions: Vec<DecisionId>,
    pub wip_weight:          u32,
    pub parked_reason:       Option<String>,
    pub blocked_reason:      Option<String>,
    pub dispatched_at:       Option<EpochMillis>,
    pub completed_at:        Option<EpochMillis>,
    pub created_at:          EpochMillis,
    pub updated_at:          EpochMillis,
    pub meta:                Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlanStatus {
    Idea,
    Scoped,
    Dispatched,
    Blocked,
    Done,
    Parked,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanDraft {
    pub title:               String,
    pub description:         Option<String>,
    pub substrate:           Option<String>,
    pub touched_capabilities: Vec<String>,    // by name; resolver maps to IDs at query time
    pub touched_paths:       Vec<PathBuf>,    // for intent overlap detection
    pub space_id:            SpaceId,
    pub project_id:          Option<ProjectId>,
}

// ───────────────── FitReport (the planner's output) ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FitReport {
    pub draft:            PlanDraft,
    pub existing_matches: Vec<ExistingMatch>,
    pub budget_status:    Option<BudgetStatus>,        // None if no substrate hint in draft
    pub bottlenecks:      Vec<BottleneckPlan>,
    pub drift_hints:      Vec<DriftHint>,
    pub intent_overlaps:  Vec<IntentOverlap>,
    pub parked_ideas:     Vec<ParkedIdeaMatch>,
    pub decision_input:   DecisionInput,
    pub recommendation:   Recommendation,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExistingMatch {
    pub id:         PlanId,
    pub title:      String,
    pub status:     PlanStatus,
    pub substrate:  Option<String>,
    pub match_kind: MatchKind,                // SubstrateOverlap | CapabilityOverlap | TitleOverlap
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    SubstrateOverlap,
    CapabilityOverlap,
    TitleOverlap,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BudgetStatus {
    pub substrate:       String,
    pub wip_cap:         u32,
    pub wip_current:     u32,
    pub wip_remaining:   i64,                 // signed: negative = over cap
    pub at_cap:          bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BottleneckPlan {
    pub id:             PlanId,
    pub title:          String,
    pub status:         PlanStatus,
    pub blocked_reason: Option<String>,
    pub days_in_flight: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DriftHint {
    pub id:         PlanId,
    pub title:      String,
    pub status:     PlanStatus,
    pub days_stale: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntentOverlap {
    pub id:          IntentId,
    pub title:       String,                  // heading from the AGENTS.md section
    pub substrate:   Option<String>,
    pub source_kind: IntentSourceKind,
    pub project:     Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParkedIdeaMatch {
    pub id:            PlanId,
    pub title:         String,
    pub parked_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DecisionInput {
    pub any_over_budget:  bool,
    pub has_exact_match:  bool,
    pub near_match_count: u32,
    pub has_parked_match: bool,
    pub bottleneck_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Recommendation {
    Park,        // budget full or active bottlenecks
    Drop,        // exact-name match exists in non-parked state
    Wait,        // budget full but no bottleneck; queue and retry
    Extend,      // near-match exists; extend it instead of new plan
    Rename,      // title collision; rename and retry
    Pivot,       // bottleneck on substrate suggests deferring
    Proceed,     // all gates pass
}
// Recommendation is derived from DecisionInput by `g8_core::plan::recommend(input)`
// (pure function; lookup table). See §8.2 below.

// ───────────────── Conflict ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    pub kind:     ConflictKind,
    pub severity: Severity,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    DuplicateIntent,
    CapabilityNameCollision,
    ContradictoryDecisions,
    GovernanceViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Evidence {
    PlanRef       { plan_id: PlanId, title: String, space: String },
    CapabilityRef { capability_id: CapabilityId, name: String, project: String },
    IntentRef     { intent_id: IntentId, heading: String, path: PathBuf },
    DecisionRef   { decision_id: DecisionId, title: String },
    Note          { text: String },
}

// ───────────────── Annotation (extractor output) ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    pub kind:           AnnotationKind,
    pub name:           Option<String>,             // for Capability/Plan
    pub heading:        Option<String>,             // for Intent
    pub fields:         BTreeMap<String, AnnotationValue>,
    pub source:         SourceLocation,
    pub source_kind:    IntentSourceKind,
    pub conditional:    bool,                       // dev_only / cfg_attr-equivalent
    pub raw_text:       String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Capability,
    Intent,
    ConvergenceTest,
    Decision,
    Plan,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum AnnotationValue {
    String(String),
    Bool(bool),
    Number(i64),
    List(Vec<String>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceLocation {
    pub file:   PathBuf,
    pub line:   u32,
    pub column: u32,
}

// ───────────────── Owner ─────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub agent:   Option<String>,    // references .claude/agents/<name>.md
    pub team:    Option<String>,
    pub contact: Option<String>,
}

// ───────────────── Error ─────────────────
#[derive(Debug, thiserror::Error)]
pub enum G8Error {
    #[error("invalid ID: {0}")]
    InvalidId(String),
    #[error("invalid annotation: {0}")]
    InvalidAnnotation(String),
    #[error("invalid plan status transition: {from:?} -> {to:?}")]
    InvalidStatusTransition { from: PlanStatus, to: PlanStatus },
    #[error("unknown substrate: {0}")]
    UnknownSubstrate(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}
```

**Public function inventory (free functions in `g8-core`):**

```rust
pub fn now_millis() -> EpochMillis;
pub fn parse_annotation_grammar(raw: &str) -> Result<RawAnnotation, G8Error>;
pub fn recommend(input: &DecisionInput) -> Recommendation;
pub fn validate_status_transition(from: PlanStatus, to: PlanStatus) -> Result<(), G8Error>;
```

**Doctests:** B1 ships at least one doctest per `pub fn` and per `pub enum`. The doctest for `parse_annotation_grammar` must include both single-line and multi-line magic-comment forms.

---

### 3.2 `g8-extractor` (owns: annotation extraction)

**Purpose:** read a directory tree, produce `Vec<Annotation>` typed via `g8-core`.

**Strategy:** ast-grep CLI shell-out (per R2 §4) for source-code annotations; native Markdown-section parser for AGENTS.md / CLAUDE.md / `*.g8.md` (per R2 §5.6, R3 §5.1).

**Cargo deps:** `g8-core`, `serde`, `serde_json`, `thiserror`, `tracing`, `regex`, `ignore`.

**Runtime dep:** `ast-grep` binary on `PATH` (extractor checks at construction time; returns `ExtractorError::BinaryNotFound` if absent with install instructions in the error message).

**Public API:**

```rust
pub struct Extractor {
    rules_path: PathBuf,
    ast_grep_bin: PathBuf,
}

impl Extractor {
    pub fn new() -> Result<Self, ExtractorError>;
    pub fn with_rules(rules_path: PathBuf) -> Result<Self, ExtractorError>;
    pub fn with_binary(rules_path: PathBuf, ast_grep_bin: PathBuf) -> Result<Self, ExtractorError>;

    #[tracing::instrument(skip(self))]
    pub fn scan_dir(&self, root: &Path) -> Result<ScanResult, ExtractorError>;

    #[tracing::instrument(skip(self))]
    pub fn scan_file(&self, file: &Path) -> Result<Vec<Annotation>, ExtractorError>;
}

pub struct ScanResult {
    pub annotations: Vec<Annotation>,
    pub warnings:    Vec<ScanWarning>,
    pub stats:       ScanStats,
}

pub struct ScanWarning {
    pub file: PathBuf,
    pub line: Option<u32>,
    pub message: String,
}

pub struct ScanStats {
    pub files_scanned: u32,
    pub annotations_found: u32,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractorError {
    #[error("ast-grep binary not found on PATH; install with `cargo install ast-grep`")]
    BinaryNotFound,
    #[error("ast-grep subprocess failed: {0}")]
    SubprocessFailed(String),
    #[error("failed to parse ast-grep JSON: {0}")]
    JsonParse(String),
    #[error("invalid magic-comment annotation at {file}:{line}: {message}")]
    InvalidAnnotation { file: PathBuf, line: u32, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

**Magic-comment grammar parser:**

The extractor's `parse_magic_comment(raw: &str) -> Result<RawAnnotation, _>` parses strings of the form:

```
@g8.<kind>(key1 = "value1", key2 = ["a", "b"], key3 = true)
```

Grammar (informal EBNF; B2 implements this with a small hand-rolled scanner, NOT pest/nom, keep deps small):

```
annotation := "@g8." kind "(" args ")"
kind       := "capability" | "intent" | "convergence_test" | "decision" | "plan"
args       := arg ("," arg)*
arg        := ident "=" value
ident      := [a-zA-Z_][a-zA-Z0-9_]*
value      := string | bool | int | list
string     := '"' [^"]* '"'
bool       := "true" | "false"
int        := -?[0-9]+
list       := "[" string ("," string)* "]"
```

Whitespace between tokens is ignored. Multi-line forms wrap each continuation line with `//` (the extractor concatenates consecutive `//`-prefixed lines preceding a symbol before parsing).

**ast-grep rules file (B2 ships):** `crates/g8-extractor/rules/g8-all-languages.yaml`, one rule per (language × annotation-kind). The full set per R2 §5 with adjustments to use `@g8.<kind>` instead of `@capability` (since v0.1 uses the magic-comment-only form):

```yaml
id: g8-rust-magic-comment
language: rust
rule:
  kind: line_comment
  regex: "@g8\\.(capability|intent|convergence_test|decision|plan)"

---

id: g8-ts-magic-comment
language: typescript
rule:
  kind: comment
  regex: "@g8\\.(capability|intent|convergence_test|decision|plan)"

---

id: g8-tsx-magic-comment
language: tsx
rule:
  kind: comment
  regex: "@g8\\.(capability|intent|convergence_test|decision|plan)"

---

id: g8-python-magic-comment
language: python
rule:
  kind: comment
  regex: "@g8\\.(capability|intent|convergence_test|decision|plan)"

---

id: g8-go-magic-comment
language: go
rule:
  kind: comment
  regex: "@g8\\.(capability|intent|convergence_test|decision|plan)"
```

**Conditional / dev-only detection:** if the matched comment line contains the literal token `dev_only=true` or `dev_only = true`, the extractor sets `Annotation::conditional = true`. Additionally, for Rust files, if the line *immediately preceding* the `// @g8.capability(...)` magic-comment block is `#[cfg(...)]` or `#[cfg_attr(...)]`, the same flag is set (preserves the R1 §2.4.1 cfg_attr exemption).

**`tests/`-dir exemption (R1 §2.4.2):** if the matched file's path contains a component named `tests`, `tests-*`, or `*-tests`, the extractor still emits the annotation but flags `conditional = true` (the downstream gate excludes conditional capabilities from unpaired-filter, see §8.5).

**Markdown sidecar parser:** the extractor walks the tree separately for `AGENTS.md`, `CLAUDE.md`, `*.g8.md`. Per Decision 4, sections become Annotations. The parser is ~100 lines of Rust.

**The extractor adds `.g8/INTENT_SUMMARY.md` to its ignore list explicitly** to avoid a feedback loop.

**Tracing:** every `scan_dir` opens span `extractor.scan_dir(root=...)`; every subprocess invocation opens child span `extractor.ast_grep(rule_file=..., target=...)` with `duration_ms` recorded on exit.

---

### 3.3 `g8-store` (owns: substrate + queries)

**Purpose:** embedded SQLite store; trait-abstracted so future Turso swap is mechanical.

**Cargo deps:** `g8-core`, `rusqlite` (bundled, serde_json), `refinery`, `serde`, `serde_json`, `thiserror`, `tracing`, `chrono`, `nanoid`.

**Public API:**

```rust
pub trait StoreConnection: Send + Sync {
    // ── Lifecycle ───────────────────────────────────────────────────
    fn init_space(&mut self, space: &ConvergenceSpace) -> Result<(), StoreError>;
    fn get_space(&self, id: &SpaceId) -> Result<Option<ConvergenceSpace>, StoreError>;
    fn get_default_space(&self) -> Result<Option<ConvergenceSpace>, StoreError>;
    fn upsert_project(&mut self, project: &Project) -> Result<(), StoreError>;
    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, StoreError>;
    fn list_projects(&self, space: &SpaceId) -> Result<Vec<Project>, StoreError>;

    // ── Bulk scan apply ─────────────────────────────────────────────
    /// Apply a complete extractor scan result for one project. Idempotent:
    /// removes prior capability/intent rows for the project, then inserts the
    /// new ones. Uses one write transaction.
    fn apply_scan(&mut self, project: &ProjectId, scan: ScanResult)
        -> Result<ApplyScanReport, StoreError>;

    // ── Capability CRUD ────────────────────────────────────────────
    fn upsert_capability(&mut self, cap: &Capability) -> Result<(), StoreError>;
    fn get_capability(&self, id: &CapabilityId) -> Result<Option<Capability>, StoreError>;
    fn find_capability_by_name(&self, project: &ProjectId, name: &str)
        -> Result<Option<Capability>, StoreError>;
    fn list_capabilities(&self, project: &ProjectId) -> Result<Vec<Capability>, StoreError>;
    fn delete_capabilities_for_project(&mut self, project: &ProjectId)
        -> Result<u64, StoreError>;

    // ── Intent CRUD ────────────────────────────────────────────────
    fn upsert_intent(&mut self, intent: &Intent) -> Result<(), StoreError>;
    fn list_intents_at_path(&self, space: &SpaceId, path: &Path)
        -> Result<Vec<Intent>, StoreError>;       // ordered by scope_depth DESC
    fn delete_intents_for_project(&mut self, project: &ProjectId)
        -> Result<u64, StoreError>;

    // ── Plan CRUD ──────────────────────────────────────────────────
    fn create_plan(&mut self, plan: &Plan) -> Result<(), StoreError>;
    fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError>;
    fn list_plans(&self, space: &SpaceId, filter: PlanFilter)
        -> Result<Vec<Plan>, StoreError>;
    fn update_plan_status(&mut self, id: &PlanId, new_status: PlanStatus,
                          reason: Option<String>) -> Result<(), StoreError>;
    fn link_plan_capability(&mut self, plan: &PlanId, capability: &CapabilityId,
                            overlap_kind: OverlapKind) -> Result<(), StoreError>;
    fn link_plan_intent(&mut self, plan: &PlanId, intent: &IntentId,
                        relation: PlanIntentRelation) -> Result<(), StoreError>;

    // ── Decision CRUD ──────────────────────────────────────────────
    fn upsert_decision(&mut self, decision: &Decision) -> Result<(), StoreError>;
    fn list_decisions(&self, space: &SpaceId) -> Result<Vec<Decision>, StoreError>;

    // ── Substrate budget CRUD ──────────────────────────────────────
    fn set_substrate_budget(&mut self, space: &SpaceId, substrate: &str,
                            wip_cap: u32, stale_threshold_days: u32)
        -> Result<(), StoreError>;
    fn get_substrate_budget(&self, space: &SpaceId, substrate: &str)
        -> Result<SubstrateBudget, StoreError>;        // returns default if not set

    // ── Cross-project links (aliases) ──────────────────────────────
    fn add_capability_alias(&mut self, canonical: &str, alias: &str)
        -> Result<(), StoreError>;
    fn resolve_canonical(&self, ref_name: &str) -> Result<String, StoreError>;

    // ── Composite queries (the load-bearing surface for g8-planner) ─
    /// Single-transaction composite query. Returns six JSON-deserializable
    /// payloads. Used by g8-planner to build FitReport.
    fn planner_intent_check(&self, draft: &PlanDraft)
        -> Result<RawIntentCheckPayload, StoreError>;

    /// One-shot pairing-gate check for a project (or all projects in a space).
    /// Used by g8's `check` subcommand.
    fn pairing_check(&self, project: Option<&ProjectId>)
        -> Result<Vec<PairingError>, StoreError>;

    /// Find stale Dispatched plans (updated_at older than the substrate's
    /// stale_threshold_days). Used by g8 status for stale-promotion proposals.
    fn stale_dispatched_plans(&self, space: &SpaceId)
        -> Result<Vec<StalePlanCandidate>, StoreError>;

    // ── Cross-store attach (multi-project federation) ──────────────
    /// ATTACHes another g8-store SQLite db at the given path as a named alias.
    /// After attach, federated queries can reference rows in the remote store.
    fn attach_remote_store(&mut self, alias: &str, path: &Path)
        -> Result<(), StoreError>;
    fn detach_remote_store(&mut self, alias: &str) -> Result<(), StoreError>;

    // ── Tracing ────────────────────────────────────────────────────
    /// Every implementor must open a tracing span around each method call.
    /// This is enforced by convention (no compile-time check); B3 wraps each
    /// trait method body with `#[tracing::instrument(skip(self))]`.
}

pub struct RusqliteStore {
    conn:     rusqlite::Connection,
    store_path: PathBuf,
}

impl RusqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError>;
    pub fn open_in_memory() -> Result<Self, StoreError>;        // for tests
    /// Run all pending refinery migrations.
    pub fn migrate(&mut self) -> Result<(), StoreError>;
}

impl StoreConnection for RusqliteStore { … }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanFilter {
    pub status:    Option<Vec<PlanStatus>>,
    pub substrate: Option<String>,
    pub project:   Option<ProjectId>,
    pub limit:     Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapKind { Touches, Extends, Replaces, Conflicts }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanIntentRelation { Satisfies, Contradicts, Extends, DerivedFrom }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplyScanReport {
    pub inserted_capabilities: u32,
    pub inserted_intents:      u32,
    pub inserted_decisions:    u32,
    pub deleted_capabilities:  u32,
    pub deleted_intents:       u32,
    pub warnings:              Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawIntentCheckPayload {
    pub existing_matches: serde_json::Value,
    pub budget:           serde_json::Value,
    pub bottlenecks:      serde_json::Value,
    pub drift_hints:      serde_json::Value,
    pub intent_overlaps:  serde_json::Value,
    pub parked_ideas:     serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PairingError {
    pub capability: String,
    pub file:       PathBuf,
    pub line:       u32,
    pub reason:     PairingErrorReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingErrorReason {
    NoMatchingConvergenceTest,
    StubExpired,
    StubMissingSince,
    DuplicateCapabilityName,
    InvalidRoot,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StalePlanCandidate {
    pub plan_id:    PlanId,
    pub title:      String,
    pub substrate:  Option<String>,
    pub dispatched_at: EpochMillis,
    pub days_stale: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubstrateBudget {
    pub substrate:            String,
    pub wip_cap:              u32,
    pub stale_threshold_days: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration: {0}")]
    Migration(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid state transition: {0}")]
    InvalidTransition(String),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("g8-core: {0}")]
    Core(#[from] G8Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
```

**`RusqliteStore::open(path)` connection setup:**

```rust
pub fn open(path: &Path) -> Result<Self, StoreError> {
    let conn = rusqlite::Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-8000;"
    )?;
    Ok(Self { conn, store_path: path.to_path_buf() })
}
```

(These PRAGMAs are R4 §3-mandated and are part of the v0.1 contract; do not weaken them.)

---

### 3.4 `g8-planner` (owns: composite-query orchestration + recommendation derivation)

**Purpose:** turn a `PlanDraft` into a `FitReport` by calling `StoreConnection::planner_intent_check` and post-processing.

**Cargo deps:** `g8-core`, `g8-store`, `serde`, `serde_json`, `thiserror`, `tracing`.

**Public API:**

```rust
pub struct Planner;

impl Planner {
    /// Pure function over `&dyn StoreConnection`. Returns the full FitReport.
    /// All non-determinism lives in the store; this function is a thin
    /// transformer.
    #[tracing::instrument(skip(store))]
    pub fn plan_check(
        store: &dyn StoreConnection,
        draft: &PlanDraft,
    ) -> Result<FitReport, PlannerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Core(#[from] G8Error),
    #[error("invalid draft: {0}")]
    InvalidDraft(String),
}
```

The planner does NOT take the cross-cutting `LLM-augmentation` parameter. The non-deterministic seam lives upstream (`PlanDraft` is the in-edge).

**Recommendation derivation logic (lookup table; pure):**

```rust
// Lives in g8_core::plan::recommend(input)
// Order matters: first matching branch wins.
pub fn recommend(input: &DecisionInput) -> Recommendation {
    if input.has_exact_match                                 { return Recommendation::Drop; }
    if input.any_over_budget && input.bottleneck_count > 0   { return Recommendation::Pivot; }
    if input.any_over_budget                                 { return Recommendation::Park; }
    if input.bottleneck_count > 0                            { return Recommendation::Wait; }
    if input.near_match_count > 0                            { return Recommendation::Extend; }
    if input.has_parked_match                                { return Recommendation::Extend; }
    Recommendation::Proceed
}
```

(`Rename` is reserved for v0.2, it requires a fuzzy-match score we don't compute in v0.1; lookup table omits it. Builder B5 sees the `Rename` variant in the enum and may add a fuzzy-match path post-Phase-3 if trivially extractable from `drift_hints`. Otherwise, leave for v0.2.)

---

### 3.5 `g8-conflict` (owns: cross-store overlap detection)

**Purpose:** given two stores (typically a local store + an ATTACHed remote store), return `Vec<Conflict>`.

**Cargo deps:** `g8-core`, `g8-store`, `serde`, `serde_json`, `thiserror`, `tracing`.

**Public API:**

```rust
pub struct ConflictDetector;

impl ConflictDetector {
    /// Compare two stores. Both must be StoreConnection-implementors with
    /// fully-loaded scan data. Returns conflicts grouped by kind, ordered
    /// by descending severity.
    #[tracing::instrument(skip(local, remote))]
    pub fn compare_spaces(
        local:  &dyn StoreConnection,
        local_space:  &SpaceId,
        remote: &dyn StoreConnection,
        remote_space: &SpaceId,
    ) -> Result<Vec<Conflict>, ConflictError>;

    /// Single-store variant: detect intra-space conflicts (duplicate intents,
    /// contradictory decisions, etc.) without cross-store comparison.
    /// Used by `g8 status` to surface conflicts within one workspace.
    #[tracing::instrument(skip(store))]
    pub fn detect_intra_space(
        store: &dyn StoreConnection,
        space: &SpaceId,
    ) -> Result<Vec<Conflict>, ConflictError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error(transparent)]
    Store(#[from] StoreError),
}
```

**Detection rules (exhaustive for v0.1):**

| Kind | Rule | Severity |
|---|---|---|
| `DuplicateIntent` | Same `kind + scope_path` exists in both spaces with different `description` | `Warn` |
| `CapabilityNameCollision` | Same kebab-case `name` exists in both spaces AND no alias resolves them | `Error` |
| `CapabilityNameCollision` | Same kebab-case `name` exists in both spaces AND alias DOES resolve them | `Info` (acknowledgment) |
| `ContradictoryDecisions` | Same `title` exists in both spaces with `status=accepted` AND `body` differs (string-equality) | `Error` |
| `GovernanceViolation` | A Plan in space A has `substrate = X`; space B has a `Boundary` intent at any ancestor path saying "never modify X" | `Error` |

---

### 3.6 `g8` (owns: subcommand surface)

See §9 for the full clap tree. The CLI is the only binary in the workspace. It depends on every other crate but contains no domain logic, just argument parsing, dispatch to library functions, and result rendering.

**Cargo deps:** all workspace crates + `clap`, `serde`, `serde_json`, `anyhow`, `tracing`, `tracing-subscriber`, `toml`, `notify`.

---

## 4. `g8-core` data model summary table

Every entity, where it lives, what scopes it.

| Entity | Scope | Owns | ID |
|---|---|---|---|
| `ConvergenceSpace` | global | Projects, Decisions, Intents, SubstrateBudgets, Plans | `SpaceId` |
| `Project` | space | Capabilities | `ProjectId` |
| `Capability` | project | (none, leaves of tree) | `CapabilityId` |
| `Intent` | space (scoped by path) | (attaches to an Owner) | `IntentId` |
| `Plan` | space (project-tagged) | (links to Capabilities, Intents, Decisions) | `PlanId` |
| `Decision` | space | (none, leaves) | `DecisionId` |
| `SubstrateBudget` | space × substrate-name | (configuration) | `(SpaceId, name)` |
| `Annotation` | extractor-output, transient | (never persisted directly; converted to Capability/Intent/Decision/Plan rows) | `AnnotationId` |
| `Owner` | embedded on Intent | (just data) | n/a |

---

## 5. `StoreConnection` trait, call surface for `g8-planner`

These are the EXACT methods `g8-planner::Planner::plan_check` calls in order. B5 implements against this list; B3 implements the underlying SQL.

```rust
// Pseudocode for plan_check:
fn plan_check(store: &dyn StoreConnection, draft: &PlanDraft) -> Result<FitReport, _> {
    let raw = store.planner_intent_check(draft)?;     // single composite query

    let existing_matches: Vec<ExistingMatch> = serde_json::from_value(raw.existing_matches)?;
    let budget_status: Option<BudgetStatus>   = serde_json::from_value(raw.budget)?;
    let bottlenecks: Vec<BottleneckPlan>      = serde_json::from_value(raw.bottlenecks)?;
    let drift_hints: Vec<DriftHint>           = serde_json::from_value(raw.drift_hints)?;
    let intent_overlaps: Vec<IntentOverlap>   = serde_json::from_value(raw.intent_overlaps)?;
    let parked_ideas: Vec<ParkedIdeaMatch>    = serde_json::from_value(raw.parked_ideas)?;

    let decision_input = DecisionInput {
        any_over_budget:  budget_status.as_ref().map(|b| b.at_cap).unwrap_or(false),
        has_exact_match:  existing_matches.iter().any(|m| m.title == draft.title),
        near_match_count: existing_matches.len() as u32,
        has_parked_match: !parked_ideas.is_empty(),
        bottleneck_count: bottlenecks.len() as u32,
    };

    let recommendation = g8_core::plan::recommend(&decision_input);

    Ok(FitReport {
        draft: draft.clone(),
        existing_matches,
        budget_status,
        bottlenecks,
        drift_hints,
        intent_overlaps,
        parked_ideas,
        decision_input,
        recommendation,
    })
}
```

`g8-planner` calls EXACTLY ONE store method: `planner_intent_check`. Everything else is pure transformation. This is the composite-queries-not-verb-chains principle in code.

---

## 6. Annotation grammar, full contract

### 6.1 Magic-comment form (uniform across languages)

Single-line form (preferred in v0.1; no ambiguity in error position):

```
// @g8.capability(name = "claim-validation", substrate = "claims", status = "in_flight")
fn validate_claim(c: Claim) -> Result<Outcome> { … }
```

Multi-line continuation form (continuation lines must each carry a `//` prefix;
the extractor joins them with a space before parsing, SPEC Decision 1 §6.1):

```
// @g8.capability(name = "claim-validation",
//                 description = "Algebraic claim validation",
//                 substrate = "claims",
//                 consumes = ["substrate::Claim", "substrate::Source"],
//                 produces = ["substrate::ContradictionEvent"],
//                 status = "in_flight",
//                 version = "0.1.0",
//                 dev_only = false)
fn validate_claim(c: Claim) -> Result<Outcome> { … }
```

**v0.2 roadmap note:** in v0.1 the multi-line join (`strip_comment_prefixes`) produces
a flat string before parsing, so error messages reference the joined string position
rather than the original line number.  A token-aware splitter that preserves per-line
source positions is deferred to v0.2 (SPEC §10 "native language idioms" row).

### 6.2 Accepted keys per kind

| Kind | Required keys | Optional keys |
|---|---|---|
| `capability` | `name` | `description`, `substrate`, `consumes`, `produces`, `status`, `stub`, `since`, `version`, `owner`, `dev_only` |
| `intent` | (none; heading from doc/Markdown serves as title) | `substrate`, `description`, `owner` |
| `convergence_test` | `for_capability` | `against`, `scenario` |
| `decision` | `title` | `status`, `context`, `body`, `source_file` |
| `plan` | `title` | `status`, `substrate`, `wip_weight`, `parked_reason`, `blocked_reason`, `description` |

Unknown keys are accepted and stored in `Annotation.fields` for forward-compat (per R1 §8.8 #21). v0.2 may add a `--strict` lint mode.

### 6.3 Conditional / dev-only / cfg-attr exemption

Three triggers set `Annotation::conditional = true`:

1. The annotation contains `dev_only = true`.
2. (Rust only) the immediately preceding line is `#[cfg(...)]` or `#[cfg_attr(...)]`.
3. The file lives under a path component named `tests`, `tests-*`, or `*-tests`.

Conditional capabilities are extracted and stored normally; the *gate* (§8.5) excludes them from the unpaired-filter.

### 6.4 Convergence-test pairing

For each `// @g8.capability(name = X)` with `stub != true`, there must exist a `// @g8.convergence_test(for_capability = X)` annotation somewhere in the same project. The pairing gate (§8.5) enforces this. Stubs are exempt for 7 days from `since`.

---

## 7. Composite `planner_intent_check` query, full SQL

Bound parameters: `:space_id`, `:project_id?`, `:title`, `:substrate?`, `:cap_names` (JSON array string).

```sql
WITH

existing_matches AS (
    SELECT
        p.id,
        p.title,
        p.status,
        p.substrate,
        p.wip_weight,
        p.updated_at,
        'substrate_overlap' AS match_kind
    FROM plan p
    WHERE p.space_id = :space_id
      AND p.status NOT IN ('Done', 'Parked')
      AND (
            p.substrate = :substrate
            OR p.title LIKE '%' || :title || '%'
            OR :title LIKE '%' || p.title || '%'
          )

    UNION

    SELECT
        p.id, p.title, p.status, p.substrate, p.wip_weight, p.updated_at,
        'capability_overlap' AS match_kind
    FROM plan p
    JOIN plan_capability pc ON pc.plan_id = p.id
    JOIN capability c        ON c.id = pc.capability_id
    WHERE p.space_id = :space_id
      AND p.status NOT IN ('Done', 'Parked')
      AND EXISTS (
            SELECT 1 FROM json_each(:cap_names) jn WHERE jn.value = c.name
          )
),

budget AS (
    SELECT
        sb.substrate,
        sb.wip_cap,
        sb.stale_threshold_days,
        COUNT(p.id) AS wip_current,
        sb.wip_cap - COUNT(p.id) AS wip_remaining,
        CASE WHEN COUNT(p.id) >= sb.wip_cap THEN 1 ELSE 0 END AS at_cap
    FROM substrate_budget sb
    LEFT JOIN plan p
           ON p.space_id = sb.space_id
          AND p.substrate = sb.substrate
          AND p.status   = 'Dispatched'
    WHERE sb.space_id = :space_id
      AND sb.substrate = :substrate
    GROUP BY sb.substrate, sb.wip_cap, sb.stale_threshold_days
),

bottlenecks AS (
    SELECT
        p.id, p.title, p.status, p.blocked_reason,
        p.dispatched_at, p.updated_at,
        CASE WHEN p.dispatched_at IS NOT NULL
             THEN (unixepoch('now') * 1000 - p.dispatched_at) / 86400000
             ELSE NULL END AS days_in_flight
    FROM plan p
    WHERE p.space_id = :space_id
      AND p.substrate = :substrate
      AND p.status IN ('Dispatched', 'Blocked')
    ORDER BY p.dispatched_at ASC
    LIMIT 3
),

drift_hints AS (
    SELECT
        p.id, p.title, p.status, p.updated_at,
        (unixepoch('now') * 1000 - p.updated_at) / 86400000 AS days_stale
    FROM plan p
    JOIN substrate_budget sb
           ON sb.space_id = p.space_id
          AND sb.substrate = p.substrate
    WHERE p.space_id = :space_id
      AND p.substrate = :substrate
      AND p.status NOT IN ('Done', 'Parked')
      AND ((unixepoch('now') * 1000 - p.updated_at) / 86400000)
              > sb.stale_threshold_days
),

intent_overlaps AS (
    SELECT
        i.id, i.heading AS title, i.substrate, i.source_kind, i.source_file,
        pr.name AS project_name
    FROM intent i
    LEFT JOIN project pr ON pr.id = i.project_id
    WHERE i.space_id = :space_id
      AND i.kind != 'operational'
      AND (
            i.substrate = :substrate
            OR i.heading LIKE '%' || :title || '%'
          )
    ORDER BY i.scope_depth DESC, i.updated_at DESC
    LIMIT 20
),

parked_ideas AS (
    SELECT
        p.id, p.title, p.parked_reason, p.updated_at
    FROM plan p
    WHERE p.space_id = :space_id
      AND p.status   = 'Parked'
      AND (
            p.substrate = :substrate
            OR p.title LIKE '%' || :title || '%'
          )
    ORDER BY p.updated_at DESC
    LIMIT 10
)

SELECT
    (SELECT json_group_array(json_object(
        'id', em.id, 'title', em.title, 'status', em.status,
        'substrate', em.substrate, 'match_kind', em.match_kind
    )) FROM existing_matches em) AS existing_matches,

    (SELECT json_object(
        'substrate',     b.substrate,
        'wip_cap',       b.wip_cap,
        'wip_current',   b.wip_current,
        'wip_remaining', b.wip_remaining,
        'at_cap',        b.at_cap
    ) FROM budget b LIMIT 1) AS budget,

    (SELECT json_group_array(json_object(
        'id', bn.id, 'title', bn.title, 'status', bn.status,
        'blocked_reason', bn.blocked_reason,
        'days_in_flight', bn.days_in_flight
    )) FROM bottlenecks bn) AS bottlenecks,

    (SELECT json_group_array(json_object(
        'id', dh.id, 'title', dh.title, 'status', dh.status,
        'days_stale', dh.days_stale
    )) FROM drift_hints dh) AS drift_hints,

    (SELECT json_group_array(json_object(
        'id', io.id, 'title', io.title, 'substrate', io.substrate,
        'source_kind', io.source_kind, 'project', io.project_name
    )) FROM intent_overlaps io) AS intent_overlaps,

    (SELECT json_group_array(json_object(
        'id', pk.id, 'title', pk.title, 'parked_reason', pk.parked_reason
    )) FROM parked_ideas pk) AS parked_ideas;
```

The query runs in one read transaction. The Rust wrapper deserializes each `serde_json::Value` column into `RawIntentCheckPayload`, and `g8-planner` finishes the job. All six payload sections come back regardless of input shape (empty arrays when no rows match), this is critical for downstream JSON consumers.

---

## 8. Store schema, full DDL (V1 migration)

This is the V1 migration that B3 ships at `crates/g8-store/src/migrations/V1__init.sql`.

```sql
-- ============================================================
-- g8-store schema v1
-- Applied via refinery on `g8 init` / first store open
-- ============================================================

CREATE TABLE IF NOT EXISTS convergence_space (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    root_path   TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS project (
    id          TEXT PRIMARY KEY,
    space_id    TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    root_path   TEXT NOT NULL,
    language    TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS capability (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    substrate       TEXT,
    consumes        TEXT NOT NULL DEFAULT '[]',     -- JSON array
    produces        TEXT NOT NULL DEFAULT '[]',     -- JSON array
    status          TEXT NOT NULL DEFAULT 'in_flight'
        CHECK (status IN ('proposed','in_flight','landed','stale','superseded','parked')),
    stub            INTEGER NOT NULL DEFAULT 0,
    stub_since      TEXT,                            -- ISO date YYYY-MM-DD
    owner_agent     TEXT,
    owner_team      TEXT,
    owner_contact   TEXT,
    file_path       TEXT,
    line_number     INTEGER,
    column_number   INTEGER,
    annotation_raw  TEXT,
    conditional     INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    meta            TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS intent (
    id            TEXT PRIMARY KEY,
    space_id      TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    project_id    TEXT          REFERENCES project(id) ON DELETE SET NULL,
    kind          TEXT NOT NULL
        CHECK (kind IN ('architectural_scope','boundary','operational','tech_stack','explicit_intent','unclassified')),
    heading       TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    scope_path    TEXT NOT NULL,
    scope_depth   INTEGER NOT NULL,
    substrate     TEXT,
    owner_agent   TEXT,
    owner_team    TEXT,
    owner_contact TEXT,
    source_file   TEXT NOT NULL,
    source_line   INTEGER,
    source_kind   TEXT NOT NULL
        CHECK (source_kind IN ('agents_md','claude_md','g8_sidecar','inline_comment','manual')),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    meta          TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS plan (
    id              TEXT PRIMARY KEY,
    space_id        TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    project_id      TEXT          REFERENCES project(id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    status          TEXT NOT NULL DEFAULT 'Idea'
        CHECK (status IN ('Idea','Scoped','Dispatched','Blocked','Done','Parked')),
    substrate       TEXT,
    wip_weight      INTEGER NOT NULL DEFAULT 1,
    parked_reason   TEXT,
    blocked_reason  TEXT,
    dispatched_at   INTEGER,
    completed_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    meta            TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS plan_capability (
    plan_id       TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    capability_id TEXT NOT NULL REFERENCES capability(id) ON DELETE CASCADE,
    overlap_kind  TEXT NOT NULL DEFAULT 'touches'
        CHECK (overlap_kind IN ('touches','extends','replaces','conflicts')),
    PRIMARY KEY (plan_id, capability_id)
) STRICT;

CREATE TABLE IF NOT EXISTS plan_intent (
    plan_id   TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    intent_id TEXT NOT NULL REFERENCES intent(id) ON DELETE CASCADE,
    relation  TEXT NOT NULL DEFAULT 'satisfies'
        CHECK (relation IN ('satisfies','contradicts','extends','derived_from')),
    PRIMARY KEY (plan_id, intent_id)
) STRICT;

CREATE TABLE IF NOT EXISTS plan_decision (
    plan_id     TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    decision_id TEXT NOT NULL REFERENCES decision(id) ON DELETE CASCADE,
    relation    TEXT NOT NULL DEFAULT 'governed_by'
        CHECK (relation IN ('governed_by','violates','supersedes')),
    PRIMARY KEY (plan_id, decision_id)
) STRICT;

CREATE TABLE IF NOT EXISTS decision (
    id          TEXT PRIMARY KEY,
    space_id    TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed','accepted','rejected','superseded','ruled_out')),
    context     TEXT,
    body        TEXT NOT NULL DEFAULT '',
    source_file TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS substrate_budget (
    space_id              TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    substrate             TEXT NOT NULL,
    wip_cap               INTEGER NOT NULL DEFAULT 3,
    stale_threshold_days  INTEGER NOT NULL DEFAULT 14,
    last_updated          INTEGER NOT NULL,
    PRIMARY KEY (space_id, substrate)
) STRICT;

CREATE TABLE IF NOT EXISTS capability_alias (
    canonical  TEXT NOT NULL,
    alias      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (canonical, alias)
) STRICT;

CREATE TABLE IF NOT EXISTS annotation (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    payload         TEXT NOT NULL,                 -- JSON: full Annotation.fields
    file_path       TEXT NOT NULL,
    line_number     INTEGER,
    column_number   INTEGER,
    source_lang     TEXT,
    source_kind     TEXT,
    conditional     INTEGER NOT NULL DEFAULT 0,
    extracted_at    INTEGER NOT NULL,
    meta            TEXT
) STRICT;

-- ============================================================
-- Indexes
-- ============================================================
CREATE INDEX IF NOT EXISTS idx_plan_space_status        ON plan(space_id, status);
CREATE INDEX IF NOT EXISTS idx_plan_project             ON plan(project_id);
CREATE INDEX IF NOT EXISTS idx_plan_substrate           ON plan(substrate);
CREATE INDEX IF NOT EXISTS idx_capability_project_name  ON capability(project_id, name);
CREATE INDEX IF NOT EXISTS idx_capability_substrate     ON capability(substrate);
CREATE INDEX IF NOT EXISTS idx_capability_status        ON capability(status);
CREATE INDEX IF NOT EXISTS idx_intent_space_scope       ON intent(space_id, scope_path, scope_depth);
CREATE INDEX IF NOT EXISTS idx_intent_substrate         ON intent(substrate);
CREATE INDEX IF NOT EXISTS idx_intent_kind              ON intent(kind);
CREATE INDEX IF NOT EXISTS idx_annotation_project_kind  ON annotation(project_id, kind);
CREATE INDEX IF NOT EXISTS idx_plan_capability_cap      ON plan_capability(capability_id);
CREATE INDEX IF NOT EXISTS idx_plan_intent_intent       ON plan_intent(intent_id);
CREATE INDEX IF NOT EXISTS idx_capability_alias_alias   ON capability_alias(alias);
```

**Notes for B3:**
- All tables use `STRICT` mode (SQLite ≥ 3.37) so type coercion is disabled.
- `JSON` columns (consumes, produces, payload, meta) are stored as `TEXT`; SQLite's `json_*` functions operate on them transparently.
- All timestamps are `INTEGER` epoch-milliseconds (matches `g8_core::EpochMillis`).
- Dates (`stub_since`) are ISO `YYYY-MM-DD` `TEXT`, `chrono::NaiveDate::format("%Y-%m-%d")` produces the canonical form.
- Foreign keys are enforced by `PRAGMA foreign_keys=ON` in `open()` (not a table-level setting).

**Refinery migration layout:**

```
crates/g8-store/src/migrations/
├── mod.rs               # refinery::embed_migrations!("src/migrations")
└── V1__init.sql         # the DDL above
```

B3 adds future migrations as `V2__<name>.sql`, `V3__<name>.sql`, etc. Refinery applies in name order; checksum verification prevents tampering.

---

## 9. CLI surface, full `clap` subcommand tree

This is the v0.1 LOCKED CLI. B4 implements this verbatim. Adding subcommands or flags is v0.2 work.

```
g8 [GLOBAL OPTIONS] <SUBCOMMAND>

GLOBAL OPTIONS
  --output <pretty|json>      Output mode (default: pretty)
  --quiet                     Suppress non-error output (still emits JSON when --output=json)
  --verbose                   Increase log verbosity (-v info, -vv debug, -vvv trace)
  --store <PATH>              Override .g8/store.db location (default: ./.g8/store.db)
  --space <ID>                Operate on this space (default: read from .g8/config.toml)
  --no-color                  Disable ANSI color in pretty output
  -h, --help                  Print help
  -V, --version               Print version

SUBCOMMANDS

  init                        Initialize g8 in the current directory
    --enforce                 Set enforcement = on (default: off; AUDIT-before-enforce)
    --install-pre-commit      Install .git/hooks/pre-commit calling `g8 check`
    --no-claude-import        Don't touch CLAUDE.md
    --no-subagents            Don't drop .claude/agents/*.md
    --name <STR>              Space name (default: directory basename)

  scan [PATH]                 Scan a directory tree and populate the store
    --project <NAME>          Override project name (default: from .g8/config.toml)
    --report-only             Print AUDIT report; don't write to store
    --no-watch                Don't enable file-watch after scan completes

  plan <SUBCOMMAND>
    new <TITLE>               Create a new plan; runs planner_intent_check
      --substrate <NAME>      Primary substrate for the plan
      --description <STR>     Plan description
      --touches <NAME>...     Capability names this plan touches (repeatable)
      --park                  Create directly as Parked
      --dispatch              Create as Dispatched (subject to WIP cap)
      --json                  Output FitReport as JSON (equiv: --output json)
    list                      List plans in the current space
      --status <STATUS>       Filter by status (repeatable)
      --substrate <NAME>      Filter by substrate
      --project <NAME>        Filter by project
      --parked                Shortcut for --status Parked
    show <ID>                 Show one plan in detail
    park <ID>                 Move plan to Parked
      --reason <STR>          Parked reason
    promote <ID>              Move parked plan to Scoped (re-runs planner)
    scope <ID>                Idea -> Scoped (runs planner)
    dispatch <ID>             Scoped -> Dispatched (WIP cap check)
    block <ID>                Dispatched -> Blocked
      --reason <STR>          Blocked reason
    unblock <ID>              Blocked -> Dispatched
    complete <ID>             Dispatched -> Done

  status                      Show INTENT_SUMMARY; regenerate .g8/INTENT_SUMMARY.md
    --no-regen                Don't regenerate the file (just print)
    --intra-conflicts         Include detected intra-space conflicts in output

  check                       Run pairing-gate; emit errors
    --json                    Equivalent to --output json
    --strict                  Fail on warnings as well as errors
    --project <NAME>          Check just this project (default: all)

  merge --from <PATH>         Cross-space conflict detection
    --from <PATH>             Path to remote space root (must contain .g8/)
    --json                    Equivalent to --output json
    --kind <KIND>...          Filter by conflict kind (repeatable)

  query <QUERY-DSL>           Ad-hoc query against the store
                              (v0.1 supports a small DSL, see ARCHITECTURE.md §9.1)

  space <SUBCOMMAND>
    add <PATH>                Add a project to the current space
      --name <STR>            Override project name
    remove <PROJECT>          Remove a project from the space
    list                      List members of the current space

  link --canonical <REF> --alias <REF>
                              Declare cross-project capability identity
                              REF format: <project>::<name>

  substrate <SUBCOMMAND>
    add <NAME>                Register a substrate (Vocabulary Council pattern)
      --wip-cap <N>           Override default WIP cap (default: 3)
      --stale-days <N>        Override stale threshold (default: 14)
    list                      List substrates with budgets and current WIP

  serve                       Serve the live convergence-space web dashboard
      --port <N>              Bind port (default: 8799)
      --host <STR>            Bind host/interface (default: 127.0.0.1)
      --no-open               Don't open a browser on start
      --agentviz-url <URL>    AgentViz explorer base URL for the deep-link
                              (default: http://localhost:8090)
```

> `serve` is a post-v0.1 addition. It lives entirely at the binary edge
> (`g8::serve`, `tiny_http`) and reads the store read-only for the graph
> view; mutations route through the typed `g8-store` API so the lifecycle state
> machine and WIP gates apply identically to CLI and dashboard. The library
> crates (`g8-core/extractor/store/planner/conflict`) gain no HTTP dependency.
> See §9.2.

### 9.2 Live dashboard (`g8 serve`)

`g8 serve` boots a synchronous `tiny_http` server (no async runtime, fits the
dependency-minimalism principle) exposing:

| Route | Method | Purpose |
|---|---|---|
| `/` | GET | Embedded single-page dashboard (vanilla JS + canvas, no external assets) |
| `/api/health` | GET | `g8_version` + AgentViz deep-link base |
| `/api/graph` | GET | Convergence graph: `{nodes, edges, stats, space}` |
| `/api/scene` | GET | AgentViz `graph_explorer` scene (`?download=1` to save) |
| `/events` | GET | Server-Sent Events; emits on any `.g8/store.db` change |
| `/api/version` | GET | Monotonic change counter (SSE poll fallback) |
| `/api/plan` | POST | Create a plan (runs the planner fit-check) |
| `/api/plan/<id>/<action>` | POST | Lifecycle transition (`scope`/`dispatch`/`block`/…) |
| `/api/substrate` | POST | Register/update a WIP budget |

A `notify` watcher on the store directory bumps a version counter under a
`Condvar`; each SSE handler wakes on the bump and pushes an `update` event, so
every open tab, and any external `g8` CLI edit, converges on the same state
within a frame.

### 9.1 Query DSL (minimal v0.1)

`g8 query <STR>` accepts a tiny DSL, enough to support the four use cases without exposing SQL injection or SQLite's full surface to end users.

Supported forms:

| Form | Meaning |
|---|---|
| `plans:in_flight` | List all Plans with status In-flight |
| `plans:parked` | Same for Parked |
| `capabilities --substrate=auth` | All capabilities in substrate `auth` |
| `intents --path=src/auth` | All intents whose scope_path is or contains `src/auth` |
| `bottlenecks --substrate=auth` | Bottleneck plans (Dispatched + Blocked) for substrate |
| `drift --substrate=auth` | Drift hints for substrate |
| `parked --like=streaming` | Parked plans whose title matches LIKE pattern |

Each query has a canonical SQL behind it; the DSL is essentially a typed dispatch table to those queries. The DSL is NOT raw SQL; it's intentionally limited (per the no-MCP, no-LLM, deterministic-zone principles).

---

## 10. `.g8/` directory layout convention

Every project rooted by `g8` has this exact layout:

```
.g8/
├── config.toml              # local-project config (NOT a space.toml; see below)
├── space.toml               # ONLY when this directory is a space root
├── store.db                 # SQLite store (project-scoped or space-scoped)
├── store.db-wal             # SQLite WAL sidecar (auto-managed)
├── store.db-shm             # SQLite shared-memory sidecar (auto-managed)
├── INTENT_SUMMARY.md        # auto-regenerated by `g8 status` / `g8 scan`
└── intents/                 # optional sidecar storage for *.g8.md files
    └── (user-managed)
```

### 10.1 `.g8/config.toml` shape

```toml
[g8]
version       = "0.1.0"
enforcement   = "off"        # "off" | "on"
project_name  = "checkout-api"
project_id    = "ABC123..."  # nanoid, generated at init
space_id      = "DEF456..."  # nanoid; same as in space.toml if present

[extractor]
languages     = ["rust", "typescript", "python", "go"]
ignore_paths  = []           # additional glob patterns beyond gitignore
ast_grep_bin  = "ast-grep"   # override binary path

[ui]
default_output = "pretty"
```

### 10.2 `.g8/space.toml` shape

(Already specified in SPEC.md Decision 3; reproduced here for builder reference.)

```toml
[space]
id          = "DEF456..."     # nanoid; generated at `g8 init` if space root
name        = "platform"
description = ""

[[members]]
name = "checkout-api"
root = "../checkout-api"

[[members]]
name = "render-kit"
root = "../render-kit"

[substrate_budgets]
"export-pipeline" = { wip_cap = 5, stale_threshold_days = 14 }
"auth-layer"      = { wip_cap = 2, stale_threshold_days = 21 }

[[links.aliases]]
canonical = "checkout-api::http-fetch"
alias     = "render-kit::http-fetch"
```

### 10.3 Single-repo vs space-rooted

A directory containing `.g8/space.toml` IS a space root. The store at `.g8/store.db` holds space-scoped entities (Decisions, Intents, SubstrateBudgets, Plans). Each member project has its own `.g8/store.db` for its Capabilities; the planner ATTACHes member stores at query time.

A directory containing `.g8/config.toml` but NOT `.g8/space.toml` is a single-project repo. Its `.g8/store.db` holds everything (capabilities AND a single-project ConvergenceSpace).

Detection logic (in `g8`):

```rust
fn resolve_space_root(start: &Path) -> Result<PathBuf, _> {
    // Walk upward from `start` looking for `.g8/space.toml` first,
    // falling back to `.g8/config.toml`. The directory containing the
    // first match is the space root.
}
```

### 10.4 `INTENT_SUMMARY.md` template

Written by `g8 status`. Exact template (this IS the contract, downstream Claude Code sessions read this):

<!-- credo-lint:allow-fenced AUTO-GENERATED is the do-not-edit marker the extractor keys on, not an AI disclosure -->
```markdown
<!-- AUTO-GENERATED by g8 v0.1.0 at {ISO8601_TIMESTAMP} -->
<!-- Do not edit by hand. Regenerated on each `g8 scan` and `g8 status`. -->

# G8 Intent Summary

## Space
Name: {space_name}
Root: {space_root_path}
Members: {N} project(s)

## In-Flight Plans
{table-or-none: id | title | status | substrate | wip}

## WIP Cap Status
{table: substrate | active | cap | available}

## Parked Ideas (top 5 by recency)
{list-or-none}

## Stale Promotion Proposals
{list-or-none: plans in Dispatched > stale_threshold_days}

## Top Architectural Intents
{list-or-none: heading | scope_path | owner_agent}

## Boundary Alerts
{list-or-none: detected by g8-conflict::detect_intra_space}

## Decisions (recent 5 accepted)
{list-or-none}
```

`g8 status --no-regen` prints the same content to stdout but doesn't touch the file.

---

## 11. Non-deterministic seam (out of scope v0.1)

The future `g8-plan-draft` crate (NOT in v0.1):

```rust
// crates/g8-plan-draft/src/lib.rs (v0.2+)
pub struct PlanDrafter {
    llm_client: Box<dyn LlmClient>,
}

impl PlanDrafter {
    /// Convert a free-text feature request into a structured PlanDraft.
    /// The LLM never sees the FitReport; it only generates the draft.
    /// All decision logic stays in g8-planner.
    pub async fn draft_from_request(
        &self,
        request: &str,
        context: &PlanContext,
    ) -> Result<PlanDraft, DraftError>;
}
```

`PlanDraft` is the single in-edge to the deterministic zone. The CLI command `g8 plan new --from-text "I want to add..."` in v0.2 would call `PlanDrafter::draft_from_request` → `Planner::plan_check` → format. The boundary is enforced by the type system: `PlanDraft` is in `g8-core`; nothing in `g8-core` knows what an LLM is.

---

## 12. Tracing, span inventory and field schema

Every crate that does work opens spans. Field naming is consistent so a trace consumer (Jaeger, Honeycomb, or `tracing-subscriber` JSON) can correlate.

| Span | Crate | Fields | When |
|---|---|---|---|
| `cli.command` | `g8` | `command`, `args`, `space_id`, `duration_ms` | wraps entire CLI invocation |
| `extractor.scan_dir` | `g8-extractor` | `root`, `files_scanned`, `annotations_found`, `duration_ms` | per `Extractor::scan_dir` |
| `extractor.ast_grep` | `g8-extractor` | `rule_file`, `target`, `exit_code`, `duration_ms` | per subprocess |
| `extractor.parse_markdown` | `g8-extractor` | `file`, `sections_found` | per AGENTS.md/CLAUDE.md |
| `store.query` | `g8-store` | `query_name`, `rows_returned`, `duration_ms` | per Surrealql/sql call |
| `store.apply_scan` | `g8-store` | `project_id`, `inserted`, `deleted`, `duration_ms` | per `apply_scan` |
| `store.planner_intent_check` | `g8-store` | `space_id`, `substrate`, `cap_names_count`, `duration_ms` | per composite query |
| `planner.plan_check` | `g8-planner` | `space_id`, `recommendation`, `duration_ms` | per `Planner::plan_check` |
| `conflict.compare_spaces` | `g8-conflict` | `local_space`, `remote_space`, `conflicts_found` | per `compare_spaces` |
| `obligations.run_all` | `g8-obligations` | `artifact_version`, `obligations_count`, `duration_ms` | per `run_obligations` (the `g8 check` obligations self-audit, `specs/obligation-checker-contract.md` §4.1/§5) |

Initialization in `g8::main`:

```rust
tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::from_default_env().add_directive("g8=info".parse()?))
    .with_writer(std::io::stderr)        // never stdout (would break JSON mode)
    .with_target(false)
    .compact()
    .init();
```

JSON-format traces are exported when `G8_TRACE_JSON=1` env var is set; otherwise compact human format on stderr.

---

## 13. Adaptive substrate budget, v0.2 plug seam

An adaptive formula:

```
budget = clamp(2 + drift_bonus + failing_bonus + time_bonus, MIN, MAX)
```

requires query inputs we don't ship in v0.1: `drift_count`, `failing_count`, `days_since_last_drift`. The v0.2 seam:

- Add `g8_store::AdaptiveBudgetInputs` struct with the three counts.
- Add `StoreConnection::adaptive_budget_inputs(&self, space, substrate) -> Result<AdaptiveBudgetInputs>`.
- Add `g8_core::budget::compute_adaptive_budget(inputs, MIN=2, MAX=20) -> u32`.
- Have `g8 scan` populate the inputs into `substrate_budget` rows.

V0.1 ships only the static `wip_cap` field on `substrate_budget`; v0.2 adds the dynamic computation without breaking the trait.

---

## 14. In-loop validator JSON contract (locked, §SPEC Decision 10)

Reproduced here for B4 reference:

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

`reason` is one of:
`no_matching_convergence_test | stub_expired | stub_missing_since | duplicate_capability_name | invalid_root | invalid_checker_json | enforcement_disabled`

Exit codes:
- `0`, no errors (or enforcement_disabled)
- `1`, pairing errors found
- `2`, internal failure (store unreachable, malformed config, etc.)

The CLI writes JSON to stdout, human text to stderr, in both `--output pretty` and `--output json` modes for `g8 check` (the JSON contract is stable regardless of `--output` flag because external CI consumers need it).

---

## 15. Multi-project topology, cross-store query mechanics

Per SPEC Decision 3, multi-project queries use SQLite `ATTACH DATABASE`. The mechanics:

```rust
// Pseudocode for cross-space planner_intent_check
let mut local = RusqliteStore::open(&local_path)?;
for member in space.members {
    let member_db = member.root.join(".g8/store.db");
    local.attach_remote_store(&member.name, &member_db)?;
}

// Now SQL queries can reference attached DBs by alias:
//   SELECT * FROM main.plan UNION ALL SELECT * FROM checkout_api.plan ...
//
// The planner_intent_check query is rewritten to UNION across attached DBs
// when the space has > 1 member. Single-member spaces use the non-federated
// query for simplicity.
```

**Caveats (B3 must handle):**
- ATTACH requires the same SQLite version on both stores (bundled rusqlite guarantees this).
- The remote store opens in read-only mode by default for safety: `ATTACH DATABASE 'file:/path/store.db?mode=ro' AS checkout_api`.
- Maximum 10 attached DBs by default; documented limit.
- Detach on `g8` exit (defer-guard in CLI).

**Cross-project capability identity:** the `capability_alias` table (DDL §8) records canonical/alias pairs from `space.toml::links.aliases`. When `planner_intent_check` builds its `cap_names` list, it expands each name via `capability_alias` resolution before the JSON1 match. So `checkout-api::http-fetch` and `render-kit::http-fetch`, if aliased, are treated as one capability.

---

## 16. Subagent definition shipped by `g8 init`

`g8 init` writes `.claude/agents/g8-planner.md` (creating `.claude/agents/` if absent):

```markdown
---
name: g8-planner
description: Query the g8 intent store before any plan is finalized. Surfaces overlapping in-flight plans, WIP-cap violations, parked-idea matches, and boundary violations. Pre-PRD coordination across the codebase.
tools: Bash, Read
model: haiku
---
You are a plan-coordination specialist. Your role is to query the local
g8 store BEFORE any implementation plan is finalized.

On any planning request:
1. Run `g8 plan new "<title>" --substrate "<substrate>" --json` and parse the FitReport.
2. If `recommendation` is `Drop`, `Park`, `Wait`, or `Pivot`, BLOCK the plan and surface:
   - the existing matches that triggered the recommendation
   - the budget status if at cap
   - the bottleneck plans if relevant
3. If `recommendation` is `Extend`, suggest extending the matching plan and surface its ID.
4. If `recommendation` is `Rename`, surface the title-collision plan.
5. If `recommendation` is `Proceed`, summarize the clear path and emit PLAN_APPROVED.

You are read-only. You do not modify code or annotate files. You do not call any
g8 subcommand other than `g8 plan new` (which is itself read-only when invoked
without `--dispatch`).
```

B4 ships this content as a `const SUBAGENT_PLANNER: &str` baked into the binary; `g8 init` writes it byte-identical.

---

## 17. Builder-by-builder acceptance criteria



**B1, g8-core:** `cargo check -p g8-core` green. Every public type has a doctest. `parse_annotation_grammar` round-trips through `serde_json` and produces stable output for the canonical fixtures.

**B2, g8-extractor:** `Extractor::new()` succeeds when `ast-grep` is on PATH; returns `BinaryNotFound` with install message otherwise. `scan_dir` on `crates/g8-extractor/tests/fixtures/` returns the expected `Vec<Annotation>` (insta snapshot or hand-checked). AGENTS.md / CLAUDE.md / `*.g8.md` parsing produces `Intent` annotations with correct `scope_depth`.

**B3, g8-store:** `RusqliteStore::open_in_memory()` + `migrate()` succeeds. All `StoreConnection` trait methods have implementations (no `unimplemented!()`). `planner_intent_check` returns a structurally-correct payload for an empty store (six empty JSON arrays/objects). Cross-store ATTACH smoke test passes.

**B4, g8:** every subcommand parses without panic. `--help` output matches §9 verbatim. `--output json` emits parseable JSON for every subcommand. `g8 init` + `g8 scan` + `g8 plan new` + `g8 status` round-trips on an empty directory.

**B5, g8-planner:** `Planner::plan_check` returns a `FitReport` whose `recommendation` matches the lookup table for known `DecisionInput` shapes (one unit test per lookup-table branch).

**B6, g8-conflict:** `compare_spaces` returns the expected `Vec<Conflict>` for hand-constructed pairs of stores. Each of the four conflict kinds has at least one positive test case.

---

## 18. Test strategy

- **Unit tests** live in each crate. Workspace `cargo test --workspace` is the smoke gate.
- **Integration fixtures** live in `crates/g8-extractor/tests/fixtures/` (sample repos with known annotations). B2 owns these.
- **End-to-end smoke** lives in `tests/e2e/` (workspace-root). I1 in Phase 3 writes these against checkout-api, render-kit, docs-hub.
- **Insta snapshots** for CLI output (pretty and JSON). One snapshot per subcommand × output mode.

---

**End of ARCHITECTURE.md.** When in conflict with SPEC.md, ARCHITECTURE.md is the technical truth; SPEC.md is the product truth. Builders should read both, then implement.
