//! `StoreConnection` trait + `RusqliteStore` implementation.
//!
//! Every public method opens a tracing span per ARCHITECTURE.md §12.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::migrations;

use g8_core::{
    model::{
        Capability, CapabilityStatus, ConvergenceSpace, Decision, DecisionStatus, Intent,
        IntentKind, IntentSourceKind, Plan, Project,
    },
    owner::Owner,
    plan::{OverlapKind, PlanDraft, PlanIntentRelation, PlanStatus},
    substrate::SubstrateBudget,
    AnnotationId, CapabilityId, DecisionId, EpochMillis, G8Error, IntentId, PlanId, ProjectId,
    SpaceId,
};

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors originating from `g8-store` operations.
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
    #[error("notify: {0}")]
    Notify(String),
}

// ── Auxiliary public types ───────────────────────────────────────────────────

/// Summary returned by [`StoreConnection::apply_scan`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyScanReport {
    pub inserted_capabilities: u32,
    pub inserted_intents: u32,
    pub inserted_decisions: u32,
    pub deleted_capabilities: u32,
    pub deleted_intents: u32,
    pub warnings: Vec<String>,
}

/// Raw JSON payloads returned by the composite `planner_intent_check` query.
///
/// `g8-planner` deserializes these into typed structs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawIntentCheckPayload {
    pub existing_matches: serde_json::Value,
    pub budget: serde_json::Value,
    pub bottlenecks: serde_json::Value,
    pub drift_hints: serde_json::Value,
    pub intent_overlaps: serde_json::Value,
    pub parked_ideas: serde_json::Value,
}

/// A capability that lacks its required paired convergence test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingError {
    pub capability: String,
    pub file: PathBuf,
    pub line: u32,
    pub reason: PairingErrorReason,
}

/// Reason codes matching ARCH §14 stable vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingErrorReason {
    NoMatchingConvergenceTest,
    StubExpired,
    StubMissingSince,
    DuplicateCapabilityName,
}

/// Filter for [`StoreConnection::list_plans`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanFilter {
    pub status: Option<Vec<PlanStatus>>,
    pub substrate: Option<String>,
    pub project: Option<ProjectId>,
    pub limit: Option<u32>,
}

/// A dispatched plan that has exceeded the substrate's stale threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalePlanCandidate {
    pub plan_id: PlanId,
    pub title: String,
    pub substrate: Option<String>,
    pub dispatched_at: EpochMillis,
    pub days_stale: i64,
}

/// Handle returned by [`StoreConnection::watch`].
///
/// Dropping this handle stops the file watcher.
pub struct WatchHandle {
    _watcher: Arc<Mutex<Option<notify::RecommendedWatcher>>>,
}

impl WatchHandle {
    fn new(watcher: notify::RecommendedWatcher) -> Self {
        Self {
            _watcher: Arc::new(Mutex::new(Some(watcher))),
        }
    }
}

// ── ScanResult re-export stub (used by apply_scan) ──────────────────────────

/// Minimal scan result accepted by [`StoreConnection::apply_scan`].
///
/// The full `ScanResult` lives in `g8-extractor`; this mirrors its shape so
/// `g8-store` can accept scans without depending on `g8-extractor`.
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub annotations: Vec<g8_core::annotation::Annotation>,
    pub warnings: Vec<String>,
}

// ── StoreConnection trait ────────────────────────────────────────────────────

/// The trait abstraction over the underlying storage.
///
/// `RusqliteStore` is the v0.1 implementation. A future `TursoStore` can be
/// added without touching `g8-planner`, `g8-conflict`, or `g8`.
pub trait StoreConnection: Send + Sync {
    // ── Lifecycle ──────────────────────────────────────────────────────────

    fn init_space(&mut self, space: &ConvergenceSpace) -> Result<(), StoreError>;
    fn get_space(&self, id: &SpaceId) -> Result<Option<ConvergenceSpace>, StoreError>;
    fn get_default_space(&self) -> Result<Option<ConvergenceSpace>, StoreError>;
    fn upsert_project(&mut self, project: &Project) -> Result<(), StoreError>;
    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, StoreError>;
    fn list_projects(&self, space: &SpaceId) -> Result<Vec<Project>, StoreError>;
    /// Find a project in `space` by exact name match.
    fn find_project_by_name(
        &self,
        space: &SpaceId,
        name: &str,
    ) -> Result<Option<Project>, StoreError>;
    /// Delete a project and ON DELETE CASCADE its capabilities / intents / plans / annotations.
    fn delete_project(&mut self, id: &ProjectId) -> Result<(), StoreError>;

    // ── Bulk scan apply ────────────────────────────────────────────────────

    /// Apply a complete extractor scan for one project.  Idempotent: deletes
    /// prior rows for the project then inserts the new ones.  Uses **one**
    /// write transaction (R4 §6 Risk 2 mitigation).
    fn apply_scan(
        &mut self,
        project: &ProjectId,
        scan: ScanResult,
    ) -> Result<ApplyScanReport, StoreError>;

    // ── Capability CRUD ────────────────────────────────────────────────────

    fn upsert_capability(&mut self, cap: &Capability) -> Result<(), StoreError>;
    fn get_capability(&self, id: &CapabilityId) -> Result<Option<Capability>, StoreError>;
    fn find_capability_by_name(
        &self,
        project: &ProjectId,
        name: &str,
    ) -> Result<Option<Capability>, StoreError>;
    fn list_capabilities(&self, project: &ProjectId) -> Result<Vec<Capability>, StoreError>;
    fn delete_capabilities_for_project(&mut self, project: &ProjectId) -> Result<u64, StoreError>;

    // ── Intent CRUD ────────────────────────────────────────────────────────

    fn upsert_intent(&mut self, intent: &Intent) -> Result<(), StoreError>;
    /// Returns intents ordered by `scope_depth DESC` (most-specific first).
    fn list_intents_at_path(&self, space: &SpaceId, path: &Path)
        -> Result<Vec<Intent>, StoreError>;
    fn delete_intents_for_project(&mut self, project: &ProjectId) -> Result<u64, StoreError>;

    // ── Plan CRUD ──────────────────────────────────────────────────────────

    fn create_plan(&mut self, plan: &Plan) -> Result<(), StoreError>;
    fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError>;
    fn list_plans(&self, space: &SpaceId, filter: PlanFilter) -> Result<Vec<Plan>, StoreError>;
    fn update_plan_status(
        &mut self,
        id: &PlanId,
        new_status: PlanStatus,
        reason: Option<String>,
    ) -> Result<(), StoreError>;
    fn link_plan_capability(
        &mut self,
        plan: &PlanId,
        capability: &CapabilityId,
        overlap_kind: OverlapKind,
    ) -> Result<(), StoreError>;
    fn link_plan_intent(
        &mut self,
        plan: &PlanId,
        intent: &IntentId,
        relation: PlanIntentRelation,
    ) -> Result<(), StoreError>;

    // ── Decision CRUD ──────────────────────────────────────────────────────

    fn upsert_decision(&mut self, decision: &Decision) -> Result<(), StoreError>;
    fn list_decisions(&self, space: &SpaceId) -> Result<Vec<Decision>, StoreError>;

    // ── Substrate budget CRUD ──────────────────────────────────────────────

    fn set_substrate_budget(
        &mut self,
        space: &SpaceId,
        substrate: &str,
        wip_cap: u32,
        stale_threshold_days: u32,
    ) -> Result<(), StoreError>;
    /// Returns the stored budget or a default if none has been configured.
    fn get_substrate_budget(
        &self,
        space: &SpaceId,
        substrate: &str,
    ) -> Result<SubstrateBudget, StoreError>;
    /// List every substrate_budget row registered in `space`. Returned in
    /// substrate-name order. Substrates derived only from in-flight plans are
    /// NOT included — registration (via [`set_substrate_budget`]) is required.
    fn list_substrate_budgets(&self, space: &SpaceId) -> Result<Vec<SubstrateBudget>, StoreError>;

    // ── Cross-project capability aliases ──────────────────────────────────

    fn add_capability_alias(&mut self, canonical: &str, alias: &str) -> Result<(), StoreError>;
    fn resolve_canonical(&self, ref_name: &str) -> Result<String, StoreError>;

    // ── Composite queries (load-bearing surface for g8-planner) ──────────

    /// Single-transaction composite query (ARCHITECTURE.md §7).
    /// Returns six JSON-deserializable payloads that `g8-planner` assembles
    /// into a `FitReport`.
    fn planner_intent_check(&self, draft: &PlanDraft) -> Result<RawIntentCheckPayload, StoreError>;

    /// One-shot pairing-gate check for a project (or all projects in the store).
    fn pairing_check(&self, project: Option<&ProjectId>) -> Result<Vec<PairingError>, StoreError>;

    /// Find dispatched plans older than the substrate's stale_threshold_days.
    fn stale_dispatched_plans(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<StalePlanCandidate>, StoreError>;

    // ── Cross-store attach (multi-project federation) ──────────────────────

    /// Attach another `.g8/store.db` as a named read-only alias.
    fn attach_remote_store(&mut self, alias: &str, path: &Path) -> Result<(), StoreError>;
    fn detach_remote_store(&mut self, alias: &str) -> Result<(), StoreError>;

    // ── File-watch hook ────────────────────────────────────────────────────

    /// Watch the store file for external changes.  Calls `callback` each time
    /// the store file is modified.  Returns a handle that stops the watch when
    /// dropped.
    fn watch<F>(&self, callback: F) -> Result<WatchHandle, StoreError>
    where
        F: Fn() + Send + 'static;
}

// ── RusqliteStore ────────────────────────────────────────────────────────────

/// SQLite-backed implementation of [`StoreConnection`].
///
/// Uses rusqlite with the `bundled` SQLite amalgamation so the binary is
/// fully self-contained (no system libsqlite3 required).
pub struct RusqliteStore {
    conn: Mutex<Connection>,
    store_path: PathBuf,
}

impl RusqliteStore {
    /// Open (or create) the store at `path`.
    ///
    /// Applies the PRAGMA block mandated by ARCHITECTURE.md §3.3:
    /// WAL mode, foreign keys, NORMAL sync, 5 s busy timeout, 8 MB page cache.
    ///
    /// Call [`migrate`](Self::migrate) after opening to apply schema migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::apply_pragmas(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            store_path: path.to_path_buf(),
        })
    }

    /// Open an in-memory store.  Useful in tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::apply_pragmas(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            store_path: PathBuf::from(":memory:"),
        })
    }

    /// Apply all pending refinery migrations.
    ///
    /// Safe to call multiple times — refinery is idempotent.
    ///
    /// Foreign-key enforcement is switched off for the duration of the run
    /// and restored afterwards. Migrations that rebuild a table (V2 rebuilds
    /// `intent` to change a CHECK constraint) need this: `PRAGMA foreign_keys`
    /// is a no-op inside the transaction refinery opens per migration, and
    /// dropping a referenced table with enforcement on fails. A
    /// `PRAGMA foreign_key_check` after the run turns any dangling reference
    /// into an error instead of a silently inconsistent store.
    pub fn migrate(&mut self) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
        let outcome = Self::run_migrations(&mut conn);
        let restored = conn.execute_batch("PRAGMA foreign_keys=ON;");
        outcome?;
        restored?;
        Ok(())
    }

    fn run_migrations(conn: &mut Connection) -> Result<(), StoreError> {
        migrations::migrations::runner()
            .run(conn)
            .map_err(|e| StoreError::Migration(e.to_string()))?;
        let violations: i64 =
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if violations > 0 {
            return Err(StoreError::Migration(format!(
                "{violations} foreign key violation(s) after migrations (PRAGMA foreign_key_check)"
            )));
        }
        Ok(())
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;
             PRAGMA cache_size=-8000;",
        )?;
        Ok(())
    }

    fn now() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Lock the connection mutex.  Call once at the top of each method; do not
    /// hold the guard across `?` / await points.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

// ── Conversion helpers ───────────────────────────────────────────────────────

fn capability_status_to_str(s: CapabilityStatus) -> &'static str {
    match s {
        CapabilityStatus::Proposed => "proposed",
        CapabilityStatus::InFlight => "in_flight",
        CapabilityStatus::Landed => "landed",
        CapabilityStatus::Stale => "stale",
        CapabilityStatus::Superseded => "superseded",
        CapabilityStatus::Parked => "parked",
    }
}

fn capability_status_from_str(s: &str) -> Result<CapabilityStatus, StoreError> {
    match s {
        "proposed" => Ok(CapabilityStatus::Proposed),
        "in_flight" => Ok(CapabilityStatus::InFlight),
        "landed" => Ok(CapabilityStatus::Landed),
        "stale" => Ok(CapabilityStatus::Stale),
        "superseded" => Ok(CapabilityStatus::Superseded),
        "parked" => Ok(CapabilityStatus::Parked),
        other => Err(StoreError::NotFound(format!(
            "unknown capability status: {other}"
        ))),
    }
}

fn plan_status_to_str(s: PlanStatus) -> &'static str {
    match s {
        PlanStatus::Idea => "Idea",
        PlanStatus::Scoped => "Scoped",
        PlanStatus::Dispatched => "Dispatched",
        PlanStatus::Blocked => "Blocked",
        PlanStatus::Done => "Done",
        PlanStatus::Parked => "Parked",
    }
}

/// Percent-encode the characters that are significant in the path component of
/// a SQLite `file:` URI. `%` is encoded first so we never double-encode an
/// escape we just produced. ASCII control characters are encoded too. Everything
/// else (including `/` and unicode) is passed through — SQLite accepts raw UTF-8
/// in `file:` URIs and only these few bytes carry URI-syntactic meaning.
fn encode_uri_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3F"),
            '#' => out.push_str("%23"),
            ' ' => out.push_str("%20"),
            c if (c as u32) < 0x20 => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn plan_status_from_str(s: &str) -> Result<PlanStatus, StoreError> {
    match s {
        "Idea" => Ok(PlanStatus::Idea),
        "Scoped" => Ok(PlanStatus::Scoped),
        "Dispatched" => Ok(PlanStatus::Dispatched),
        "Blocked" => Ok(PlanStatus::Blocked),
        "Done" => Ok(PlanStatus::Done),
        "Parked" => Ok(PlanStatus::Parked),
        other => Err(StoreError::NotFound(format!(
            "unknown plan status: {other}"
        ))),
    }
}

fn intent_kind_to_str(k: IntentKind) -> &'static str {
    match k {
        IntentKind::ArchitecturalScope => "architectural_scope",
        IntentKind::Boundary => "boundary",
        IntentKind::Operational => "operational",
        IntentKind::TechStack => "tech_stack",
        IntentKind::ExplicitIntent => "explicit_intent",
        IntentKind::Unclassified => "unclassified",
    }
}

fn intent_kind_from_str(s: &str) -> Result<IntentKind, StoreError> {
    match s {
        "architectural_scope" => Ok(IntentKind::ArchitecturalScope),
        "boundary" => Ok(IntentKind::Boundary),
        "operational" => Ok(IntentKind::Operational),
        "tech_stack" => Ok(IntentKind::TechStack),
        "explicit_intent" => Ok(IntentKind::ExplicitIntent),
        _ => Ok(IntentKind::Unclassified),
    }
}

fn intent_source_kind_to_str(k: IntentSourceKind) -> &'static str {
    match k {
        IntentSourceKind::AgentsMd => "agents_md",
        IntentSourceKind::ClaudeMd => "claude_md",
        IntentSourceKind::G8Sidecar => "g8_sidecar",
        IntentSourceKind::InlineComment => "inline_comment",
        IntentSourceKind::Manual => "manual",
    }
}

fn intent_source_kind_from_str(s: &str) -> Result<IntentSourceKind, StoreError> {
    match s {
        "agents_md" => Ok(IntentSourceKind::AgentsMd),
        "claude_md" => Ok(IntentSourceKind::ClaudeMd),
        "g8_sidecar" => Ok(IntentSourceKind::G8Sidecar),
        "inline_comment" => Ok(IntentSourceKind::InlineComment),
        _ => Ok(IntentSourceKind::Manual),
    }
}

fn decision_status_to_str(s: DecisionStatus) -> &'static str {
    match s {
        DecisionStatus::Proposed => "proposed",
        DecisionStatus::Accepted => "accepted",
        DecisionStatus::Rejected => "rejected",
        DecisionStatus::Superseded => "superseded",
        DecisionStatus::RuledOut => "ruled_out",
    }
}

fn decision_status_from_str(s: &str) -> Result<DecisionStatus, StoreError> {
    match s {
        "proposed" => Ok(DecisionStatus::Proposed),
        "accepted" => Ok(DecisionStatus::Accepted),
        "rejected" => Ok(DecisionStatus::Rejected),
        "superseded" => Ok(DecisionStatus::Superseded),
        "ruled_out" => Ok(DecisionStatus::RuledOut),
        other => Err(StoreError::NotFound(format!(
            "unknown decision status: {other}"
        ))),
    }
}

fn overlap_kind_to_str(k: OverlapKind) -> &'static str {
    match k {
        OverlapKind::Touches => "touches",
        OverlapKind::Extends => "extends",
        OverlapKind::Replaces => "replaces",
        OverlapKind::Conflicts => "conflicts",
    }
}

fn plan_intent_relation_to_str(r: PlanIntentRelation) -> &'static str {
    match r {
        PlanIntentRelation::Satisfies => "satisfies",
        PlanIntentRelation::Contradicts => "contradicts",
        PlanIntentRelation::Extends => "extends",
        PlanIntentRelation::DerivedFrom => "derived_from",
    }
}

// ── Row-to-struct helpers ────────────────────────────────────────────────────

fn row_to_capability(row: &rusqlite::Row<'_>) -> rusqlite::Result<Capability> {
    let id_str: String = row.get(0)?;
    let project_id_str: String = row.get(1)?;
    let name: String = row.get(2)?;
    let description: Option<String> = row.get(3)?;
    let substrate: Option<String> = row.get(4)?;
    let consumes_json: String = row.get(5)?;
    let produces_json: String = row.get(6)?;
    let status_str: String = row.get(7)?;
    let stub: bool = row.get::<_, i64>(8).map(|v| v != 0)?;
    let stub_since_str: Option<String> = row.get(9)?;
    let owner_agent: Option<String> = row.get(10)?;
    let owner_team: Option<String> = row.get(11)?;
    let owner_contact: Option<String> = row.get(12)?;
    let file_path_str: Option<String> = row.get(13)?;
    let line_number: Option<u32> = row.get(14)?;
    let column_number: Option<u32> = row.get(15)?;
    let annotation_raw: Option<String> = row.get(16)?;
    let conditional: bool = row.get::<_, i64>(17).map(|v| v != 0)?;
    let created_at: i64 = row.get(18)?;
    let updated_at: i64 = row.get(19)?;
    let meta_json: Option<String> = row.get(20)?;

    let consumes: Vec<String> = serde_json::from_str(&consumes_json).unwrap_or_default();
    let produces: Vec<String> = serde_json::from_str(&produces_json).unwrap_or_default();

    let stub_since = stub_since_str
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let owner = if owner_agent.is_some() || owner_team.is_some() || owner_contact.is_some() {
        Some(Owner {
            agent: owner_agent,
            team: owner_team,
            contact: owner_contact,
        })
    } else {
        None
    };

    let source = g8_core::annotation::SourceLocation {
        file: file_path_str
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("")),
        line: line_number.unwrap_or(0),
        column: column_number.unwrap_or(0),
    };

    let meta = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Capability {
        id: CapabilityId::from_string(id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        project_id: ProjectId::from_string(project_id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        name,
        description,
        substrate,
        consumes,
        produces,
        status: capability_status_from_str(&status_str).unwrap_or(CapabilityStatus::InFlight),
        stub,
        stub_since,
        owner,
        source,
        annotation_raw: annotation_raw.unwrap_or_default(),
        conditional,
        created_at: EpochMillis::from_i64(created_at),
        updated_at: EpochMillis::from_i64(updated_at),
        meta,
    })
}

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let id_str: String = row.get(0)?;
    let space_id_str: String = row.get(1)?;
    let name: String = row.get(2)?;
    let description: Option<String> = row.get(3)?;
    let root_path_str: String = row.get(4)?;
    let language: Option<String> = row.get(5)?;
    let created_at: i64 = row.get(6)?;
    let updated_at: i64 = row.get(7)?;
    let meta_json: Option<String> = row.get(8)?;
    let meta = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Project {
        id: ProjectId::from_string(id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        space_id: SpaceId::from_string(space_id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        name,
        description,
        root_path: PathBuf::from(root_path_str),
        language,
        created_at: EpochMillis::from_i64(created_at),
        updated_at: EpochMillis::from_i64(updated_at),
        meta,
    })
}

fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<Plan> {
    let id_str: String = row.get(0)?;
    let space_id_str: String = row.get(1)?;
    let project_id_str: Option<String> = row.get(2)?;
    let title: String = row.get(3)?;
    let description: Option<String> = row.get(4)?;
    let status_str: String = row.get(5)?;
    let substrate: Option<String> = row.get(6)?;
    let wip_weight: u32 = row.get(7)?;
    let parked_reason: Option<String> = row.get(8)?;
    let blocked_reason: Option<String> = row.get(9)?;
    let dispatched_at: Option<i64> = row.get(10)?;
    let completed_at: Option<i64> = row.get(11)?;
    let created_at: i64 = row.get(12)?;
    let updated_at: i64 = row.get(13)?;
    let meta_json: Option<String> = row.get(14)?;
    let meta = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    // Load linked capability/intent/decision IDs (not fetched in bulk queries;
    // they are populated on individual get_plan calls or from context).
    Ok(Plan {
        id: PlanId::from_string(id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        space_id: SpaceId::from_string(space_id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        project_id: project_id_str
            .map(ProjectId::from_string)
            .transpose()
            .map_err(|e: G8Error| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        title,
        description,
        status: plan_status_from_str(&status_str).unwrap_or(PlanStatus::Idea),
        substrate,
        touched_capabilities: vec![],
        derived_from_intents: vec![],
        governed_by_decisions: vec![],
        wip_weight,
        parked_reason,
        blocked_reason,
        dispatched_at: dispatched_at.map(EpochMillis::from_i64),
        completed_at: completed_at.map(EpochMillis::from_i64),
        created_at: EpochMillis::from_i64(created_at),
        updated_at: EpochMillis::from_i64(updated_at),
        meta,
    })
}

fn row_to_intent_with_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Intent> {
    let id_str: String = row.get(0)?;
    let space_id_str: String = row.get(1)?;
    let project_id_str: Option<String> = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let heading: String = row.get(4)?;
    let description: String = row.get(5)?;
    let scope_path_str: String = row.get(6)?;
    let scope_depth: u32 = row.get(7)?;
    let substrate: Option<String> = row.get(8)?;
    let owner_agent: Option<String> = row.get(9)?;
    let owner_team: Option<String> = row.get(10)?;
    let owner_contact: Option<String> = row.get(11)?;
    let source_file_str: String = row.get(12)?;
    let source_line: Option<u32> = row.get(13)?;
    let source_kind_str: String = row.get(14)?;
    let created_at: i64 = row.get(15)?;
    let updated_at: i64 = row.get(16)?;
    let meta_json: Option<String> = row.get(17)?;
    let meta = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let owner = if owner_agent.is_some() || owner_team.is_some() || owner_contact.is_some() {
        Some(Owner {
            agent: owner_agent,
            team: owner_team,
            contact: owner_contact,
        })
    } else {
        None
    };

    let project_id = project_id_str.and_then(|s| ProjectId::from_string(s).ok());

    Ok(Intent {
        id: IntentId::from_string(id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        space_id: SpaceId::from_string(space_id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        project_id,
        kind: intent_kind_from_str(&kind_str).unwrap_or(IntentKind::Unclassified),
        heading,
        description,
        scope_path: PathBuf::from(scope_path_str),
        scope_depth,
        substrate,
        owner,
        source: g8_core::annotation::SourceLocation {
            file: PathBuf::from(&source_file_str),
            line: source_line.unwrap_or(0),
            column: 0,
        },
        source_kind: intent_source_kind_from_str(&source_kind_str)
            .unwrap_or(IntentSourceKind::Manual),
        created_at: EpochMillis::from_i64(created_at),
        updated_at: EpochMillis::from_i64(updated_at),
        meta,
    })
}

fn row_to_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<Decision> {
    let id_str: String = row.get(0)?;
    let space_id_str: String = row.get(1)?;
    let title: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    let context: Option<String> = row.get(4)?;
    let body: String = row.get(5)?;
    let source_file_str: Option<String> = row.get(6)?;
    let created_at: i64 = row.get(7)?;
    let updated_at: i64 = row.get(8)?;
    let meta_json: Option<String> = row.get(9)?;
    let meta = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Decision {
        id: DecisionId::from_string(id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        space_id: SpaceId::from_string(space_id_str)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?,
        title,
        status: decision_status_from_str(&status_str).unwrap_or(DecisionStatus::Proposed),
        context,
        body,
        source_file: source_file_str.map(PathBuf::from),
        created_at: EpochMillis::from_i64(created_at),
        updated_at: EpochMillis::from_i64(updated_at),
        meta,
    })
}

// ── StoreConnection impl ─────────────────────────────────────────────────────

impl StoreConnection for RusqliteStore {
    // ── Lifecycle ──────────────────────────────────────────────────────────

    #[instrument(skip(self, space), fields(space_id = space.id.as_str()))]
    fn init_space(&mut self, space: &ConvergenceSpace) -> Result<(), StoreError> {
        let meta = space.meta.as_ref().map(|v| v.to_string());
        self.lock().execute(
            "INSERT INTO convergence_space (id, name, root_path, created_at, updated_at, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name       = excluded.name,
               root_path  = excluded.root_path,
               updated_at = excluded.updated_at,
               meta       = excluded.meta",
            params![
                space.id.as_str(),
                space.name,
                space.root_path.to_string_lossy().as_ref(),
                space.created_at.as_i64(),
                space.updated_at.as_i64(),
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(space_id = id.as_str()))]
    fn get_space(&self, id: &SpaceId) -> Result<Option<ConvergenceSpace>, StoreError> {
        let conn = self.lock();
        let result = conn
            .query_row(
                "SELECT id, name, root_path, created_at, updated_at, meta
             FROM convergence_space WHERE id = ?1",
                params![id.as_str()],
                |row| {
                    let id_str: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let root_path_str: String = row.get(2)?;
                    let created_at: i64 = row.get(3)?;
                    let updated_at: i64 = row.get(4)?;
                    let meta_json: Option<String> = row.get(5)?;
                    Ok((
                        id_str,
                        name,
                        root_path_str,
                        created_at,
                        updated_at,
                        meta_json,
                    ))
                },
            )
            .optional()?;

        if let Some((id_str, name, root_path_str, created_at, updated_at, meta_json)) = result {
            // Load member project IDs
            let mut stmt =
                conn.prepare("SELECT id FROM project WHERE space_id = ?1 ORDER BY created_at")?;
            let members: Vec<ProjectId> = stmt
                .query_map(params![id_str.as_str()], |row| {
                    let s: String = row.get(0)?;
                    Ok(s)
                })?
                .filter_map(|r| r.ok())
                .filter_map(|s| ProjectId::from_string(s).ok())
                .collect();

            let meta = meta_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            Ok(Some(ConvergenceSpace {
                id: SpaceId::from_string(id_str)?,
                name,
                root_path: PathBuf::from(root_path_str),
                created_at: EpochMillis::from_i64(created_at),
                updated_at: EpochMillis::from_i64(updated_at),
                members,
                meta,
            }))
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self))]
    fn get_default_space(&self) -> Result<Option<ConvergenceSpace>, StoreError> {
        let id_opt: Option<String> = self
            .lock()
            .query_row(
                "SELECT id FROM convergence_space ORDER BY created_at LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        match id_opt {
            Some(id_str) => {
                let sid = SpaceId::from_string(id_str)?;
                self.get_space(&sid)
            }
            None => Ok(None),
        }
    }

    #[instrument(skip(self, project), fields(project_id = project.id.as_str()))]
    fn upsert_project(&mut self, project: &Project) -> Result<(), StoreError> {
        let meta = project.meta.as_ref().map(|v| v.to_string());
        self.lock().execute(
            "INSERT INTO project
               (id, space_id, name, description, root_path, language, created_at, updated_at, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET
               space_id    = excluded.space_id,
               name        = excluded.name,
               description = excluded.description,
               root_path   = excluded.root_path,
               language    = excluded.language,
               updated_at  = excluded.updated_at,
               meta        = excluded.meta",
            params![
                project.id.as_str(),
                project.space_id.as_str(),
                project.name,
                project.description,
                project.root_path.to_string_lossy().as_ref(),
                project.language,
                project.created_at.as_i64(),
                project.updated_at.as_i64(),
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(project_id = id.as_str()))]
    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, StoreError> {
        self.lock()
            .query_row(
                "SELECT id, space_id, name, description, root_path, language,
                        created_at, updated_at, meta
                 FROM project WHERE id = ?1",
                params![id.as_str()],
                row_to_project,
            )
            .optional()
            .map_err(Into::into)
    }

    #[instrument(skip(self), fields(space_id = space.as_str()))]
    fn list_projects(&self, space: &SpaceId) -> Result<Vec<Project>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, space_id, name, description, root_path, language,
                    created_at, updated_at, meta
             FROM project WHERE space_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![space.as_str()], row_to_project)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    #[instrument(skip(self), fields(space_id = space.as_str(), name))]
    fn find_project_by_name(
        &self,
        space: &SpaceId,
        name: &str,
    ) -> Result<Option<Project>, StoreError> {
        let conn = self.lock();
        let project_opt = conn
            .query_row(
                "SELECT id, space_id, name, description, root_path, language,
                        created_at, updated_at, meta
                 FROM project WHERE space_id = ?1 AND name = ?2",
                params![space.as_str(), name],
                row_to_project,
            )
            .optional()?;
        Ok(project_opt)
    }

    #[instrument(skip(self), fields(project_id = id.as_str()))]
    fn delete_project(&mut self, id: &ProjectId) -> Result<(), StoreError> {
        let n = self
            .lock()
            .execute("DELETE FROM project WHERE id = ?1", params![id.as_str()])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("project {}", id.as_str())));
        }
        Ok(())
    }

    // ── Bulk scan apply ────────────────────────────────────────────────────

    #[instrument(skip(self, scan), fields(project_id = project.as_str()))]
    fn apply_scan(
        &mut self,
        project: &ProjectId,
        scan: ScanResult,
    ) -> Result<ApplyScanReport, StoreError> {
        let mut conn = self.lock();
        let now = Self::now();
        let mut report = ApplyScanReport {
            inserted_capabilities: 0,
            inserted_intents: 0,
            inserted_decisions: 0,
            deleted_capabilities: 0,
            deleted_intents: 0,
            warnings: scan.warnings,
        };

        // One transaction for the whole scan (R4 §6 Risk 2 mitigation)
        let tx = conn.transaction()?;

        // Delete prior derived rows and raw annotations for this project.
        // Pairing checks read convergence tests from `annotation`, so a scan
        // must replace those rows in the same transaction as capabilities.
        let del_caps = tx.execute(
            "DELETE FROM capability WHERE project_id = ?1",
            params![project.as_str()],
        )?;
        report.deleted_capabilities = del_caps as u32;

        let del_intents = tx.execute(
            "DELETE FROM intent WHERE project_id = ?1",
            params![project.as_str()],
        )?;
        report.deleted_intents = del_intents as u32;

        tx.execute(
            "DELETE FROM annotation WHERE project_id = ?1",
            params![project.as_str()],
        )?;

        // We need to know the space_id for intent inserts; grab it from the project row.
        let space_id_str: Option<String> = tx
            .query_row(
                "SELECT space_id FROM project WHERE id = ?1",
                params![project.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let space_id_str =
            space_id_str.ok_or_else(|| StoreError::NotFound(format!("project {project}")))?;

        for ann in &scan.annotations {
            use g8_core::annotation::AnnotationKind;

            let kind = match ann.kind {
                AnnotationKind::Capability => "capability",
                AnnotationKind::Intent => "intent",
                AnnotationKind::ConvergenceTest => "convergence_test",
                AnnotationKind::Decision => "decision",
                AnnotationKind::Plan => "plan",
            };
            let annotation_id = AnnotationId::derive(&[
                project.as_str(),
                kind,
                ann.source.file.to_string_lossy().as_ref(),
                &ann.source.line.to_string(),
                &ann.source.column.to_string(),
            ]);
            let payload = serde_json::to_string(&ann.fields)?;
            let source_lang = ann
                .source
                .file
                .extension()
                .and_then(|extension| extension.to_str());
            tx.execute(
                "INSERT INTO annotation
                   (id, project_id, kind, payload, file_path, line_number,
                    column_number, source_lang, source_kind, conditional,
                    extracted_at, meta)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL)",
                params![
                    annotation_id.as_str(),
                    project.as_str(),
                    kind,
                    payload,
                    ann.source.file.to_string_lossy().as_ref(),
                    ann.source.line,
                    ann.source.column,
                    source_lang,
                    intent_source_kind_to_str(ann.source_kind),
                    ann.conditional as i64,
                    now,
                ],
            )?;

            match ann.kind {
                AnnotationKind::Capability => {
                    let name = ann.name.clone().unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    // Content-derived ID (Q4 ruling): stable across rescans of
                    // an unchanged tree; location keeps same-name duplicates
                    // distinct so conflict detection still sees both.
                    let id = AnnotationId::derive(&[
                        project.as_str(),
                        "capability",
                        &name,
                        ann.source.file.to_string_lossy().as_ref(),
                        &ann.source.line.to_string(),
                    ])
                    .as_str()
                    .to_owned();
                    let status = ann
                        .fields
                        .get("status")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.as_str())
                            } else {
                                None
                            }
                        })
                        .unwrap_or("in_flight");
                    let stub = ann
                        .fields
                        .get("stub")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::Bool(b) = v {
                                Some(*b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(false);
                    let stub_since = ann.fields.get("since").and_then(|v| {
                        if let g8_core::annotation::AnnotationValue::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });
                    let substrate = ann.fields.get("substrate").and_then(|v| {
                        if let g8_core::annotation::AnnotationValue::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });
                    let description = ann.fields.get("description").and_then(|v| {
                        if let g8_core::annotation::AnnotationValue::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });
                    let consumes = ann
                        .fields
                        .get("consumes")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::List(l) = v {
                                Some(serde_json::to_string(l).unwrap_or_else(|_| "[]".to_owned()))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "[]".to_owned());
                    let produces = ann
                        .fields
                        .get("produces")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::List(l) = v {
                                Some(serde_json::to_string(l).unwrap_or_else(|_| "[]".to_owned()))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "[]".to_owned());

                    tx.execute(
                        "INSERT INTO capability
                           (id, project_id, name, description, substrate, consumes, produces,
                            status, stub, stub_since, file_path, line_number, column_number,
                            annotation_raw, conditional, created_at, updated_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                        params![
                            id,
                            project.as_str(),
                            name,
                            description,
                            substrate,
                            consumes,
                            produces,
                            status,
                            stub as i64,
                            stub_since,
                            ann.source.file.to_string_lossy().as_ref(),
                            ann.source.line,
                            ann.source.column,
                            ann.raw_text,
                            ann.conditional as i64,
                            now,
                            now,
                        ],
                    )?;
                    report.inserted_capabilities += 1;
                }
                AnnotationKind::Intent => {
                    let heading = ann
                        .heading
                        .clone()
                        .or_else(|| ann.name.clone())
                        .unwrap_or_default();
                    if heading.is_empty() {
                        continue;
                    }
                    let id = AnnotationId::derive(&[
                        project.as_str(),
                        "intent",
                        &heading,
                        ann.source.file.to_string_lossy().as_ref(),
                        &ann.source.line.to_string(),
                    ])
                    .as_str()
                    .to_owned();
                    let scope_path = ann
                        .source
                        .file
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .to_string();
                    let scope_depth = ann.source.file.components().count() as u32;
                    let source_kind = intent_source_kind_to_str(ann.source_kind);
                    let substrate = ann.fields.get("substrate").and_then(|v| {
                        if let g8_core::annotation::AnnotationValue::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });
                    let description = ann
                        .fields
                        .get("description")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    // Forward the extractor's classify_heading kind through to the
                    // store so downstream consumers (e.g. g8-conflict's Boundary
                    // filter for GovernanceViolation detection) actually see it.
                    // Previously this column was hard-coded to "unclassified",
                    // which silently broke every `IntentKind != Unclassified` rule.
                    let intent_kind = ann
                        .fields
                        .get("intent_kind")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unclassified".to_string());

                    tx.execute(
                        "INSERT INTO intent
                           (id, space_id, project_id, kind, heading, description,
                            scope_path, scope_depth, substrate, source_file, source_line,
                            source_kind, created_at, updated_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                        params![
                            id,
                            space_id_str,
                            project.as_str(),
                            intent_kind,
                            heading,
                            description,
                            scope_path,
                            scope_depth,
                            substrate,
                            ann.source.file.to_string_lossy().as_ref(),
                            ann.source.line,
                            source_kind,
                            now,
                            now,
                        ],
                    )?;
                    report.inserted_intents += 1;
                }
                AnnotationKind::Decision => {
                    let title = ann
                        .fields
                        .get("title")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .or_else(|| ann.name.clone())
                        .unwrap_or_default();
                    if title.is_empty() {
                        continue;
                    }
                    let id = AnnotationId::derive(&[
                        &space_id_str,
                        "decision",
                        &title,
                        ann.source.file.to_string_lossy().as_ref(),
                        &ann.source.line.to_string(),
                    ])
                    .as_str()
                    .to_owned();
                    let body = ann
                        .fields
                        .get("body")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    let status = ann
                        .fields
                        .get("status")
                        .and_then(|v| {
                            if let g8_core::annotation::AnnotationValue::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "proposed".to_owned());

                    tx.execute(
                        "INSERT INTO decision
                           (id, space_id, title, status, body, source_file, created_at, updated_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                         ON CONFLICT DO NOTHING",
                        params![
                            id,
                            space_id_str,
                            title,
                            status,
                            body,
                            ann.source.file.to_string_lossy().as_ref(),
                            now,
                            now,
                        ],
                    )?;
                    report.inserted_decisions += 1;
                }
                // These remain raw annotations in v0.1. ConvergenceTest is
                // consumed by pairing_check; Plan materialization remains a
                // separate lifecycle operation.
                AnnotationKind::ConvergenceTest | AnnotationKind::Plan => {}
            }
        }

        tx.commit()?;
        Ok(report)
    }

    // ── Capability CRUD ────────────────────────────────────────────────────

    #[instrument(skip(self, cap), fields(capability_id = cap.id.as_str()))]
    fn upsert_capability(&mut self, cap: &Capability) -> Result<(), StoreError> {
        let now = Self::now();
        let meta = cap.meta.as_ref().map(|v| v.to_string());
        let consumes = serde_json::to_string(&cap.consumes)?;
        let produces = serde_json::to_string(&cap.produces)?;
        let stub_since = cap.stub_since.map(|d| d.format("%Y-%m-%d").to_string());
        self.lock().execute(
            "INSERT INTO capability
               (id, project_id, name, description, substrate, consumes, produces, status,
                stub, stub_since, owner_agent, owner_team, owner_contact, file_path,
                line_number, column_number, annotation_raw, conditional, created_at, updated_at, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
             ON CONFLICT(id) DO UPDATE SET
               name          = excluded.name,
               description   = excluded.description,
               substrate     = excluded.substrate,
               consumes      = excluded.consumes,
               produces      = excluded.produces,
               status        = excluded.status,
               stub          = excluded.stub,
               stub_since    = excluded.stub_since,
               owner_agent   = excluded.owner_agent,
               owner_team    = excluded.owner_team,
               owner_contact = excluded.owner_contact,
               file_path     = excluded.file_path,
               line_number   = excluded.line_number,
               column_number = excluded.column_number,
               annotation_raw= excluded.annotation_raw,
               conditional   = excluded.conditional,
               updated_at    = excluded.updated_at,
               meta          = excluded.meta",
            params![
                cap.id.as_str(),
                cap.project_id.as_str(),
                cap.name,
                cap.description,
                cap.substrate,
                consumes,
                produces,
                capability_status_to_str(cap.status),
                cap.stub as i64,
                stub_since,
                cap.owner.as_ref().and_then(|o| o.agent.as_deref()),
                cap.owner.as_ref().and_then(|o| o.team.as_deref()),
                cap.owner.as_ref().and_then(|o| o.contact.as_deref()),
                cap.source.file.to_string_lossy().as_ref(),
                cap.source.line,
                cap.source.column,
                cap.annotation_raw,
                cap.conditional as i64,
                cap.created_at.as_i64(),
                now,
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(capability_id = id.as_str()))]
    fn get_capability(&self, id: &CapabilityId) -> Result<Option<Capability>, StoreError> {
        self.lock()
            .query_row(
                "SELECT id, project_id, name, description, substrate, consumes, produces,
                        status, stub, stub_since, owner_agent, owner_team, owner_contact,
                        file_path, line_number, column_number, annotation_raw, conditional,
                        created_at, updated_at, meta
                 FROM capability WHERE id = ?1",
                params![id.as_str()],
                row_to_capability,
            )
            .optional()
            .map_err(Into::into)
    }

    #[instrument(skip(self), fields(project_id = project.as_str(), name))]
    fn find_capability_by_name(
        &self,
        project: &ProjectId,
        name: &str,
    ) -> Result<Option<Capability>, StoreError> {
        self.lock()
            .query_row(
                "SELECT id, project_id, name, description, substrate, consumes, produces,
                        status, stub, stub_since, owner_agent, owner_team, owner_contact,
                        file_path, line_number, column_number, annotation_raw, conditional,
                        created_at, updated_at, meta
                 FROM capability WHERE project_id = ?1 AND name = ?2 LIMIT 1",
                params![project.as_str(), name],
                row_to_capability,
            )
            .optional()
            .map_err(Into::into)
    }

    #[instrument(skip(self), fields(project_id = project.as_str()))]
    fn list_capabilities(&self, project: &ProjectId) -> Result<Vec<Capability>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, description, substrate, consumes, produces,
                    status, stub, stub_since, owner_agent, owner_team, owner_contact,
                    file_path, line_number, column_number, annotation_raw, conditional,
                    created_at, updated_at, meta
             FROM capability WHERE project_id = ?1 ORDER BY name",
        )?;
        let rows = stmt
            .query_map(params![project.as_str()], row_to_capability)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    #[instrument(skip(self), fields(project_id = project.as_str()))]
    fn delete_capabilities_for_project(&mut self, project: &ProjectId) -> Result<u64, StoreError> {
        let n = self.lock().execute(
            "DELETE FROM capability WHERE project_id = ?1",
            params![project.as_str()],
        )?;
        Ok(n as u64)
    }

    // ── Intent CRUD ────────────────────────────────────────────────────────

    #[instrument(skip(self, intent), fields(intent_id = intent.id.as_str()))]
    fn upsert_intent(&mut self, intent: &Intent) -> Result<(), StoreError> {
        let now = Self::now();
        let meta = intent.meta.as_ref().map(|v| v.to_string());
        self.lock().execute(
            "INSERT INTO intent
               (id, space_id, project_id, kind, heading, description, scope_path, scope_depth,
                substrate, owner_agent, owner_team, owner_contact, source_file, source_line,
                source_kind, created_at, updated_at, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(id) DO UPDATE SET
               kind          = excluded.kind,
               heading       = excluded.heading,
               description   = excluded.description,
               scope_path    = excluded.scope_path,
               scope_depth   = excluded.scope_depth,
               substrate     = excluded.substrate,
               owner_agent   = excluded.owner_agent,
               owner_team    = excluded.owner_team,
               owner_contact = excluded.owner_contact,
               source_file   = excluded.source_file,
               source_line   = excluded.source_line,
               source_kind   = excluded.source_kind,
               updated_at    = excluded.updated_at,
               meta          = excluded.meta",
            params![
                intent.id.as_str(),
                intent.space_id.as_str(),
                intent.project_id.as_ref().map(|p| p.as_str()),
                intent_kind_to_str(intent.kind),
                intent.heading,
                intent.description,
                intent.scope_path.to_string_lossy().as_ref(),
                intent.scope_depth,
                intent.substrate,
                intent.owner.as_ref().and_then(|o| o.agent.as_deref()),
                intent.owner.as_ref().and_then(|o| o.team.as_deref()),
                intent.owner.as_ref().and_then(|o| o.contact.as_deref()),
                intent.source.file.to_string_lossy().as_ref(),
                intent.source.line,
                intent_source_kind_to_str(intent.source_kind),
                intent.created_at.as_i64(),
                now,
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(space_id = space.as_str()))]
    fn list_intents_at_path(
        &self,
        space: &SpaceId,
        path: &Path,
    ) -> Result<Vec<Intent>, StoreError> {
        let conn = self.lock();
        let path_str = path.to_string_lossy();
        // Empty path means "all intents in the space"; non-empty filters to intents
        // whose scope_path is a prefix of `path` (most-specific first).
        if path_str.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, project_id, kind, heading, description, scope_path,
                        scope_depth, substrate, owner_agent, owner_team, owner_contact,
                        source_file, source_line, source_kind, created_at, updated_at, meta
                 FROM intent
                 WHERE space_id = ?1
                 ORDER BY scope_depth DESC",
            )?;
            let rows = stmt
                .query_map(params![space.as_str()], row_to_intent_with_project)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, project_id, kind, heading, description, scope_path,
                        scope_depth, substrate, owner_agent, owner_team, owner_contact,
                        source_file, source_line, source_kind, created_at, updated_at, meta
                 FROM intent
                 WHERE space_id = ?1 AND ?2 LIKE scope_path || '%'
                 ORDER BY scope_depth DESC",
            )?;
            let rows = stmt
                .query_map(
                    params![space.as_str(), path_str.as_ref()],
                    row_to_intent_with_project,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }

    #[instrument(skip(self), fields(project_id = project.as_str()))]
    fn delete_intents_for_project(&mut self, project: &ProjectId) -> Result<u64, StoreError> {
        let n = self.lock().execute(
            "DELETE FROM intent WHERE project_id = ?1",
            params![project.as_str()],
        )?;
        Ok(n as u64)
    }

    // ── Plan CRUD ──────────────────────────────────────────────────────────

    #[instrument(skip(self, plan), fields(plan_id = plan.id.as_str()))]
    fn create_plan(&mut self, plan: &Plan) -> Result<(), StoreError> {
        let meta = plan.meta.as_ref().map(|v| v.to_string());
        self.lock().execute(
            "INSERT INTO plan
               (id, space_id, project_id, title, description, status, substrate,
                wip_weight, parked_reason, blocked_reason, dispatched_at, completed_at,
                created_at, updated_at, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                plan.id.as_str(),
                plan.space_id.as_str(),
                plan.project_id.as_ref().map(|p| p.as_str()),
                plan.title,
                plan.description,
                plan_status_to_str(plan.status),
                plan.substrate,
                plan.wip_weight,
                plan.parked_reason,
                plan.blocked_reason,
                plan.dispatched_at.as_ref().map(|e| e.as_i64()),
                plan.completed_at.as_ref().map(|e| e.as_i64()),
                plan.created_at.as_i64(),
                plan.updated_at.as_i64(),
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(plan_id = id.as_str()))]
    fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError> {
        let conn = self.lock();
        let plan_opt = conn
            .query_row(
                "SELECT id, space_id, project_id, title, description, status, substrate,
                        wip_weight, parked_reason, blocked_reason, dispatched_at, completed_at,
                        created_at, updated_at, meta
                 FROM plan WHERE id = ?1",
                params![id.as_str()],
                row_to_plan,
            )
            .optional()?;

        if let Some(mut plan) = plan_opt {
            // Load linked capability IDs
            let mut stmt =
                conn.prepare("SELECT capability_id FROM plan_capability WHERE plan_id = ?1")?;
            plan.touched_capabilities = stmt
                .query_map(params![id.as_str()], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|s| CapabilityId::from_string(s).ok())
                .collect();

            // Load linked intent IDs
            let mut stmt = conn.prepare("SELECT intent_id FROM plan_intent WHERE plan_id = ?1")?;
            plan.derived_from_intents = stmt
                .query_map(params![id.as_str()], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|s| IntentId::from_string(s).ok())
                .collect();

            // Load linked decision IDs
            let mut stmt =
                conn.prepare("SELECT decision_id FROM plan_decision WHERE plan_id = ?1")?;
            plan.governed_by_decisions = stmt
                .query_map(params![id.as_str()], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|s| DecisionId::from_string(s).ok())
                .collect();

            Ok(Some(plan))
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self, filter), fields(space_id = space.as_str()))]
    fn list_plans(&self, space: &SpaceId, filter: PlanFilter) -> Result<Vec<Plan>, StoreError> {
        let conn = self.lock();
        // Build the query dynamically based on filter fields.
        let mut sql = String::from(
            "SELECT id, space_id, project_id, title, description, status, substrate,
                    wip_weight, parked_reason, blocked_reason, dispatched_at, completed_at,
                    created_at, updated_at, meta
             FROM plan WHERE space_id = ?1",
        );
        let mut bindings: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(space.as_str().to_owned())];

        if let Some(ref statuses) = filter.status {
            if !statuses.is_empty() {
                let placeholders: Vec<String> = statuses
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect();
                sql.push_str(&format!(" AND status IN ({})", placeholders.join(",")));
                for s in statuses {
                    bindings.push(Box::new(plan_status_to_str(*s).to_owned()));
                }
            }
        }
        if let Some(ref sub) = filter.substrate {
            let idx = bindings.len() + 1;
            sql.push_str(&format!(" AND substrate = ?{idx}"));
            bindings.push(Box::new(sub.clone()));
        }
        if let Some(ref proj) = filter.project {
            let idx = bindings.len() + 1;
            sql.push_str(&format!(" AND project_id = ?{idx}"));
            bindings.push(Box::new(proj.as_str().to_owned()));
        }
        sql.push_str(" ORDER BY updated_at DESC");
        if let Some(lim) = filter.limit {
            let idx = bindings.len() + 1;
            sql.push_str(&format!(" LIMIT ?{idx}"));
            bindings.push(Box::new(lim));
        }

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = bindings.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), row_to_plan)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    #[instrument(skip(self), fields(plan_id = id.as_str()))]
    fn update_plan_status(
        &mut self,
        id: &PlanId,
        new_status: PlanStatus,
        reason: Option<String>,
    ) -> Result<(), StoreError> {
        let conn = self.lock();
        // Enforce the lifecycle graph: load the current status and reject
        // illegal transitions before issuing the UPDATE. validate_status_transition
        // returns G8Error::InvalidStatusTransition, which the From impl maps to
        // StoreError::Core — fine for callers but loses the structured payload.
        // Wrap in StoreError::InvalidTransition so the CLI surfaces it clearly.
        let current_status: String = conn
            .query_row(
                "SELECT status FROM plan WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("plan {}", id.as_str())))?;
        let from = plan_status_from_str(&current_status).map_err(|e| {
            StoreError::InvalidTransition(format!("unknown current status '{current_status}': {e}"))
        })?;
        if from != new_status {
            g8_core::validate_status_transition(from, new_status).map_err(|e| {
                StoreError::InvalidTransition(format!(
                    "{from:?} → {new_status:?} is not a legal transition: {e}"
                ))
            })?;
        }
        let now = Self::now();
        let status_str = plan_status_to_str(new_status);
        match new_status {
            PlanStatus::Parked => {
                conn.execute(
                    "UPDATE plan SET status = ?1, parked_reason = ?2, updated_at = ?3 WHERE id = ?4",
                    params![status_str, reason, now, id.as_str()],
                )?;
            }
            PlanStatus::Blocked => {
                conn.execute(
                    "UPDATE plan SET status = ?1, blocked_reason = ?2, updated_at = ?3 WHERE id = ?4",
                    params![status_str, reason, now, id.as_str()],
                )?;
            }
            PlanStatus::Dispatched => {
                conn.execute(
                    "UPDATE plan SET status = ?1, dispatched_at = ?2, updated_at = ?2 WHERE id = ?3",
                    params![status_str, now, id.as_str()],
                )?;
            }
            PlanStatus::Done => {
                conn.execute(
                    "UPDATE plan SET status = ?1, completed_at = ?2, updated_at = ?2 WHERE id = ?3",
                    params![status_str, now, id.as_str()],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE plan SET status = ?1, updated_at = ?2 WHERE id = ?3",
                    params![status_str, now, id.as_str()],
                )?;
            }
        }
        Ok(())
    }

    #[instrument(skip(self), fields(plan_id = plan.as_str(), capability_id = capability.as_str()))]
    fn link_plan_capability(
        &mut self,
        plan: &PlanId,
        capability: &CapabilityId,
        overlap_kind: OverlapKind,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO plan_capability (plan_id, capability_id, overlap_kind)
             VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            params![
                plan.as_str(),
                capability.as_str(),
                overlap_kind_to_str(overlap_kind),
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(plan_id = plan.as_str(), intent_id = intent.as_str()))]
    fn link_plan_intent(
        &mut self,
        plan: &PlanId,
        intent: &IntentId,
        relation: PlanIntentRelation,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO plan_intent (plan_id, intent_id, relation)
             VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            params![
                plan.as_str(),
                intent.as_str(),
                plan_intent_relation_to_str(relation),
            ],
        )?;
        Ok(())
    }

    // ── Decision CRUD ──────────────────────────────────────────────────────

    #[instrument(skip(self, decision), fields(decision_id = decision.id.as_str()))]
    fn upsert_decision(&mut self, decision: &Decision) -> Result<(), StoreError> {
        let now = Self::now();
        let meta = decision.meta.as_ref().map(|v| v.to_string());
        self.lock().execute(
            "INSERT INTO decision
               (id, space_id, title, status, context, body, source_file, created_at, updated_at, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(id) DO UPDATE SET
               title      = excluded.title,
               status     = excluded.status,
               context    = excluded.context,
               body       = excluded.body,
               source_file= excluded.source_file,
               updated_at = excluded.updated_at,
               meta       = excluded.meta",
            params![
                decision.id.as_str(),
                decision.space_id.as_str(),
                decision.title,
                decision_status_to_str(decision.status),
                decision.context,
                decision.body,
                decision.source_file.as_ref().map(|p| p.to_string_lossy().into_owned()),
                decision.created_at.as_i64(),
                now,
                meta,
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(space_id = space.as_str()))]
    fn list_decisions(&self, space: &SpaceId) -> Result<Vec<Decision>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, space_id, title, status, context, body, source_file,
                    created_at, updated_at, meta
             FROM decision WHERE space_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![space.as_str()], row_to_decision)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Substrate budget CRUD ──────────────────────────────────────────────

    #[instrument(skip(self), fields(space_id = space.as_str(), substrate))]
    fn set_substrate_budget(
        &mut self,
        space: &SpaceId,
        substrate: &str,
        wip_cap: u32,
        stale_threshold_days: u32,
    ) -> Result<(), StoreError> {
        let now = Self::now();
        self.lock().execute(
            "INSERT INTO substrate_budget
               (space_id, substrate, wip_cap, stale_threshold_days, last_updated)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(space_id, substrate) DO UPDATE SET
               wip_cap              = excluded.wip_cap,
               stale_threshold_days = excluded.stale_threshold_days,
               last_updated         = excluded.last_updated",
            params![
                space.as_str(),
                substrate,
                wip_cap,
                stale_threshold_days,
                now
            ],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(space_id = space.as_str(), substrate))]
    fn get_substrate_budget(
        &self,
        space: &SpaceId,
        substrate: &str,
    ) -> Result<SubstrateBudget, StoreError> {
        let result = self
            .lock()
            .query_row(
                "SELECT substrate, wip_cap, stale_threshold_days
             FROM substrate_budget WHERE space_id = ?1 AND substrate = ?2",
                params![space.as_str(), substrate],
                |row| {
                    Ok(SubstrateBudget {
                        substrate: row.get(0)?,
                        wip_cap: row.get(1)?,
                        stale_threshold_days: row.get(2)?,
                    })
                },
            )
            .optional()?;

        Ok(result.unwrap_or_else(|| SubstrateBudget::default_for(substrate)))
    }

    #[instrument(skip(self), fields(space_id = space.as_str()))]
    fn list_substrate_budgets(&self, space: &SpaceId) -> Result<Vec<SubstrateBudget>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT substrate, wip_cap, stale_threshold_days
             FROM substrate_budget WHERE space_id = ?1 ORDER BY substrate",
        )?;
        let rows = stmt
            .query_map(params![space.as_str()], |row| {
                Ok(SubstrateBudget {
                    substrate: row.get(0)?,
                    wip_cap: row.get(1)?,
                    stale_threshold_days: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Capability aliases ─────────────────────────────────────────────────

    #[instrument(skip(self), fields(canonical, alias))]
    fn add_capability_alias(&mut self, canonical: &str, alias: &str) -> Result<(), StoreError> {
        let now = Self::now();
        self.lock().execute(
            "INSERT INTO capability_alias (canonical, alias, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            params![canonical, alias, now],
        )?;
        Ok(())
    }

    #[instrument(skip(self), fields(ref_name))]
    fn resolve_canonical(&self, ref_name: &str) -> Result<String, StoreError> {
        // Look up the canonical for this alias; if not found, return as-is.
        let result: Option<String> = self
            .lock()
            .query_row(
                "SELECT canonical FROM capability_alias WHERE alias = ?1 LIMIT 1",
                params![ref_name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result.unwrap_or_else(|| ref_name.to_owned()))
    }

    // ── Composite planner_intent_check query ──────────────────────────────

    /// Execute the ARCH §7 six-CTE composite query in one read transaction.
    ///
    /// Returns [`RawIntentCheckPayload`] with six `serde_json::Value` fields.
    /// All six are always populated (empty arrays / null on empty store).
    #[instrument(skip(self, draft),
        fields(
            space_id = draft.space_id.as_str(),
            substrate = draft.substrate.as_deref().unwrap_or(""),
            cap_names_count = draft.touched_capabilities.len()
        ))]
    fn planner_intent_check(&self, draft: &PlanDraft) -> Result<RawIntentCheckPayload, StoreError> {
        let space_id = draft.space_id.as_str();
        let substrate = draft.substrate.as_deref().unwrap_or("");
        let title = draft.title.as_str();
        let _cap_names_json = serde_json::to_string(&draft.touched_capabilities)?;

        // Resolve aliases in cap_names
        let resolved_caps: Vec<String> = draft
            .touched_capabilities
            .iter()
            .map(|n| self.resolve_canonical(n).unwrap_or_else(|_| n.clone()))
            .collect();
        let resolved_caps_json = serde_json::to_string(&resolved_caps)?;

        let sql = r#"
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
    WHERE p.space_id = ?1
      AND p.status NOT IN ('Done', 'Parked')
      AND (
            p.substrate = ?2
            OR p.title LIKE '%' || ?3 || '%'
            OR ?3 LIKE '%' || p.title || '%'
          )

    UNION

    SELECT
        p.id, p.title, p.status, p.substrate, p.wip_weight, p.updated_at,
        'capability_overlap' AS match_kind
    FROM plan p
    JOIN plan_capability pc ON pc.plan_id = p.id
    JOIN capability c        ON c.id = pc.capability_id
    WHERE p.space_id = ?1
      AND p.status NOT IN ('Done', 'Parked')
      AND EXISTS (
            SELECT 1 FROM json_each(?4) jn WHERE jn.value = c.name
          )
),

budget AS (
    -- Returns one row whenever ?2 (substrate) is non-empty, falling back to
    -- the defaults from g8_core (wip_cap=3, stale_threshold_days=14) when
    -- no substrate_budget row has been registered yet.
    SELECT
        ?2 AS substrate,
        cap.wip_cap,
        cap.stale_threshold_days,
        cnt.wip_current,
        (cap.wip_cap - cnt.wip_current) AS wip_remaining,
        CASE WHEN cnt.wip_current >= cap.wip_cap THEN 1 ELSE 0 END AS at_cap
    FROM
        (SELECT
            COALESCE(
                (SELECT wip_cap FROM substrate_budget
                  WHERE space_id = ?1 AND substrate = ?2),
                3
            ) AS wip_cap,
            COALESCE(
                (SELECT stale_threshold_days FROM substrate_budget
                  WHERE space_id = ?1 AND substrate = ?2),
                14
            ) AS stale_threshold_days
        ) AS cap,
        -- WIP counts both Dispatched AND Blocked plans: per SPEC §7, "Blocked
        -- doesn't free a slot — blocked work still occupies the substrate's
        -- attention surface." Counting only Dispatched would let a Blocked plan
        -- silently free a slot, contradicting that intent (and the `bottlenecks`
        -- CTE below, which already counts both).
        (SELECT COUNT(*) AS wip_current
           FROM plan
          WHERE space_id = ?1
            AND substrate = ?2
            AND status IN ('Dispatched', 'Blocked')
        ) AS cnt
    WHERE ?2 != ''
),

bottlenecks AS (
    SELECT
        p.id, p.title, p.status, p.blocked_reason,
        p.dispatched_at, p.updated_at,
        CASE WHEN p.dispatched_at IS NOT NULL
             THEN (unixepoch('now') * 1000 - p.dispatched_at) / 86400000
             ELSE NULL END AS days_in_flight
    FROM plan p
    WHERE p.space_id = ?1
      AND p.substrate = ?2
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
    WHERE p.space_id = ?1
      AND p.substrate = ?2
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
    WHERE i.space_id = ?1
      AND i.kind != 'operational'
      AND (
            i.substrate = ?2
            OR i.heading LIKE '%' || ?3 || '%'
          )
    ORDER BY i.scope_depth DESC, i.updated_at DESC
    LIMIT 20
),

parked_ideas AS (
    SELECT
        p.id, p.title, p.parked_reason, p.updated_at
    FROM plan p
    WHERE p.space_id = ?1
      AND p.status   = 'Parked'
      AND (
            p.substrate = ?2
            OR p.title LIKE '%' || ?3 || '%'
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
        -- json('true'/'false') ensures a real JSON boolean, not 0/1 ints
        -- (BudgetStatus.at_cap is bool — int trips serde deserialization).
        'at_cap',        json(CASE WHEN b.at_cap = 1 THEN 'true' ELSE 'false' END)
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
    )) FROM parked_ideas pk) AS parked_ideas
"#;

        let result = self.lock().query_row(
            sql,
            params![space_id, substrate, title, resolved_caps_json],
            |row| {
                let existing_matches_str: Option<String> = row.get(0)?;
                let budget_str: Option<String> = row.get(1)?;
                let bottlenecks_str: Option<String> = row.get(2)?;
                let drift_hints_str: Option<String> = row.get(3)?;
                let intent_overlaps_str: Option<String> = row.get(4)?;
                let parked_ideas_str: Option<String> = row.get(5)?;
                Ok((
                    existing_matches_str,
                    budget_str,
                    bottlenecks_str,
                    drift_hints_str,
                    intent_overlaps_str,
                    parked_ideas_str,
                ))
            },
        )?;

        let (
            existing_matches_str,
            budget_str,
            bottlenecks_str,
            drift_hints_str,
            intent_overlaps_str,
            parked_ideas_str,
        ) = result;

        let parse_array = |s: Option<String>| -> serde_json::Value {
            s.and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Array(vec![]))
        };
        let parse_obj = |s: Option<String>| -> serde_json::Value {
            s.and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Null)
        };

        Ok(RawIntentCheckPayload {
            existing_matches: parse_array(existing_matches_str),
            budget: parse_obj(budget_str),
            bottlenecks: parse_array(bottlenecks_str),
            drift_hints: parse_array(drift_hints_str),
            intent_overlaps: parse_array(intent_overlaps_str),
            parked_ideas: parse_array(parked_ideas_str),
        })
    }

    // ── Pairing check ─────────────────────────────────────────────────────

    #[instrument(skip(self), fields(project = project.as_ref().map(|p| p.as_str()).unwrap_or("all")))]
    fn pairing_check(&self, project: Option<&ProjectId>) -> Result<Vec<PairingError>, StoreError> {
        let conn = self.lock();
        let mut errors: Vec<PairingError> = Vec::new();
        let now_ms = Self::now();
        let seven_days_ms: i64 = 7 * 24 * 60 * 60 * 1000;

        // Build capability query
        let (cap_sql, cap_params): (String, Vec<String>) = if let Some(proj) = project {
            (
                "SELECT id, name, stub, stub_since, file_path, line_number, project_id
                 FROM capability WHERE project_id = ?1 AND conditional = 0"
                    .to_owned(),
                vec![proj.as_str().to_owned()],
            )
        } else {
            (
                "SELECT id, name, stub, stub_since, file_path, line_number, project_id
                 FROM capability WHERE conditional = 0"
                    .to_owned(),
                vec![],
            )
        };

        let mut stmt = conn.prepare(&cap_sql)?;
        let cap_refs: Vec<&dyn rusqlite::ToSql> = cap_params
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        struct CapRow {
            id: String,
            name: String,
            stub: bool,
            stub_since: Option<String>,
            file_path: String,
            line_number: u32,
            project_id: String,
        }

        let capabilities: Vec<CapRow> = stmt
            .query_map(cap_refs.as_slice(), |row| {
                Ok(CapRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    stub: row.get::<_, i64>(2).map(|v| v != 0)?,
                    stub_since: row.get(3)?,
                    file_path: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    line_number: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
                    project_id: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Check for duplicate names within each project
        {
            type NameCountMap =
                std::collections::HashMap<(String, String), Vec<(String, String, u32)>>;
            let mut name_counts: NameCountMap = std::collections::HashMap::new();
            for cap in &capabilities {
                name_counts
                    .entry((cap.project_id.clone(), cap.name.clone()))
                    .or_default()
                    .push((cap.id.clone(), cap.file_path.clone(), cap.line_number));
            }
            for ((_, name), entries) in &name_counts {
                if entries.len() > 1 {
                    for (_, file_path, line_number) in entries {
                        errors.push(PairingError {
                            capability: name.clone(),
                            file: PathBuf::from(file_path),
                            line: *line_number,
                            reason: PairingErrorReason::DuplicateCapabilityName,
                        });
                    }
                }
            }
        }

        // Check stub validity and convergence test pairing
        for cap in &capabilities {
            if cap.stub {
                match &cap.stub_since {
                    None => {
                        errors.push(PairingError {
                            capability: cap.name.clone(),
                            file: PathBuf::from(&cap.file_path),
                            line: cap.line_number,
                            reason: PairingErrorReason::StubMissingSince,
                        });
                        continue;
                    }
                    Some(since_str) => {
                        if let Ok(since_date) = NaiveDate::parse_from_str(since_str, "%Y-%m-%d") {
                            let since_ts = since_date
                                .and_hms_opt(0, 0, 0)
                                .map(|dt| dt.and_utc().timestamp_millis())
                                .unwrap_or(0);
                            if now_ms - since_ts > seven_days_ms {
                                errors.push(PairingError {
                                    capability: cap.name.clone(),
                                    file: PathBuf::from(&cap.file_path),
                                    line: cap.line_number,
                                    reason: PairingErrorReason::StubExpired,
                                });
                            }
                        }
                        // Stubs don't need convergence tests
                        continue;
                    }
                }
            }

            // Non-stub: check for a paired convergence test annotation
            let has_test: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM annotation
                     WHERE project_id = ?1
                       AND kind = 'convergence_test'
                       AND json_extract(payload, '$.for_capability') = ?2",
                    params![cap.project_id, cap.name],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if !has_test {
                errors.push(PairingError {
                    capability: cap.name.clone(),
                    file: PathBuf::from(&cap.file_path),
                    line: cap.line_number,
                    reason: PairingErrorReason::NoMatchingConvergenceTest,
                });
            }
        }

        Ok(errors)
    }

    // ── Stale dispatched plans ────────────────────────────────────────────

    #[instrument(skip(self), fields(space_id = space.as_str()))]
    fn stale_dispatched_plans(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<StalePlanCandidate>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.title, p.substrate, p.dispatched_at,
                    (unixepoch('now') * 1000 - p.dispatched_at) / 86400000 AS days_stale
             FROM plan p
             JOIN substrate_budget sb
                    ON sb.space_id = p.space_id
                   AND sb.substrate = p.substrate
             WHERE p.space_id = ?1
               AND p.status = 'Dispatched'
               AND p.dispatched_at IS NOT NULL
               AND ((unixepoch('now') * 1000 - p.dispatched_at) / 86400000) > sb.stale_threshold_days
             ORDER BY days_stale DESC",
        )?;
        let rows = stmt
            .query_map(params![space.as_str()], |row| {
                let id_str: String = row.get(0)?;
                let title: String = row.get(1)?;
                let substrate: Option<String> = row.get(2)?;
                let dispatched_at: i64 = row.get(3)?;
                let days_stale: i64 = row.get(4)?;
                Ok((id_str, title, substrate, dispatched_at, days_stale))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter()
            .map(|(id_str, title, substrate, dispatched_at, days_stale)| {
                Ok(StalePlanCandidate {
                    plan_id: PlanId::from_string(id_str)?,
                    title,
                    substrate,
                    dispatched_at: EpochMillis::from_i64(dispatched_at),
                    days_stale,
                })
            })
            .collect()
    }

    // ── Cross-store attach ────────────────────────────────────────────────

    #[instrument(skip(self), fields(alias, path = path.display().to_string().as_str()))]
    fn attach_remote_store(&mut self, alias: &str, path: &Path) -> Result<(), StoreError> {
        // Read-only ATTACH per ARCHITECTURE.md §15 safety requirement.
        //
        // Build a SQLite `file:` URI. The path may contain characters that are
        // significant in a URI (`?` would start the query string, `#` a
        // fragment, `%` an escape, space is reserved) — percent-encode them so a
        // path like `/repos/a?b/.g8/store.db` doesn't get truncated at the `?`.
        // The full URI is then *bound* as a parameter (rather than interpolated
        // into a quoted SQL literal) so a path containing `'` can't break out of
        // the statement. Alias is internally generated, but we still validate it
        // is a bare identifier since it can't be parameterised in `ATTACH`.
        if alias.is_empty() || !alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(StoreError::InvalidTransition(format!(
                "attach alias '{alias}' must be a non-empty [A-Za-z0-9_] identifier"
            )));
        }
        let uri = format!("file:{}?mode=ro", encode_uri_path(&path.to_string_lossy()));
        self.lock()
            .execute(&format!("ATTACH DATABASE ?1 AS \"{alias}\""), params![uri])?;
        Ok(())
    }

    #[instrument(skip(self), fields(alias))]
    fn detach_remote_store(&mut self, alias: &str) -> Result<(), StoreError> {
        self.lock()
            .execute_batch(&format!("DETACH DATABASE \"{alias}\""))?;
        Ok(())
    }

    // ── File-watch ────────────────────────────────────────────────────────

    fn watch<F>(&self, callback: F) -> Result<WatchHandle, StoreError>
    where
        F: Fn() + Send + 'static,
    {
        use notify::{Event, EventKind, RecursiveMode, Watcher};

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                        callback();
                    }
                    _ => {}
                }
            }
        })
        .map_err(|e| StoreError::Notify(e.to_string()))?;

        if self.store_path.as_os_str() != ":memory:" {
            watcher
                .watch(&self.store_path, RecursiveMode::NonRecursive)
                .map_err(|e| StoreError::Notify(e.to_string()))?;
        }

        Ok(WatchHandle::new(watcher))
    }
}

// Note: EpochMillis::as_i64() is provided by g8-core directly.
