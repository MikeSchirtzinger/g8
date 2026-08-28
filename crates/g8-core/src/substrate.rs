//! Substrate budget types.
//!
//! A substrate is a named architectural layer (e.g. `auth`, `export-pipeline`).
//! Each substrate in a space carries a WIP cap and a stale-threshold.

use serde::{Deserialize, Serialize};

/// WIP cap + stale-promotion threshold for a single substrate in a space.
///
/// The store returns this (with defaults applied) from
/// `StoreConnection::get_substrate_budget`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateBudget {
    /// Substrate name (kebab-case).
    pub substrate: String,
    /// Maximum number of plans in `Dispatched` status before the gate fires.
    /// Default: [`crate::DEFAULT_WIP_CAP`] (3).
    pub wip_cap: u32,
    /// Plans dispatched longer than this many days are surfaced as stale-
    /// promotion candidates. Default: 14.
    pub stale_threshold_days: u32,
}

impl SubstrateBudget {
    /// Create a budget with the default values from the spec.
    pub fn default_for(substrate: impl Into<String>) -> Self {
        Self {
            substrate: substrate.into(),
            wip_cap: crate::DEFAULT_WIP_CAP,
            stale_threshold_days: 14,
        }
    }
}
