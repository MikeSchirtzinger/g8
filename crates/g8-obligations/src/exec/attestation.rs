//! Hash-pinned attestation sidecar loading and verification.
//!
//! Attestations are supplemental evidence: a fresh record can elevate a
//! static `Checked` result to `Asserted`, while a stale or missing record can
//! never claim the code passed. The staleness policy is content-only; wall
//! clock time is audit metadata, not an expiry mechanism.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::hashing::sha256_repo_files;
use crate::result::{aggregate, Evidence, ObligationResult, ObligationStatus, SubCheckResult};

pub const ATTESTATIONS_RELATIVE_PATH: &str = "specs/attestations-v0.1.json";
pub const STALE_ATTESTATION_CLASSIFICATION: &str = "stale_attestation_pin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationClaim {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationRecord {
    pub id: String,
    pub obligation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_at: Option<String>,
    pub claim: AttestationClaim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_pointer: Option<String>,
    /// Compatibility with the originally documented §7 sidecar field. New
    /// writers use `evidence_pointer`; readers accept either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_summary: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    pub pinned_content_hash: String,
    #[serde(default = "default_expires_policy")]
    pub expires_policy: String,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl AttestationRecord {
    fn evidence(&self) -> &str {
        self.evidence_pointer
            .as_deref()
            .or(self.evidence_summary.as_deref())
            .unwrap_or("no evidence pointer recorded")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationSidecar {
    #[serde(default = "default_g8_version", alias = "govern_version")]
    pub g8_version: String,
    #[serde(default)]
    pub attestations: Vec<AttestationRecord>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for AttestationSidecar {
    fn default() -> Self {
        Self {
            g8_version: default_g8_version(),
            attestations: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_g8_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_expires_policy() -> String {
    "stale_on_hash_mismatch".to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn attestation_sidecar_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(ATTESTATIONS_RELATIVE_PATH)
}

/// Load the sidecar once per obligation run. Absence is a normal `None`.
pub fn load_attestation_sidecar(
    workspace_root: &Path,
) -> Result<Option<AttestationSidecar>, AttestationError> {
    let path = attestation_sidecar_path(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|source| AttestationError::Io {
        path: path.clone(),
        source,
    })?;
    let sidecar =
        serde_json::from_str(&text).map_err(|source| AttestationError::Json { path, source })?;
    Ok(Some(sidecar))
}

pub(crate) type LoadedAttestations = Result<Option<AttestationSidecar>, String>;

pub(crate) fn load_for_run(workspace_root: &Path) -> LoadedAttestations {
    load_attestation_sidecar(workspace_root).map_err(|error| error.to_string())
}

pub(crate) enum AttestationEvaluation {
    Missing {
        summary: String,
        detail: serde_json::Value,
    },
    Evaluated(SubCheckResult),
}

/// Evaluate the active record for one obligation. `required_attestation_id`
/// is used by legacy `checker.mode = attestation`; supplemental attestations
/// on typed obligations match by obligation ID alone.
pub(crate) fn evaluate(
    loaded: &LoadedAttestations,
    obligation_id: &str,
    required_attestation_id: Option<&str>,
    workspace_root: &Path,
) -> AttestationEvaluation {
    let start = Instant::now();
    let sidecar = match loaded {
        Ok(Some(sidecar)) => sidecar,
        Ok(None) => {
            return AttestationEvaluation::Missing {
                summary: format!("no attestation sidecar at {ATTESTATIONS_RELATIVE_PATH}"),
                detail: serde_json::json!({
                    "attestation_state": "missing",
                    "sidecar": ATTESTATIONS_RELATIVE_PATH,
                }),
            };
        }
        Err(error) => {
            return AttestationEvaluation::Evaluated(SubCheckResult {
                backend: "attestation".to_string(),
                status: ObligationStatus::Error,
                detail: serde_json::json!({
                    "classification": "attestation_reader_error",
                    "summary": format!("attestation sidecar is unusable: {error}"),
                    "error": error,
                }),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
    };

    let record = sidecar.attestations.iter().rev().find(|record| {
        record.obligation_id == obligation_id
            && required_attestation_id
                .map(|required| record.id == required)
                .unwrap_or(true)
    });
    let Some(record) = record else {
        return AttestationEvaluation::Missing {
            summary: match required_attestation_id {
                Some(required) => {
                    format!("no attestation record for {obligation_id} with id {required}")
                }
                None => format!("no attestation record for {obligation_id}"),
            },
            detail: serde_json::json!({
                "attestation_state": "missing",
                "obligation_id": obligation_id,
                "required_attestation_id": required_attestation_id,
            }),
        };
    };

    if record.files.is_empty() {
        return stale_result(
            record,
            "attestation names no files, so its pin cannot be verified".to_string(),
            None,
            start,
        );
    }

    let current_pin = match sha256_repo_files(workspace_root, &record.files) {
        Ok(pin) => pin,
        Err(error) => {
            return stale_result(
                record,
                format!("attested files cannot be re-hashed: {error}"),
                None,
                start,
            );
        }
    };

    if current_pin != record.pinned_content_hash {
        return stale_result(
            record,
            "attested file content changed after the evidence was recorded".to_string(),
            Some(current_pin),
            start,
        );
    }

    let status = match record.claim {
        AttestationClaim::Passed => ObligationStatus::Passed,
        AttestationClaim::Failed => ObligationStatus::Failed,
    };
    AttestationEvaluation::Evaluated(SubCheckResult {
        backend: "attestation".to_string(),
        status,
        detail: serde_json::json!({
            "attestation_state": "fresh",
            "summary": format!("fresh attestation {}: {}", record.id, record.evidence()),
            "attestation_id": record.id,
            "evidence_pointer": record.evidence(),
            "files": record.files,
            "pinned_content_hash": record.pinned_content_hash,
            "attested_at": record.attested_at,
        }),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

fn stale_result(
    record: &AttestationRecord,
    reason: String,
    current_pin: Option<String>,
    start: Instant,
) -> AttestationEvaluation {
    AttestationEvaluation::Evaluated(SubCheckResult {
        backend: "attestation".to_string(),
        status: ObligationStatus::Unknown,
        detail: serde_json::json!({
            "classification": STALE_ATTESTATION_CLASSIFICATION,
            "attestation_state": "stale",
            "summary": format!("stale attestation pin for {}: {reason}", record.id),
            "attestation_id": record.id,
            "evidence_pointer": record.evidence(),
            "files": record.files,
            "expected_pin": record.pinned_content_hash,
            "current_pin": current_pin,
        }),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

pub(crate) fn run(
    obligation_id: &str,
    attestation_id: &str,
    loaded: &LoadedAttestations,
    workspace_root: &Path,
) -> ObligationResult {
    match evaluate(loaded, obligation_id, Some(attestation_id), workspace_root) {
        AttestationEvaluation::Missing { summary, detail } => ObligationResult {
            id: obligation_id.to_string(),
            status: ObligationStatus::Unknown,
            trust: None,
            evidence: Evidence {
                backend: "attestation".to_string(),
                summary,
                detail,
                duration_ms: 0,
                checks_run: vec![],
            },
        },
        AttestationEvaluation::Evaluated(check) => {
            let (status, trust) = aggregate(std::slice::from_ref(&check));
            let summary = check
                .detail
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("attestation evaluated")
                .to_string();
            ObligationResult {
                id: obligation_id.to_string(),
                status,
                trust,
                evidence: Evidence {
                    backend: check.backend.clone(),
                    summary,
                    detail: check.detail.clone(),
                    duration_ms: check.duration_ms,
                    checks_run: vec![check],
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashing::sha256_repo_files;
    use crate::result::TrustLevel;

    fn record(root: &Path, claim: AttestationClaim) -> AttestationRecord {
        let files = vec!["evidence.txt".to_string()];
        let pin = sha256_repo_files(root, &files).expect("pin evidence");
        AttestationRecord {
            id: "ATT-1".to_string(),
            obligation_id: "OBL-X-01".to_string(),
            attested_at: Some("2026-07-16T00:00:00Z".to_string()),
            claim,
            evidence_pointer: Some("proof.md#obl-x-01".to_string()),
            evidence_summary: None,
            files,
            pinned_content_hash: pin,
            expires_policy: default_expires_policy(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_record_is_honestly_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loaded = Ok(None);
        let result = run("OBL-X-01", "ATT-1", &loaded, dir.path());
        assert_eq!(result.status, ObligationStatus::Unknown);
        assert_eq!(result.trust, None);
    }

    #[test]
    fn fresh_passed_record_is_asserted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("evidence.txt"), b"stable").expect("write evidence");
        let loaded = Ok(Some(AttestationSidecar {
            attestations: vec![record(dir.path(), AttestationClaim::Passed)],
            ..AttestationSidecar::default()
        }));

        let result = run("OBL-X-01", "ATT-1", &loaded, dir.path());
        assert_eq!(result.status, ObligationStatus::Passed);
        assert_eq!(result.trust, Some(TrustLevel::Asserted));
    }

    #[test]
    fn changed_file_makes_attestation_stale_and_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("evidence.txt"), b"before").expect("write before");
        let sidecar = AttestationSidecar {
            attestations: vec![record(dir.path(), AttestationClaim::Passed)],
            ..AttestationSidecar::default()
        };
        std::fs::write(dir.path().join("evidence.txt"), b"after").expect("write after");
        let loaded = Ok(Some(sidecar));

        let result = run("OBL-X-01", "ATT-1", &loaded, dir.path());
        assert_eq!(result.status, ObligationStatus::Unknown);
        assert_eq!(result.trust, None);
        assert_eq!(
            result.evidence.detail["classification"],
            STALE_ATTESTATION_CLASSIFICATION
        );
    }

    #[test]
    fn fresh_failed_claim_remains_a_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("evidence.txt"), b"stable").expect("write evidence");
        let loaded = Ok(Some(AttestationSidecar {
            attestations: vec![record(dir.path(), AttestationClaim::Failed)],
            ..AttestationSidecar::default()
        }));

        let result = run("OBL-X-01", "ATT-1", &loaded, dir.path());
        assert_eq!(result.status, ObligationStatus::Failed);
        assert_eq!(result.trust, Some(TrustLevel::Asserted));
    }
}
