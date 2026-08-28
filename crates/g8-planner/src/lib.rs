//! `g8-planner` — composite-query orchestration and recommendation derivation.
//!
//! This crate is a **pure deterministic transformer** over [`StoreConnection`].
//! It calls **exactly one** store method — [`StoreConnection::planner_intent_check`]
//! — and post-processes the six JSON payloads it receives into a structured
//! [`FitReport`] with a deterministic [`Recommendation`].
//!
//! # Design constraints (from ARCHITECTURE.md §5 and §11)
//!
//! - **No LLM calls.** No async runtime. No HTTP. No direct file I/O.
//! - **One store call per `plan_check` invocation.**  The composite-query
//!   principle (SPEC §3 principle 2) forbids verb chains; all SQL work is done
//!   inside `planner_intent_check` on the store side.
//! - **Deterministic.** Same `PlanDraft` + same store contents → same `FitReport`
//!   and `Recommendation` every time.
//!
//! # ARCHITECTURE_GAP
//!
//! `ARCHITECTURE.md §3.4` specifies `pub fn plan_check(store: &dyn StoreConnection, …)`.
//! However, `StoreConnection` contains a generic method (`watch<F>`) which makes
//! it object-unsafe (not dyn-compatible).  The equivalent and idiomatic Rust fix
//! is to accept `store: &S where S: StoreConnection`, which is what this crate
//! implements.  All call sites are unaffected.
//!
//! # Example
//!
//! ```no_run
//! use g8_planner::Planner;
//! use g8_core::{plan::PlanDraft, SpaceId};
//! use g8_store::RusqliteStore;
//!
//! // In production the CLI opens a RusqliteStore and passes it here.
//! let mut store = RusqliteStore::open_in_memory().unwrap();
//! store.migrate().unwrap();
//!
//! let draft = PlanDraft {
//!     title: "streaming-export".into(),
//!     description: None,
//!     substrate: Some("export-pipeline".into()),
//!     touched_capabilities: vec![],
//!     touched_paths: vec![],
//!     space_id: SpaceId::derive(&["docs", "space"]),
//!     project_id: None,
//! };
//! let report = Planner::plan_check(&store, &draft).unwrap();
//! assert_eq!(report.recommendation, g8_core::Recommendation::Proceed);
//! ```

use g8_core::{
    plan::{
        BottleneckPlan, BudgetStatus, DecisionInput, DriftHint, ExistingMatch, FitReport,
        IntentOverlap, ParkedIdeaMatch, PlanDraft,
    },
    recommend, G8Error,
};
use g8_store::{StoreConnection, StoreError};
use tracing::instrument;

// ── Error ────────────────────────────────────────────────────────────────────

/// Errors produced by [`Planner::plan_check`].
///
/// The three variants map to the three failure modes:
/// - The underlying store query failed.
/// - Deserialization of a JSON payload column failed (store contract violation).
/// - The `PlanDraft` itself is structurally invalid before the query runs.
#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    /// An error propagated from the store layer.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// An `g8-core` error (e.g. a deserialized value failed a type contract).
    #[error(transparent)]
    Core(#[from] G8Error),
    /// The draft is malformed and cannot be planned.
    #[error("invalid draft: {0}")]
    InvalidDraft(String),
}

// ── Planner ──────────────────────────────────────────────────────────────────

/// Stateless planner that turns a [`PlanDraft`] into a [`FitReport`].
///
/// `Planner` is a unit struct; all behaviour is on the associated function
/// [`Planner::plan_check`].  There is no state to initialise.
///
/// # Note on the store parameter
///
/// `plan_check` accepts `store: &S where S: StoreConnection`.
/// `ARCHITECTURE.md §3.4` specifies `&dyn StoreConnection`, but
/// `StoreConnection` is not object-safe (it contains a generic `watch<F>`
/// method).  The generic-parameter form is semantically identical at all
/// call sites.  See ARCHITECTURE_GAP in the crate docs.
pub struct Planner;

impl Planner {
    /// Evaluate a [`PlanDraft`] against the store and return a [`FitReport`].
    ///
    /// This function makes **exactly one** store call:
    /// [`StoreConnection::planner_intent_check`].  All other work is pure
    /// in-memory deserialization and the deterministic `recommend` lookup.
    ///
    /// # Tracing
    ///
    /// Opens a span `planner.plan_check` with fields:
    /// - `space_id` — from the draft
    /// - `recommendation` — emitted on span exit
    ///
    /// # Errors
    ///
    /// Returns [`PlannerError::Store`] if the composite query fails.
    /// Returns [`PlannerError::InvalidDraft`] if `draft.title` is empty or
    /// whitespace-only.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use g8_planner::Planner;
    /// use g8_core::{plan::PlanDraft, SpaceId, Recommendation};
    /// use g8_store::RusqliteStore;
    ///
    /// let mut store = RusqliteStore::open_in_memory().unwrap();
    /// store.migrate().unwrap();
    ///
    /// let draft = PlanDraft {
    ///     title: "new-feature".into(),
    ///     description: None,
    ///     substrate: None,
    ///     touched_capabilities: vec![],
    ///     touched_paths: vec![],
    ///     space_id: SpaceId::derive(&["docs", "space"]),
    ///     project_id: None,
    /// };
    /// let report = Planner::plan_check(&store, &draft).unwrap();
    /// assert_eq!(report.recommendation, Recommendation::Proceed);
    /// ```
    #[instrument(
        name = "planner.plan_check",
        skip(store, draft),
        fields(
            space_id = %draft.space_id.as_str(),
            recommendation = tracing::field::Empty,
        )
    )]
    pub fn plan_check<S>(store: &S, draft: &PlanDraft) -> Result<FitReport, PlannerError>
    where
        S: StoreConnection,
    {
        // ── Input validation ───────────────────────────────────────────────
        if draft.title.trim().is_empty() {
            return Err(PlannerError::InvalidDraft(
                "plan title must not be empty".into(),
            ));
        }

        // ── SINGLE composite store call ────────────────────────────────────
        // This is the only store call in the entire function.  The composite
        // query returns six JSON columns in one read transaction.
        let raw = store.planner_intent_check(draft)?;

        // ── Deserialize six JSON payloads ──────────────────────────────────
        let existing_matches: Vec<ExistingMatch> =
            deserialize_array(raw.existing_matches, "existing_matches")?;
        let budget_status: Option<BudgetStatus> = deserialize_optional(raw.budget, "budget")?;
        let bottlenecks: Vec<BottleneckPlan> = deserialize_array(raw.bottlenecks, "bottlenecks")?;
        let drift_hints: Vec<DriftHint> = deserialize_array(raw.drift_hints, "drift_hints")?;
        let intent_overlaps: Vec<IntentOverlap> =
            deserialize_array(raw.intent_overlaps, "intent_overlaps")?;
        let parked_ideas: Vec<ParkedIdeaMatch> =
            deserialize_array(raw.parked_ideas, "parked_ideas")?;

        // ── Build DecisionInput from deserialized data ─────────────────────
        let decision_input = DecisionInput {
            any_over_budget: budget_status.as_ref().map(|b| b.at_cap).unwrap_or(false),
            has_exact_match: existing_matches.iter().any(|m| m.title == draft.title),
            near_match_count: existing_matches.len() as u32,
            has_parked_match: !parked_ideas.is_empty(),
            bottleneck_count: bottlenecks.len() as u32,
        };

        // ── Derive recommendation (pure lookup table) ──────────────────────
        let recommendation = recommend(&decision_input);

        // Record the recommendation on the tracing span for observability.
        tracing::Span::current().record("recommendation", format!("{recommendation:?}").as_str());

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
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Deserialize a JSON value that must be an array (possibly empty).
///
/// `json_group_array(...)` from SQLite returns `NULL` when there are no rows
/// and a JSON array string otherwise.  We normalise both cases to `Vec<T>`.
fn deserialize_array<T>(value: serde_json::Value, field: &str) -> Result<Vec<T>, PlannerError>
where
    T: serde::de::DeserializeOwned,
{
    match value {
        serde_json::Value::Null => Ok(vec![]),
        other => serde_json::from_value::<Vec<T>>(other).map_err(|e| deserialize_err(field, e)),
    }
}

/// Deserialize a JSON value that represents an optional object.
///
/// `json_object(...)` from SQLite returns `NULL` when there are no matching
/// budget rows; we convert that to `None`.
fn deserialize_optional<T>(value: serde_json::Value, field: &str) -> Result<Option<T>, PlannerError>
where
    T: serde::de::DeserializeOwned,
{
    match value {
        serde_json::Value::Null => Ok(None),
        other => serde_json::from_value::<T>(other)
            .map(Some)
            .map_err(|e| deserialize_err(field, e)),
    }
}

/// Convert a `serde_json::Error` into a `PlannerError` with a diagnostic
/// message that includes the field name.
///
/// We roundtrip through a synthetic invalid JSON string to obtain a
/// `serde_json::Error` carrying our message, then wrap it in `StoreError::Serde`.
fn deserialize_err(field: &str, e: serde_json::Error) -> PlannerError {
    let msg = format!("planner: failed to deserialize '{field}': {e}");
    // serde_json::from_str on invalid JSON always produces an Err.
    let inner: serde_json::Error = serde_json::from_str::<()>(&msg).unwrap_err();
    PlannerError::Store(StoreError::Serde(inner))
}
