//! Integration test: [`g8_conflict::ConflictAdapter`] against the real
//! SQLite store.
//!
//! Lives in `tests/` (not `src/`) deliberately: OBL-D5-01 forbids naming the
//! concrete store type anywhere in `g8-conflict/src/**` — the adapter under
//! test is generic over `StoreConnection`; only this test binds it to the
//! concrete implementation to prove the delegation is real, not mocked.

use g8_conflict::{ConflictAdapter, ConflictStore};
use g8_core::{
    now_millis, Capability, CapabilityId, CapabilityStatus, ConvergenceSpace, Project, ProjectId,
    SourceLocation, SpaceId,
};
use g8_store::{RusqliteStore, StoreConnection};

fn seeded_store() -> (RusqliteStore, SpaceId) {
    let mut store = RusqliteStore::open_in_memory().expect("open in-memory store");
    store.migrate().expect("migrate");

    let space = ConvergenceSpace {
        id: SpaceId::sequential_for_tests(),
        name: "test-space".into(),
        root_path: "/tmp/test-space".into(),
        created_at: now_millis(),
        updated_at: now_millis(),
        members: vec![],
        meta: None,
    };
    store.init_space(&space).expect("init_space");

    let project = Project {
        id: ProjectId::sequential_for_tests(),
        space_id: space.id.clone(),
        name: "member-project".into(),
        description: None,
        root_path: "/tmp/test-space/member".into(),
        language: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store.upsert_project(&project).expect("upsert_project");

    let cap = Capability {
        id: CapabilityId::sequential_for_tests(),
        project_id: project.id.clone(),
        name: "http-fetch".into(),
        description: Some("fetches things".into()),
        substrate: Some("net".into()),
        consumes: vec![],
        produces: vec![],
        status: CapabilityStatus::InFlight,
        stub: false,
        stub_since: None,
        owner: None,
        source: SourceLocation::unknown("src/lib.rs"),
        annotation_raw:
            "// @g8.capability(name = \"http-fetch\", status = \"in_flight\", substrate = \"net\")"
                .into(),
        conditional: false,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store.upsert_capability(&cap).expect("upsert_capability");

    (store, space.id)
}

#[test]
fn adapter_delegates_list_capabilities_to_the_real_store() {
    let (store, space_id) = seeded_store();
    let adapter = ConflictAdapter::new(&store);

    let rows = adapter.list_capabilities(&space_id).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "http-fetch");
    assert_eq!(rows[0].project_name, "member-project");
}

#[test]
fn adapter_lists_are_empty_for_unknown_space() {
    let (store, _) = seeded_store();
    let adapter = ConflictAdapter::new(&store);

    let other = SpaceId::sequential_for_tests();
    assert!(adapter.list_capabilities(&other).expect("caps").is_empty());
    assert!(adapter
        .list_decisions(&other)
        .expect("decisions")
        .is_empty());
    assert!(adapter.list_plans(&other).expect("plans").is_empty());
}

#[test]
fn adapter_resolve_canonical_is_identity_when_no_alias_registered() {
    let (store, _) = seeded_store();
    let adapter = ConflictAdapter::new(&store);

    // The real store's contract (store_impl.rs::resolve_canonical): an
    // unaliased name resolves to itself, not to an error. (MemoryStore's
    // Err-on-missing is the stricter test-double behavior; the adapter must
    // surface the real semantics.)
    let resolved = adapter.resolve_canonical("ghost::name").expect("identity");
    assert_eq!(resolved, "ghost::name");
}
