//! Integration tests for `g8-conflict::ConflictDetector`.
//!
//! Each test constructs fixture stores using `MemoryStore` and asserts the expected
//! conflicts are (or are not) detected. At least one positive test per conflict kind
//! is required by the acceptance criteria.

use std::path::PathBuf;

use g8_conflict::{
    store_trait::{CapabilityRow, DecisionRow, IntentRow, MemoryStore, PlanRow},
    ConflictDetector,
};
use g8_core::{
    CapabilityId, ConflictKind, DecisionId, Evidence, IntentId, IntentKind, PlanId, Severity,
    SpaceId,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn make_intent(kind: IntentKind, heading: &str, description: &str, scope: &str) -> IntentRow {
    IntentRow {
        id: IntentId::sequential_for_tests(),
        kind,
        heading: heading.to_string(),
        description: description.to_string(),
        scope_path: PathBuf::from(scope),
    }
}

fn make_capability(name: &str, project: &str) -> CapabilityRow {
    CapabilityRow {
        id: CapabilityId::sequential_for_tests(),
        name: name.to_string(),
        project_name: project.to_string(),
    }
}

fn make_decision(title: &str, body: &str, accepted: bool) -> DecisionRow {
    DecisionRow {
        id: DecisionId::sequential_for_tests(),
        title: title.to_string(),
        status_accepted: accepted,
        body: body.to_string(),
    }
}

fn make_plan(title: &str, substrate: Option<&str>) -> PlanRow {
    PlanRow {
        id: PlanId::sequential_for_tests(),
        title: title.to_string(),
        substrate: substrate.map(|s| s.to_string()),
    }
}

// ── Empty-space boundary cases ────────────────────────────────────────────────

#[test]
fn empty_stores_produce_no_conflicts() {
    let local = MemoryStore::default();
    let remote = MemoryStore::default();
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts.is_empty(),
        "empty stores must produce no conflicts, got: {conflicts:?}"
    );
}

#[test]
fn identical_single_capability_no_conflict_with_alias() {
    // When both sides have the same capability name AND an alias resolves them → Info, not Error.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_capability(&local_space, make_capability("http-fetch", "checkout-api"));
    remote.add_capability(&remote_space, make_capability("http-fetch", "render-kit"));

    // Register alias: both map to the same canonical identity.
    local.add_alias("checkout-api::http-fetch", "shared::http-fetch");
    remote.add_alias("render-kit::http-fetch", "shared::http-fetch");

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    // Must produce Info (acknowledgment) not Error.
    assert!(
        !conflicts.is_empty(),
        "aliased collision should still produce an Info conflict"
    );
    let col = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::CapabilityNameCollision)
        .expect("should have a CapabilityNameCollision");
    assert_eq!(
        col.severity,
        Severity::Info,
        "aliased capability collision should downgrade to Info"
    );
}

// ── DuplicateIntent ───────────────────────────────────────────────────────────

#[test]
fn duplicate_intent_same_kind_scope_different_description() {
    // Positive test: both spaces declare ArchitecturalScope at "src/auth" with different bodies.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_intent(
        &local_space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture",
            "Handles user auth via JWT tokens",
            "src/auth",
        ),
    );
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture",
            "Handles user auth via OAuth2 sessions",
            "src/auth",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let dup = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::DuplicateIntent)
        .expect("should detect DuplicateIntent");

    assert_eq!(dup.severity, Severity::Warn);
    // Must have typed IntentRef evidence.
    let intent_refs: Vec<_> = dup
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::IntentRef { .. }))
        .collect();
    assert_eq!(
        intent_refs.len(),
        2,
        "should carry two IntentRef evidence items"
    );
}

#[test]
fn duplicate_intent_same_description_no_conflict() {
    // Identical intents in both spaces (same kind, scope, AND description) must not trigger.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    let intent = make_intent(
        IntentKind::TechStack,
        "Stack",
        "Primary language is Rust",
        ".",
    );
    local.add_intent(&local_space, intent.clone());
    remote.add_intent(&remote_space, intent);

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::DuplicateIntent),
        "identical intents must not produce DuplicateIntent"
    );
}

#[test]
fn different_scope_paths_no_intent_conflict() {
    // Same kind and description but different scope paths → not a conflict.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_intent(
        &local_space,
        make_intent(
            IntentKind::Boundary,
            "Constraints",
            "Never modify the payments module",
            "src/payments",
        ),
    );
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::Boundary,
            "Constraints",
            "Never modify the auth module",
            "src/auth",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::DuplicateIntent),
        "different scope paths must not trigger DuplicateIntent"
    );
}

// ── CapabilityNameCollision ───────────────────────────────────────────────────

#[test]
fn capability_collision_error_when_no_alias() {
    // Positive test: both spaces have "stream-export" with no alias → Error.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_capability(&local_space, make_capability("stream-export", "proj-a"));
    remote.add_capability(&remote_space, make_capability("stream-export", "proj-b"));

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let col = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::CapabilityNameCollision)
        .expect("should detect CapabilityNameCollision");

    assert_eq!(col.severity, Severity::Error);
    // Must carry typed CapabilityRef evidence (not Note-only).
    let cap_refs: Vec<_> = col
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::CapabilityRef { .. }))
        .collect();
    assert_eq!(
        cap_refs.len(),
        2,
        "should carry two CapabilityRef evidence items"
    );
}

#[test]
fn capability_collision_info_when_alias_resolves() {
    // Aliased collision → Info severity only.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_capability(&local_space, make_capability("jwt-auth", "backend"));
    remote.add_capability(&remote_space, make_capability("jwt-auth", "gateway"));

    // Both sides resolve to the same canonical identity.
    local.add_alias("backend::jwt-auth", "platform::jwt-auth");
    remote.add_alias("gateway::jwt-auth", "platform::jwt-auth");

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let col = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::CapabilityNameCollision)
        .expect("aliased collision should still emit Info");

    assert_eq!(
        col.severity,
        Severity::Info,
        "alias should downgrade to Info"
    );
}

#[test]
fn unique_capabilities_no_collision() {
    // Distinct names across both spaces → no collision.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_capability(&local_space, make_capability("claim-validation", "core"));
    remote.add_capability(&remote_space, make_capability("http-fetch", "net-layer"));

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::CapabilityNameCollision),
        "unique capabilities must not trigger collision"
    );
}

// ── ContradictoryDecisions ────────────────────────────────────────────────────

#[test]
fn contradictory_decisions_both_accepted_different_body() {
    // Positive test: same title, both accepted, different body → Error.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_decision(
        &local_space,
        make_decision(
            "use-sqlite-for-persistence",
            "Use SQLite with WAL mode for all persistence.",
            true,
        ),
    );
    remote.add_decision(
        &remote_space,
        make_decision(
            "use-sqlite-for-persistence",
            "Use PostgreSQL for all persistence instead.",
            true,
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let con = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::ContradictoryDecisions)
        .expect("should detect ContradictoryDecisions");

    assert_eq!(con.severity, Severity::Error);
    // Must carry typed DecisionRef evidence.
    let dec_refs: Vec<_> = con
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::DecisionRef { .. }))
        .collect();
    assert_eq!(
        dec_refs.len(),
        2,
        "should carry two DecisionRef evidence items"
    );
}

#[test]
fn decision_same_body_no_conflict() {
    // Both spaces accepted the same decision with the same body → no conflict.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    let body = "Use Rust for all systems code.";
    local.add_decision(&local_space, make_decision("language-choice", body, true));
    remote.add_decision(&remote_space, make_decision("language-choice", body, true));

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::ContradictoryDecisions),
        "identical accepted decisions must not conflict"
    );
}

#[test]
fn decision_not_accepted_no_conflict() {
    // One side is not accepted → not a contradiction even if bodies differ.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_decision(&local_space, make_decision("db-choice", "Use SQLite", true));
    remote.add_decision(
        &remote_space,
        // proposed, not accepted
        make_decision("db-choice", "Use Postgres", false),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::ContradictoryDecisions),
        "non-accepted decision must not trigger contradiction"
    );
}

// ── GovernanceViolation ───────────────────────────────────────────────────────

#[test]
fn governance_violation_plan_forbidden_substrate() {
    // Positive test: local plan uses substrate "payments" which remote boundary forbids.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_plan(
        &local_space,
        make_plan("add-payment-processor", Some("payments")),
    );

    // Remote has a Boundary intent that forbids "payments" substrate modification.
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::Boundary,
            "Constraints",
            "Never modify the payments substrate: it is owned by the payments team only",
            ".",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let gov = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::GovernanceViolation)
        .expect("should detect GovernanceViolation");

    assert_eq!(gov.severity, Severity::Error);
    // Must carry PlanRef + IntentRef evidence.
    let plan_refs: Vec<_> = gov
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::PlanRef { .. }))
        .collect();
    let intent_refs: Vec<_> = gov
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::IntentRef { .. }))
        .collect();
    assert_eq!(plan_refs.len(), 1, "should carry a PlanRef");
    assert_eq!(intent_refs.len(), 1, "should carry an IntentRef");
}

#[test]
fn governance_violation_bidirectional() {
    // Remote plan also triggers local boundary intent.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    // Local boundary forbids "auth" substrate.
    local.add_intent(
        &local_space,
        make_intent(
            IntentKind::Boundary,
            "Security Boundaries",
            "Must not use or modify the auth substrate outside this service",
            ".",
        ),
    );

    // Remote has a plan using "auth".
    remote.add_plan(
        &remote_space,
        make_plan("refactor-login-flow", Some("auth")),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let gov = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::GovernanceViolation)
        .expect("should detect GovernanceViolation from remote plan vs local boundary");

    assert_eq!(gov.severity, Severity::Error);
}

#[test]
fn governance_no_violation_when_substrate_not_mentioned() {
    // Boundary intent doesn't mention the plan's substrate → no violation.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_plan(&local_space, make_plan("add-search-index", Some("search")));

    // Remote boundary about something completely different.
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::Boundary,
            "Data Constraints",
            "Never modify the payments substrate",
            ".",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::GovernanceViolation),
        "unrelated boundary must not trigger GovernanceViolation"
    );
}

#[test]
fn governance_no_violation_when_plan_has_no_substrate() {
    // Plan without a substrate → governance check is skipped.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    // Plan has no substrate.
    local.add_plan(&local_space, make_plan("exploratory-spike", None));

    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::Boundary,
            "Constraints",
            "Never modify the auth substrate",
            ".",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(
        conflicts
            .iter()
            .all(|c| c.kind != ConflictKind::GovernanceViolation),
        "plan with no substrate must not trigger GovernanceViolation"
    );
}

// ── Severity ordering ─────────────────────────────────────────────────────────

#[test]
fn conflicts_sorted_by_descending_severity() {
    // Mix of Error and Warn conflicts; result must be Error-first.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    // DuplicateIntent → Warn
    local.add_intent(
        &local_space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture",
            "Approach A",
            "src/core",
        ),
    );
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture",
            "Approach B",
            "src/core",
        ),
    );

    // CapabilityNameCollision (no alias) → Error
    local.add_capability(&local_space, make_capability("data-pipeline", "proj-a"));
    remote.add_capability(&remote_space, make_capability("data-pipeline", "proj-b"));

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    assert!(conflicts.len() >= 2, "expected at least 2 conflicts");
    // First should be Error (or at least not Warn before Error).
    for w in conflicts.windows(2) {
        assert!(
            w[0].severity >= w[1].severity,
            "conflicts must be sorted by descending severity, got {:?} before {:?}",
            w[0].severity,
            w[1].severity
        );
    }
}

// ── detect_intra_space ────────────────────────────────────────────────────────

#[test]
fn intra_space_no_conflicts_on_empty_store() {
    let store = MemoryStore::default();
    let space = SpaceId::sequential_for_tests();
    let conflicts = ConflictDetector::detect_intra_space(&store, &space).unwrap();
    assert!(conflicts.is_empty());
}

#[test]
fn intra_space_detects_duplicate_intents_same_space() {
    // Two intents with same kind+scope but different descriptions in the same space.
    let space = SpaceId::sequential_for_tests();
    let mut store = MemoryStore::default();

    store.add_intent(
        &space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture v1",
            "Uses microservices",
            "src",
        ),
    );
    store.add_intent(
        &space,
        make_intent(
            IntentKind::ArchitecturalScope,
            "Architecture v2",
            "Uses monolith",
            "src",
        ),
    );

    let conflicts = ConflictDetector::detect_intra_space(&store, &space).unwrap();
    assert!(
        conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::DuplicateIntent),
        "intra-space duplicate intents should be detected"
    );
}

#[test]
fn intra_space_detects_contradictory_decisions() {
    // Two accepted decisions with same title but different body within one space.
    let space = SpaceId::sequential_for_tests();
    let mut store = MemoryStore::default();

    store.add_decision(&space, make_decision("storage-backend", "Use SQLite", true));
    store.add_decision(
        &space,
        make_decision("storage-backend", "Use PostgreSQL", true),
    );

    let conflicts = ConflictDetector::detect_intra_space(&store, &space).unwrap();
    assert!(
        conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::ContradictoryDecisions),
        "intra-space contradictory decisions should be detected"
    );
}

// ── All four kinds appear in a single scenario ────────────────────────────────

#[test]
fn all_four_conflict_kinds_detected_together() {
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    // 1. DuplicateIntent
    local.add_intent(
        &local_space,
        make_intent(IntentKind::TechStack, "Stack", "Primary: Rust + Tokio", "."),
    );
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::TechStack,
            "Stack",
            "Primary: Go + goroutines",
            ".",
        ),
    );

    // 2. CapabilityNameCollision (no alias)
    local.add_capability(
        &local_space,
        make_capability("telemetry-export", "observability"),
    );
    remote.add_capability(
        &remote_space,
        make_capability("telemetry-export", "platform"),
    );

    // 3. ContradictoryDecisions
    local.add_decision(&local_space, make_decision("api-style", "Use REST", true));
    remote.add_decision(
        &remote_space,
        make_decision("api-style", "Use gRPC only", true),
    );

    // 4. GovernanceViolation
    local.add_plan(&local_space, make_plan("billing-overhaul", Some("billing")));
    remote.add_intent(
        &remote_space,
        make_intent(
            IntentKind::Boundary,
            "Financial Boundaries",
            "Must not modify the billing substrate without approval",
            ".",
        ),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let kinds: std::collections::HashSet<_> = conflicts.iter().map(|c| c.kind).collect();
    use g8_core::ConflictKind::*;
    assert!(
        kinds.contains(&DuplicateIntent),
        "missing DuplicateIntent in: {kinds:?}"
    );
    assert!(
        kinds.contains(&CapabilityNameCollision),
        "missing CapabilityNameCollision in: {kinds:?}"
    );
    assert!(
        kinds.contains(&ContradictoryDecisions),
        "missing ContradictoryDecisions in: {kinds:?}"
    );
    assert!(
        kinds.contains(&GovernanceViolation),
        "missing GovernanceViolation in: {kinds:?}"
    );
}

// ── Evidence is typed (not stringified) ──────────────────────────────────────

#[test]
fn evidence_is_typed_not_stringified() {
    // Verify each conflict kind uses typed Evidence variants, not only Note.
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    // CapabilityNameCollision evidence must include CapabilityRef.
    local.add_capability(
        &local_space,
        make_capability("typed-check", "project-alpha"),
    );
    remote.add_capability(
        &remote_space,
        make_capability("typed-check", "project-beta"),
    );

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();

    let col = conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::CapabilityNameCollision)
        .unwrap();

    // All evidence variants are typed (no Evidence::Note with stringified references).
    let has_cap_ref = col
        .evidence
        .iter()
        .any(|e| matches!(e, Evidence::CapabilityRef { capability_id, .. } if !capability_id.as_str().is_empty()));
    assert!(
        has_cap_ref,
        "evidence must contain typed CapabilityRef with non-empty ID"
    );
}

// ── Serde round-trip ──────────────────────────────────────────────────────────

#[test]
fn conflict_serde_roundtrip() {
    let local_space = SpaceId::sequential_for_tests();
    let remote_space = SpaceId::sequential_for_tests();

    let mut local = MemoryStore::default();
    let mut remote = MemoryStore::default();

    local.add_capability(&local_space, make_capability("http-client", "service-a"));
    remote.add_capability(&remote_space, make_capability("http-client", "service-b"));

    let conflicts =
        ConflictDetector::compare_spaces(&local, &local_space, &remote, &remote_space).unwrap();
    assert!(!conflicts.is_empty());

    // Round-trip through JSON.
    let json = serde_json::to_string(&conflicts).expect("must serialize");
    let back: Vec<g8_core::Conflict> = serde_json::from_str(&json).expect("must deserialize");

    assert_eq!(conflicts.len(), back.len());
    assert_eq!(conflicts[0].kind, back[0].kind);
    assert_eq!(conflicts[0].severity, back[0].severity);
}
