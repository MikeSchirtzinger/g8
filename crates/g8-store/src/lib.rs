//! `g8-store` — embedded SQLite persistence layer for the G8.
//!
//! Provides the [`StoreConnection`] trait and its rusqlite-backed implementation
//! [`RusqliteStore`].  All schema migrations are embedded in the binary via
//! `refinery`.
//!
//! # Opening a store
//!
//! ```no_run
//! use g8_store::RusqliteStore;
//! use std::path::Path;
//!
//! let mut store = RusqliteStore::open(Path::new(".g8/store.db")).unwrap();
//! store.migrate().unwrap();
//! ```
//!
//! # In-memory store (tests)
//!
//! ```
//! use g8_store::RusqliteStore;
//!
//! let mut store = RusqliteStore::open_in_memory().unwrap();
//! store.migrate().unwrap();
//! ```

pub mod migrations;
mod store_impl;

pub use store_impl::{
    ApplyScanReport, PairingError, PairingErrorReason, PlanFilter, RawIntentCheckPayload,
    RusqliteStore, ScanResult, StalePlanCandidate, StoreConnection, StoreError, WatchHandle,
};

// Re-export OverlapKind + PlanIntentRelation from g8-core so callers don't
// need to import g8-core just to call link_plan_capability / link_plan_intent.
pub use g8_core::plan::{OverlapKind, PlanIntentRelation};
