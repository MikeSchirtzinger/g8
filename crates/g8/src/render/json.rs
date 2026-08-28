//! JSON output renderer — stable, versioned contract.
//!
//! All JSON payloads include `"g8_version": "0.1.0"` per ARCH §9 Decision 7.
//! Output goes to stdout; all human text goes to stderr.

use serde::Serialize;
use serde_json::Value;

use g8_core::{CheckErrorItem, CheckPayload, Conflict, FitReport, Plan, PlanId};
use g8_obligations::ObligationResult;
use g8_store::PairingError;

pub const G8_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Named reason an enforcement run cannot pass. These classifications are
/// additive to the stable check payload and keep spec-ratification drift
/// distinct from drift in the implementation/evidence itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementClassification {
    UnaccountedDrift,
    InsufficientRigor,
    StaleAttestationPin,
    UnratifiedSpecChange,
    /// A gating `receipt_query` obligation resolved to `Failed`/`Error`
    /// against the checked receipt (contract addendum, o-g8-receipts-
    /// 20260721 §3) — a proposed *action* violating the contract, distinct
    /// from `UnaccountedDrift` (the repo diverging).
    ReceiptViolation,
    /// The `--receipt <path>` file was missing or not valid JSON (contract
    /// addendum §3) — a refusal, never a silent pass and never exit 2.
    InvalidReceipt,
}

impl EnforcementClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnaccountedDrift => "unaccounted_drift",
            Self::InsufficientRigor => "insufficient_rigor",
            Self::StaleAttestationPin => "stale_attestation_pin",
            Self::UnratifiedSpecChange => "unratified_spec_change",
            Self::ReceiptViolation => "receipt_violation",
            Self::InvalidReceipt => "invalid_receipt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnforcementFailure {
    pub classification: EnforcementClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligation_id: Option<String>,
    pub detail: String,
}

/// `g8 check --receipt <path>`'s self-evidencing decision record
/// (contract addendum §3): the checked file, hashed once, alongside the
/// scope marker. `sha256` is `None` only when the file could not even be
/// read (a malformed-JSON receipt still has a real hash — see
/// `g8_obligations::ReceiptSubjectError::sha256`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptInfo {
    pub path: String,
    pub sha256: Option<String>,
}

// ── Generic wrapper ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Versioned<T: Serialize> {
    g8_version: &'static str,
    #[serde(flatten)]
    inner: T,
}

fn versioned<T: Serialize>(inner: T) -> Versioned<T> {
    Versioned {
        g8_version: G8_VERSION,
        inner,
    }
}

// ── FitReport ─────────────────────────────────────────────────────────────────

/// Serialize a `FitReport` to stdout as JSON.
pub fn render_fit_report(report: &FitReport) {
    let v = versioned(report);
    println!(
        "{}",
        serde_json::to_string_pretty(&v).expect("FitReport serializable")
    );
}

/// `plan new` JSON: a `FitReport` payload with `plan_id` added at the top
/// level so downstream scripts can reference the new plan without scraping
/// stderr. The previous shape (versioned FitReport with fields flattened to
/// the top level) is preserved; this only ADDS the `plan_id` key.
pub fn render_plan_new_result(report: &FitReport, plan_id: &PlanId) {
    // Serialize once, then splice in `plan_id` at the top object.
    let mut value =
        serde_json::to_value(report).expect("FitReport serializable into serde_json::Value");
    if let Some(obj) = value.as_object_mut() {
        obj.insert("g8_version".to_string(), serde_json::json!(G8_VERSION));
        obj.insert(
            "plan_id".to_string(),
            serde_json::Value::String(plan_id.as_str().to_string()),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("plan new payload serializable")
    );
}

// ── Plan list ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct PlanListPayload<'a> {
    plans: &'a [Plan],
    count: usize,
}

/// Serialize a list of plans to stdout as JSON.
pub fn render_plan_list(plans: &[Plan]) {
    let payload = versioned(PlanListPayload {
        plans,
        count: plans.len(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("Plan list serializable")
    );
}

/// Serialize a single plan to stdout as JSON.
pub fn render_plan_detail(plan: &Plan) {
    let v = versioned(plan);
    println!(
        "{}",
        serde_json::to_string_pretty(&v).expect("Plan serializable")
    );
}

// ── Check contract (ARCH §14 — locked, stable; extended per
//    specs/obligation-checker-contract.md §5 with the `obligations` self-
//    audit array) ────────────────────────────────────────────────────────────

/// Emit the stable `g8 check --json` contract to stdout (always, even on
/// success).
///
/// `CheckPayload` (the `g8_version`/`errors`/`exit_code`/
/// `enforcement_disabled` shape) lives in `g8_core::contract` — relocated
/// there per obligation-checker-contract.md §8 as part of resolving the
/// `G8CheckContract` obligation backend's recursion hazard. `errors` and
/// `exit_code` keep their pre-existing enforcement-gated behavior exactly:
/// callers pass real pairing-check output only when enforcement is on.
///
/// `obligations`/`obligations_note` are spliced onto the serialized payload
/// as additional top-level keys — the same pattern `render_plan_new_result`
/// already uses for `plan_id` — rather than being fields of `CheckPayload`
/// itself, because `CheckPayload` lives in `g8-core` (the root of the crate
/// DAG) and cannot name `g8_obligations::ObligationResult` without a
/// reverse (cyclic) dependency edge. `obligations` is ALWAYS present, even
/// when empty (e.g. a project with no `specs/obligations-v0.1.json` of its
/// own — every `g8`-adopting project except g8's own repository, today);
/// `obligations_note` is present only to explain a non-empty-for-a-reason
/// empty array.
///
/// Human-readable text must go to stderr; this function writes ONLY to stdout.
///
/// `receipt` is `Some` only for `g8 check --receipt <path>` (contract
/// addendum, o-g8-receipts-20260721 §3); repo-mode callers pass `None`
/// and the JSON shape is byte-for-byte unchanged (the `receipt` key is
/// omitted entirely, same "unchanged shape, unchanged appearance rule" as
/// `enforcement_disabled`/`obligations_note`).
pub fn render_check(
    errors: &[PairingError],
    exit_code: i32,
    enforcement_disabled: bool,
    obligations: &[ObligationResult],
    obligations_note: Option<&str>,
    enforcement_failures: &[EnforcementFailure],
    receipt: Option<&ReceiptInfo>,
) {
    let check_errors: Vec<CheckErrorItem> = errors
        .iter()
        .map(|e| CheckErrorItem {
            capability: e.capability.clone(),
            file: e.file.display().to_string(),
            line: e.line,
            reason: super::to_pairing_reason(e.reason),
        })
        .collect();

    let payload = CheckPayload {
        g8_version: G8_VERSION.to_string(),
        errors: check_errors,
        exit_code,
        enforcement_disabled: enforcement_disabled.then_some(true),
    };

    let mut value = serde_json::to_value(&payload).expect("CheckPayload serializable");
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "obligations".to_string(),
            serde_json::to_value(obligations).expect("obligations serializable"),
        );
        let failures = enforcement_failures
            .iter()
            .map(|failure| {
                let mut entry = serde_json::Map::new();
                entry.insert(
                    "classification".to_string(),
                    Value::String(failure.classification.as_str().to_string()),
                );
                if let Some(obligation_id) = failure.obligation_id.as_ref() {
                    entry.insert(
                        "obligation_id".to_string(),
                        Value::String(obligation_id.clone()),
                    );
                }
                entry.insert("detail".to_string(), Value::String(failure.detail.clone()));
                Value::Object(entry)
            })
            .collect();
        obj.insert("enforcement_failures".to_string(), Value::Array(failures));
        if let Some(note) = obligations_note {
            obj.insert("obligations_note".to_string(), serde_json::json!(note));
        }
        if let Some(receipt) = receipt {
            obj.insert(
                "receipt".to_string(),
                serde_json::json!({
                    "path": receipt.path,
                    "sha256": receipt.sha256,
                    "scope": "action_receipt",
                }),
            );
        }
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("check payload serializable")
    );
}

// ── Conflicts ─────────────────────────────────────────────────────────────────

/// Serialize conflict results.
pub fn render_conflicts(conflicts: &[Conflict]) {
    #[derive(Serialize)]
    struct Payload<'a> {
        g8_version: &'static str,
        conflicts: &'a [Conflict],
        count: usize,
    }
    let payload = Payload {
        g8_version: G8_VERSION,
        conflicts,
        count: conflicts.len(),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}

// ── Scan result ──────────────────────────────────────────────────────────────

/// Emit a structured `g8 scan --output json` result to stdout.
///
/// This makes scan consistent with all other `--output json` paths (cf. SMOKE_REPORT §5).
pub fn render_scan_result(
    inserted_capabilities: u32,
    inserted_intents: u32,
    inserted_decisions: u32,
    files_scanned: u32,
    duration_ms: u64,
) {
    let payload = serde_json::json!({
        "g8_version": G8_VERSION,
        "status": "ok",
        "inserted_capabilities": inserted_capabilities,
        "inserted_intents": inserted_intents,
        "inserted_decisions": inserted_decisions,
        "files_scanned": files_scanned,
        "duration_ms": duration_ms,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}

// ── Generic value output ──────────────────────────────────────────────────────

/// Emit any serializable value with the version wrapper.
pub fn render_value(inner: &Value) {
    let payload = serde_json::json!({
        "g8_version": G8_VERSION,
        "data": inner,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}

/// Emit a simple success/status message as JSON.
pub fn render_ok(message: impl Into<String>) {
    let payload = serde_json::json!({
        "g8_version": G8_VERSION,
        "status": "ok",
        "message": message.into(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}

/// Emit a structured error envelope. `exit_code` is included so CI tooling
/// can correlate the JSON with the process exit. The envelope intentionally
/// uses `"status": "error"`: never `"ok"` — so consumers can branch on it.
pub fn render_error(message: impl Into<String>, exit_code: i32) {
    let payload = serde_json::json!({
        "g8_version": G8_VERSION,
        "status": "error",
        "exit_code": exit_code,
        "error": message.into(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serializable")
    );
}
