//! Integration tests for `g8-planner`.
//!
//! Uses a `MockStore` that implements only the methods `g8-planner` calls
//! (`planner_intent_check`).  All other methods return trivially correct
//! results so the trait is satisfied without involving a real SQLite database.
//!
//! Test matrix (one test per required scenario from ARCHITECTURE.md §17 B5):
//! - empty store → `Recommendation::Proceed`
//! - populated store with exact title match → `Recommendation::Drop`
//! - budget exhausted, no bottleneck → `Recommendation::Park`
//! - budget exhausted, active bottleneck → `Recommendation::Pivot`
//! - near-match (substrate/capability overlap, no exact title) → `Recommendation::Extend`
//! - parked idea match, no other signals → `Recommendation::Extend`
//! - bottleneck only (no budget constraint) → `Recommendation::Wait`
//! - drift hints present → populated in `FitReport` (drift hints do not affect recommendation)
//! - empty draft title → `PlannerError::InvalidDraft`

use std::path::Path;

use g8_core::{
    model::{Capability, ConvergenceSpace, Decision, Intent, Plan, Project},
    plan::{MatchKind, OverlapKind, PlanDraft, PlanIntentRelation, PlanStatus},
    substrate::SubstrateBudget,
    CapabilityId, IntentId, PlanId, ProjectId, SpaceId,
};
use g8_planner::{Planner, PlannerError};
use g8_store::{
    ApplyScanReport, PairingError, PlanFilter, RawIntentCheckPayload, ScanResult,
    StalePlanCandidate, StoreConnection, StoreError, WatchHandle,
};

// ── MockStore ─────────────────────────────────────────────────────────────────

/// A minimal `StoreConnection` stub for planner tests.
///
/// The only meaningful field is `payload`: the `RawIntentCheckPayload` that
/// `planner_intent_check` will return.  All other methods are no-ops that
/// return empty / default values — they are never called by `g8-planner`.
struct MockStore {
    payload: RawIntentCheckPayload,
}

impl MockStore {
    /// Construct a store that returns the given payload from
    /// `planner_intent_check`.
    fn with_payload(payload: RawIntentCheckPayload) -> Self {
        Self { payload }
    }

    /// Convenience: return an empty payload (all arrays empty, budget null).
    fn empty() -> Self {
        Self::with_payload(RawIntentCheckPayload {
            existing_matches: serde_json::Value::Array(vec![]),
            budget: serde_json::Value::Null,
            bottlenecks: serde_json::Value::Array(vec![]),
            drift_hints: serde_json::Value::Array(vec![]),
            intent_overlaps: serde_json::Value::Array(vec![]),
            parked_ideas: serde_json::Value::Array(vec![]),
        })
    }
}

// Implement the full StoreConnection trait; only `planner_intent_check` is
// load-bearing for these tests.  Everything else is a no-op stub.
impl StoreConnection for MockStore {
    // ── Lifecycle ──────────────────────────────────────────────────────────
    fn init_space(&mut self, _space: &ConvergenceSpace) -> Result<(), StoreError> {
        Ok(())
    }
    fn get_space(&self, _id: &SpaceId) -> Result<Option<ConvergenceSpace>, StoreError> {
        Ok(None)
    }
    fn get_default_space(&self) -> Result<Option<ConvergenceSpace>, StoreError> {
        Ok(None)
    }
    fn upsert_project(&mut self, _project: &Project) -> Result<(), StoreError> {
        Ok(())
    }
    fn get_project(&self, _id: &ProjectId) -> Result<Option<Project>, StoreError> {
        Ok(None)
    }
    fn list_projects(&self, _space: &SpaceId) -> Result<Vec<Project>, StoreError> {
        Ok(vec![])
    }
    fn find_project_by_name(
        &self,
        _space: &SpaceId,
        _name: &str,
    ) -> Result<Option<Project>, StoreError> {
        Ok(None)
    }
    fn delete_project(&mut self, _id: &ProjectId) -> Result<(), StoreError> {
        Ok(())
    }

    // ── Bulk scan ──────────────────────────────────────────────────────────
    fn apply_scan(
        &mut self,
        _project: &ProjectId,
        _scan: ScanResult,
    ) -> Result<ApplyScanReport, StoreError> {
        Ok(ApplyScanReport {
            inserted_capabilities: 0,
            inserted_intents: 0,
            inserted_decisions: 0,
            deleted_capabilities: 0,
            deleted_intents: 0,
            warnings: vec![],
        })
    }

    // ── Capability CRUD ────────────────────────────────────────────────────
    fn upsert_capability(&mut self, _cap: &Capability) -> Result<(), StoreError> {
        Ok(())
    }
    fn get_capability(&self, _id: &CapabilityId) -> Result<Option<Capability>, StoreError> {
        Ok(None)
    }
    fn find_capability_by_name(
        &self,
        _project: &ProjectId,
        _name: &str,
    ) -> Result<Option<Capability>, StoreError> {
        Ok(None)
    }
    fn list_capabilities(&self, _project: &ProjectId) -> Result<Vec<Capability>, StoreError> {
        Ok(vec![])
    }
    fn delete_capabilities_for_project(&mut self, _project: &ProjectId) -> Result<u64, StoreError> {
        Ok(0)
    }

    // ── Intent CRUD ────────────────────────────────────────────────────────
    fn upsert_intent(&mut self, _intent: &Intent) -> Result<(), StoreError> {
        Ok(())
    }
    fn list_intents_at_path(
        &self,
        _space: &SpaceId,
        _path: &Path,
    ) -> Result<Vec<Intent>, StoreError> {
        Ok(vec![])
    }
    fn delete_intents_for_project(&mut self, _project: &ProjectId) -> Result<u64, StoreError> {
        Ok(0)
    }

    // ── Plan CRUD ──────────────────────────────────────────────────────────
    fn create_plan(&mut self, _plan: &Plan) -> Result<(), StoreError> {
        Ok(())
    }
    fn get_plan(&self, _id: &PlanId) -> Result<Option<Plan>, StoreError> {
        Ok(None)
    }
    fn list_plans(&self, _space: &SpaceId, _filter: PlanFilter) -> Result<Vec<Plan>, StoreError> {
        Ok(vec![])
    }
    fn update_plan_status(
        &mut self,
        _id: &PlanId,
        _new_status: PlanStatus,
        _reason: Option<String>,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn link_plan_capability(
        &mut self,
        _plan: &PlanId,
        _capability: &CapabilityId,
        _overlap_kind: OverlapKind,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn link_plan_intent(
        &mut self,
        _plan: &PlanId,
        _intent: &IntentId,
        _relation: PlanIntentRelation,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    // ── Decision CRUD ──────────────────────────────────────────────────────
    fn upsert_decision(&mut self, _decision: &Decision) -> Result<(), StoreError> {
        Ok(())
    }
    fn list_decisions(&self, _space: &SpaceId) -> Result<Vec<Decision>, StoreError> {
        Ok(vec![])
    }

    // ── Substrate budget ───────────────────────────────────────────────────
    fn set_substrate_budget(
        &mut self,
        _space: &SpaceId,
        _substrate: &str,
        _wip_cap: u32,
        _stale_threshold_days: u32,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    fn get_substrate_budget(
        &self,
        _space: &SpaceId,
        _substrate: &str,
    ) -> Result<SubstrateBudget, StoreError> {
        Ok(SubstrateBudget {
            substrate: "".into(),
            wip_cap: 3,
            stale_threshold_days: 14,
        })
    }
    fn list_substrate_budgets(&self, _space: &SpaceId) -> Result<Vec<SubstrateBudget>, StoreError> {
        Ok(vec![])
    }

    // ── Capability aliases ─────────────────────────────────────────────────
    fn add_capability_alias(&mut self, _canonical: &str, _alias: &str) -> Result<(), StoreError> {
        Ok(())
    }
    fn resolve_canonical(&self, ref_name: &str) -> Result<String, StoreError> {
        Ok(ref_name.to_string())
    }

    // ── Composite query (the only load-bearing method for g8-planner) ─────

    /// Returns the pre-configured payload unchanged.  This is the single
    /// store call that `g8-planner` makes per `plan_check` invocation.
    fn planner_intent_check(
        &self,
        _draft: &PlanDraft,
    ) -> Result<RawIntentCheckPayload, StoreError> {
        Ok(self.payload.clone())
    }

    fn pairing_check(&self, _project: Option<&ProjectId>) -> Result<Vec<PairingError>, StoreError> {
        Ok(vec![])
    }
    fn stale_dispatched_plans(
        &self,
        _space: &SpaceId,
    ) -> Result<Vec<StalePlanCandidate>, StoreError> {
        Ok(vec![])
    }

    // ── Cross-store attach ─────────────────────────────────────────────────
    fn attach_remote_store(&mut self, _alias: &str, _path: &Path) -> Result<(), StoreError> {
        Ok(())
    }
    fn detach_remote_store(&mut self, _alias: &str) -> Result<(), StoreError> {
        Ok(())
    }

    // ── File-watch (unused by planner) ─────────────────────────────────────
    fn watch<F>(&self, _callback: F) -> Result<WatchHandle, StoreError>
    where
        F: Fn() + Send + 'static,
    {
        Err(StoreError::Notify("not supported in mock".into()))
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_draft(title: &str, substrate: Option<&str>) -> PlanDraft {
    PlanDraft {
        title: title.into(),
        description: None,
        substrate: substrate.map(Into::into),
        touched_capabilities: vec![],
        touched_paths: vec![],
        space_id: SpaceId::sequential_for_tests(),
        project_id: None,
    }
}

fn existing_match(title: &str, match_kind: MatchKind) -> serde_json::Value {
    serde_json::json!({
        "id": PlanId::sequential_for_tests().as_str().to_owned(),
        "title": title,
        "status": "Dispatched",
        "substrate": serde_json::Value::Null,
        "match_kind": match_kind,
    })
}

fn budget_at_cap(substrate: &str) -> serde_json::Value {
    serde_json::json!({
        "substrate": substrate,
        "wip_cap": 3,
        "wip_current": 3,
        "wip_remaining": 0,
        "at_cap": true,
    })
}

fn bottleneck_plan() -> serde_json::Value {
    serde_json::json!({
        "id": PlanId::sequential_for_tests().as_str().to_owned(),
        "title": "blocked-thing",
        "status": "Blocked",
        "blocked_reason": "waiting on upstream",
        "days_in_flight": 5,
    })
}

fn drift_hint_value(days_stale: i64) -> serde_json::Value {
    serde_json::json!({
        "id": PlanId::sequential_for_tests().as_str().to_owned(),
        "title": "old-plan",
        "status": "Dispatched",
        "days_stale": days_stale,
    })
}

fn parked_idea() -> serde_json::Value {
    serde_json::json!({
        "id": PlanId::sequential_for_tests().as_str().to_owned(),
        "title": "parked-feature",
        "parked_reason": "not now",
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Empty store: no overlaps, no budget constraints → Proceed.
#[test]
fn empty_store_returns_proceed() {
    let store = MockStore::empty();
    let draft = make_draft("streaming-export", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Proceed);
    assert!(report.existing_matches.is_empty());
    assert!(report.budget_status.is_none());
    assert!(report.bottlenecks.is_empty());
    assert!(report.drift_hints.is_empty());
    assert!(report.intent_overlaps.is_empty());
    assert!(report.parked_ideas.is_empty());
    assert!(!report.decision_input.any_over_budget);
    assert!(!report.decision_input.has_exact_match);
    assert_eq!(report.decision_input.near_match_count, 0);
    assert!(!report.decision_input.has_parked_match);
    assert_eq!(report.decision_input.bottleneck_count, 0);
}

/// Exact title match in the existing matches → Drop.
#[test]
fn exact_title_match_returns_drop() {
    let title = "streaming-export";
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![existing_match(
            title,
            MatchKind::SubstrateOverlap,
        )]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft(title, None);
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Drop);
    assert!(report.decision_input.has_exact_match);
    assert_eq!(report.decision_input.near_match_count, 1);
}

/// Near-match (different title, substrate overlap) → Extend.
#[test]
fn near_match_without_exact_title_returns_extend() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![existing_match(
            "related-export-feature",
            MatchKind::SubstrateOverlap,
        )]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    // Draft has a different title than the existing match.
    let draft = make_draft("streaming-export", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Extend);
    assert!(!report.decision_input.has_exact_match);
    assert_eq!(report.decision_input.near_match_count, 1);
}

/// Budget at cap, no bottlenecks → Park.
#[test]
fn budget_exhausted_no_bottleneck_returns_park() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: budget_at_cap("export-pipeline"),
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("new-feature", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Park);
    assert!(report.decision_input.any_over_budget);
    assert_eq!(report.decision_input.bottleneck_count, 0);
    assert!(report.budget_status.is_some());
    assert!(report.budget_status.as_ref().unwrap().at_cap);
}

/// Budget at cap with an active bottleneck → Pivot.
#[test]
fn budget_exhausted_with_bottleneck_returns_pivot() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: budget_at_cap("export-pipeline"),
        bottlenecks: serde_json::Value::Array(vec![bottleneck_plan()]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("new-feature", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Pivot);
    assert!(report.decision_input.any_over_budget);
    assert_eq!(report.decision_input.bottleneck_count, 1);
    assert_eq!(report.bottlenecks.len(), 1);
}

/// Bottleneck present, no budget issue → Wait.
#[test]
fn bottleneck_no_budget_issue_returns_wait() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![bottleneck_plan()]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("new-feature", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Wait);
    assert!(!report.decision_input.any_over_budget);
    assert_eq!(report.decision_input.bottleneck_count, 1);
}

/// Parked idea match, no other signals → Extend (parked idea surfaced for reuse).
#[test]
fn parked_idea_match_returns_extend() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![parked_idea()]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("new-feature", None);
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Extend);
    assert!(report.decision_input.has_parked_match);
    assert_eq!(report.parked_ideas.len(), 1);
    assert_eq!(report.parked_ideas[0].title, "parked-feature");
    assert_eq!(
        report.parked_ideas[0].parked_reason.as_deref(),
        Some("not now")
    );
}

/// Drift hints are populated in the FitReport even though they do not affect
/// the recommendation (drift is informational only).
#[test]
fn drift_hints_are_populated_in_report() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![drift_hint_value(20), drift_hint_value(35)]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("clean-feature", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    // Drift does not affect the recommendation — should be Proceed.
    assert_eq!(report.recommendation, g8_core::Recommendation::Proceed);
    // But the hints are present in the report.
    assert_eq!(report.drift_hints.len(), 2);
    assert_eq!(report.drift_hints[0].days_stale, 20);
    assert_eq!(report.drift_hints[1].days_stale, 35);
}

/// An empty draft title must return `PlannerError::InvalidDraft`.
#[test]
fn empty_title_returns_invalid_draft_error() {
    let store = MockStore::empty();
    let draft = make_draft("", Some("export-pipeline"));
    let err = Planner::plan_check(&store, &draft).expect_err("should have failed");
    assert!(
        matches!(err, PlannerError::InvalidDraft(_)),
        "expected InvalidDraft, got {err:?}"
    );
}

/// A title containing only whitespace should also be rejected.
#[test]
fn whitespace_only_title_returns_invalid_draft_error() {
    let store = MockStore::empty();
    let draft = make_draft("   \t  ", None);
    let err = Planner::plan_check(&store, &draft).expect_err("should have failed");
    assert!(matches!(err, PlannerError::InvalidDraft(_)));
}

/// Budget NOT at cap (wip_remaining > 0) should not trigger Park/Pivot/Wait.
#[test]
fn budget_under_cap_does_not_block() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: serde_json::json!({
            "substrate": "export-pipeline",
            "wip_cap": 3,
            "wip_current": 1,
            "wip_remaining": 2,
            "at_cap": false,
        }),
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("new-feature", Some("export-pipeline"));
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");

    assert_eq!(report.recommendation, g8_core::Recommendation::Proceed);
    assert!(!report.decision_input.any_over_budget);
    assert!(report.budget_status.is_some());
    assert!(!report.budget_status.unwrap().at_cap);
}

/// The draft is echoed back verbatim in the FitReport.
#[test]
fn draft_is_echoed_in_report() {
    let store = MockStore::empty();
    let draft = PlanDraft {
        title: "echo-test".into(),
        description: Some("desc".into()),
        substrate: Some("auth".into()),
        touched_capabilities: vec!["cap-a".into(), "cap-b".into()],
        touched_paths: vec![std::path::PathBuf::from("src/auth.rs")],
        space_id: SpaceId::sequential_for_tests(),
        project_id: None,
    };
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");
    assert_eq!(report.draft.title, "echo-test");
    assert_eq!(report.draft.substrate.as_deref(), Some("auth"));
    assert_eq!(report.draft.touched_capabilities, vec!["cap-a", "cap-b"]);
}

/// NULL budget column (no substrate in draft or no budget configured)
/// should produce `budget_status = None`.
#[test]
fn null_budget_produces_none_budget_status() {
    let payload = RawIntentCheckPayload {
        existing_matches: serde_json::Value::Array(vec![]),
        budget: serde_json::Value::Null,
        bottlenecks: serde_json::Value::Array(vec![]),
        drift_hints: serde_json::Value::Array(vec![]),
        intent_overlaps: serde_json::Value::Array(vec![]),
        parked_ideas: serde_json::Value::Array(vec![]),
    };
    let store = MockStore::with_payload(payload);
    let draft = make_draft("no-substrate", None);
    let report = Planner::plan_check(&store, &draft).expect("plan_check failed");
    assert!(report.budget_status.is_none());
}
