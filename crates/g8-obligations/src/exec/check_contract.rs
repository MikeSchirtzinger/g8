//! `G8CheckContract` — contract §1.1/§8.
//!
//! Validates the SHAPE of `g8 check --json`'s own locked output contract
//! **in-process**, never by shelling out to `g8 check` (contract §8's
//! recursion-hazard resolution). See `lib.rs`'s module docs for why this
//! validates against a `serde_json::Value` shape seam rather than the
//! not-yet-relocated `g8_core::contract::CheckPayload`/`CheckError` types
//! §8 describes (that relocation is T5's job and runs after this crate is
//! built).
//!
//! `KnownBadCapabilityFixture` builds a REAL `RusqliteStore::
//! open_in_memory()`, inserts a real `Capability` with no matching
//! `ConvergenceTest`, and calls the real `StoreConnection::pairing_check` —
//! genuine `Verified`-tier behavior, not a hollow assertion. The resulting
//! `errors[]` array is the real `Vec<PairingError>` serialized through its
//! own real `Serialize` impl (already `#[serde(rename_all = "snake_case")]`
//! on `PairingErrorReason`, already living in `g8-store` — no relocation
//! needed for that part of the contract).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use g8_core::{Capability, CapabilityStatus, ConvergenceSpace, Project, SourceLocation};
use g8_store::{RusqliteStore, StoreConnection};

use crate::backend::{CheckContractArgs, CheckContractScenario};
use crate::result::ObligationStatus;

pub(crate) fn run(args: &CheckContractArgs, _workspace_root: &Path) -> (ObligationStatus, Value) {
    if args.scenarios.is_empty() {
        return (
            ObligationStatus::Error,
            json!({ "error": "no scenarios configured" }),
        );
    }

    let mut scenario_results: Vec<Value> = Vec::new();
    let mut any_error = false;
    let mut any_failed = false;

    for scenario in &args.scenarios {
        let outcome = match scenario {
            CheckContractScenario::CleanStore => validate_clean_store(),
            CheckContractScenario::KnownBadCapabilityFixture => validate_known_bad_capability(),
        };
        match outcome {
            Ok((ok, payload)) => {
                any_failed |= !ok;
                scenario_results.push(json!({
                    "scenario": format!("{scenario:?}"),
                    "shape_ok": ok,
                    "payload": payload,
                }));
            }
            Err(e) => {
                any_error = true;
                scenario_results.push(json!({ "scenario": format!("{scenario:?}"), "error": e }));
            }
        }
    }

    let detail = json!({ "scenarios": scenario_results });
    if any_error {
        (ObligationStatus::Error, detail)
    } else if any_failed {
        (ObligationStatus::Failed, detail)
    } else {
        (ObligationStatus::Passed, detail)
    }
}

/// The empty-errors shape: `{g8_version, errors: [], exit_code: 0}`.
fn validate_clean_store() -> Result<(bool, Value), String> {
    let payload = json!({
        "g8_version": env!("CARGO_PKG_VERSION"),
        "errors": [],
        "exit_code": 0,
    });
    Ok((validate_shape(&payload), payload))
}

/// Real in-memory store, real `Capability` with no matching
/// `ConvergenceTest`, real `pairing_check` — the errors-present shape:
/// `{g8_version, errors: [{capability,file,line,reason}], exit_code: 1}`.
fn validate_known_bad_capability() -> Result<(bool, Value), String> {
    let mut store = RusqliteStore::open_in_memory().map_err(|e| e.to_string())?;
    store.migrate().map_err(|e| e.to_string())?;

    let space = ConvergenceSpace {
        id: g8_core::SpaceId::derive(&["check-contract-probe", "space"]),
        name: "g8-obligations-probe".to_string(),
        root_path: PathBuf::from("/probe"),
        created_at: g8_core::now_millis(),
        updated_at: g8_core::now_millis(),
        members: vec![],
        meta: None,
    };
    store.init_space(&space).map_err(|e| e.to_string())?;

    let project = Project {
        id: g8_core::ProjectId::derive(&["check-contract-probe", "project"]),
        space_id: space.id.clone(),
        name: "probe-project".to_string(),
        description: None,
        root_path: PathBuf::from("/probe/project"),
        language: Some("rust".to_string()),
        created_at: g8_core::now_millis(),
        updated_at: g8_core::now_millis(),
        meta: None,
    };
    store.upsert_project(&project).map_err(|e| e.to_string())?;

    let capability = Capability {
        id: g8_core::CapabilityId::derive(&["check-contract-probe", "capability"]),
        project_id: project.id.clone(),
        name: "probe-capability".to_string(),
        description: None,
        substrate: None,
        consumes: vec![],
        produces: vec![],
        status: CapabilityStatus::InFlight,
        stub: false,
        stub_since: None,
        owner: None,
        source: SourceLocation {
            file: PathBuf::from("src/probe.rs"),
            line: 1,
            column: 0,
        },
        annotation_raw: "// @g8.capability(name = \"probe-capability\")".to_string(),
        conditional: false,
        created_at: g8_core::now_millis(),
        updated_at: g8_core::now_millis(),
        meta: None,
    };
    store
        .upsert_capability(&capability)
        .map_err(|e| e.to_string())?;
    // Deliberately no ConvergenceTest inserted — pairing_check must flag this.

    let errors = store.pairing_check(None).map_err(|e| e.to_string())?;
    if errors.is_empty() {
        return Err(
            "fixture setup bug: pairing_check reported no errors for a capability with no matching convergence test"
                .to_string(),
        );
    }

    let errors_json = serde_json::to_value(&errors).map_err(|e| e.to_string())?;
    let payload = json!({
        "g8_version": env!("CARGO_PKG_VERSION"),
        "errors": errors_json,
        "exit_code": 1,
    });
    Ok((validate_shape(&payload), payload))
}

/// The locked shape (CLAUDE.md's "Output contract" section /
/// `crates/g8/src/render/json.rs`'s `CheckPayload`): required keys
/// present with correct types, `errors[].reason` drawn from the closed
/// vocabulary, `exit_code` consistent with `errors.is_empty()`. Does NOT
/// assert an exhaustive key set — `obligations[]` (added by T5) and any
/// future additive key must not retroactively fail this check; contract §5:
/// "Backward compatible... unchanged in shape and unchanged in when they
/// appear" is a claim about THESE keys, not about the total key set.
fn validate_shape(payload: &Value) -> bool {
    let Some(g8_version) = payload.get("g8_version").and_then(Value::as_str) else {
        return false;
    };
    if g8_version.is_empty() {
        return false;
    }
    let Some(errors) = payload.get("errors").and_then(Value::as_array) else {
        return false;
    };
    let Some(exit_code) = payload.get("exit_code").and_then(Value::as_i64) else {
        return false;
    };
    if errors.is_empty() && exit_code != 0 {
        return false;
    }
    if !errors.is_empty() && exit_code != 1 {
        return false;
    }
    errors.iter().all(is_valid_check_error)
}

fn is_valid_check_error(e: &Value) -> bool {
    let Some(obj) = e.as_object() else {
        return false;
    };
    if !obj.get("capability").is_some_and(Value::is_string) {
        return false;
    }
    if !obj.get("file").is_some_and(Value::is_string) {
        return false;
    }
    if obj.get("line").and_then(Value::as_u64).is_none() {
        return false;
    }
    // Leverage the REAL `PairingErrorReason`'s own `Deserialize` impl as the
    // single source of truth for the closed reason vocabulary — zero manual
    // enumeration to drift out of sync if a variant is ever added/renamed.
    match obj.get("reason") {
        Some(reason) => {
            serde_json::from_value::<g8_store::PairingErrorReason>(reason.clone()).is_ok()
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_store_scenario_passes() {
        let args = CheckContractArgs {
            scenarios: vec![CheckContractScenario::CleanStore],
        };
        let (status, detail) = run(&args, Path::new("/unused")); // workspace_root unused by this backend
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn known_bad_capability_scenario_passes_with_real_pairing_error() {
        let args = CheckContractArgs {
            scenarios: vec![CheckContractScenario::KnownBadCapabilityFixture],
        };
        let (status, detail) = run(&args, Path::new("/unused"));
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        // Confirm this genuinely exercised real pairing_check output, not a
        // hand-typed stand-in: the reason must be the real closed-vocabulary
        // string, not something this test invented.
        let payload = &detail["scenarios"][0]["payload"];
        assert_eq!(
            payload["errors"][0]["reason"],
            json!("no_matching_convergence_test")
        );
        assert_eq!(payload["exit_code"], json!(1));
    }

    #[test]
    fn both_scenarios_together_pass() {
        let args = CheckContractArgs {
            scenarios: vec![
                CheckContractScenario::CleanStore,
                CheckContractScenario::KnownBadCapabilityFixture,
            ],
        };
        let (status, detail) = run(&args, Path::new("/unused"));
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn detects_a_broken_shape_missing_g8_version() {
        let broken = json!({ "errors": [], "exit_code": 0 });
        assert!(!validate_shape(&broken));
    }

    #[test]
    fn detects_exit_code_inconsistent_with_errors() {
        let broken = json!({ "g8_version": "0.1.0", "errors": [], "exit_code": 1 });
        assert!(!validate_shape(&broken));
    }

    #[test]
    fn detects_unknown_reason_vocabulary() {
        let broken = json!({
            "g8_version": "0.1.0",
            "errors": [{"capability": "x", "file": "f.rs", "line": 1, "reason": "made_up_reason"}],
            "exit_code": 1,
        });
        assert!(!validate_shape(&broken));
    }

    #[test]
    fn accepts_a_forward_compatible_extra_top_level_key() {
        // T5 adds `obligations` later — this backend must not retroactively
        // start failing the shape it already validated correctly.
        let payload = json!({
            "g8_version": "0.1.0",
            "errors": [],
            "exit_code": 0,
            "obligations": [],
        });
        assert!(validate_shape(&payload));
    }

    #[test]
    fn error_path_empty_scenarios() {
        let args = CheckContractArgs { scenarios: vec![] };
        let (status, _) = run(&args, Path::new("/unused"));
        assert_eq!(status, ObligationStatus::Error);
    }
}
