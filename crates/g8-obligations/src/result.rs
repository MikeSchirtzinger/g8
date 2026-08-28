//! Status / trust semantics + multi-check aggregation — contract §3.
//!
//! Verbatim transcription of the contract's types, plus the §3.3 aggregation
//! rule as a pure function over `&[SubCheckResult]` (no I/O — same spirit as
//! `g8-planner`'s pure post-processing functions).

use serde::{Deserialize, Serialize};

use crate::backend::trust_tier_for_name;

/// What happened this run — a claim about the *code* (`Passed`/`Failed`) or
/// about the *checker itself* (`Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus {
    /// The checker(s) executed to completion and every sub-check's actual
    /// outcome matched its expected outcome.
    Passed,
    /// The checker(s) executed to completion and at least one sub-check's
    /// actual outcome did NOT match — a genuine, actionable constraint
    /// violation.
    Failed,
    /// The checker(s) could NOT reach a conclusive result (binary missing,
    /// subprocess crashed, malformed args, timeout). Never silently reads as
    /// `Passed` just because the checker happened to crash.
    Error,
    /// No checker is configured and no non-stale attestation exists.
    Unknown,
}

/// How much the status is worth — a **static** property of *how* an
/// obligation is evaluated (fixed by its mapping-table entry), not a runtime
/// measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// The backend actually ran real `g8` code (subprocess or in-process)
    /// against a constructed fixture and observed real behavior.
    Verified,
    /// Static/structural inspection of source or build metadata; no real
    /// runtime behavior exercised.
    Checked,
    /// A content-hash-pinned attestation is fresh for the files it names, but
    /// no checker reproduced the claim *this run*.
    Asserted,
}

/// Ordinal rank used only to select the strongest evidence during aggregation.
/// A fresh attestation now participates alongside typed sub-checks so a
/// `Checked` gate can satisfy the rigor floor without pretending the static
/// checker itself became runtime verification.
fn tier_rank(t: TrustLevel) -> u8 {
    match t {
        TrustLevel::Checked => 0,
        TrustLevel::Asserted => 1,
        TrustLevel::Verified => 2,
    }
}

pub(crate) fn backend_tier_rank(name: &str) -> u8 {
    tier_rank(trust_tier_for_name(name))
}

fn max_tier(a: TrustLevel, b: TrustLevel) -> TrustLevel {
    if tier_rank(b) > tier_rank(a) {
        b
    } else {
        a
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationResult {
    pub id: String,
    pub status: ObligationStatus,
    /// `None` **iff** `status == Unknown`. Every other status has a trust.
    pub trust: Option<TrustLevel>,
    pub evidence: Evidence,
}

/// Rolled-up, top-level view. `backend`/trust-tier here are the ones that
/// determined the overall `ObligationResult` per the §3.3 aggregation rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub backend: CheckerBackendName,
    pub summary: String,
    pub detail: serde_json::Value,
    pub duration_ms: u64,
    /// One entry per element of the obligation's `checks: Vec<CheckerBackend>`
    /// — full transparency for composite/multi-check obligations, even
    /// though `status`/`trust` above are already the aggregated view.
    pub checks_run: Vec<SubCheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCheckResult {
    pub backend: CheckerBackendName,
    /// This one sub-check's own outcome.
    pub status: ObligationStatus,
    pub detail: serde_json::Value,
    pub duration_ms: u64,
}

/// The `#[serde(tag = "backend", ...)]` string from [`crate::backend::CheckerBackend`].
pub type CheckerBackendName = String;

/// Apply the contract §3.3 aggregation rule over one obligation's sub-checks.
///
/// ```text
/// status = Error   if any sub-check returned Error
///        else Failed  if any sub-check returned Failed
///        else Passed  if at least one sub-check passed
///        else Unknown (all sub-checks unknown)
///
/// trust  = max(tier) over conclusive sub-checks (Passed/Failed)
///          (falls back to the nominal tier of the highest-tier configured
///           backend if literally every sub-check errored)
/// ```
///
/// Returns `(Unknown, None)` for an empty `checks` slice — the runner must
/// never divide-by-zero or panic on a configured-but-empty check list.
pub fn aggregate(checks: &[SubCheckResult]) -> (ObligationStatus, Option<TrustLevel>) {
    if checks.is_empty() {
        return (ObligationStatus::Unknown, None);
    }

    let any_error = checks.iter().any(|c| c.status == ObligationStatus::Error);
    let any_failed = checks.iter().any(|c| c.status == ObligationStatus::Failed);
    let any_passed = checks.iter().any(|c| c.status == ObligationStatus::Passed);

    let non_errored_trust: Option<TrustLevel> = checks
        .iter()
        .filter(|c| {
            !matches!(
                c.status,
                ObligationStatus::Error | ObligationStatus::Unknown
            )
        })
        .map(|c| trust_tier_for_name(&c.backend))
        .reduce(max_tier);

    let status = if any_error {
        ObligationStatus::Error
    } else if any_failed {
        ObligationStatus::Failed
    } else if any_passed {
        // An Unknown supplemental attestation does not erase a conclusive
        // typed result. Its trust is excluded above, so it cannot elevate the
        // result either.
        ObligationStatus::Passed
    } else {
        ObligationStatus::Unknown
    };

    let trust = non_errored_trust.or_else(|| {
        if status == ObligationStatus::Unknown {
            return None;
        }
        // Every sub-check errored: fall back to the nominal tier of the
        // highest-tier CONFIGURED backend (still meaningful for filtering
        // even though status = Error).
        checks
            .iter()
            .filter(|check| check.status == ObligationStatus::Error)
            .map(|c| trust_tier_for_name(&c.backend))
            .reduce(max_tier)
    });

    (status, trust)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(backend: &str, status: ObligationStatus) -> SubCheckResult {
        SubCheckResult {
            backend: backend.to_string(),
            status,
            detail: serde_json::json!({}),
            duration_ms: 1,
        }
    }

    #[test]
    fn empty_checks_is_unknown() {
        let (status, trust) = aggregate(&[]);
        assert_eq!(status, ObligationStatus::Unknown);
        assert_eq!(trust, None);
    }

    #[test]
    fn all_passed_is_passed_with_max_trust() {
        let checks = vec![
            sub("cargo_metadata_no_dep", ObligationStatus::Passed), // Checked
            sub("fixture_integration_test", ObligationStatus::Passed), // Verified
        ];
        let (status, trust) = aggregate(&checks);
        assert_eq!(status, ObligationStatus::Passed);
        assert_eq!(trust, Some(TrustLevel::Verified));
    }

    #[test]
    fn fresh_attestation_outranks_checked_but_not_verified() {
        let checked_and_attested = vec![
            sub("rg_match_count", ObligationStatus::Passed),
            sub("attestation", ObligationStatus::Passed),
        ];
        assert_eq!(
            aggregate(&checked_and_attested),
            (ObligationStatus::Passed, Some(TrustLevel::Asserted))
        );

        let verified_and_attested = vec![
            sub("fixture_integration_test", ObligationStatus::Passed),
            sub("attestation", ObligationStatus::Passed),
        ];
        assert_eq!(
            aggregate(&verified_and_attested),
            (ObligationStatus::Passed, Some(TrustLevel::Verified))
        );
    }

    #[test]
    fn stale_attestation_does_not_elevate_checked_result() {
        let checks = vec![
            sub("rg_match_count", ObligationStatus::Passed),
            sub("attestation", ObligationStatus::Unknown),
        ];
        assert_eq!(
            aggregate(&checks),
            (ObligationStatus::Passed, Some(TrustLevel::Checked))
        );
    }

    #[test]
    fn unknown_only_has_no_trust() {
        let checks = vec![sub("attestation", ObligationStatus::Unknown)];
        assert_eq!(aggregate(&checks), (ObligationStatus::Unknown, None));
    }

    #[test]
    fn any_error_taints_whole_result_even_if_others_passed() {
        let checks = vec![
            sub("cargo_metadata_no_dep", ObligationStatus::Passed),
            sub("rg_match_count", ObligationStatus::Error),
        ];
        let (status, trust) = aggregate(&checks);
        assert_eq!(status, ObligationStatus::Error);
        // trust falls back to the max tier among non-errored sub-checks.
        assert_eq!(trust, Some(TrustLevel::Checked));
    }

    #[test]
    fn error_outranks_failed() {
        let checks = vec![
            sub("rg_match_count", ObligationStatus::Failed),
            sub("ast_grep_no_match", ObligationStatus::Error),
        ];
        let (status, _) = aggregate(&checks);
        assert_eq!(status, ObligationStatus::Error);
    }

    #[test]
    fn failed_without_error_is_failed() {
        let checks = vec![
            sub("cargo_metadata_no_dep", ObligationStatus::Passed),
            sub("byte_diff_twice", ObligationStatus::Failed),
        ];
        let (status, trust) = aggregate(&checks);
        assert_eq!(status, ObligationStatus::Failed);
        assert_eq!(trust, Some(TrustLevel::Verified));
    }

    #[test]
    fn all_errored_falls_back_to_nominal_tier_of_configured_backends() {
        let checks = vec![sub("g8_check_contract", ObligationStatus::Error)];
        let (status, trust) = aggregate(&checks);
        assert_eq!(status, ObligationStatus::Error);
        // g8_check_contract's nominal tier is Verified even though it errored.
        assert_eq!(trust, Some(TrustLevel::Verified));
    }
}
