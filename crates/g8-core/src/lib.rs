//! `g8-core` — canonical data model for the G8.
//!
//! This crate is the root of the g8 crate DAG. It owns all shared types;
//! every other workspace crate consumes them. No I/O, no tokio, no rusqlite.
//!
//! # Modules
//! - [`ids`] — typed ID newtypes (nanoid-backed)
//! - [`model`] — domain structs: ConvergenceSpace, Project, Capability, Intent, Plan, Decision
//! - [`annotation`] — Annotation, AnnotationKind, SourceLocation, RawAnnotation
//! - [`plan`] — PlanStatus, PlanDraft, FitReport, Recommendation, and free functions
//! - [`conflict`] — Conflict, ConflictKind, Severity, Evidence
//! - [`contract`] — CheckPayload, CheckErrorItem, PairingReason (`g8 check --json`)
//! - [`substrate`] — SubstrateName, SubstrateBudget
//! - [`owner`] — Owner
//! - [`error`] — G8Error top-level enum
//! - [`time`] — EpochMillis newtype + helpers

pub mod annotation;
pub mod conflict;
pub mod contract;
pub mod error;
pub mod ids;
pub mod model;
pub mod owner;
pub mod plan;
pub mod substrate;
pub mod time;

// ── Flat re-exports of all public types ─────────────────────────────────────

pub use annotation::{Annotation, AnnotationKind, AnnotationValue, RawAnnotation, SourceLocation};
pub use conflict::{Conflict, ConflictKind, Evidence, Severity};
pub use contract::{CheckErrorItem, CheckPayload, PairingReason};
pub use error::G8Error;
pub use ids::{AnnotationId, CapabilityId, DecisionId, IntentId, PlanId, ProjectId, SpaceId};
pub use model::{
    Capability, CapabilityStatus, ConvergenceSpace, Decision, DecisionStatus, Intent, IntentKind,
    IntentSourceKind, Plan, Project,
};
pub use owner::Owner;
pub use plan::{
    BottleneckPlan, BudgetStatus, DecisionInput, DriftHint, ExistingMatch, FitReport,
    IntentOverlap, MatchKind, OverlapKind, ParkedIdeaMatch, PlanDraft, PlanIntentRelation,
    PlanStatus, Recommendation,
};
pub use substrate::SubstrateBudget;
pub use time::EpochMillis;

// ── Free functions ───────────────────────────────────────────────────────────

/// Returns the current wall-clock time as epoch milliseconds.
///
/// # Examples
///
/// ```
/// let ms = g8_core::now_millis();
/// // It should be a reasonably large number (post-2020 epoch).
/// assert!(ms.as_i64() > 1_600_000_000_000);
/// ```
pub fn now_millis() -> EpochMillis {
    time::now_millis()
}

/// Parse a magic-comment annotation string into a [`RawAnnotation`].
///
/// Accepts the `@g8.<kind>(key = "value", ...)` form as produced by the
/// extractor after stripping comment prefixes (`//` or `#`).  Whitespace
/// between tokens is ignored; multi-line forms should be concatenated by the
/// caller before parsing.
///
/// # Examples
///
/// ```
/// use g8_core::parse_annotation_grammar;
///
/// // Single-line capability annotation
/// let raw = parse_annotation_grammar(
///     r#"@g8.capability(name = "http-fetch", status = "in_flight", substrate = "net")"#
/// ).unwrap();
/// assert_eq!(raw.kind_str, "capability");
/// assert!(raw.fields.contains_key("name"));
///
/// // Multi-line form (caller concatenates with spaces after stripping // prefix)
/// let multi = parse_annotation_grammar(
///     r#"@g8.capability(name = "claim-validation", consumes = ["Claim", "Source"], produces = ["Event"])"#
/// ).unwrap();
/// let produces = multi.fields.get("produces").unwrap();
/// let json = serde_json::to_string(produces).unwrap();
/// assert!(json.contains("Event"));
/// ```
pub fn parse_annotation_grammar(raw: &str) -> Result<RawAnnotation, G8Error> {
    annotation::parse_annotation_grammar(raw)
}

/// Derive a [`Recommendation`] from a [`DecisionInput`] via a fixed lookup table.
///
/// The lookup table is deterministic: same input always yields same output.
/// Order matters — first matching branch wins.
///
/// # Examples
///
/// ```
/// use g8_core::{recommend, DecisionInput, Recommendation};
///
/// let proceed = recommend(&DecisionInput {
///     any_over_budget:  false,
///     has_exact_match:  false,
///     near_match_count: 0,
///     has_parked_match: false,
///     bottleneck_count: 0,
/// });
/// assert_eq!(proceed, Recommendation::Proceed);
///
/// let drop = recommend(&DecisionInput {
///     any_over_budget:  false,
///     has_exact_match:  true,
///     near_match_count: 1,
///     has_parked_match: false,
///     bottleneck_count: 0,
/// });
/// assert_eq!(drop, Recommendation::Drop);
/// ```
pub fn recommend(input: &DecisionInput) -> Recommendation {
    plan::recommend(input)
}

/// Validate a [`PlanStatus`] transition, returning an error if the transition
/// is illegal per the plan lifecycle graph.
///
/// Legal transitions:
/// - `Idea` → `Scoped | Parked`
/// - `Scoped` → `Dispatched | Parked`
/// - `Dispatched` → `Blocked | Done | Parked`
/// - `Blocked` → `Dispatched | Parked`
/// - `Parked` → `Scoped`
/// - `Done` — terminal; no outgoing edges
///
/// # Examples
///
/// ```
/// use g8_core::{validate_status_transition, PlanStatus};
///
/// // Legal transition
/// assert!(validate_status_transition(PlanStatus::Idea, PlanStatus::Scoped).is_ok());
///
/// // Illegal: Done is terminal
/// assert!(validate_status_transition(PlanStatus::Done, PlanStatus::Idea).is_err());
/// ```
pub fn validate_status_transition(from: PlanStatus, to: PlanStatus) -> Result<(), G8Error> {
    plan::validate_status_transition(from, to)
}

/// Default WIP cap applied per-substrate when no override is configured.
pub const DEFAULT_WIP_CAP: u32 = 3;
