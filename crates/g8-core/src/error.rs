//! Top-level error enum for `g8-core`.

use crate::plan::PlanStatus;
use thiserror::Error;

/// Errors originating in `g8-core`.
///
/// Downstream crates define their own error types (e.g. `StoreError`,
/// `ExtractorError`) and convert from `G8Error` via `#[from]`.
///
/// # Examples
///
/// ```
/// use g8_core::G8Error;
///
/// let e = G8Error::InvalidId("empty string".into());
/// assert!(e.to_string().contains("invalid ID"));
/// ```
#[derive(Debug, Error)]
pub enum G8Error {
    /// An ID string was invalid (empty or otherwise malformed).
    #[error("invalid ID: {0}")]
    InvalidId(String),

    /// A magic-comment annotation could not be parsed.
    #[error("invalid annotation: {0}")]
    InvalidAnnotation(String),

    /// A plan status transition is not permitted by the lifecycle graph.
    #[error("invalid plan status transition: {from:?} -> {to:?}")]
    InvalidStatusTransition { from: PlanStatus, to: PlanStatus },

    /// A substrate name was referenced that has not been registered.
    #[error("unknown substrate: {0}")]
    UnknownSubstrate(String),

    /// An I/O error propagated from a caller.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A serde_json error propagated from a caller.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}
