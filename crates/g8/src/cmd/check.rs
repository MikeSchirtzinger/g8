//! `g8 check` — run the pairing gate + the obligations self-audit; emit the
//! stable JSON contract.
//!
//! Per ARCH §14 and SPEC Decision 10, `g8 check --json` ALWAYS emits valid
//! JSON to stdout (even on success). Human text goes to stderr.
//!
//! Exit codes: 0 = clean or enforcement disabled, 1 = errors, 2 = internal failure.
//!
//! # Obligations self-audit (obligation-checker-contract.md §5, ruling
//! 2026-07-02)
//!
//! `g8 check` loads the project's obligations artifact and runs the selected
//! checks even when enforcement is off. Findings are always emitted in JSON.

use std::path::Path;

use anyhow::{Context, Result};

use g8_obligations::{
    load_artifact, load_receipt_subject, run_obligations, ObligationArtifact, ObligationResult,
    ObligationStatus, ReceiptSubject, TrustLevel, ACTION_RECEIPT_WIRING,
    STALE_ATTESTATION_CLASSIFICATION,
};
use g8_store::StoreConnection;

use crate::cli::{CheckArgs, OutputMode};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::json::{
    CheckObligations, EnforcementClassification as Classification, EnforcementFailure,
    ObligationSelection, ReceiptInfo,
};
use crate::render::{json as json_render, pretty};

use super::ratify::{
    inspect as inspect_ratification, project_root, RatificationStatus,
    OBLIGATIONS_ARTIFACT_RELATIVE,
};

pub fn run(ctx: &Ctx, args: &CheckArgs) -> Result<i32> {
    require_init(ctx)?;
    let project_root = project_root(ctx)?;
    let ratification = inspect_ratification(&project_root);
    let enforcement_on = args.enforce || ctx.enforcement_on();
    let prepared = match prepare_obligations(&project_root, args) {
        Ok(prepared) => prepared,
        Err(error) => {
            let selection = ObligationSelection {
                patterns: &args.only,
                selected: 0,
                not_selected: error.eligible,
            };
            eprintln!("g8 check: {}", error.detail);
            print_selection(selection, enforcement_on);
            let obligations = ObligationsOutcome::with_note(error.detail);
            let failures = classify_enforcement_failures(&[], &obligations, &ratification);
            pretty::render_enforcement_failures(&failures, ctx.color);
            json_render::render_check(
                &[],
                2,
                !enforcement_on,
                obligations.report(selection),
                &failures,
                None,
            );
            return Ok(2);
        }
    };
    let selection = prepared.selection;
    print_selection(selection, enforcement_on);

    // `--receipt <path>` is an entirely different choke point (contract
    // addendum, o-g8-receipts-20260721 §1/§3): pre-act, not pre-commit.
    // It reuses ratification + the obligations runner + this same JSON/exit
    // machinery, but skips the repository pairing check and repo-wired
    // obligations outright — routed here, before the store is even opened,
    // so it never pays for what it doesn't need ("speed matters pre-act").
    if let Some(receipt_path) = args.receipt.as_ref() {
        return run_receipt_mode(ctx, receipt_path, prepared, &ratification, enforcement_on);
    }

    let store = match ctx.open_store() {
        Ok(s) => s,
        Err(e) => {
            // Exit code 2: internal failure — "unrelated to any one
            // obligation" per contract §5.1's own parenthetical, so the
            // obligations self-audit is deliberately skipped here.
            // `{e:#}` prints the whole context chain. The bare `{e}` this
            // used to print showed only the outermost label ("running store
            // migrations") and hid the cause a user needs to act on.
            let detail = format!("internal error: {e:#}");
            eprintln!("g8 check: {detail}");
            let obligations = ObligationsOutcome::with_note(detail);
            json_render::render_check(&[], 2, false, obligations.report(selection), &[], None);
            return Ok(2);
        }
    };

    // ── Obligations self-audit — ALWAYS runs, before the enforcement branch
    // even considers early-returning.
    let obligations = prepared.run(&project_root, None);

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
    let enforcement_failures = classify_enforcement_failures(&errors, &obligations, &ratification);
    if !enforcement_on {
        json_render::render_check(
            &errors,
            0,
            true,
            obligations.report(selection),
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
        obligations.report(selection),
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
    fn with_note(note: String) -> Self {
        Self {
            results: vec![],
            note: Some(note),
            advisory: Default::default(),
        }
    }

    fn report<'a>(&'a self, selection: ObligationSelection<'a>) -> CheckObligations<'a> {
        CheckObligations {
            results: &self.results,
            note: self.note.as_deref(),
            selection,
        }
    }

    fn is_gate(&self, id: &str) -> bool {
        !self.advisory.get(id).copied().unwrap_or(false)
    }
}

struct PreparedObligations<'a> {
    artifact: Result<ObligationArtifact, String>,
    selection: ObligationSelection<'a>,
}

impl PreparedObligations<'_> {
    fn run(self, project_root: &Path, receipt: Option<&ReceiptSubject>) -> ObligationsOutcome {
        let artifact = match self.artifact {
            Ok(artifact) => artifact,
            Err(note) => return ObligationsOutcome::with_note(note),
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
                        .is_some_and(|signal| signal.advisory),
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
}

struct SelectionError {
    detail: String,
    eligible: usize,
}

fn prepare_obligations<'a>(
    project_root: &Path,
    args: &'a CheckArgs,
) -> Result<PreparedObligations<'a>, SelectionError> {
    let path = project_root.join(OBLIGATIONS_ARTIFACT_RELATIVE);
    let mut selection = ObligationSelection {
        patterns: &args.only,
        selected: 0,
        not_selected: 0,
    };
    let artifact = if path.exists() {
        load_artifact(&path).map_err(|error| {
            format!("obligations artifact not usable at {OBLIGATIONS_ARTIFACT_RELATIVE}: {error}")
        })
    } else {
        Err(format!(
            "no obligations artifact at {OBLIGATIONS_ARTIFACT_RELATIVE} (relative to the project root); self-audit skipped"
        ))
    };
    let mut artifact = match artifact {
        Ok(artifact) => artifact,
        Err(note) => {
            if let Some(pattern) = args.only.first() {
                return Err(SelectionError {
                    detail: format!("cannot match --only pattern {pattern:?}: {note}"),
                    eligible: 0,
                });
            }
            return Ok(PreparedObligations {
                artifact: Err(note),
                selection,
            });
        }
    };

    if args.receipt.is_some() {
        artifact.obligations.retain(|obligation| {
            obligation
                .signal
                .as_ref()
                .is_some_and(|signal| signal.wiring == ACTION_RECEIPT_WIRING)
        });
    }

    let eligible = artifact.obligations.len();
    if !args.only.is_empty() {
        let mut selected = vec![false; eligible];
        for pattern in &args.only {
            let matcher = match globset::GlobBuilder::new(pattern)
                .backslash_escape(true)
                .build()
            {
                Ok(glob) => Some(glob.compile_matcher()),
                Err(_)
                    if artifact
                        .obligations
                        .iter()
                        .any(|obligation| &obligation.id == pattern) =>
                {
                    None
                }
                Err(error) => {
                    return Err(SelectionError {
                        detail: format!("invalid --only pattern {pattern:?}: {error}"),
                        eligible,
                    })
                }
            };
            let mut matched = false;
            for (obligation, selected) in artifact.obligations.iter().zip(&mut selected) {
                if pattern == &obligation.id
                    || matcher.as_ref().is_some_and(|m| m.is_match(&obligation.id))
                {
                    *selected = true;
                    matched = true;
                }
            }
            if !matched {
                return Err(SelectionError {
                    detail: format!("--only pattern {pattern:?} matched no obligations"),
                    eligible,
                });
            }
        }
        artifact.obligations = artifact
            .obligations
            .into_iter()
            .zip(selected)
            .filter_map(|(obligation, selected)| selected.then_some(obligation))
            .collect();
    }

    selection.selected = artifact.obligations.len();
    selection.not_selected = eligible - selection.selected;
    Ok(PreparedObligations {
        artifact: Ok(artifact),
        selection,
    })
}

fn print_selection(selection: ObligationSelection<'_>, enforcement_on: bool) {
    eprintln!(
        "g8 check: {} obligation(s) selected, {} not selected",
        selection.selected, selection.not_selected
    );
    if enforcement_on && selection.not_selected > 0 {
        eprintln!("g8 check: partial run (obligations excluded by --only)");
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
fn run_receipt_mode(
    ctx: &Ctx,
    receipt_path: &Path,
    prepared: PreparedObligations<'_>,
    ratification: &RatificationStatus,
    enforcement_on: bool,
) -> Result<i32> {
    let selection = prepared.selection;

    // Malformed/missing receipt → `invalid_receipt`, never a silent pass and
    // never exit 2 (addendum §3: "bad input is a refusal, not a crash").
    // Hashing happens even on a parse failure when the file itself was
    // readable, so the decision record still names exactly what was
    // rejected (`ReceiptSubjectError::sha256`).
    let (obligations, receipt_info, invalid_receipt_detail) =
        match load_receipt_subject(receipt_path) {
            Ok(subject) => {
                let outcome = prepared.run(&ctx.project_root, Some(&subject));
                let info = ReceiptInfo {
                    path: subject.display_path.clone(),
                    sha256: Some(subject.sha256.clone()),
                };
                (outcome, info, None)
            }
            Err(error) => {
                let outcome = ObligationsOutcome::with_note(format!("invalid receipt: {error}"));
                let info = ReceiptInfo {
                    path: receipt_path.display().to_string(),
                    sha256: error.sha256().map(str::to_string),
                };
                (outcome, info, Some(error.to_string()))
            }
        };

    let mut enforcement_failures =
        classify_enforcement_failures_receipt(&obligations, ratification);
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
            obligations.report(selection),
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
        obligations.report(selection),
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
