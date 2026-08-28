//! [`ConflictStore`] — the minimal read-only store interface needed by conflict detection.
//!
//! This trait captures exactly the four query patterns the [`ConflictDetector`] needs. It is
//! intentionally narrower than `g8_store::StoreConnection` so that:
//!
//! 1. Conflict detection stays a pure computation over lightweight rows, decoupled from the
//!    full store API surface.
//! 2. The adapter is thin: [`crate::store_adapter::ConflictAdapter`] delegates each method to
//!    the corresponding `StoreConnection` method, generically over any implementation (the
//!    SPEC Decision 8 edge, realized without naming a concrete store type — OBL-D5-01).
//! 3. Tests use the in-memory [`MemoryStore`] with zero disk I/O.
//!
//! # Row types
//!
//! The trait returns lightweight row structs instead of the full `g8-core` model types. This
//! avoids requiring callers to populate all fields just for conflict detection, while still
//! carrying typed IDs and enums (no stringly-typed evidence).

use std::path::PathBuf;

use g8_core::{CapabilityId, DecisionId, IntentId, IntentKind, PlanId, SpaceId};

// ── Row types ────────────────────────────────────────────────────────────────

/// Minimal capability row for conflict detection.
///
/// The `project_name` field is used to build the `<project>::<name>` cross-project lookup key
/// (per A1 decision #2 — used only in queries, never in annotations).
#[derive(Debug, Clone)]
pub struct CapabilityRow {
    pub id: CapabilityId,
    /// Kebab-case name, project-scoped.
    pub name: String,
    /// Human-readable project label (used for `<project>::<name>` lookup keys).
    pub project_name: String,
}

/// Minimal intent row for conflict detection.
#[derive(Debug, Clone)]
pub struct IntentRow {
    pub id: IntentId,
    pub kind: IntentKind,
    pub heading: String,
    pub description: String,
    pub scope_path: PathBuf,
}

/// Minimal decision row for conflict detection.
#[derive(Debug, Clone)]
pub struct DecisionRow {
    pub id: DecisionId,
    pub title: String,
    /// `true` when `status = 'accepted'` (per ARCH §3.5 ContradictoryDecisions rule).
    pub status_accepted: bool,
    /// Body text (string-equality comparison for contradiction detection).
    pub body: String,
}

/// Minimal plan row for conflict detection.
#[derive(Debug, Clone)]
pub struct PlanRow {
    pub id: PlanId,
    pub title: String,
    /// Substrate name, if set (used for GovernanceViolation detection).
    pub substrate: Option<String>,
}

// ── ConflictStore trait ───────────────────────────────────────────────────────

/// Minimal read-only store surface required by [`ConflictDetector`].
///
/// Implementors return lightweight row types so conflict detection is a pure computation
/// over the data — no full entity hydration required.
///
/// # Thread safety
///
/// The trait requires `Send + Sync` so detectors can be called from multi-threaded contexts.
/// Read-only implementations (like `MemoryStore`) can always satisfy this.
pub trait ConflictStore: Send + Sync {
    /// List all capabilities in the given space (across all projects in the space).
    fn list_capabilities(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<CapabilityRow>, Box<dyn std::error::Error + Send + Sync>>;

    /// List all intents in the given space.
    fn list_intents(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<IntentRow>, Box<dyn std::error::Error + Send + Sync>>;

    /// List all decisions in the given space.
    fn list_decisions(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<DecisionRow>, Box<dyn std::error::Error + Send + Sync>>;

    /// List all plans in the given space.
    fn list_plans(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<PlanRow>, Box<dyn std::error::Error + Send + Sync>>;

    /// Resolve a `<project>::<name>` reference to its canonical alias (if declared).
    ///
    /// Returns `Ok(canonical)` when an alias is found, or `Err(...)` when no alias is
    /// registered. The detector interprets `Err` as "no alias" (not as a hard failure).
    fn resolve_canonical(
        &self,
        ref_name: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

// ── MemoryStore (in-memory implementation for tests) ─────────────────────────

/// In-memory implementation of [`ConflictStore`] for unit tests.
///
/// All rows are keyed by space ID. Use the builder methods to seed test data.
///
/// # Examples
///
/// ```
/// use g8_conflict::store_trait::{MemoryStore, CapabilityRow};
/// use g8_conflict::ConflictStore; // needed to call list_capabilities
/// use g8_core::{CapabilityId, SpaceId};
///
/// let space = SpaceId::derive(&["docs", "space"]);
/// let mut store = MemoryStore::default();
/// store.add_capability(
///     &space,
///     CapabilityRow {
///         id: CapabilityId::derive(&["docs", "http-fetch"]),
///         name: "http-fetch".into(),
///         project_name: "checkout-api".into(),
///     },
/// );
/// let caps = store.list_capabilities(&space).unwrap();
/// assert_eq!(caps.len(), 1);
/// ```
#[derive(Default)]
pub struct MemoryStore {
    capabilities: std::collections::HashMap<String, Vec<CapabilityRow>>,
    intents: std::collections::HashMap<String, Vec<IntentRow>>,
    decisions: std::collections::HashMap<String, Vec<DecisionRow>>,
    plans: std::collections::HashMap<String, Vec<PlanRow>>,
    /// Alias map: `<project>::<name>` → canonical `<project>::<name>`.
    aliases: std::collections::HashMap<String, String>,
}

impl MemoryStore {
    /// Insert a capability into the store for the given space.
    pub fn add_capability(&mut self, space: &SpaceId, row: CapabilityRow) {
        self.capabilities
            .entry(space.as_str().to_string())
            .or_default()
            .push(row);
    }

    /// Insert an intent into the store for the given space.
    pub fn add_intent(&mut self, space: &SpaceId, row: IntentRow) {
        self.intents
            .entry(space.as_str().to_string())
            .or_default()
            .push(row);
    }

    /// Insert a decision into the store for the given space.
    pub fn add_decision(&mut self, space: &SpaceId, row: DecisionRow) {
        self.decisions
            .entry(space.as_str().to_string())
            .or_default()
            .push(row);
    }

    /// Insert a plan into the store for the given space.
    pub fn add_plan(&mut self, space: &SpaceId, row: PlanRow) {
        self.plans
            .entry(space.as_str().to_string())
            .or_default()
            .push(row);
    }

    /// Register a cross-project alias: `ref_name` → `canonical`.
    ///
    /// `ref_name` is typically `<project_b>::<capability>` and `canonical` is
    /// `<project_a>::<capability>` (or a shared canonical identity string).
    pub fn add_alias(&mut self, ref_name: impl Into<String>, canonical: impl Into<String>) {
        self.aliases.insert(ref_name.into(), canonical.into());
    }
}

impl ConflictStore for MemoryStore {
    fn list_capabilities(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<CapabilityRow>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .capabilities
            .get(space.as_str())
            .cloned()
            .unwrap_or_default())
    }

    fn list_intents(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<IntentRow>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .intents
            .get(space.as_str())
            .cloned()
            .unwrap_or_default())
    }

    fn list_decisions(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<DecisionRow>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .decisions
            .get(space.as_str())
            .cloned()
            .unwrap_or_default())
    }

    fn list_plans(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<PlanRow>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.plans.get(space.as_str()).cloned().unwrap_or_default())
    }

    fn resolve_canonical(
        &self,
        ref_name: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.aliases
            .get(ref_name)
            .cloned()
            .ok_or_else(|| format!("no alias for '{ref_name}'").into())
    }
}
