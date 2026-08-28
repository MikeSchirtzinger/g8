//! `g8 check` — run the pairing gate + the obligations self-audit; emit the
//! stable JSON contract.
//!
//! Per ARCH §14 and SPEC Decision 10, `g8 check --json` ALWAYS emits valid
//! JSON to stdout (even on success). Human text goes to stderr.
//!
//! Exit codes: 0 = clean or enforcement disabled, 1 = errors, 2 = internal failure.
//!
//! # Obligations self-audit (obligation-checker-contract.md §5, Mike's
//! ruling 2026-07-02)
//!
//! `g8 check` ALWAYS loads and runs the 27-obligation self-audit artifact
//! at `<project_root>/specs/obligations-v0.1.json`, regardless of
//! enforcement — audit checks run even when enforcement is off; findings
//! are always emitted in JSON; enforcement affects the EXIT CODE only. This
//! artifact only exists in g8's own repository today (the 27 obligations
//! are hardcoded constraints on g8's own crates, e.g. "no `surrealdb`
//! dependency"); every other `g8`-adopting project simply has no such
//! file, which is NOT an error — see [`load_and_run_obligations`].

use std::path::Path;

use anyhow::{Context, Result};

use g8_obligations::{
    load_artifact, load_receipt_subject, run_obligations, ObligationResult, ObligationStatus,
    ReceiptSubject, TrustLevel, ACTION_RECEIPT_WIRING, STALE_ATTESTATION_CLASSIFICATION,
};
use g8_store::StoreConnection;

use crate::cli::{CheckArgs, OutputMode};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::json::{
    EnforcementClassification as Classification, EnforcementFailure, ReceiptInfo,
};
use crate::render::{json as json_render, pretty};

use super::ratify::{
    inspect as inspect_ratification, project_root, RatificationStatus,
    OBLIGATIONS_ARTIFACT_RELATIVE,
};

pub fn run(ctx: &Ctx, args: &CheckArgs) -> Result<i32> {
    require_init(ctx)?;

    // `--receipt <path>` is an entirely different choke point (contract
    // addendum, o-g8-receipts-20260721 §1/§3): pre-act, not pre-commit.
    // It reuses ratification + the obligations runner + this same JSON/exit
    // machinery, but skips the repository pairing check and repo-wired
    // obligations outright — routed here, before the store is even opened,
    // so it never pays for what it doesn't need ("speed matters pre-act").
    if let Some(receipt_path) = args.receipt.as_ref() {
        return run_receipt_mode(ctx, args, receipt_path);
    }

    let store = match ctx.open_store() {
        Ok(s) => s,
        Err(e) => {
            // Exit code 2: internal failure — "unrelated to any one
            // obligation" per contract §5.1's own parenthetical, so the
            // obligations self-audit is deliberately skipped here.
            eprintln!("g8 check: internal error: {e}");
            json_render::render_check(&[], 2, false, &[], None, &[], None);
            return Ok(2);
        }
    };

    // ── Obligations self-audit — ALWAYS runs, before the enforcement branch
    // even considers early-returning.
    let project_root = project_root(ctx)?;
    let obligations = load_and_run_obligations(&project_root, None);
    let ratification = inspect_ratification(&project_root);

    // Resolve optional project filter by name.
    let project_filter = if let Some(name) = args.project.as_ref() {
        let space = require_space(&store, ctx.space_id())?;
        let project = store
            .find_project_by_name(&space.id, name)
            .context("find_project_by_name")?
            .with_context(|| format!("project '{name}' not found in current space"))?;
        Some(project.id)
    } else {
        None
    };

    let errors = store
        .pairing_check(project_filter.as_ref())
        .context("running pairing check")?;

    // Audit-before-enforce must still expose the real pairing findings. Only
    // the process exit status is relaxed while enforcement is disabled.
    // `--enforce` is an invocation-local override for CI and clean-checkout
    // probes; it deliberately does not rewrite `.g8/config.toml`.
    let enforcement_on = args.enforce || ctx.enforcement_on();
    let enforcement_failures = classify_enforcement_failures(&errors, &obligations, &ratification);
    if !enforcement_on {
        json_render::render_check(
            &errors,
            0,
            true,
            &obligations.results,
            obligations.note.as_deref(),
            &enforcement_failures,
            None,
        );
        eprintln!(
            "g8 check: enforcement is OFF; findings are advisory. Use `g8 check --enforce` or `g8 init --enforce` to gate."
        );
        pretty::render_check_errors(&errors, ctx.color);
        pretty::render_obligations(&obligations.results, ctx.color);
        pretty::render_enforcement_failures(&enforcement_failures, ctx.color);
        return Ok(0);
    }

    // `--strict` fails on warnings too — for v0.1 pairing errors are always errors.
    let exit_code = if enforcement_failures.is_empty() {
        0
    } else {
        1
    };

    // ARCH §14: JSON contract always emitted on stdout regardless of --output mode,
    // because CI consumers need it reliably. The `--json` flag is kept for backward compat.
    let _use_json = args.json || ctx.output == OutputMode::Json;
    json_render::render_check(
        &errors,
        exit_code,
        false,
        &obligations.results,
        obligations.note.as_deref(),
        &enforcement_failures,
        None,
    );

    // Human text to stderr.
    pretty::render_check_errors(&errors, ctx.color);
    pretty::render_obligations(&obligations.results, ctx.color);
    pretty::render_enforcement_failures(&enforcement_failures, ctx.color);

    Ok(exit_code)
}

fn classify_enforcement_failures(
    pairing_errors: &[g8_store::PairingError],
    obligations: &ObligationsOutcome,
    ratification: &RatificationStatus,
) -> Vec<EnforcementFailure> {
    let mut failures = Vec::new();

    if !pairing_errors.is_empty() {
        failures.push(EnforcementFailure {
            classification: Classification::UnaccountedDrift,
            obligation_id: None,
            detail: format!(
                "{} capability pairing finding(s) are unresolved",
                pairing_errors.len()
            ),
        });
    }

    for result in &obligations.results {
        // Receipt-wired obligations surfaced in REPO mode are a placeholder
        // for a different choke point (contract addendum §1) — exempt from
        // gate classification entirely, not just incidentally non-failing.
        if is_receipt_scoped_placeholder(result) {
            continue;
        }

        if !obligations.is_gate(&result.id) {
            continue;
        }

        if matches!(
            result.status,
            ObligationStatus::Failed | ObligationStatus::Error
        ) {
            failures.push(EnforcementFailure {
                classification: Classification::UnaccountedDrift,
                obligation_id: Some(result.id.clone()),
                detail: result.evidence.summary.clone(),
            });
            continue;
        }

        if has_stale_attestation_pin(result) {
            failures.push(EnforcementFailure {
                classification: Classification::StaleAttestationPin,
                obligation_id: Some(result.id.clone()),
                detail: result.evidence.summary.clone(),
            });
            continue;
        }

        if !meets_rigor_floor(result) {
            failures.push(EnforcementFailure {
                classification: Classification::InsufficientRigor,
                obligation_id: Some(result.id.clone()),
                detail: format!(
                    "gate requires verified or fresh attested evidence; status={:?}, trust={:?}",
                    result.status, result.trust
                ),
            });
        }
    }

    if let RatificationStatus::Unratified { detail } = ratification {
        failures.push(EnforcementFailure {
            classification: Classification::UnratifiedSpecChange,
            obligation_id: None,
            detail: detail.clone(),
        });
    }

    failures
}

fn meets_rigor_floor(result: &ObligationResult) -> bool {
    result.status == ObligationStatus::Passed
        && matches!(
            result.trust,
            Some(TrustLevel::Verified | TrustLevel::Asserted)
        )
}

fn has_stale_attestation_pin(result: &ObligationResult) -> bool {
    result.evidence.checks_run.iter().any(|check| {
        check
            .detail
            .get("classification")
            .and_then(serde_json::Value::as_str)
            == Some(STALE_ATTESTATION_CLASSIFICATION)
    }) || result
        .evidence
        .detail
        .get("classification")
        .and_then(serde_json::Value::as_str)
        == Some(STALE_ATTESTATION_CLASSIFICATION)
}

/// `true` iff `result` is the repo-mode `Unknown` placeholder
/// `g8_obligations::run_obligations` produces for a receipt-wired
/// obligation (contract addendum §1: `detail.scope = "action_receipt"`).
/// These are never gate failures in repo mode — their choke point is
/// `g8 check --receipt`, not this one.
fn is_receipt_scoped_placeholder(result: &ObligationResult) -> bool {
    result
        .evidence
        .detail
        .get("scope")
        .and_then(serde_json::Value::as_str)
        == Some(ACTION_RECEIPT_WIRING)
}

/// Outcome of attempting to load + run the obligations self-audit.
struct ObligationsOutcome {
    results: Vec<ObligationResult>,
    /// Set when the artifact was absent/unusable — NOT an error condition:
    /// most `g8`-adopting projects simply have no
    /// `specs/obligations-v0.1.json` of their own.
    note: Option<String>,
    /// `obligation id -> signal.advisory`. Gate-ness is the inverse of this
    /// same artifact channel; no parallel gate field is introduced.
    advisory: std::collections::BTreeMap<String, bool>,
}

impl ObligationsOutcome {
    fn is_gate(&self, id: &str) -> bool {
        !self.advisory.get(id).copied().unwrap_or(false)
    }
}

/// Loads `specs/obligations-v0.1.json` (if present) and runs it.
///
/// `receipt` selects the choke point exactly as `g8_obligations::run_obligations`
/// documents: `None` runs repo-wired obligations (unchanged pre-addendum
/// behavior — the only caller in repo mode passes `None`); `Some(subject)`
/// runs only `action_receipt`-wired obligations, against `subject`.
fn load_and_run_obligations(
    project_root: &Path,
    receipt: Option<&ReceiptSubject>,
) -> ObligationsOutcome {
    let path = project_root.join(OBLIGATIONS_ARTIFACT_RELATIVE);

    // Absent artifact: the normal case for every g8-adopting project
    // except g8's own repository. The note deliberately names the RELATIVE
    // path only — the artifact's own determinism rule is "all paths
    // repo-relative", and an absolute path here is what smuggled scratch-dir
    // noise into OBL-D11-04's byte-diff (contract §2.6).
    if !path.exists() {
        return ObligationsOutcome {
            results: vec![],
            note: Some(format!(
                "no obligations artifact at {OBLIGATIONS_ARTIFACT_RELATIVE} \
                 (relative to the project root); self-audit skipped"
            )),
            advisory: Default::default(),
        };
    }

    let artifact = match load_artifact(&path) {
        Ok(a) => a,
        Err(e) => {
            return ObligationsOutcome {
                results: vec![],
                note: Some(format!(
                    "obligations artifact not usable at {OBLIGATIONS_ARTIFACT_RELATIVE}: {e}"
                )),
                advisory: Default::default(),
            };
        }
    };

    let advisory = artifact
        .obligations
        .iter()
        .map(|obligation| {
            (
                obligation.id.clone(),
                obligation
                    .signal
                    .as_ref()
                    .map(|signal| signal.advisory)
                    .unwrap_or(false),
            )
        })
        .collect();
    let results = run_obligations(&artifact, project_root, receipt);
    ObligationsOutcome {
        results,
        note: None,
        advisory,
    }
}

// ── Receipt mode — contract addendum, o-g8-receipts-20260721 §1/§3/§4 ──

/// `g8 check --receipt <path>`: the pre-act gate. Reuses `check`'s
/// enforcement/JSON machinery and exit-code semantics (0 clean / 1 gate
/// failures under `--enforce` / 2 internal); differs from repo mode in three
/// ways per the addendum: (1) the capability pairing check is skipped
/// entirely — no store is even opened, "speed matters pre-act"; (2) only
/// `action_receipt`-wired obligations run, against the parsed+hashed
/// receipt; (3) the enforcement floor for those gates is `Passed` + tier ≥
/// `Checked` (see `classify_enforcement_failures_receipt`), not the
/// repo-mode Verified/Asserted floor. Ratification is still enforced either
/// way — an unratified/edited spec fails the receipt gate too, so an agent
/// cannot weaken its own leash by editing its own obligations.
fn run_receipt_mode(ctx: &Ctx, args: &CheckArgs, receipt_path: &Path) -> Result<i32> {
    let project_root = project_root(ctx)?;
    let ratification = inspect_ratification(&project_root);
    let enforcement_on = args.enforce || ctx.enforcement_on();

    // Malformed/missing receipt → `invalid_receipt`, never a silent pass and
    // never exit 2 (addendum §3: "bad input is a refusal, not a crash").
    // Hashing happens even on a parse failure when the file itself was
    // readable, so the decision record still names exactly what was
    // rejected (`ReceiptSubjectError::sha256`).
    let (obligations, receipt_info, invalid_receipt_detail) =
        match load_receipt_subject(receipt_path) {
            Ok(subject) => {
                let outcome = load_and_run_obligations(&project_root, Some(&subject));
                let info = ReceiptInfo {
                    path: subject.display_path.clone(),
                    sha256: Some(subject.sha256.clone()),
                };
                (outcome, info, None)
            }
            Err(error) => {
                let outcome = ObligationsOutcome {
                    results: vec![],
                    note: Some(format!("invalid receipt: {error}")),
                    advisory: Default::default(),
                };
                let info = ReceiptInfo {
                    path: receipt_path.display().to_string(),
                    sha256: error.sha256().map(str::to_string),
                };
                (outcome, info, Some(error.to_string()))
            }
        };

    let mut enforcement_failures =
        classify_enforcement_failures_receipt(&obligations, &ratification);
    if let Some(detail) = invalid_receipt_detail {
        enforcement_failures.insert(
            0,
            EnforcementFailure {
                classification: Classification::InvalidReceipt,
                obligation_id: None,
                detail,
            },
        );
    }

    eprintln!(
        "g8 check --receipt {}: {} receipt obligation(s) evaluated",
        receipt_path.display(),
        obligations.results.len()
    );

    if !enforcement_on {
        json_render::render_check(
            &[],
            0,
            true,
            &obligations.results,
            obligations.note.as_deref(),
            &enforcement_failures,
            Some(&receipt_info),
        );
        eprintln!(
            "g8 check --receipt: enforcement is OFF; findings are advisory. Use --enforce to gate."
        );
        pretty::render_obligations(&obligations.results, ctx.color);
        pretty::render_enforcement_failures(&enforcement_failures, ctx.color);
        return Ok(0);
    }

    let exit_code = if enforcement_failures.is_empty() {
        0
    } else {
        1
    };
    json_render::render_check(
        &[],
        exit_code,
        false,
        &obligations.results,
        obligations.note.as_deref(),
        &enforcement_failures,
        Some(&receipt_info),
    );
    pretty::render_obligations(&obligations.results, ctx.color);
    pretty::render_enforcement_failures(&enforcement_failures, ctx.color);
    Ok(exit_code)
}

/// Receipt-mode enforcement classification (contract addendum §3/§4).
/// Deliberately narrower than repo mode's `classify_enforcement_failures`:
/// no `insufficient_rigor`/`stale_attestation_pin` — the addendum's §4
/// ruling makes `Passed` + tier ≥ `Checked` the WHOLE rigor floor for
/// receipt gates (every tier in this system is ≥ `Checked`, so trust never
/// disqualifies a passed receipt-wired obligation here). Failed/errored
/// gating obligations classify as `receipt_violation` — a proposed act
/// violating the contract, not `unaccounted_drift` (the repo diverging).
/// Ratification is still enforced, same as repo mode.
fn classify_enforcement_failures_receipt(
    obligations: &ObligationsOutcome,
    ratification: &RatificationStatus,
) -> Vec<EnforcementFailure> {
    let mut failures = Vec::new();

    for result in &obligations.results {
        if !obligations.is_gate(&result.id) {
            continue;
        }
        if matches!(
            result.status,
            ObligationStatus::Failed | ObligationStatus::Error
        ) {
            failures.push(EnforcementFailure {
                classification: Classification::ReceiptViolation,
                obligation_id: Some(result.id.clone()),
                detail: result.evidence.summary.clone(),
            });
        }
    }

    if let RatificationStatus::Unratified { detail } = ratification {
        failures.push(EnforcementFailure {
            classification: Classification::UnratifiedSpecChange,
            obligation_id: None,
            detail: detail.clone(),
        });
    }

    failures
}
