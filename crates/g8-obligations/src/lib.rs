//! `g8-obligations` — typed obligation-checker backends for `g8 check`.
//!
//! Executes the 27-obligation self-audit (`specs/obligations-v0.1.json`)
//! against g8's own repository. Every obligation resolves to a member of a
//! **closed Rust enum** of checker backends with structured, typed arguments
//! — see [`backend::CheckerBackend`]. There is no `shell_command: String`
//! field anywhere in this crate (`specs/obligation-checker-contract.md` §0).
//!
//! # Deterministic zone
//!
//! No LLM calls, no network, no unseeded randomness. Subprocess calls to
//! `cargo`/`ast-grep`/`rg`/`current_exe()` are the same class of dependency
//! `g8-extractor` already has on `ast-grep`.
//!
//! # `G8CheckContract` design note (contract §8)
//!
//! §8 describes this backend's implementation in terms of
//! `g8_core::contract::CheckPayload`/`CheckError` — types that do not exist
//! yet. Their relocation from `crates/g8/src/render/json.rs` into
//! `g8-core` is explicitly T5's job ("a small, precisely-scoped refactor for
//! T5... not something I am doing here" — contract §8 point 1), and
//! `g8-core`/`g8` are both outside this crate's writable (and, for
//! `g8`, even dependency-able) scope. Since T3 runs before T5 in the
//! plan's dependency order, those types cannot exist yet when this code is
//! written.
//!
//! This crate's [`exec::check_contract`] therefore validates against a
//! `serde_json::Value` shape seam instead of the not-yet-relocated concrete
//! types: it builds the check-contract envelope directly (mirroring exactly
//! what `render_check`/`render_check_enforcement_disabled` in
//! `crates/g8/src/render/json.rs` produce today, and what
//! `CLAUDE.md`'s locked "Output contract" section documents), using the
//! REAL `g8_store::{PairingError, PairingErrorReason}` types (already
//! `Serialize`, already living in the allowed `g8-store` dependency — no
//! relocation needed for those) for the `errors[]` array. This keeps the
//! backend's trust tier honestly `Verified` (it runs `g8-store`'s real
//! `pairing_check` against a real in-memory `RusqliteStore`, not a hollow
//! shape assertion) while depending on nothing that doesn't exist today.
//! Once T5 relocates `CheckPayload`/`CheckError`, this seam continues to
//! work unchanged — it was never coupled to their Rust type identity, only
//! to the JSON shape they serialize to.
//!
//! # Receipt mode (contract addendum, o-g8-receipts-20260721)
//!
//! `Signal.wiring == "action_receipt"` declares an obligation's subject is a
//! parsed *action receipt*, not the repository. [`run_obligations`]'s
//! `receipt` parameter selects the choke point:
//!
//! - `receipt = None` (repo mode): receipt-wired obligations are **not
//!   executed** — they surface as `status = Unknown` with
//!   `evidence.detail.scope = "action_receipt"` and are exempt from gate
//!   classification (their choke point is elsewhere; a green repo stays
//!   green). Repo-wired obligations run exactly as before this addendum.
//! - `receipt = Some(subject)` (receipt mode): *only* receipt-wired
//!   obligations run, against `subject`; repo-wired obligations are skipped
//!   entirely (excluded from the returned `Vec`, not represented).
//!
//! Subject binding is validated either way: a receipt-wired obligation may
//! only contain `receipt_query` checks, and `receipt_query` may not appear
//! in a repo-wired obligation. A violation is `ObligationStatus::Error`
//! (loud, gates) rather than a silent skip.

pub mod artifact;
pub mod backend;
mod exec;
pub mod hashing;
mod proc;
pub mod result;
mod scratch;

use std::path::{Path, PathBuf};
use std::time::Instant;

pub use artifact::{
    load_artifact, Obligation, ObligationArtifact, ObligationChecker, ObligationsError, Signal,
};
pub use backend::*;
pub use exec::attestation::{
    attestation_sidecar_path, load_attestation_sidecar, AttestationClaim, AttestationError,
    AttestationRecord, AttestationSidecar, ATTESTATIONS_RELATIVE_PATH,
    STALE_ATTESTATION_CLASSIFICATION,
};
pub use result::*;

/// The `signal.wiring` value that binds an obligation to the receipt choke
/// point (contract addendum, o-g8-receipts-20260721 §1). Every other
/// (or absent/default) wiring value is repo-wired.
pub const ACTION_RECEIPT_WIRING: &str = "action_receipt";

/// A parsed, content-hashed action receipt — the subject `receipt_query`
/// checks evaluate against. Built exactly once per `g8 check --receipt`
/// invocation via [`load_receipt_subject`] and threaded through
/// [`run_obligations`], never re-parsed per obligation.
#[derive(Debug, Clone)]
pub struct ReceiptSubject {
    /// The path exactly as given to `g8 check --receipt`, for
    /// self-evidencing JSON output (`render_check`'s `receipt.path`) — not
    /// necessarily canonicalized.
    pub display_path: String,
    /// `sha256:<hex>` of the receipt file's raw bytes (same `sha256:`-prefixed
    /// form [`hashing::sha256_file`] produces elsewhere in this crate).
    pub sha256: String,
    /// The parsed receipt document.
    pub value: serde_json::Value,
}

/// Failure to load a receipt file into a [`ReceiptSubject`]. Carries the
/// SHA-256 when one could still be computed (contract: "even a malformed
/// receipt's hash is meaningful evidence of exactly what bad input was
/// rejected") — only a wholly unreadable file has none.
#[derive(Debug, thiserror::Error)]
pub enum ReceiptSubjectError {
    #[error("cannot hash receipt file {path}: {source}")]
    Hash {
        path: PathBuf,
        #[source]
        source: hashing::HashingError,
    },
    #[error("cannot read receipt file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("receipt file {path} is not valid JSON (sha256 {sha256}): {source}")]
    Json {
        path: PathBuf,
        sha256: String,
        #[source]
        source: serde_json::Error,
    },
}

impl ReceiptSubjectError {
    /// The receipt's content hash, if loading got far enough to compute one.
    pub fn sha256(&self) -> Option<&str> {
        match self {
            ReceiptSubjectError::Json { sha256, .. } => Some(sha256),
            ReceiptSubjectError::Hash { .. } | ReceiptSubjectError::Read { .. } => None,
        }
    }
}

/// Parse+hash one receipt file. The file is hashed first (so a merely
/// malformed-JSON receipt — the file exists and is readable — still yields a
/// real `sha256` on its `Err`), then parsed as JSON.
pub fn load_receipt_subject(path: &Path) -> Result<ReceiptSubject, ReceiptSubjectError> {
    let sha256 = hashing::sha256_file(path).map_err(|source| ReceiptSubjectError::Hash {
        path: path.to_path_buf(),
        source,
    })?;
    let text = std::fs::read_to_string(path).map_err(|source| ReceiptSubjectError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|source| ReceiptSubjectError::Json {
            path: path.to_path_buf(),
            sha256: sha256.clone(),
            source,
        })?;
    Ok(ReceiptSubject {
        display_path: path.display().to_string(),
        sha256,
        value,
    })
}

/// The one entry point. Never panics; a single backend crashing surfaces as
/// that one obligation's `status = Error`, not a propagated panic — every
/// backend invocation is isolated (caught, not just `?`-propagated).
///
/// `receipt` selects the choke point (module docs above): `None` runs the
/// repo-wired obligations (unchanged behavior pre-dating this addendum);
/// `Some(subject)` runs only the `action_receipt`-wired obligations, against
/// `subject`.
///
/// Opens tracing span `obligations.run_all` (fields: `artifact_version`,
/// `obligations_count`, `duration_ms`) per SPEC Principle 6 / the P6-01 span
/// table — flagged in contract §4.1 as a gap in ARCHITECTURE.md §12's
/// existing 9-span inventory that needs a 10th row; no task in the current
/// plan has ARCHITECTURE.md in scope to add it, so the span is opened here
/// (harmless, forward-compatible) but the table itself is not this crate's
/// to edit.
#[tracing::instrument(skip(artifact, workspace_root, receipt), fields(obligations_count = artifact.obligations.len()))]
pub fn run_obligations(
    artifact: &ObligationArtifact,
    workspace_root: &Path,
    receipt: Option<&ReceiptSubject>,
) -> Vec<ObligationResult> {
    let run_start = Instant::now();
    let attestations = exec::attestation::load_for_run(workspace_root);
    let results: Vec<ObligationResult> = artifact
        .obligations
        .iter()
        .filter_map(|o| run_one_scoped(o, workspace_root, &attestations, receipt))
        .collect();
    tracing::debug!(
        duration_ms = run_start.elapsed().as_millis() as u64,
        "obligations.run_all"
    );
    results
}

/// Scope-filtering pass (contract addendum §1): decides, per obligation,
/// whether THIS run (repo mode vs. receipt mode) executes it at all, and if
/// so, with what wiring/subject. Returns `None` to mean "excluded from this
/// run's results entirely" (repo-wired obligations in receipt mode) — the
/// only such case; every other combination produces a real result (either an
/// executed check or the receipt-scoped `Unknown` placeholder).
fn run_one_scoped(
    obligation: &Obligation,
    workspace_root: &Path,
    attestations: &exec::attestation::LoadedAttestations,
    receipt: Option<&ReceiptSubject>,
) -> Option<ObligationResult> {
    let wiring_is_receipt = obligation
        .signal
        .as_ref()
        .map(|signal| signal.wiring == ACTION_RECEIPT_WIRING)
        .unwrap_or(false);

    match (receipt, wiring_is_receipt) {
        // Repo mode, receipt-wired: different choke point, not executed here.
        (None, true) => Some(receipt_scoped_unknown_result(&obligation.id)),
        // Repo mode, repo-wired: unchanged pre-addendum behavior.
        (None, false) => Some(run_one(
            obligation,
            workspace_root,
            attestations,
            false,
            None,
        )),
        // Receipt mode, receipt-wired: the whole point of this run.
        (Some(subject), true) => Some(run_one(
            obligation,
            workspace_root,
            attestations,
            true,
            Some(subject),
        )),
        // Receipt mode, repo-wired: different choke point, excluded entirely.
        (Some(_), false) => None,
    }
}

/// The repo-mode placeholder for a receipt-wired obligation (contract
/// addendum §1): `status = Unknown`, `detail.scope = "action_receipt"` — the
/// literal marker `g8`'s enforcement classifier keys off of to
/// exempt these from gate classification.
fn receipt_scoped_unknown_result(id: &str) -> ObligationResult {
    ObligationResult {
        id: id.to_string(),
        status: ObligationStatus::Unknown,
        trust: None,
        evidence: Evidence {
            backend: "none".to_string(),
            summary: "receipt-wired obligation; run `g8 check --receipt <path>` to evaluate"
                .to_string(),
            detail: serde_json::json!({ "scope": ACTION_RECEIPT_WIRING }),
            duration_ms: 0,
            checks_run: vec![],
        },
    }
}

fn run_one(
    obligation: &Obligation,
    workspace_root: &Path,
    attestations: &exec::attestation::LoadedAttestations,
    wiring_is_receipt: bool,
    receipt: Option<&ReceiptSubject>,
) -> ObligationResult {
    match &obligation.checker {
        None => unknown_result(&obligation.id, "no checker configured for this obligation"),
        Some(ObligationChecker::ProseOnly) => {
            unknown_result(&obligation.id, "checker.mode = prose_only")
        }
        Some(ObligationChecker::Attestation { attestation_id }) => {
            exec::attestation::run(&obligation.id, attestation_id, attestations, workspace_root)
        }
        Some(ObligationChecker::Typed { checks }) => run_typed(
            &obligation.id,
            checks,
            workspace_root,
            attestations,
            wiring_is_receipt,
            receipt,
        ),
    }
}

fn unknown_result(id: &str, reason: &str) -> ObligationResult {
    ObligationResult {
        id: id.to_string(),
        status: ObligationStatus::Unknown,
        trust: None,
        evidence: Evidence {
            backend: "none".to_string(),
            summary: reason.to_string(),
            detail: serde_json::json!({}),
            duration_ms: 0,
            checks_run: vec![],
        },
    }
}

fn run_typed(
    id: &str,
    checks: &[CheckerBackend],
    workspace_root: &Path,
    attestations: &exec::attestation::LoadedAttestations,
    wiring_is_receipt: bool,
    receipt: Option<&ReceiptSubject>,
) -> ObligationResult {
    let start = Instant::now();

    // Subject-binding validation (contract addendum §1): a receipt-wired
    // obligation may only use `receipt_query`; `receipt_query` may not
    // appear in a repo-wired obligation. Checked before dispatching anything
    // — a violation is a claim about the ARTIFACT's own authoring, not a
    // per-backend outcome.
    if let Some(message) = subject_binding_violation(checks, wiring_is_receipt) {
        return ObligationResult {
            id: id.to_string(),
            status: ObligationStatus::Error,
            trust: Some(TrustLevel::Checked),
            evidence: Evidence {
                backend: "none".to_string(),
                summary: message.clone(),
                detail: serde_json::json!({ "error": message }),
                duration_ms: start.elapsed().as_millis() as u64,
                checks_run: vec![],
            },
        };
    }

    let mut checks_run: Vec<SubCheckResult> = checks
        .iter()
        .map(|backend| exec::dispatch(backend, workspace_root, receipt))
        .collect();

    // Attestations supplement typed checks. Missing is intentionally quiet;
    // the CLI's rigor floor will classify a Checked-only gate. A present
    // fresh/stale/error record stays visible as a real sub-check.
    if let exec::attestation::AttestationEvaluation::Evaluated(attestation) =
        exec::attestation::evaluate(attestations, id, None, workspace_root)
    {
        checks_run.push(attestation);
    }

    let (status, trust) = result::aggregate(&checks_run);

    // The overall Evidence mirrors the sub-check that determined the
    // aggregated status (the erroring/failing one if any, else the
    // highest-trust one) — per Evidence's own doc comment (contract §3).
    let representative = checks_run
        .iter()
        .find(|c| c.status == ObligationStatus::Error)
        .or_else(|| {
            checks_run
                .iter()
                .find(|c| c.status == ObligationStatus::Failed)
        })
        .or_else(|| {
            checks_run
                .iter()
                .find(|c| c.status == ObligationStatus::Unknown)
        })
        .or_else(|| {
            checks_run
                .iter()
                .max_by_key(|check| result::backend_tier_rank(&check.backend))
        });

    let (backend, summary, detail) = match representative {
        Some(r) => (
            r.backend.clone(),
            format!("{:?}: {}", r.status, summarize_detail(&r.detail)),
            r.detail.clone(),
        ),
        None => (
            "none".to_string(),
            "no checks configured".to_string(),
            serde_json::json!({}),
        ),
    };

    ObligationResult {
        id: id.to_string(),
        status,
        trust,
        evidence: Evidence {
            backend,
            summary,
            detail,
            duration_ms: start.elapsed().as_millis() as u64,
            checks_run,
        },
    }
}

fn summarize_detail(detail: &serde_json::Value) -> String {
    if let Some(s) = detail.get("summary").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        detail.to_string()
    }
}

/// `Some(message)` iff `checks` violates subject binding for `wiring_is_receipt`
/// (contract addendum §1): a receipt-wired obligation's checks must ALL be
/// `receipt_query`; a repo-wired obligation's checks must contain NONE.
fn subject_binding_violation(checks: &[CheckerBackend], wiring_is_receipt: bool) -> Option<String> {
    checks.iter().find_map(|backend| {
        let is_receipt_backend = backend.name() == "receipt_query";
        if is_receipt_backend == wiring_is_receipt {
            return None;
        }
        Some(if wiring_is_receipt {
            format!(
                "subject-binding violation: obligation is wired to '{ACTION_RECEIPT_WIRING}' but \
                 its checks include non-receipt backend '{}'; a receipt-wired obligation may only \
                 use receipt_query",
                backend.name()
            )
        } else {
            "subject-binding violation: 'receipt_query' backend used in an obligation not wired \
             to 'action_receipt'; receipt_query may only appear in an action_receipt-wired \
             obligation"
                .to_string()
        })
    })
}

#[cfg(test)]
mod receipt_scope_tests {
    use super::*;

    fn receipt_wired_obligation(id: &str, checks: Vec<CheckerBackend>) -> Obligation {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "checker": { "mode": "typed", "checks": checks },
            "signal": { "wiring": "action_receipt", "advisory": false },
        }))
        .expect("must deserialize a hand-built obligation")
    }

    fn repo_wired_obligation(id: &str, checks: Vec<CheckerBackend>) -> Obligation {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "checker": { "mode": "typed", "checks": checks },
            "signal": { "wiring": "compiled_intent_extension", "advisory": false },
        }))
        .expect("must deserialize a hand-built obligation")
    }

    fn receipt_query_backend() -> CheckerBackend {
        CheckerBackend::ReceiptQuery(ReceiptQueryArgs {
            select: "$.actions[*]".to_string(),
            where_clauses: vec![],
            aggregate: ReceiptAggregate::Count,
            expected: ReceiptExpectation::Zero,
        })
    }

    fn repo_backend() -> CheckerBackend {
        CheckerBackend::RgMatchCount(RgArgs {
            pattern: "ZZZ_NEVER_PRESENT_ZZZ".to_string(),
            glob: vec!["Cargo.toml".to_string()],
            expected: CountExpectation::Zero,
        })
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn subject() -> ReceiptSubject {
        ReceiptSubject {
            display_path: "receipts/test.json".to_string(),
            sha256: "sha256:test".to_string(),
            value: serde_json::json!({ "actions": [] }),
        }
    }

    #[test]
    fn repo_mode_receipt_wired_obligation_is_unknown_and_scoped_not_executed() {
        let obligation = receipt_wired_obligation("OBL-RCPT-01", vec![receipt_query_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let result = run_one_scoped(&obligation, &workspace_root(), &attestations, None)
            .expect("repo mode always produces a result for every obligation");
        assert_eq!(result.status, ObligationStatus::Unknown);
        assert_eq!(result.trust, None);
        assert_eq!(
            result.evidence.detail.get("scope").and_then(|v| v.as_str()),
            Some(ACTION_RECEIPT_WIRING)
        );
    }

    #[test]
    fn repo_mode_repo_wired_obligation_runs_unaffected() {
        let obligation = repo_wired_obligation("OBL-REPO-01", vec![repo_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let result = run_one_scoped(&obligation, &workspace_root(), &attestations, None)
            .expect("repo mode always produces a result for every obligation");
        assert_eq!(result.status, ObligationStatus::Passed);
    }

    #[test]
    fn receipt_mode_repo_wired_obligation_is_excluded_entirely() {
        let obligation = repo_wired_obligation("OBL-REPO-01", vec![repo_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let subject = subject();
        let result = run_one_scoped(
            &obligation,
            &workspace_root(),
            &attestations,
            Some(&subject),
        );
        assert!(result.is_none());
    }

    #[test]
    fn receipt_mode_receipt_wired_obligation_runs_against_the_subject() {
        let obligation = receipt_wired_obligation("OBL-RCPT-01", vec![receipt_query_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let subject = subject();
        let result = run_one_scoped(
            &obligation,
            &workspace_root(),
            &attestations,
            Some(&subject),
        )
        .expect("receipt mode runs receipt-wired obligations");
        assert_eq!(result.status, ObligationStatus::Passed); // zero actions, expected zero
    }

    #[test]
    fn receipt_query_in_a_repo_wired_obligation_is_a_binding_error() {
        let obligation = repo_wired_obligation("OBL-BAD-01", vec![receipt_query_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let result = run_one_scoped(&obligation, &workspace_root(), &attestations, None)
            .expect("repo mode always produces a result for every obligation");
        assert_eq!(result.status, ObligationStatus::Error);
        assert_eq!(result.trust, Some(TrustLevel::Checked));
    }

    #[test]
    fn repo_backend_in_a_receipt_wired_obligation_is_a_binding_error() {
        let obligation = receipt_wired_obligation("OBL-BAD-02", vec![repo_backend()]);
        let attestations = exec::attestation::load_for_run(&workspace_root());
        let subject = subject();
        let result = run_one_scoped(
            &obligation,
            &workspace_root(),
            &attestations,
            Some(&subject),
        )
        .expect("receipt mode runs receipt-wired obligations, including a broken one");
        assert_eq!(result.status, ObligationStatus::Error);
    }

    #[test]
    fn load_receipt_subject_hashes_and_parses_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        std::fs::write(&path, r#"{"actions": []}"#).unwrap();
        let subject = load_receipt_subject(&path).expect("must load");
        assert!(subject.sha256.starts_with("sha256:"));
        assert_eq!(subject.value, serde_json::json!({"actions": []}));
    }

    #[test]
    fn load_receipt_subject_missing_file_has_no_sha256() {
        let error = load_receipt_subject(std::path::Path::new("/nonexistent/receipt.json"))
            .expect_err("missing file must error");
        assert_eq!(error.sha256(), None);
    }

    #[test]
    fn load_receipt_subject_malformed_json_still_reports_a_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        std::fs::write(&path, "{ not json").unwrap();
        let error = load_receipt_subject(&path).expect_err("malformed JSON must error");
        assert!(error.sha256().is_some());
    }
}
