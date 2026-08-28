//! Core domain structs: ConvergenceSpace, Project, Capability, Intent, Decision, Plan.
//!
//! These are the persisted entities.  Annotation is the *transient* extractor
//! output (see [`crate::annotation`]) that the store converts into these rows.

use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{
    annotation::SourceLocation,
    ids::{CapabilityId, DecisionId, IntentId, PlanId, ProjectId, SpaceId},
    owner::Owner,
    plan::PlanStatus,
    time::EpochMillis,
};

/// A ConvergenceSpace groups one or more projects for federated queries.
///
/// # Examples
///
/// ```
/// use g8_core::{ConvergenceSpace, SpaceId, EpochMillis};
/// use std::path::PathBuf;
///
/// let space = ConvergenceSpace {
///     id:         SpaceId::derive(&["docs", "space"]),
///     name:       "platform".into(),
///     root_path:  PathBuf::from("/home/user/platform"),
///     created_at: EpochMillis::from_i64(1_700_000_000_000),
///     updated_at: EpochMillis::from_i64(1_700_000_000_000),
///     members:    vec![],
///     meta:       None,
/// };
/// assert_eq!(space.name, "platform");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceSpace {
    pub id: SpaceId,
    pub name: String,
    /// Absolute, canonicalized path to the space root directory.
    pub root_path: PathBuf,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// ProjectIds that are members of this space; populated by the store loader.
    pub members: Vec<ProjectId>,
    /// Arbitrary extensibility bag — serde_json::Value for v0.1 flexibility.
    pub meta: Option<serde_json::Value>,
}

/// A single source-controlled directory tree within a space.
///
/// # Examples
///
/// ```
/// use g8_core::{Project, ProjectId, SpaceId, EpochMillis};
/// use std::path::PathBuf;
///
/// let proj = Project {
///     id:          ProjectId::derive(&["docs", "project"]),
///     space_id:    SpaceId::derive(&["docs", "space"]),
///     name:        "checkout-api".into(),
///     description: Some("News ranking service".into()),
///     root_path:   PathBuf::from("/home/user/checkout-api"),
///     language:    Some("rust".into()),
///     created_at:  EpochMillis::from_i64(1_700_000_000_000),
///     updated_at:  EpochMillis::from_i64(1_700_000_000_000),
///     meta:        None,
/// };
/// assert_eq!(proj.name, "checkout-api");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub space_id: SpaceId,
    /// Human-readable label; not globally unique (unique within a space).
    pub name: String,
    pub description: Option<String>,
    /// Absolute, canonicalized path to the project root.
    pub root_path: PathBuf,
    /// Detected primary language: `"rust"` | `"ts"` | `"py"` | `"go"` |
    /// `"mixed"` | or an arbitrary unknown value.
    pub language: Option<String>,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// Arbitrary extensibility bag — serde_json::Value for v0.1 flexibility.
    pub meta: Option<serde_json::Value>,
}

/// A typed unit of behavior declared by a `// @g8.capability(...)` annotation.
///
/// Capabilities are project-scoped; the same kebab-case name in two projects
/// is two distinct capabilities unless an alias resolves them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    pub project_id: ProjectId,
    /// Kebab-case name; project-scoped (per SPEC Decision 3).
    pub name: String,
    pub description: Option<String>,
    /// Substrate this capability belongs to (string label, not a foreign key).
    pub substrate: Option<String>,
    /// Free-form substrate-typed identifiers of things this capability consumes.
    pub consumes: Vec<String>,
    /// Free-form substrate-typed identifiers of things this capability produces.
    pub produces: Vec<String>,
    pub status: CapabilityStatus,
    /// `true` when the annotation carries `stub = true`.
    pub stub: bool,
    /// ISO date from which the stub exemption was granted (`since` field).
    pub stub_since: Option<NaiveDate>,
    pub owner: Option<Owner>,
    /// Where in source the annotation was found.
    pub source: SourceLocation,
    /// The raw annotation string (for debugging / round-trip).
    pub annotation_raw: String,
    /// Set when extractor flagged `dev_only=true`, `cfg_attr`, or `tests/` dir.
    pub conditional: bool,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// Arbitrary extensibility bag — serde_json::Value for v0.1 flexibility.
    pub meta: Option<serde_json::Value>,
}

/// Six-state lifecycle for a capability (SPEC Decision 2).
///
/// # Examples
///
/// ```
/// use g8_core::CapabilityStatus;
///
/// let s = CapabilityStatus::InFlight;
/// assert!(matches!(s, CapabilityStatus::InFlight));
/// let json = serde_json::to_string(&s).unwrap();
/// assert_eq!(json, r#""in_flight""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Proposed,
    InFlight,
    Landed,
    Stale,
    Superseded,
    Parked,
}

/// A directional statement about what a directory or file is *for*.
///
/// Parsed from AGENTS.md, CLAUDE.md, or `*.g8.md` sidecar files. Lives at
/// directory level; Capability lives at symbol level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub id: IntentId,
    pub space_id: SpaceId,
    /// `None` when the intent attaches to the space root (not a specific project).
    pub project_id: Option<ProjectId>,
    pub kind: IntentKind,
    /// Heading text from the Markdown section.
    pub heading: String,
    /// Body text of the section.
    pub description: String,
    /// Directory path this intent applies to.
    pub scope_path: PathBuf,
    /// Directory depth from space root; higher = more specific = higher precedence.
    pub scope_depth: u32,
    pub substrate: Option<String>,
    pub owner: Option<Owner>,
    pub source: SourceLocation,
    pub source_kind: IntentSourceKind,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// Arbitrary extensibility bag — serde_json::Value for v0.1 flexibility.
    pub meta: Option<serde_json::Value>,
}

/// Classifies an intent by the Markdown section heading it was parsed from.
///
/// # Examples
///
/// ```
/// use g8_core::IntentKind;
///
/// let k = IntentKind::Boundary;
/// assert!(matches!(k, IntentKind::Boundary));
/// let json = serde_json::to_string(&k).unwrap();
/// assert_eq!(json, r#""boundary""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// `# Architecture`, `# Design`, `# Overview`
    ArchitecturalScope,
    /// `# Boundaries`, `# Never do`, `# Constraints`
    Boundary,
    /// `# Commands`, `# Build`, `# Test`, `# Testing` — deprioritized in planner
    Operational,
    /// `# Stack`, `# Dependencies`
    TechStack,
    /// `# Intent` — g8-native explicit section
    ExplicitIntent,
    /// Any heading not matching a known pattern
    Unclassified,
}

/// Which file type was the source of an intent annotation.
///
/// # Examples
///
/// ```
/// use g8_core::IntentSourceKind;
///
/// let k = IntentSourceKind::AgentsMd;
/// assert!(matches!(k, IntentSourceKind::AgentsMd));
/// let json = serde_json::to_string(&k).unwrap();
/// assert_eq!(json, r#""agents_md""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSourceKind {
    AgentsMd,
    ClaudeMd,
    G8Sidecar,
    InlineComment,
    Manual,
}

/// An architectural decision record stored in the space.
///
/// Decisions are space-scoped (per the data-model table in ARCHITECTURE.md §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub space_id: SpaceId,
    pub title: String,
    pub status: DecisionStatus,
    /// The situation or problem statement (context for the decision).
    pub context: Option<String>,
    /// Body text — the decision, rationale, and consequences.
    pub body: String,
    /// Source file that produced this decision record (e.g. an AGENTS.md).
    pub source_file: Option<PathBuf>,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// Arbitrary extensibility bag — serde_json::Value for v0.1 flexibility.
    pub meta: Option<serde_json::Value>,
}

/// Lifecycle state for a decision record.
///
/// # Examples
///
/// ```
/// use g8_core::DecisionStatus;
///
/// let s = DecisionStatus::Accepted;
/// assert!(matches!(s, DecisionStatus::Accepted));
/// let json = serde_json::to_string(&s).unwrap();
/// assert_eq!(json, r#""accepted""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Rejected,
    Superseded,
    RuledOut,
}

/// A unit of work tracked through the plan lifecycle.
///
/// # Examples
///
/// ```
/// use g8_core::{
///     model::Plan, PlanId, SpaceId, EpochMillis,
///     plan::PlanStatus,
/// };
///
/// let plan = Plan {
///     id:                     PlanId::derive(&["docs", "plan"]),
///     space_id:               SpaceId::derive(&["docs", "space"]),
///     project_id:             None,
///     title:                  "streaming-export".into(),
///     description:            Some("Add streaming export to the export pipeline".into()),
///     status:                 PlanStatus::Idea,
///     substrate:              Some("export-pipeline".into()),
///     touched_capabilities:   vec![],
///     derived_from_intents:   vec![],
///     governed_by_decisions:  vec![],
///     wip_weight:             1,
///     parked_reason:          None,
///     blocked_reason:         None,
///     dispatched_at:          None,
///     completed_at:           None,
///     created_at:             EpochMillis::from_i64(1_700_000_000_000),
///     updated_at:             EpochMillis::from_i64(1_700_000_000_000),
///     // Plan.meta: serde_json::Value chosen for v0.1 flexibility;
///     // typed PlanMeta with extra: HashMap deferred to v0.2
///     meta:                   None,
/// };
/// assert_eq!(plan.title, "streaming-export");
/// assert_eq!(plan.status, PlanStatus::Idea);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: PlanId,
    pub space_id: SpaceId,
    /// `None` for space-level plans not tied to a single project.
    pub project_id: Option<ProjectId>,
    pub title: String,
    pub description: Option<String>,
    pub status: PlanStatus,
    pub substrate: Option<String>,
    pub touched_capabilities: Vec<CapabilityId>,
    pub derived_from_intents: Vec<IntentId>,
    pub governed_by_decisions: Vec<DecisionId>,
    /// WIP weight: how many WIP-cap slots this plan consumes (almost always 1).
    pub wip_weight: u32,
    pub parked_reason: Option<String>,
    pub blocked_reason: Option<String>,
    pub dispatched_at: Option<EpochMillis>,
    pub completed_at: Option<EpochMillis>,
    pub created_at: EpochMillis,
    pub updated_at: EpochMillis,
    /// Plan.meta: serde_json::Value chosen for v0.1 flexibility;
    /// typed PlanMeta with extra: HashMap deferred to v0.2
    pub meta: Option<serde_json::Value>,
}
