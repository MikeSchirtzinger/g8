//! Plan lifecycle types: PlanStatus, PlanDraft, FitReport, Recommendation.
//!
//! Also contains the `recommend` lookup table and `validate_status_transition`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    error::G8Error,
    ids::{IntentId, PlanId, ProjectId, SpaceId},
};

// ── Plan status lifecycle ────────────────────────────────────────────────────

/// Six-state lifecycle for a plan (per SPEC Decision 2).
///
/// State machine:
/// ```text
/// Idea → Scoped → Dispatched → Done
///              ↘             ↗
///               → Blocked ──
///  (any) → Parked → Scoped
/// ```
///
/// # Examples
///
/// ```
/// use g8_core::PlanStatus;
///
/// let s = PlanStatus::Idea;
/// assert!(matches!(s, PlanStatus::Idea));
/// let json = serde_json::to_string(&s).unwrap();
/// assert_eq!(json, r#""Idea""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlanStatus {
    Idea,
    Scoped,
    Dispatched,
    Blocked,
    Done,
    Parked,
}

/// Validate whether a status transition is legal per the plan lifecycle graph.
///
/// Legal transitions:
/// - `Idea`       → `Scoped | Parked`
/// - `Scoped`     → `Dispatched | Parked`
/// - `Dispatched` → `Blocked | Done | Parked`
/// - `Blocked`    → `Dispatched | Parked`
/// - `Parked`     → `Scoped`
/// - `Done`       → (terminal; no outgoing edges)
pub fn validate_status_transition(from: PlanStatus, to: PlanStatus) -> Result<(), G8Error> {
    let legal = matches!(
        (from, to),
        (PlanStatus::Idea, PlanStatus::Scoped)
            | (PlanStatus::Idea, PlanStatus::Parked)
            | (PlanStatus::Scoped, PlanStatus::Dispatched)
            | (PlanStatus::Scoped, PlanStatus::Parked)
            | (PlanStatus::Dispatched, PlanStatus::Blocked)
            | (PlanStatus::Dispatched, PlanStatus::Done)
            | (PlanStatus::Dispatched, PlanStatus::Parked)
            | (PlanStatus::Blocked, PlanStatus::Dispatched)
            | (PlanStatus::Blocked, PlanStatus::Parked)
            | (PlanStatus::Parked, PlanStatus::Scoped)
    );

    if legal {
        Ok(())
    } else {
        Err(G8Error::InvalidStatusTransition { from, to })
    }
}

// ── PlanDraft ────────────────────────────────────────────────────────────────

/// The in-edge to the deterministic planning zone.
///
/// A `PlanDraft` is created by the CLI (or, in v0.2, by `g8-plan-draft`) and
/// passed unchanged to `g8-planner::Planner::plan_check`.  Nothing in the
/// deterministic zone ever knows how the draft was produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDraft {
    pub title: String,
    pub description: Option<String>,
    /// Primary substrate hint for WIP-cap + bottleneck queries.
    pub substrate: Option<String>,
    /// Capability *names* (not IDs); the store resolver maps these to IDs at query time.
    pub touched_capabilities: Vec<String>,
    /// File paths touched by this plan (for intent overlap detection).
    pub touched_paths: Vec<PathBuf>,
    pub space_id: SpaceId,
    pub project_id: Option<ProjectId>,
}

// ── FitReport and sub-types ──────────────────────────────────────────────────

/// The structured output of `g8-planner::Planner::plan_check`.
///
/// # Examples
///
/// ```
/// use g8_core::plan::{FitReport, PlanDraft, DecisionInput, Recommendation};
/// use g8_core::SpaceId;
///
/// let draft = PlanDraft {
///     title:               "streaming-export".into(),
///     description:         None,
///     substrate:           Some("export-pipeline".into()),
///     touched_capabilities: vec![],
///     touched_paths:       vec![],
///     space_id:            SpaceId::derive(&["docs", "space"]),
///     project_id:          None,
/// };
/// let di = DecisionInput {
///     any_over_budget:  false,
///     has_exact_match:  false,
///     near_match_count: 0,
///     has_parked_match: false,
///     bottleneck_count: 0,
/// };
/// let report = FitReport {
///     draft:            draft.clone(),
///     existing_matches: vec![],
///     budget_status:    None,
///     bottlenecks:      vec![],
///     drift_hints:      vec![],
///     intent_overlaps:  vec![],
///     parked_ideas:     vec![],
///     decision_input:   di,
///     recommendation:   Recommendation::Proceed,
/// };
/// assert_eq!(report.recommendation, Recommendation::Proceed);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitReport {
    pub draft: PlanDraft,
    pub existing_matches: Vec<ExistingMatch>,
    /// `None` when no substrate hint was given in the draft.
    pub budget_status: Option<BudgetStatus>,
    pub bottlenecks: Vec<BottleneckPlan>,
    pub drift_hints: Vec<DriftHint>,
    pub intent_overlaps: Vec<IntentOverlap>,
    pub parked_ideas: Vec<ParkedIdeaMatch>,
    pub decision_input: DecisionInput,
    pub recommendation: Recommendation,
}

/// An existing plan that overlaps with the draft in some dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingMatch {
    pub id: PlanId,
    pub title: String,
    pub status: PlanStatus,
    pub substrate: Option<String>,
    pub match_kind: MatchKind,
}

/// How an existing plan overlaps with the draft.
///
/// # Examples
///
/// ```
/// use g8_core::MatchKind;
///
/// let k = MatchKind::SubstrateOverlap;
/// assert!(matches!(k, MatchKind::SubstrateOverlap));
/// let json = serde_json::to_string(&k).unwrap();
/// assert_eq!(json, r#""substrate_overlap""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    SubstrateOverlap,
    CapabilityOverlap,
    TitleOverlap,
}

/// WIP cap status for the draft's substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub substrate: String,
    pub wip_cap: u32,
    pub wip_current: u32,
    /// Signed: negative means over cap.
    pub wip_remaining: i64,
    pub at_cap: bool,
}

/// A plan in Dispatched or Blocked state on the same substrate — a potential bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckPlan {
    pub id: PlanId,
    pub title: String,
    pub status: PlanStatus,
    pub blocked_reason: Option<String>,
    pub days_in_flight: Option<i64>,
}

/// A plan that hasn't been updated in longer than the substrate's stale threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftHint {
    pub id: PlanId,
    pub title: String,
    pub status: PlanStatus,
    pub days_stale: i64,
}

/// An intent that overlaps with the draft (same substrate or title similarity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentOverlap {
    pub id: IntentId,
    /// Heading from the AGENTS.md/CLAUDE.md section.
    pub title: String,
    pub substrate: Option<String>,
    pub source_kind: crate::model::IntentSourceKind,
    pub project: Option<String>,
}

/// A parked plan whose substrate or title is similar to the draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParkedIdeaMatch {
    pub id: PlanId,
    pub title: String,
    pub parked_reason: Option<String>,
}

/// Boolean gate inputs that feed the deterministic recommendation lookup table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DecisionInput {
    pub any_over_budget: bool,
    pub has_exact_match: bool,
    /// Count of near-matching plans (substrate overlap OR capability overlap).
    pub near_match_count: u32,
    pub has_parked_match: bool,
    pub bottleneck_count: u32,
}

/// The recommendation output of `recommend(input)`.
///
/// `Rename` is reserved for v0.2 (requires fuzzy-match score not computed in v0.1).
///
/// # Examples
///
/// ```
/// use g8_core::Recommendation;
///
/// let r = Recommendation::Proceed;
/// assert!(matches!(r, Recommendation::Proceed));
/// let json = serde_json::to_string(&r).unwrap();
/// assert_eq!(json, r#""Proceed""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Recommendation {
    /// Budget full or active bottlenecks — park the idea.
    Park,
    /// Exact-name match exists in non-parked state — this plan already exists.
    Drop,
    /// Budget full but no bottleneck; queue and retry when a slot opens.
    Wait,
    /// Near-match exists — extend that plan instead of creating a new one.
    Extend,
    /// Title collision — rename to avoid confusion. (Reserved for v0.2.)
    Rename,
    /// Bottleneck on substrate suggests deferring — pivot to a different substrate.
    Pivot,
    /// All gates pass — proceed with the plan.
    Proceed,
}

/// Derive a [`Recommendation`] from a [`DecisionInput`] using a fixed lookup table.
///
/// Order matters: first matching branch wins.
///
/// # Examples
///
/// ```
/// use g8_core::plan::{recommend, DecisionInput, Recommendation};
///
/// assert_eq!(
///     recommend(&DecisionInput {
///         has_exact_match:  true,
///         any_over_budget:  false,
///         near_match_count: 0,
///         has_parked_match: false,
///         bottleneck_count: 0,
///     }),
///     Recommendation::Drop,
/// );
///
/// assert_eq!(
///     recommend(&DecisionInput {
///         any_over_budget:  true,
///         has_exact_match:  false,
///         near_match_count: 0,
///         has_parked_match: false,
///         bottleneck_count: 2,
///     }),
///     Recommendation::Pivot,
/// );
/// ```
pub fn recommend(input: &DecisionInput) -> Recommendation {
    if input.has_exact_match {
        return Recommendation::Drop;
    }
    if input.any_over_budget && input.bottleneck_count > 0 {
        return Recommendation::Pivot;
    }
    if input.any_over_budget {
        return Recommendation::Park;
    }
    if input.bottleneck_count > 0 {
        return Recommendation::Wait;
    }
    if input.near_match_count > 0 {
        return Recommendation::Extend;
    }
    if input.has_parked_match {
        return Recommendation::Extend;
    }
    Recommendation::Proceed
}

// ── OverlapKind / PlanIntentRelation (used by StoreConnection trait) ─────────

/// How a plan relates to a capability it touches (junction table column).
///
/// # Examples
///
/// ```
/// use g8_core::OverlapKind;
///
/// let k = OverlapKind::Touches;
/// assert!(matches!(k, OverlapKind::Touches));
/// let json = serde_json::to_string(&k).unwrap();
/// assert_eq!(json, r#""touches""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapKind {
    Touches,
    Extends,
    Replaces,
    Conflicts,
}

/// How a plan relates to an intent it is linked to (junction table column).
///
/// # Examples
///
/// ```
/// use g8_core::PlanIntentRelation;
///
/// let r = PlanIntentRelation::Satisfies;
/// assert!(matches!(r, PlanIntentRelation::Satisfies));
/// let json = serde_json::to_string(&r).unwrap();
/// assert_eq!(json, r#""satisfies""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanIntentRelation {
    Satisfies,
    Contradicts,
    Extends,
    DerivedFrom,
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // One test per Recommendation variant.
    #[test]
    fn recommend_drop_on_exact_match() {
        let r = recommend(&DecisionInput {
            has_exact_match: true,
            any_over_budget: false,
            near_match_count: 0,
            has_parked_match: false,
            bottleneck_count: 0,
        });
        assert_eq!(r, Recommendation::Drop);
    }

    #[test]
    fn recommend_pivot_on_over_budget_with_bottleneck() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: true,
            near_match_count: 0,
            has_parked_match: false,
            bottleneck_count: 1,
        });
        assert_eq!(r, Recommendation::Pivot);
    }

    #[test]
    fn recommend_park_on_over_budget_no_bottleneck() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: true,
            near_match_count: 0,
            has_parked_match: false,
            bottleneck_count: 0,
        });
        assert_eq!(r, Recommendation::Park);
    }

    #[test]
    fn recommend_wait_on_bottleneck_no_budget_issue() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: false,
            near_match_count: 0,
            has_parked_match: false,
            bottleneck_count: 3,
        });
        assert_eq!(r, Recommendation::Wait);
    }

    #[test]
    fn recommend_extend_on_near_match() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: false,
            near_match_count: 2,
            has_parked_match: false,
            bottleneck_count: 0,
        });
        assert_eq!(r, Recommendation::Extend);
    }

    #[test]
    fn recommend_extend_on_parked_match() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: false,
            near_match_count: 0,
            has_parked_match: true,
            bottleneck_count: 0,
        });
        assert_eq!(r, Recommendation::Extend);
    }

    #[test]
    fn recommend_proceed_on_clean_slate() {
        let r = recommend(&DecisionInput {
            has_exact_match: false,
            any_over_budget: false,
            near_match_count: 0,
            has_parked_match: false,
            bottleneck_count: 0,
        });
        assert_eq!(r, Recommendation::Proceed);
    }

    // Status transition: one legal and one illegal per from-state.
    #[test]
    fn transition_idea_to_scoped_ok() {
        assert!(validate_status_transition(PlanStatus::Idea, PlanStatus::Scoped).is_ok());
    }

    #[test]
    fn transition_idea_to_dispatched_illegal() {
        assert!(validate_status_transition(PlanStatus::Idea, PlanStatus::Dispatched).is_err());
    }

    #[test]
    fn transition_scoped_to_dispatched_ok() {
        assert!(validate_status_transition(PlanStatus::Scoped, PlanStatus::Dispatched).is_ok());
    }

    #[test]
    fn transition_scoped_to_done_illegal() {
        assert!(validate_status_transition(PlanStatus::Scoped, PlanStatus::Done).is_err());
    }

    #[test]
    fn transition_dispatched_to_done_ok() {
        assert!(validate_status_transition(PlanStatus::Dispatched, PlanStatus::Done).is_ok());
    }

    #[test]
    fn transition_dispatched_to_idea_illegal() {
        assert!(validate_status_transition(PlanStatus::Dispatched, PlanStatus::Idea).is_err());
    }

    #[test]
    fn transition_blocked_to_dispatched_ok() {
        assert!(validate_status_transition(PlanStatus::Blocked, PlanStatus::Dispatched).is_ok());
    }

    #[test]
    fn transition_done_is_terminal() {
        for to in [
            PlanStatus::Idea,
            PlanStatus::Scoped,
            PlanStatus::Dispatched,
            PlanStatus::Blocked,
            PlanStatus::Parked,
        ] {
            assert!(
                validate_status_transition(PlanStatus::Done, to).is_err(),
                "Done -> {to:?} should be illegal"
            );
        }
    }

    #[test]
    fn transition_parked_to_scoped_ok() {
        assert!(validate_status_transition(PlanStatus::Parked, PlanStatus::Scoped).is_ok());
    }

    #[test]
    fn transition_parked_to_dispatched_illegal() {
        assert!(validate_status_transition(PlanStatus::Parked, PlanStatus::Dispatched).is_err());
    }

    #[test]
    fn serde_planstatus_pascal_case() {
        let s = serde_json::to_string(&PlanStatus::Dispatched).unwrap();
        assert_eq!(s, r#""Dispatched""#);
    }

    #[test]
    fn serde_recommendation_pascal_case() {
        let s = serde_json::to_string(&Recommendation::Proceed).unwrap();
        assert_eq!(s, r#""Proceed""#);
    }
}
