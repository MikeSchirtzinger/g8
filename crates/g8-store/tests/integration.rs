//! Integration tests for `g8-store`.
//!
//! Covers:
//! 1. Schema migration on fresh DB (in-memory and file-backed)
//! 2. CRUD round-trip for all primary entities
//! 3. `planner_intent_check` end-to-end with sample data
//! 4. Cross-project ATTACH federation (read-only)

use std::collections::BTreeMap;
use std::path::PathBuf;

use g8_core::{
    now_millis,
    plan::{OverlapKind, PlanDraft, PlanIntentRelation},
    Annotation, AnnotationKind, AnnotationValue, Capability, CapabilityId, CapabilityStatus,
    ConvergenceSpace, Decision, DecisionStatus, EpochMillis, Intent, IntentId, IntentKind,
    IntentSourceKind, Plan, PlanId, PlanStatus, Project, ProjectId, SourceLocation, SpaceId,
};
use g8_store::{RusqliteStore, ScanResult, StoreConnection};

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_store() -> RusqliteStore {
    let mut s = RusqliteStore::open_in_memory().expect("open_in_memory");
    s.migrate().expect("migrate");
    s
}

fn ts() -> EpochMillis {
    now_millis()
}

fn make_space(id: SpaceId) -> ConvergenceSpace {
    ConvergenceSpace {
        id,
        name: "test-space".into(),
        root_path: PathBuf::from("/tmp/test"),
        created_at: ts(),
        updated_at: ts(),
        members: vec![],
        meta: None,
    }
}

fn make_project(id: ProjectId, space_id: SpaceId) -> Project {
    Project {
        id,
        space_id,
        name: "test-project".into(),
        description: None,
        root_path: PathBuf::from("/tmp/test/proj"),
        language: Some("rust".into()),
        created_at: ts(),
        updated_at: ts(),
        meta: None,
    }
}

fn make_capability(id: CapabilityId, project_id: ProjectId, name: &str) -> Capability {
    Capability {
        id,
        project_id,
        name: name.into(),
        description: Some("test cap".into()),
        substrate: Some("api".into()),
        consumes: vec!["input".into()],
        produces: vec!["output".into()],
        status: CapabilityStatus::InFlight,
        stub: false,
        stub_since: None,
        owner: None,
        source: SourceLocation {
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            column: 0,
        },
        annotation_raw: "@g8.capability(name = \"test-cap\")".into(),
        conditional: false,
        created_at: ts(),
        updated_at: ts(),
        meta: None,
    }
}

fn make_intent(id: IntentId, space_id: SpaceId, project_id: Option<ProjectId>) -> Intent {
    Intent {
        id,
        space_id,
        project_id,
        kind: IntentKind::ArchitecturalScope,
        heading: "Keep the API simple".into(),
        description: "Minimal surface area for the public API".into(),
        scope_path: PathBuf::from("/tmp/test"),
        scope_depth: 2,
        substrate: Some("api".into()),
        owner: None,
        source: SourceLocation {
            file: PathBuf::from("AGENTS.md"),
            line: 5,
            column: 0,
        },
        source_kind: IntentSourceKind::AgentsMd,
        created_at: ts(),
        updated_at: ts(),
        meta: None,
    }
}

fn make_plan(id: PlanId, space_id: SpaceId) -> Plan {
    Plan {
        id,
        space_id,
        project_id: None,
        title: "Add streaming export".into(),
        description: Some("Export data as NDJSON stream".into()),
        status: PlanStatus::Idea,
        substrate: Some("api".into()),
        touched_capabilities: vec![],
        derived_from_intents: vec![],
        governed_by_decisions: vec![],
        wip_weight: 1,
        parked_reason: None,
        blocked_reason: None,
        dispatched_at: None,
        completed_at: None,
        created_at: ts(),
        updated_at: ts(),
        meta: None,
    }
}

// ── 1. Schema migration ───────────────────────────────────────────────────────

#[test]
fn test_migration_in_memory() {
    let mut store = RusqliteStore::open_in_memory().expect("open_in_memory failed");
    store.migrate().expect("migrate failed");

    // migrate() is idempotent — calling it again should not fail
    store.migrate().expect("second migrate failed");
}

#[test]
fn test_migration_file_backed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("store.db");

    let mut store = RusqliteStore::open(&db_path).expect("open file failed");
    store.migrate().expect("migrate failed");
}

// ── 2. CRUD round-trips ───────────────────────────────────────────────────────

#[test]
fn test_space_project_crud() {
    let mut store = make_store();

    let space_id = SpaceId::sequential_for_tests();
    let space = make_space(space_id.clone());
    store.init_space(&space).expect("init_space");

    // Retrieve
    let got = store
        .get_space(&space_id)
        .expect("get_space")
        .expect("should exist");
    assert_eq!(got.id, space_id);
    assert_eq!(got.name, "test-space");

    // get_default_space
    let default = store
        .get_default_space()
        .expect("get_default_space")
        .expect("should exist");
    assert_eq!(default.id, space_id);

    // Project CRUD
    let proj_id = ProjectId::sequential_for_tests();
    let proj = make_project(proj_id.clone(), space_id.clone());
    store.upsert_project(&proj).expect("upsert_project");

    let got_proj = store
        .get_project(&proj_id)
        .expect("get_project")
        .expect("should exist");
    assert_eq!(got_proj.id, proj_id);
    assert_eq!(got_proj.space_id, space_id);
    assert_eq!(got_proj.name, "test-project");

    let projects = store.list_projects(&space_id).expect("list_projects");
    assert_eq!(projects.len(), 1);
}

#[test]
fn test_capability_crud() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    let cap_id = CapabilityId::sequential_for_tests();
    let cap = make_capability(cap_id.clone(), proj_id.clone(), "stream-export");
    store.upsert_capability(&cap).expect("upsert_capability");

    // get
    let got = store
        .get_capability(&cap_id)
        .expect("get_capability")
        .expect("should exist");
    assert_eq!(got.id, cap_id);
    assert_eq!(got.name, "stream-export");
    assert_eq!(got.status, CapabilityStatus::InFlight);

    // find by name
    let found = store
        .find_capability_by_name(&proj_id, "stream-export")
        .expect("find_capability_by_name")
        .expect("should exist");
    assert_eq!(found.id, cap_id);

    // list
    let caps = store
        .list_capabilities(&proj_id)
        .expect("list_capabilities");
    assert_eq!(caps.len(), 1);

    // delete
    let deleted = store
        .delete_capabilities_for_project(&proj_id)
        .expect("delete_capabilities");
    assert_eq!(deleted, 1);
    assert!(store.list_capabilities(&proj_id).expect("list").is_empty());
}

#[test]
fn test_intent_crud() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    let intent_id = IntentId::sequential_for_tests();
    let intent = make_intent(intent_id.clone(), space_id.clone(), Some(proj_id.clone()));
    store.upsert_intent(&intent).expect("upsert_intent");

    // list_intents_at_path
    let intents = store
        .list_intents_at_path(&space_id, &PathBuf::from("/tmp/test/subdir"))
        .expect("list_intents_at_path");
    assert!(!intents.is_empty());
    assert_eq!(intents[0].id, intent_id);

    // delete
    let n = store
        .delete_intents_for_project(&proj_id)
        .expect("delete_intents");
    assert_eq!(n, 1);
}

#[test]
fn test_plan_crud() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    let plan_id = PlanId::sequential_for_tests();
    let plan = make_plan(plan_id.clone(), space_id.clone());
    store.create_plan(&plan).expect("create_plan");

    // get
    let got = store
        .get_plan(&plan_id)
        .expect("get_plan")
        .expect("should exist");
    assert_eq!(got.id, plan_id);
    assert_eq!(got.status, PlanStatus::Idea);

    // update status
    store
        .update_plan_status(&plan_id, PlanStatus::Scoped, None)
        .expect("update_plan_status");
    let updated = store
        .get_plan(&plan_id)
        .expect("get")
        .expect("should exist");
    assert_eq!(updated.status, PlanStatus::Scoped);

    // list
    let plans = store
        .list_plans(&space_id, Default::default())
        .expect("list_plans");
    assert_eq!(plans.len(), 1);
}

#[test]
fn test_decision_crud() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    let dec_id = g8_core::DecisionId::sequential_for_tests();
    let decision = Decision {
        id: dec_id.clone(),
        space_id: space_id.clone(),
        title: "Use SQLite for persistence".into(),
        status: DecisionStatus::Accepted,
        context: Some("v0 constraints".into()),
        body: "SQLite is fast enough and requires no daemon.".into(),
        source_file: None,
        created_at: ts(),
        updated_at: ts(),
        meta: None,
    };
    store.upsert_decision(&decision).expect("upsert_decision");

    let decisions = store.list_decisions(&space_id).expect("list_decisions");
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].title, "Use SQLite for persistence");
    assert_eq!(decisions[0].status, DecisionStatus::Accepted);
}

#[test]
fn test_substrate_budget_crud() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    // Default
    let default_budget = store
        .get_substrate_budget(&space_id, "api")
        .expect("get_substrate_budget");
    assert_eq!(default_budget.substrate, "api");
    assert_eq!(default_budget.wip_cap, 3);

    // Set
    store
        .set_substrate_budget(&space_id, "api", 5, 21)
        .expect("set_substrate_budget");
    let custom = store
        .get_substrate_budget(&space_id, "api")
        .expect("get_substrate_budget");
    assert_eq!(custom.wip_cap, 5);
    assert_eq!(custom.stale_threshold_days, 21);
}

#[test]
fn test_capability_alias() {
    let mut store = make_store();
    store
        .add_capability_alias("stream-export", "streaming-export")
        .expect("add alias");

    let canonical = store
        .resolve_canonical("streaming-export")
        .expect("resolve");
    assert_eq!(canonical, "stream-export");

    // Unknown resolves to self
    let unknown = store
        .resolve_canonical("totally-unknown")
        .expect("resolve unknown");
    assert_eq!(unknown, "totally-unknown");
}

#[test]
fn test_link_plan_capability_intent() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    let plan_id = PlanId::sequential_for_tests();
    store
        .create_plan(&make_plan(plan_id.clone(), space_id.clone()))
        .unwrap();

    let cap_id = CapabilityId::sequential_for_tests();
    store
        .upsert_capability(&make_capability(
            cap_id.clone(),
            proj_id.clone(),
            "export-cap",
        ))
        .unwrap();

    let intent_id = IntentId::sequential_for_tests();
    store
        .upsert_intent(&make_intent(intent_id.clone(), space_id.clone(), None))
        .unwrap();

    store
        .link_plan_capability(&plan_id, &cap_id, OverlapKind::Touches)
        .expect("link_plan_capability");
    store
        .link_plan_intent(&plan_id, &intent_id, PlanIntentRelation::Satisfies)
        .expect("link_plan_intent");

    // Retrieve plan and verify linked IDs
    let plan = store.get_plan(&plan_id).expect("get_plan").expect("exists");
    assert!(plan.touched_capabilities.contains(&cap_id));
    assert!(plan.derived_from_intents.contains(&intent_id));
}

// ── 3. planner_intent_check end-to-end ───────────────────────────────────────

#[test]
fn test_planner_intent_check_empty_store() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    let draft = PlanDraft {
        title: "New feature".into(),
        description: None,
        substrate: Some("api".into()),
        touched_capabilities: vec![],
        touched_paths: vec![],
        space_id: space_id.clone(),
        project_id: None,
    };

    let payload = store
        .planner_intent_check(&draft)
        .expect("planner_intent_check");

    // Empty store: all six fields return empty arrays or null
    assert!(payload.existing_matches.is_array());
    assert_eq!(payload.existing_matches.as_array().unwrap().len(), 0);
    assert!(payload.bottlenecks.is_array());
    assert!(payload.drift_hints.is_array());
    assert!(payload.intent_overlaps.is_array());
    assert!(payload.parked_ideas.is_array());
    // budget is null when no substrate_budget row exists
    assert!(payload.budget.is_null() || payload.budget.is_object());
}

#[test]
fn test_planner_intent_check_with_data() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    // Add a substrate budget
    store.set_substrate_budget(&space_id, "api", 3, 14).unwrap();

    // Add an existing plan in the same substrate
    let existing_plan_id = PlanId::sequential_for_tests();
    let mut existing_plan = make_plan(existing_plan_id.clone(), space_id.clone());
    existing_plan.title = "Existing API plan".into();
    existing_plan.status = PlanStatus::Dispatched;
    existing_plan.substrate = Some("api".into());
    store.create_plan(&existing_plan).unwrap();

    // Add a capability
    let cap_id = CapabilityId::sequential_for_tests();
    store
        .upsert_capability(&make_capability(
            cap_id.clone(),
            proj_id.clone(),
            "api-core",
        ))
        .unwrap();
    store
        .link_plan_capability(&existing_plan_id, &cap_id, OverlapKind::Touches)
        .unwrap();

    // Add an intent
    store
        .upsert_intent(&make_intent(
            IntentId::sequential_for_tests(),
            space_id.clone(),
            Some(proj_id.clone()),
        ))
        .unwrap();

    let draft = PlanDraft {
        title: "New API feature".into(),
        description: None,
        substrate: Some("api".into()),
        touched_capabilities: vec!["api-core".into()],
        touched_paths: vec![],
        space_id: space_id.clone(),
        project_id: None,
    };

    let payload = store
        .planner_intent_check(&draft)
        .expect("planner_intent_check");

    // existing_matches: should find the existing dispatched plan (capability overlap)
    let matches = payload.existing_matches.as_array().unwrap();
    assert!(!matches.is_empty(), "expected at least one existing match");

    // budget: should return the api budget
    assert!(payload.budget.is_object(), "budget should be an object");
    let budget = payload.budget.as_object().unwrap();
    assert_eq!(budget["substrate"].as_str().unwrap(), "api");
    assert_eq!(budget["wip_cap"].as_i64().unwrap(), 3);

    // intent_overlaps: should find the intent we added (same substrate)
    let overlaps = payload.intent_overlaps.as_array().unwrap();
    assert!(!overlaps.is_empty(), "expected at least one intent overlap");
}

#[test]
fn test_planner_intent_check_returns_parked() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    // Add a parked plan
    let parked_id = PlanId::sequential_for_tests();
    let mut parked = make_plan(parked_id.clone(), space_id.clone());
    parked.status = PlanStatus::Parked;
    parked.parked_reason = Some("deferred for now".into());
    parked.title = "API streaming parked".into();
    store.create_plan(&parked).unwrap();

    let draft = PlanDraft {
        title: "API streaming feature".into(),
        description: None,
        substrate: Some("api".into()),
        touched_capabilities: vec![],
        touched_paths: vec![],
        space_id: space_id.clone(),
        project_id: None,
    };

    let payload = store
        .planner_intent_check(&draft)
        .expect("planner_intent_check");

    let parked_ideas = payload.parked_ideas.as_array().unwrap();
    assert!(
        !parked_ideas.is_empty(),
        "expected parked ideas from title match"
    );
}

// ── 4. apply_scan round-trip ──────────────────────────────────────────────────

#[test]
fn test_apply_scan() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    // Build a ScanResult with a capability annotation
    let mut fields = BTreeMap::new();
    fields.insert("status".into(), AnnotationValue::String("in_flight".into()));
    fields.insert("substrate".into(), AnnotationValue::String("api".into()));

    let scan = ScanResult {
        annotations: vec![Annotation {
            kind: AnnotationKind::Capability,
            name: Some("scan-exported-cap".into()),
            heading: None,
            fields: fields.clone(),
            source: SourceLocation {
                file: PathBuf::from("src/api.rs"),
                line: 10,
                column: 0,
            },
            source_kind: IntentSourceKind::InlineComment,
            conditional: false,
            raw_text: "@g8.capability(name = \"scan-exported-cap\", status = \"in_flight\")".into(),
        }],
        warnings: vec![],
    };

    let report = store.apply_scan(&proj_id, scan).expect("apply_scan");
    assert_eq!(report.inserted_capabilities, 1);
    assert_eq!(report.deleted_capabilities, 0);

    // Verify the capability landed
    let caps = store
        .list_capabilities(&proj_id)
        .expect("list_capabilities");
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].name, "scan-exported-cap");

    // Apply again (idempotent delete+re-insert)
    let scan2 = ScanResult {
        annotations: vec![],
        warnings: vec![],
    };
    let report2 = store.apply_scan(&proj_id, scan2).expect("apply_scan again");
    assert_eq!(report2.deleted_capabilities, 1);
    assert!(store.list_capabilities(&proj_id).expect("list").is_empty());
}

// ── 5. pairing_check ─────────────────────────────────────────────────────────

#[test]
fn test_pairing_check_missing_test() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id.clone()))
        .unwrap();

    // Add a non-stub capability
    let cap_id = CapabilityId::sequential_for_tests();
    store
        .upsert_capability(&make_capability(
            cap_id.clone(),
            proj_id.clone(),
            "unpaired-cap",
        ))
        .unwrap();

    let errors = store.pairing_check(Some(&proj_id)).expect("pairing_check");
    // Should flag unpaired-cap as missing a convergence test
    assert!(
        errors.iter().any(|e| e.capability == "unpaired-cap"),
        "expected pairing error for unpaired-cap, got: {:?}",
        errors
    );
}

#[test]
fn test_apply_scan_persists_convergence_test_for_pairing() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    let proj_id = ProjectId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store
        .upsert_project(&make_project(proj_id.clone(), space_id))
        .unwrap();

    let mut capability_fields = BTreeMap::new();
    capability_fields.insert("status".into(), AnnotationValue::String("in_flight".into()));
    let mut test_fields = BTreeMap::new();
    test_fields.insert(
        "for_capability".into(),
        AnnotationValue::String("paired-cap".into()),
    );
    test_fields.insert(
        "scenario".into(),
        AnnotationValue::String("real scenario".into()),
    );

    let annotations = vec![
        Annotation {
            kind: AnnotationKind::Capability,
            name: Some("paired-cap".into()),
            heading: None,
            fields: capability_fields,
            source: SourceLocation {
                file: PathBuf::from("src/lib.rs"),
                line: 1,
                column: 0,
            },
            source_kind: IntentSourceKind::InlineComment,
            conditional: false,
            raw_text: "@g8.capability(name = \"paired-cap\")".into(),
        },
        Annotation {
            kind: AnnotationKind::ConvergenceTest,
            name: None,
            heading: None,
            fields: test_fields,
            source: SourceLocation {
                file: PathBuf::from("src/lib.rs"),
                line: 2,
                column: 0,
            },
            source_kind: IntentSourceKind::InlineComment,
            conditional: false,
            raw_text:
                "@g8.convergence_test(for_capability = \"paired-cap\", scenario = \"real scenario\")"
                    .into(),
        },
    ];

    store
        .apply_scan(
            &proj_id,
            ScanResult {
                annotations,
                warnings: vec![],
            },
        )
        .expect("apply paired scan");

    let errors = store.pairing_check(Some(&proj_id)).expect("pairing_check");
    assert!(errors.is_empty(), "expected paired capability: {errors:?}");
}

// ── 6. stale_dispatched_plans ────────────────────────────────────────────────

#[test]
fn test_stale_dispatched_plans_empty() {
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();

    let stale = store
        .stale_dispatched_plans(&space_id)
        .expect("stale_dispatched_plans");
    assert!(stale.is_empty());
}

// ── 7. Cross-store ATTACH ────────────────────────────────────────────────────

#[test]
fn test_attach_remote_store() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create a remote store with some data
    let remote_path = tmp.path().join("remote.db");
    {
        let mut remote = RusqliteStore::open(&remote_path).expect("open remote");
        remote.migrate().expect("migrate remote");
        let sid = SpaceId::sequential_for_tests();
        remote
            .init_space(&make_space(sid))
            .expect("init remote space");
    }

    // Open local store and attach the remote
    let local_path = tmp.path().join("local.db");
    let mut local = RusqliteStore::open(&local_path).expect("open local");
    local.migrate().expect("migrate local");

    local
        .attach_remote_store("remote", &remote_path)
        .expect("attach_remote_store");

    // Detach
    local
        .detach_remote_store("remote")
        .expect("detach_remote_store");
}

#[test]
fn test_attach_remote_store_path_with_special_chars() {
    // A remote path containing characters that are significant in a `file:` URI
    // (`?`, `#`, space) must still attach — the path is percent-encoded and
    // bound, not interpolated into a quoted literal. Regression test for the
    // old `.replace(' ', "%20")`-only encoding that truncated paths at `?`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let weird_dir = tmp.path().join("a ? weird # dir");
    std::fs::create_dir_all(&weird_dir).expect("create weird dir");

    let remote_path = weird_dir.join("remote.db");
    {
        let mut remote = RusqliteStore::open(&remote_path).expect("open remote");
        remote.migrate().expect("migrate remote");
        remote
            .init_space(&make_space(SpaceId::sequential_for_tests()))
            .expect("init remote space");
    }

    let mut local = RusqliteStore::open(&tmp.path().join("local.db")).expect("open local");
    local.migrate().expect("migrate local");
    local
        .attach_remote_store("remote", &remote_path)
        .expect("attach remote with special-char path");
    local.detach_remote_store("remote").expect("detach remote");
}

#[test]
fn test_attach_remote_store_rejects_bad_alias() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let remote_path = tmp.path().join("remote.db");
    {
        let mut remote = RusqliteStore::open(&remote_path).expect("open remote");
        remote.migrate().expect("migrate remote");
    }
    let mut local = RusqliteStore::open(&tmp.path().join("local.db")).expect("open local");
    local.migrate().expect("migrate local");

    // An alias that isn't a bare identifier must be rejected (it can't be
    // parameterised in ATTACH, so it would otherwise be a SQL-injection seam).
    assert!(local
        .attach_remote_store("evil\" AS x; DROP TABLE plan; --", &remote_path)
        .is_err());
}

#[test]
fn test_blocked_plan_counts_toward_wip() {
    // SPEC §7: "Blocked doesn't free a slot — blocked work still occupies the
    // substrate's attention surface." A Blocked plan must count toward wip_current
    // exactly like a Dispatched one.
    let mut store = make_store();
    let space_id = SpaceId::sequential_for_tests();
    store.init_space(&make_space(space_id.clone())).unwrap();
    store.set_substrate_budget(&space_id, "api", 2, 14).unwrap();

    // One Dispatched + one Blocked on the same substrate = 2 occupied slots.
    let mut dispatched = make_plan(PlanId::sequential_for_tests(), space_id.clone());
    dispatched.title = "dispatched-one".into();
    dispatched.status = PlanStatus::Dispatched;
    dispatched.substrate = Some("api".into());
    store.create_plan(&dispatched).unwrap();

    let mut blocked = make_plan(PlanId::sequential_for_tests(), space_id.clone());
    blocked.title = "blocked-one".into();
    blocked.status = PlanStatus::Blocked;
    blocked.substrate = Some("api".into());
    store.create_plan(&blocked).unwrap();

    let draft = PlanDraft {
        title: "new-api-thing".into(),
        description: None,
        substrate: Some("api".into()),
        touched_capabilities: vec![],
        touched_paths: vec![],
        space_id: space_id.clone(),
        project_id: None,
    };
    let payload = store
        .planner_intent_check(&draft)
        .expect("planner_intent_check");
    let budget = payload.budget.as_object().expect("budget object");
    assert_eq!(
        budget["wip_current"].as_i64().unwrap(),
        2,
        "Blocked plan must count toward WIP alongside Dispatched"
    );
    assert!(budget["at_cap"].as_bool().unwrap(), "2/2 should be at cap");
}

// ── 8. file-watch hook ────────────────────────────────────────────────────────

#[test]
fn test_watch_returns_handle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("watch.db");
    let mut store = RusqliteStore::open(&db_path).expect("open");
    store.migrate().expect("migrate");

    // watch() should succeed and return a handle
    let _handle = store.watch(|| {}).expect("watch");
    // Dropping the handle stops the watcher (no assertion needed)
}
