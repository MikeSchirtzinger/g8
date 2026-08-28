//! Integration tests for `g8 check --receipt <path>` — the pre-act gate
//! for standing agents (contract addendum, o-g8-receipts-20260721).
//!
//! Mirrors `obligations_check.rs`'s conventions (`Command::cargo_bin`,
//! inline artifact JSON consts, a `TempDir` with `.g8` init +
//! `specs/obligations-v0.1.json`, assertions on stdout JSON + exit code).
//!
//! # Coverage (acceptance criteria, o-g8-receipts-20260721)
//!
//! 1. Clean receipt + `--enforce` → exit 0; JSON has `receipt.sha256`; every
//!    receipt obligation `Passed`.
//! 2. Violating receipt + `--enforce` → exit 1; each violated gate named
//!    `receipt_violation`; the advisory violation is visible but non-blocking.
//! 3. Repo mode with receipt-wired obligations present: `Unknown`/scoped,
//!    exit code unaffected, existing repo gate behavior unchanged.
//! 4. Receipt mode: repo-wired obligations not executed (excluded from the
//!    `obligations` array entirely); pairing skipped (`errors` always `[]`);
//!    an edited-without-re-ratify spec still fails `unratified_spec_change`.
//! 5. Malformed/absent receipt → `invalid_receipt`, exit 1 under `--enforce`
//!    (0 without), never exit 2.
//! 6. Subject-binding violations (either direction) → `Error`, gates.
//! 7. Op variety flows correctly end-to-end through the real CLI (the
//!    exhaustive per-op/missing-path/expectation-kind matrix lives in
//!    `g8-obligations`'s own inline unit tests — this only proves the
//!    wiring, not re-derives that matrix).
//! 9. Receipt-mode rigor floor (`Passed` + tier >= `Checked`, no
//!    Verified/Asserted requirement) — proven by the clean-run test still
//!    exiting 0 with zero `insufficient_rigor` despite every receipt
//!    obligation being `Checked`-tier only.
//!
//! AC8 (fmt/clippy/test-green/g8's-own-gate) and AC10 (the example
//! walkthrough) are validated by running the documented commands directly,
//! not by a `#[test]` here.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

fn g8() -> Command {
    Command::cargo_bin("g8").expect("g8 binary not found")
}

/// `g8 init` (optionally `--enforce`) in a fresh tempdir; returns the
/// `TempDir` (must stay alive) and its path.
fn init_temp_project(enforce: bool) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().to_path_buf();

    let mut args = vec!["init", "--no-claude-import", "--no-subagents"];
    if enforce {
        args.push("--enforce");
    }
    g8().current_dir(&path).args(args).assert().success();

    (dir, path)
}

fn write_obligations_artifact(root: &Path, artifact_json: &str) {
    std::fs::write(root.join("marker.txt"), "harmless\n").unwrap();
    let specs_dir = root.join("specs");
    std::fs::create_dir_all(&specs_dir).unwrap();
    std::fs::write(specs_dir.join("obligations-v0.1.json"), artifact_json).unwrap();
}

fn write_receipt(root: &Path, name: &str, receipt_json: &str) -> PathBuf {
    let receipts_dir = root.join("receipts");
    std::fs::create_dir_all(&receipts_dir).unwrap();
    let path = receipts_dir.join(name);
    std::fs::write(&path, receipt_json).unwrap();
    path
}

fn ratify(root: &Path) {
    g8().current_dir(root).arg("ratify").assert().success();
}

fn classifications(json: &serde_json::Value) -> Vec<&str> {
    json.get("enforcement_failures")
        .and_then(|value| value.as_array())
        .expect("enforcement_failures must be an array")
        .iter()
        .map(|failure| {
            failure
                .get("classification")
                .and_then(|value| value.as_str())
                .expect("classification must be a string")
        })
        .collect()
}

fn obligations_by_id(
    json: &serde_json::Value,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    json.get("obligations")
        .and_then(|v| v.as_array())
        .expect("obligations must be an array")
        .iter()
        .map(|o| {
            (
                o.get("id")
                    .and_then(|v| v.as_str())
                    .expect("obligation entry must have a string id")
                    .to_string(),
                o.clone(),
            )
        })
        .collect()
}

fn enforcement_failure_for<'a>(
    json: &'a serde_json::Value,
    obligation_id: &str,
) -> Option<&'a serde_json::Value> {
    json.get("enforcement_failures")
        .and_then(|v| v.as_array())
        .and_then(|failures| {
            failures
                .iter()
                .find(|f| f.get("obligation_id").and_then(|v| v.as_str()) == Some(obligation_id))
        })
}

/// One repo-wired obligation (`OBL-REPO-01`, a clean `rg_match_count`), plus
/// three `action_receipt`-wired `receipt_query` obligations: a provenance
/// gate (`absent` op), a spend cap (`sum`/`at_most`), and an advisory
/// escalation-marking gate (`ne` op).
const FIXTURE_ARTIFACT_MIXED: &str = r#"{
  "meta": { "artifact": "g8-test-receipt-fixture", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-REPO-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "THIS_STRING_MUST_NEVER_APPEAR_XYZZY",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": false }
    },
    {
      "id": "OBL-RCPT-PROVENANCE-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "receipt_query",
            "args": {
              "select": "$.actions[*]",
              "where": [
                { "path": "$.kind", "op": "eq", "value": "publish" },
                { "path": "$.source_ref", "op": "absent" }
              ],
              "aggregate": { "kind": "count" },
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "wiring": "action_receipt", "advisory": false }
    },
    {
      "id": "OBL-RCPT-SPENDCAP-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "receipt_query",
            "args": {
              "select": "$.actions[*]",
              "where": [{ "path": "$.kind", "op": "eq", "value": "spend" }],
              "aggregate": { "kind": "sum", "path": "$.amount" },
              "expected": { "kind": "at_most", "value": 100.0 }
            }
          }
        ]
      },
      "signal": { "wiring": "action_receipt", "advisory": false }
    },
    {
      "id": "OBL-RCPT-ESCALATE-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "receipt_query",
            "args": {
              "select": "$.actions[*]",
              "where": [
                { "path": "$.kind", "op": "eq", "value": "post" },
                { "path": "$.escalate", "op": "ne", "value": true }
              ],
              "aggregate": { "kind": "count" },
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "wiring": "action_receipt", "advisory": true }
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

const CLEAN_RECEIPT: &str = r#"{
  "meta": { "run_id": "run-clean-1", "agent": "test-agent", "git_head": "deadbeef" },
  "actions": [
    { "kind": "publish", "target": { "domain": "example.com" }, "source_ref": "docs/a.md" },
    { "kind": "spend", "amount": 40 },
    { "kind": "spend", "amount": 30 },
    { "kind": "post", "escalate": true }
  ]
}"#;

/// Violates both non-advisory gates (missing `source_ref`; spend 140 > 100)
/// and the advisory one (a post without `escalate: true`).
const VIOLATING_RECEIPT: &str = r#"{
  "meta": { "run_id": "run-violating-1", "agent": "test-agent", "git_head": "deadbeef" },
  "actions": [
    { "kind": "publish", "target": { "domain": "evil.example" } },
    { "kind": "spend", "amount": 90 },
    { "kind": "spend", "amount": 50 },
    { "kind": "post" }
  ]
}"#;

const FIXTURE_ARTIFACT_BAD_REPO_WITH_RECEIPT_QUERY: &str = r#"{
  "meta": { "artifact": "g8-test-binding-violation-a", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-BAD-REPO-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "receipt_query",
            "args": {
              "select": "$.actions[*]",
              "where": [],
              "aggregate": { "kind": "count" },
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": false }
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

const FIXTURE_ARTIFACT_BAD_RECEIPT_WITH_REPO_BACKEND: &str = r#"{
  "meta": { "artifact": "g8-test-binding-violation-b", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-BAD-RECEIPT-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "anything",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "wiring": "action_receipt", "advisory": false }
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

// ── AC1: clean receipt + --enforce → exit 0; receipt.sha256 present; all
//    receipt obligations Passed. Also proves AC9 (Checked-only tier still
//    clears the receipt-mode floor: zero insufficient_rigor). ────────────

#[test]
fn ac1_ac9_clean_receipt_enforced_passes_with_receipt_hash_and_no_rigor_failures() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);
    let receipt_path = write_receipt(&path, "clean-run.json", CLEAN_RECEIPT);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("valid JSON on stdout");

    assert_eq!(json["exit_code"], 0);
    assert_eq!(
        json["errors"],
        serde_json::json!([]),
        "pairing check must be skipped in receipt mode"
    );

    let receipt = &json["receipt"];
    assert_eq!(receipt["scope"], "action_receipt");
    assert_eq!(receipt["path"], receipt_path.display().to_string());
    let sha256 = receipt["sha256"]
        .as_str()
        .expect("receipt.sha256 must be a string");
    assert!(sha256.starts_with("sha256:"), "sha256: {sha256}");

    let by_id = obligations_by_id(&json);
    for id in [
        "OBL-RCPT-PROVENANCE-01",
        "OBL-RCPT-SPENDCAP-01",
        "OBL-RCPT-ESCALATE-01",
    ] {
        let o = by_id
            .get(id)
            .unwrap_or_else(|| panic!("{id} missing from obligations"));
        assert_eq!(o["status"], "passed", "{id}: {o}");
        assert_eq!(o["trust"], "checked", "{id}: {o}");
    }

    assert_eq!(
        classifications(&json),
        Vec::<&str>::new(),
        "AC9: Checked-tier-only Passed obligations must clear the receipt-mode floor with zero enforcement failures"
    );
}

// ── AC2: violating receipt → exit 1; both non-advisory gates classify as
//    receipt_violation; the advisory gate is visible (Failed) but does not
//    appear among enforcement_failures. ──────────────────────────────────

#[test]
fn ac2_violating_receipt_enforced_fails_with_receipt_violation_and_advisory_stays_non_blocking() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);
    let receipt_path = write_receipt(&path, "violating-run.json", VIOLATING_RECEIPT);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("valid JSON on stdout");
    assert_eq!(json["exit_code"], 1);

    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-RCPT-PROVENANCE-01"]["status"], "failed");
    assert_eq!(by_id["OBL-RCPT-SPENDCAP-01"]["status"], "failed");
    assert_eq!(
        by_id["OBL-RCPT-ESCALATE-01"]["status"], "failed",
        "advisory obligations still surface their real status"
    );

    let provenance_failure = enforcement_failure_for(&json, "OBL-RCPT-PROVENANCE-01")
        .expect("provenance gate must produce an enforcement failure");
    assert_eq!(provenance_failure["classification"], "receipt_violation");
    let spendcap_failure = enforcement_failure_for(&json, "OBL-RCPT-SPENDCAP-01")
        .expect("spend cap gate must produce an enforcement failure");
    assert_eq!(spendcap_failure["classification"], "receipt_violation");

    assert!(
        enforcement_failure_for(&json, "OBL-RCPT-ESCALATE-01").is_none(),
        "advisory violation must be visible but non-blocking: no enforcement_failures entry: {json}"
    );

    // The repo-wired obligation must not even appear — receipt mode only
    // runs action_receipt-wired obligations (AC4).
    assert!(!by_id.contains_key("OBL-REPO-01"));
}

// ── AC3: repo mode with receipt-wired obligations present — Unknown/scoped,
//    exit code unaffected, existing repo gate behavior unchanged. ────────

#[test]
fn ac3_repo_mode_reports_receipt_wired_obligations_as_unknown_and_scoped() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);

    // Enforcement OFF: audit still runs, exit 0.
    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");

    let by_id = obligations_by_id(&json);
    for id in [
        "OBL-RCPT-PROVENANCE-01",
        "OBL-RCPT-SPENDCAP-01",
        "OBL-RCPT-ESCALATE-01",
    ] {
        let o = &by_id[id];
        assert_eq!(o["status"], "unknown", "{id}: {o}");
        assert_eq!(o["trust"], serde_json::Value::Null, "{id}: {o}");
        assert_eq!(
            o["evidence"]["detail"]["scope"], "action_receipt",
            "{id}: {o}"
        );
    }
    // The repo-wired obligation is completely unaffected by the presence of
    // receipt-wired obligations alongside it.
    assert_eq!(by_id["OBL-REPO-01"]["status"], "passed");
    assert!(
        json.get("receipt").is_none(),
        "repo mode must never emit a receipt block"
    );

    // OBL-REPO-01 is a plain `rg_match_count` (Checked tier) — repo mode's
    // OWN pre-existing rigor floor (untouched by this addendum) requires
    // Verified/Asserted for a gate to pass under --enforce, so attest it
    // first, exactly like a real adopting project would. This isolates
    // the assertion below to what AC3 actually claims: receipt-wired
    // obligations never perturb repo-mode's gate outcome.
    g8().current_dir(&path)
        .args(["attest", "OBL-REPO-01", "--files", "marker.txt"])
        .assert()
        .success();

    // Enforcement ON: existing repo gate behavior (a clean, attested
    // OBL-REPO-01, no pairing errors) must still exit 0 — receipt-wired
    // obligations never perturb repo-mode's gate outcome, and never appear
    // in enforcement_failures.
    g8().current_dir(&path)
        .args(["init", "--enforce", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();
    let enforced = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        enforced.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&enforced.stderr)
    );
    let enforced_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&enforced.stdout)).expect("valid JSON");
    assert_eq!(
        classifications(&enforced_json),
        Vec::<&str>::new(),
        "receipt-wired obligations must be exempt from gate classification in repo mode"
    );
}

// ── AC4: receipt mode — repo-wired obligations not executed (excluded
//    entirely); pairing skipped; unratified spec change still gates. ─────

#[test]
fn ac4_receipt_mode_excludes_repo_wired_obligations_and_still_enforces_ratification() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);
    let receipt_path = write_receipt(&path, "clean-run.json", CLEAN_RECEIPT);

    // Edit the spec after ratifying, without re-ratifying.
    let edited = FIXTURE_ARTIFACT_MIXED.replace("0.0.0-test", "0.0.0-test-edited");
    assert_ne!(
        edited, FIXTURE_ARTIFACT_MIXED,
        "the edit must actually change the file's bytes"
    );
    std::fs::write(path.join("specs/obligations-v0.1.json"), edited).unwrap();

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert!(
        classifications(&json).contains(&"unratified_spec_change"),
        "classifications: {:?}",
        classifications(&json)
    );

    let by_id = obligations_by_id(&json);
    assert!(
        !by_id.contains_key("OBL-REPO-01"),
        "repo-wired obligations must not be executed (or represented) in receipt mode: {json}"
    );
    assert_eq!(
        json["errors"],
        serde_json::json!([]),
        "pairing check must be skipped in receipt mode"
    );
}

// ── AC5: malformed/absent receipt → invalid_receipt; exit 1 under
//    --enforce, exit 0 without, never exit 2. ────────────────────────────

#[test]
fn ac5_absent_receipt_is_invalid_receipt_exit_1_under_enforce_never_exit_2() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args([
            "check",
            "--enforce",
            "--receipt",
            "receipts/does-not-exist.json",
        ])
        .output()
        .unwrap();
    assert_ne!(
        output.status.code(),
        Some(2),
        "bad input is a refusal, never a crash"
    );
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert!(classifications(&json).contains(&"invalid_receipt"));
    assert_eq!(
        json["receipt"]["sha256"],
        serde_json::Value::Null,
        "an unreadable file has no hash"
    );
}

#[test]
fn ac5_absent_receipt_without_enforce_is_exit_0_but_still_names_invalid_receipt() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--receipt", "receipts/does-not-exist.json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert!(
        classifications(&json).contains(&"invalid_receipt"),
        "findings must still be emitted in JSON even though enforcement is off: {json}"
    );
}

#[test]
fn ac5_malformed_json_receipt_still_reports_a_real_sha256() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);
    let receipt_path = write_receipt(&path, "broken.json", "{ not json");

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    assert!(classifications(&json).contains(&"invalid_receipt"));
    let sha256 = json["receipt"]["sha256"]
        .as_str()
        .expect("a readable-but-malformed file still hashes");
    assert!(sha256.starts_with("sha256:"));
}

// ── AC6: subject-binding violations → Error, gates (both directions). ────

#[test]
fn ac6_receipt_query_in_a_repo_wired_obligation_errors_and_gates_in_repo_mode() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_BAD_REPO_WITH_RECEIPT_QUERY);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");

    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-BAD-REPO-01"]["status"], "error");
    assert!(
        enforcement_failure_for(&json, "OBL-BAD-REPO-01").is_some(),
        "a subject-binding violation must gate: {json}"
    );
}

#[test]
fn ac6_repo_backend_in_a_receipt_wired_obligation_errors_and_gates_in_receipt_mode() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_BAD_RECEIPT_WITH_REPO_BACKEND);
    ratify(&path);
    // Content is irrelevant — the binding violation is caught before the
    // receipt is ever consulted.
    let receipt_path = write_receipt(&path, "clean-run.json", CLEAN_RECEIPT);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");

    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-BAD-RECEIPT-01"]["status"], "error");
    let failure = enforcement_failure_for(&json, "OBL-BAD-RECEIPT-01")
        .expect("a subject-binding violation must gate");
    assert_eq!(failure["classification"], "receipt_violation");
}

// ── AC7 (CLI-level slice): the clean/violating fixtures above already
//    exercise eq/absent/ne/sum+at_most/count+zero end-to-end through the
//    real CLI — this test additionally proves an empty-select receipt
//    (no `actions` key at all) counts as zero without erroring, wired
//    correctly from JSON all the way through the evaluator. ──────────────

#[test]
fn ac7_receipt_missing_the_selected_path_entirely_counts_as_zero_candidates() {
    let (_dir, path) = init_temp_project(false);
    write_obligations_artifact(&path, FIXTURE_ARTIFACT_MIXED);
    ratify(&path);
    let receipt_path = write_receipt(
        &path,
        "no-actions.json",
        r#"{ "meta": { "run_id": "run-empty-1" } }"#,
    );

    let output = g8()
        .current_dir(&path)
        .args(["check", "--enforce", "--receipt"])
        .arg(&receipt_path)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("valid JSON");
    let by_id = obligations_by_id(&json);
    for id in [
        "OBL-RCPT-PROVENANCE-01",
        "OBL-RCPT-SPENDCAP-01",
        "OBL-RCPT-ESCALATE-01",
    ] {
        assert_eq!(by_id[id]["status"], "passed", "{id}: {}", by_id[id]);
    }
}
