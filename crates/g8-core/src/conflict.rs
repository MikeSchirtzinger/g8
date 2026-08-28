//! Conflict detection types: Conflict, ConflictKind, Severity, Evidence.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::{CapabilityId, DecisionId, IntentId, PlanId};

/// A detected conflict between two stores (or within one space).
///
/// Produced by `g8-conflict::ConflictDetector`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub severity: Severity,
    pub evidence: Vec<Evidence>,
}

/// Four exhaustively enumerated conflict kinds for v0.1.
///
/// # Examples
///
/// ```
/// use g8_core::ConflictKind;
///
/// let kind = ConflictKind::CapabilityNameCollision;
/// assert!(matches!(kind, ConflictKind::CapabilityNameCollision));
/// let json = serde_json::to_string(&kind).unwrap();
/// assert_eq!(json, r#""capability_name_collision""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Same intent `kind + scope_path` in both spaces with different description.
    DuplicateIntent,
    /// Same kebab-case capability name in both spaces without an alias resolving them.
    CapabilityNameCollision,
    /// Same decision title with `status=accepted` in both spaces but different body.
    ContradictoryDecisions,
    /// A plan's substrate is forbidden by a `Boundary` intent in the remote space.
    GovernanceViolation,
}

/// Severity classifies how urgently a conflict must be resolved.
///
/// Variants are ordered: `Info < Warn < Error` (derives `PartialOrd + Ord`).
///
/// # Examples
///
/// ```
/// use g8_core::Severity;
///
/// let s = Severity::Warn;
/// assert!(matches!(s, Severity::Warn));
/// assert!(Severity::Error > Severity::Info);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// Typed evidence payload attached to a [`Conflict`].
///
/// Using typed evidence (instead of plain `Vec<String>`) gives JSON consumers
/// structured access to the referenced entities (per B6 builder-discretion note).
///
/// # Examples
///
/// ```
/// use g8_core::{Evidence, PlanId};
///
/// let e = Evidence::Note { text: "duplicate substrate boundary".into() };
/// assert!(matches!(e, Evidence::Note { .. }));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Evidence {
    /// Evidence pointing to a specific plan.
    PlanRef {
        plan_id: PlanId,
        title: String,
        space: String,
    },
    /// Evidence pointing to a specific capability.
    CapabilityRef {
        capability_id: CapabilityId,
        name: String,
        project: String,
    },
    /// Evidence pointing to a specific intent.
    IntentRef {
        intent_id: IntentId,
        heading: String,
        path: PathBuf,
    },
    /// Evidence pointing to a specific decision.
    DecisionRef {
        decision_id: DecisionId,
        title: String,
    },
    /// Free-text explanatory note.
    Note { text: String },
}
