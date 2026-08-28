//! `g8-conflict` — cross-project intent/capability overlap detection.
//!
//! Provides [`ConflictDetector`] with two entry points:
//!
//! - [`ConflictDetector::compare_spaces`] — compare two stores (local + remote-attached) and
//!   return all conflicts between them, ordered by descending severity.
//! - [`ConflictDetector::detect_intra_space`] — single-store variant: detect conflicts within
//!   one space (duplicate intents, contradictory decisions, governance violations).
//!
//! # Design notes
//!
//! This crate is purely computational — no I/O, no async. It reads data from the store via the
//! [`ConflictStore`] trait and produces `Vec<Conflict>` using typed [`Evidence`] variants (per
//! A1 decision #3 — `Vec<Evidence>` not stringified evidence).
//!
//! Cross-project capability identity is `<project>::<name>` (used only as lookup keys per A1
//! decision #2; annotations never carry the prefix).
//!
//! ## Four conflict kinds
//!
//! | Kind | Trigger | Severity |
//! |---|---|---|
//! | `DuplicateIntent` | Same `kind + scope_path` in both spaces, different `description` | `Warn` |
//! | `CapabilityNameCollision` | Same kebab-case `name` in both spaces, no alias resolves them | `Error` |
//! | `CapabilityNameCollision` | Same name AND alias resolves them | `Info` |
//! | `ContradictoryDecisions` | Same `title`, both `accepted`, different `body` | `Error` |
//! | `GovernanceViolation` | Plan substrate forbidden by a `Boundary` intent in the other space | `Error` |

pub mod store_adapter;
pub mod store_trait;

use std::path::PathBuf;

use g8_core::{Conflict, ConflictKind, Evidence, IntentKind, Severity, SpaceId};
use tracing::instrument;

pub use store_adapter::ConflictAdapter;
pub use store_trait::ConflictStore;

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors produced by [`ConflictDetector`] operations.
///
/// The only current variant is a transparent delegation to the store error type.
/// Downstream crates should match `ConflictError::Store` to inspect the root cause.
#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    /// A store operation failed during conflict detection.
    #[error("store error during conflict detection: {0}")]
    Store(String),
}

// ── ConflictDetector ─────────────────────────────────────────────────────────

/// Entry point for all conflict-detection operations.
///
/// All methods are associated functions (no instance state); the detector is
/// stateless — store references are passed per-call.
pub struct ConflictDetector;

impl ConflictDetector {
    /// Compare two stores and return all conflicts between them.
    ///
    /// Both stores must have fully-loaded scan data (capabilities, intents, decisions, plans).
    /// The returned `Vec<Conflict>` is sorted by descending severity (`Error` first, then `Warn`,
    /// then `Info`).
    ///
    /// # Arguments
    ///
    /// * `local` — the local store (read-only access required)
    /// * `local_space` — space ID to query in the local store
    /// * `remote` — the remote store (typically ATTACHed or opened separately)
    /// * `remote_space` — space ID to query in the remote store
    ///
    /// # Examples
    ///
    /// ```
    /// use g8_conflict::{ConflictDetector, ConflictStore};
    /// use g8_conflict::store_trait::MemoryStore;
    /// use g8_core::SpaceId;
    ///
    /// let local = MemoryStore::default();
    /// let remote = MemoryStore::default();
    /// let local_space = SpaceId::derive(&["docs", "local"]);
    /// let remote_space = SpaceId::derive(&["docs", "remote"]);
    ///
    /// let conflicts = ConflictDetector::compare_spaces(
    ///     &local, &local_space,
    ///     &remote, &remote_space,
    /// ).unwrap();
    /// assert!(conflicts.is_empty(), "empty stores produce no conflicts");
    /// ```
    #[instrument(skip(local, remote), fields(local_space = %local_space, remote_space = %remote_space, conflicts_found))]
    pub fn compare_spaces(
        local: &dyn ConflictStore,
        local_space: &SpaceId,
        remote: &dyn ConflictStore,
        remote_space: &SpaceId,
    ) -> Result<Vec<Conflict>, ConflictError> {
        let mut conflicts = Vec::new();

        // ── 1. DuplicateIntent ───────────────────────────────────────────────
        let duplicate_intent_conflicts =
            detect_duplicate_intents(local, local_space, remote, remote_space)?;
        conflicts.extend(duplicate_intent_conflicts);

        // ── 2. CapabilityNameCollision ────────────────────────────────────────
        let capability_collisions =
            detect_capability_collisions(local, local_space, remote, remote_space)?;
        conflicts.extend(capability_collisions);

        // ── 3. ContradictoryDecisions ────────────────────────────────────────
        let decision_conflicts =
            detect_contradictory_decisions(local, local_space, remote, remote_space)?;
        conflicts.extend(decision_conflicts);

        // ── 4. GovernanceViolation ────────────────────────────────────────────
        // Check plans in local against boundary intents in remote, AND
        // plans in remote against boundary intents in local.
        let gov_violations =
            detect_governance_violations(local, local_space, remote, remote_space)?;
        conflicts.extend(gov_violations);

        // Sort by descending severity (Error > Warn > Info).
        conflicts.sort_by_key(|c| std::cmp::Reverse(c.severity));

        tracing::Span::current().record("conflicts_found", conflicts.len());

        Ok(conflicts)
    }

    /// Detect conflicts within a single space (no cross-store comparison).
    ///
    /// Used by `g8 status --intra-conflicts` to surface issues in one workspace. Detects
    /// duplicate intents and contradictory decisions within the same space.
    ///
    /// # Examples
    ///
    /// ```
    /// use g8_conflict::{ConflictDetector, ConflictStore};
    /// use g8_conflict::store_trait::MemoryStore;
    /// use g8_core::SpaceId;
    ///
    /// let store = MemoryStore::default();
    /// let space = SpaceId::derive(&["docs", "space"]);
    ///
    /// let conflicts = ConflictDetector::detect_intra_space(&store, &space).unwrap();
    /// assert!(conflicts.is_empty());
    /// ```
    #[instrument(skip(store), fields(space = %space))]
    pub fn detect_intra_space(
        store: &dyn ConflictStore,
        space: &SpaceId,
    ) -> Result<Vec<Conflict>, ConflictError> {
        let mut conflicts = Vec::new();

        // Duplicate intents within the same space: compare the space against itself
        // looking for same kind+scope_path with different descriptions (e.g. from
        // different projects in the same space contributing conflicting section bodies).
        let intents = store
            .list_intents(space)
            .map_err(|e| ConflictError::Store(e.to_string()))?;

        // Group by (kind, scope_path) — if multiple intents share the key with
        // different descriptions that is an intra-space duplicate.
        let mut seen: std::collections::HashMap<(String, PathBuf), Vec<&store_trait::IntentRow>> =
            std::collections::HashMap::new();
        for intent in &intents {
            let key = (format!("{:?}", intent.kind), intent.scope_path.clone());
            seen.entry(key).or_default().push(intent);
        }

        for group in seen.values() {
            if group.len() < 2 {
                continue;
            }
            // Check if any pair has different descriptions.
            let first = &group[0];
            for other in group.iter().skip(1) {
                if other.description != first.description {
                    let evidence = vec![
                        Evidence::IntentRef {
                            intent_id: first.id.clone(),
                            heading: first.heading.clone(),
                            path: first.scope_path.clone(),
                        },
                        Evidence::IntentRef {
                            intent_id: other.id.clone(),
                            heading: other.heading.clone(),
                            path: other.scope_path.clone(),
                        },
                        Evidence::Note {
                            text: format!(
                                "Intra-space duplicate: same kind={:?} scope_path={} with different description",
                                first.kind,
                                first.scope_path.display()
                            ),
                        },
                    ];
                    conflicts.push(Conflict {
                        kind: ConflictKind::DuplicateIntent,
                        severity: Severity::Warn,
                        evidence,
                    });
                }
            }
        }

        // Contradictory decisions within the same space.
        let decisions = store
            .list_decisions(space)
            .map_err(|e| ConflictError::Store(e.to_string()))?;

        let mut decision_map: std::collections::HashMap<String, Vec<&store_trait::DecisionRow>> =
            std::collections::HashMap::new();
        for d in &decisions {
            decision_map.entry(d.title.clone()).or_default().push(d);
        }

        for (title, group) in &decision_map {
            let accepted: Vec<&&store_trait::DecisionRow> =
                group.iter().filter(|d| d.status_accepted).collect();
            if accepted.len() < 2 {
                continue;
            }
            let first = accepted[0];
            for other in accepted.iter().skip(1) {
                if other.body != first.body {
                    let evidence = vec![
                        Evidence::DecisionRef {
                            decision_id: first.id.clone(),
                            title: title.clone(),
                        },
                        Evidence::DecisionRef {
                            decision_id: other.id.clone(),
                            title: title.clone(),
                        },
                        Evidence::Note {
                            text: format!(
                                "Intra-space contradictory accepted decisions for title='{title}'"
                            ),
                        },
                    ];
                    conflicts.push(Conflict {
                        kind: ConflictKind::ContradictoryDecisions,
                        severity: Severity::Error,
                        evidence,
                    });
                }
            }
        }

        conflicts.sort_by_key(|c| std::cmp::Reverse(c.severity));
        Ok(conflicts)
    }
}

// ── Detection helpers (private) ──────────────────────────────────────────────

/// Detect `DuplicateIntent` conflicts: same `kind + scope_path`, different `description`.
///
/// Severity: `Warn` (per ARCH §3.5 detection table).
fn detect_duplicate_intents(
    local: &dyn ConflictStore,
    local_space: &SpaceId,
    remote: &dyn ConflictStore,
    remote_space: &SpaceId,
) -> Result<Vec<Conflict>, ConflictError> {
    let local_intents = local
        .list_intents(local_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;
    let remote_intents = remote
        .list_intents(remote_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;

    let mut conflicts = Vec::new();

    for li in &local_intents {
        for ri in &remote_intents {
            let same_kind = li.kind == ri.kind;
            let same_scope = li.scope_path == ri.scope_path;
            let different_description = li.description != ri.description;

            if same_kind && same_scope && different_description {
                conflicts.push(Conflict {
                    kind: ConflictKind::DuplicateIntent,
                    severity: Severity::Warn,
                    evidence: vec![
                        Evidence::IntentRef {
                            intent_id: li.id.clone(),
                            heading: li.heading.clone(),
                            path: li.scope_path.clone(),
                        },
                        Evidence::IntentRef {
                            intent_id: ri.id.clone(),
                            heading: ri.heading.clone(),
                            path: ri.scope_path.clone(),
                        },
                        Evidence::Note {
                            text: format!(
                                "Both spaces declare intent kind={:?} at '{}' with different descriptions",
                                li.kind,
                                li.scope_path.display()
                            ),
                        },
                    ],
                });
            }
        }
    }

    Ok(conflicts)
}

/// Detect `CapabilityNameCollision` conflicts.
///
/// - Same kebab-case `name` in both spaces AND no alias resolves them → `Error`
/// - Same kebab-case `name` AND alias resolves them → `Info` (acknowledgment)
///
/// Cross-project capability identity key is `<project_name>::<capability_name>` per A1 decision #2.
/// The project name is used only in lookup keys — annotations never carry the prefix.
fn detect_capability_collisions(
    local: &dyn ConflictStore,
    local_space: &SpaceId,
    remote: &dyn ConflictStore,
    remote_space: &SpaceId,
) -> Result<Vec<Conflict>, ConflictError> {
    let local_caps = local
        .list_capabilities(local_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;
    let remote_caps = remote
        .list_capabilities(remote_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;

    // Build a map from canonical name → capability for the remote side.
    // canonical key form: <project_name>::<capability_name>
    let remote_by_name: std::collections::HashMap<&str, &store_trait::CapabilityRow> =
        remote_caps.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut conflicts = Vec::new();

    for lc in &local_caps {
        if let Some(rc) = remote_by_name.get(lc.name.as_str()) {
            // Same capability name exists in both spaces.
            // Check if an alias resolves them — i.e. both are known by the same canonical name.
            let local_key = format!("{}::{}", lc.project_name, lc.name);
            let remote_key = format!("{}::{}", rc.project_name, rc.name);

            let alias_resolves = local
                .resolve_canonical(&local_key)
                .and_then(|lk| remote.resolve_canonical(&remote_key).map(|rk| lk == rk))
                .unwrap_or(false);

            let (severity, note) = if alias_resolves {
                (
                    Severity::Info,
                    format!(
                        "Capability '{}' exists in both spaces but alias resolves them to the same canonical identity",
                        lc.name
                    ),
                )
            } else {
                (
                    Severity::Error,
                    format!(
                        "Capability '{}' exists in both spaces with no alias: cross-project name collision",
                        lc.name
                    ),
                )
            };

            conflicts.push(Conflict {
                kind: ConflictKind::CapabilityNameCollision,
                severity,
                evidence: vec![
                    Evidence::CapabilityRef {
                        capability_id: lc.id.clone(),
                        name: lc.name.clone(),
                        project: lc.project_name.clone(),
                    },
                    Evidence::CapabilityRef {
                        capability_id: rc.id.clone(),
                        name: rc.name.clone(),
                        project: rc.project_name.clone(),
                    },
                    Evidence::Note { text: note },
                ],
            });
        }
    }

    Ok(conflicts)
}

/// Detect `ContradictoryDecisions` conflicts.
///
/// Rule: same `title` in both spaces, both with `status = accepted`, `body` differs.
/// Severity: `Error`.
fn detect_contradictory_decisions(
    local: &dyn ConflictStore,
    local_space: &SpaceId,
    remote: &dyn ConflictStore,
    remote_space: &SpaceId,
) -> Result<Vec<Conflict>, ConflictError> {
    let local_decisions = local
        .list_decisions(local_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;
    let remote_decisions = remote
        .list_decisions(remote_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;

    // Index remote accepted decisions by title.
    let remote_accepted: std::collections::HashMap<&str, &store_trait::DecisionRow> =
        remote_decisions
            .iter()
            .filter(|d| d.status_accepted)
            .map(|d| (d.title.as_str(), d))
            .collect();

    let mut conflicts = Vec::new();

    for ld in &local_decisions {
        if !ld.status_accepted {
            continue;
        }
        if let Some(rd) = remote_accepted.get(ld.title.as_str()) {
            if ld.body != rd.body {
                conflicts.push(Conflict {
                    kind: ConflictKind::ContradictoryDecisions,
                    severity: Severity::Error,
                    evidence: vec![
                        Evidence::DecisionRef {
                            decision_id: ld.id.clone(),
                            title: ld.title.clone(),
                        },
                        Evidence::DecisionRef {
                            decision_id: rd.id.clone(),
                            title: rd.title.clone(),
                        },
                        Evidence::Note {
                            text: format!(
                                "Both spaces have accepted decision '{}' but the body text differs",
                                ld.title
                            ),
                        },
                    ],
                });
            }
        }
    }

    Ok(conflicts)
}

/// Detect `GovernanceViolation` conflicts.
///
/// Rule: A Plan in space A has `substrate = X`; space B has a `Boundary` intent at any ancestor
/// path saying "never modify X" (the intent `description` or `heading` contains `substrate` name).
///
/// We check both directions: local plans vs remote boundaries, and remote plans vs local boundaries.
/// Severity: `Error`.
fn detect_governance_violations(
    local: &dyn ConflictStore,
    local_space: &SpaceId,
    remote: &dyn ConflictStore,
    remote_space: &SpaceId,
) -> Result<Vec<Conflict>, ConflictError> {
    let local_plans = local
        .list_plans(local_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;
    let remote_plans = remote
        .list_plans(remote_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?;

    let remote_boundary_intents: Vec<store_trait::IntentRow> = remote
        .list_intents(remote_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?
        .into_iter()
        .filter(|i| i.kind == IntentKind::Boundary)
        .collect();

    let local_boundary_intents: Vec<store_trait::IntentRow> = local
        .list_intents(local_space)
        .map_err(|e| ConflictError::Store(e.to_string()))?
        .into_iter()
        .filter(|i| i.kind == IntentKind::Boundary)
        .collect();

    let mut conflicts = Vec::new();

    // Check local plans against remote boundary intents.
    for plan in &local_plans {
        if let Some(ref substrate) = plan.substrate {
            for boundary in &remote_boundary_intents {
                if boundary_forbids(boundary, substrate) {
                    conflicts.push(Conflict {
                        kind: ConflictKind::GovernanceViolation,
                        severity: Severity::Error,
                        evidence: vec![
                            Evidence::PlanRef {
                                plan_id: plan.id.clone(),
                                title: plan.title.clone(),
                                space: local_space.as_str().to_string(),
                            },
                            Evidence::IntentRef {
                                intent_id: boundary.id.clone(),
                                heading: boundary.heading.clone(),
                                path: boundary.scope_path.clone(),
                            },
                            Evidence::Note {
                                text: format!(
                                    "Plan '{}' uses substrate '{substrate}' which is forbidden by boundary intent '{}' in the remote space",
                                    plan.title, boundary.heading
                                ),
                            },
                        ],
                    });
                }
            }
        }
    }

    // Check remote plans against local boundary intents.
    for plan in &remote_plans {
        if let Some(ref substrate) = plan.substrate {
            for boundary in &local_boundary_intents {
                if boundary_forbids(boundary, substrate) {
                    conflicts.push(Conflict {
                        kind: ConflictKind::GovernanceViolation,
                        severity: Severity::Error,
                        evidence: vec![
                            Evidence::PlanRef {
                                plan_id: plan.id.clone(),
                                title: plan.title.clone(),
                                space: remote_space.as_str().to_string(),
                            },
                            Evidence::IntentRef {
                                intent_id: boundary.id.clone(),
                                heading: boundary.heading.clone(),
                                path: boundary.scope_path.clone(),
                            },
                            Evidence::Note {
                                text: format!(
                                    "Remote plan '{}' uses substrate '{substrate}' which is forbidden by boundary intent '{}' in the local space",
                                    plan.title, boundary.heading
                                ),
                            },
                        ],
                    });
                }
            }
        }
    }

    Ok(conflicts)
}

/// Returns `true` if the boundary intent description or heading contains a reference that
/// forbids the given substrate.
///
/// The detection heuristic: the intent description contains the substring "never modify",
/// "never use", "do not use", or "forbidden" followed (within 80 chars) by the substrate name,
/// OR the heading or description contains the exact substrate name (case-insensitive).
///
/// This is intentionally simple for v0.1. Richer NL parsing is v0.2 scope.
fn boundary_forbids(boundary: &store_trait::IntentRow, substrate: &str) -> bool {
    let lower_desc = boundary.description.to_lowercase();
    let lower_heading = boundary.heading.to_lowercase();
    let lower_substrate = substrate.to_lowercase();

    // Direct mention of the substrate in the heading or description.
    let mentions_substrate =
        lower_desc.contains(&lower_substrate) || lower_heading.contains(&lower_substrate);

    if !mentions_substrate {
        return false;
    }

    // Must also contain a prohibition keyword to count as a governance constraint.
    lower_desc.contains("never modify")
        || lower_desc.contains("never use")
        || lower_desc.contains("do not use")
        || lower_desc.contains("forbidden")
        || lower_desc.contains("prohibited")
        || lower_desc.contains("must not")
        || lower_heading.contains("never modify")
        || lower_heading.contains("never use")
        || lower_heading.contains("forbidden")
        || lower_heading.contains("must not")
}
