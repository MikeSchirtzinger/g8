//! Integration tests for the obligations self-audit wired into `g8 check`.
//!
//! Per `specs/obligation-checker-contract.md` §5 and Mike's ruling
//! (2026-07-02): audit checks run even when enforcement is off; findings are
//! always emitted in JSON; enforcement affects the EXIT CODE only.
//!
//! # Coverage
//!
//! - obligations populated unconditionally, even with enforcement off
//! - a non-advisory failed/errored obligation trips the exit code once
//!   enforcement is on
//! - an advisory-only failure does NOT trip the exit code
//! - a missing `specs/obligations-v0.1.json` degrades gracefully (empty
//!   array + note), never an error, never exit != 0 by itself
//! - `#[ignore]`d: the REAL 27-obligation artifact end-to-end (T6/T7 run
//!   this explicitly — see its own doc comment for why)
//!
//! Deliberately does NOT exercise all 27 real obligations in the default
//! `cargo test` run: OBL-D7-01 uses `BinarySource::CargoRun` and can trigger
//! real recompiles (minutes). These tests use a small, self-contained
//! fixture artifact instead (one passing, one failing, one erroring, one
//! advisory-failing `rg_match_count` check — the cheapest backend, no
//! compiles).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn g8() -> Command {
    Command::cargo_bin("g8").expect("g8 binary not found")
}

// assert_cmd's `CommandCargoExt` trait, matching the other test files' style.
use assert_cmd::prelude::*;

/// A tiny, self-contained obligations artifact: one passing, one failing,
/// one erroring, and one advisory-failing check, all using the cheapest
/// backend (`rg_match_count` against a one-line fixture file this test
/// writes itself — no `cargo`/`ast-grep` subprocess, no compiles).
const FIXTURE_ARTIFACT: &str = r#"{
  "meta": { "artifact": "g8-test-fixture", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-TEST-PASS-01",
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
      "id": "OBL-TEST-FAIL-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "MARKER_PRESENT",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": false }
    },
    {
      "id": "OBL-TEST-ERROR-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "(unclosed",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": false }
    },
    {
      "id": "OBL-TEST-ADVISORY-FAIL-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "MARKER_PRESENT",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": true }
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

/// Same shape, but only the passing + advisory-failing obligations — used
/// by the "advisory never trips exit code" test so its only failure IS the
/// advisory one (a non-advisory `OBL-TEST-FAIL-01` in the mix would
/// confound the assertion).
const FIXTURE_ARTIFACT_ADVISORY_ONLY: &str = r#"{
  "meta": { "artifact": "g8-test-fixture-advisory-only", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-TEST-PASS-01",
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
      "signal": { "advisory": true }
    },
    {
      "id": "OBL-TEST-ADVISORY-FAIL-01",
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "rg_match_count",
            "args": {
              "pattern": "MARKER_PRESENT",
              "glob": ["marker.txt"],
              "expected": { "kind": "zero" }
            }
          }
        ]
      },
      "signal": { "advisory": true }
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

/// One passing static checker on a real gate. It is intentionally only
/// `Checked`; a fresh attestation is required to clear the rigor floor.
const FIXTURE_ARTIFACT_RIGOR: &str = r#"{
  "meta": { "artifact": "g8-test-rigor", "version": "0.0.0-test" },
  "obligations": [
    {
      "id": "OBL-TEST-RIGOR-01",
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
    }
  ],
  "conflicts": [],
  "open_questions": []
}"#;

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

/// Write the fixture artifact + its `marker.txt` (containing the string
/// `MARKER_PRESENT`, which the FAIL/ADVISORY-FAIL checks look for) into
/// `root`.
fn write_fixture(root: &Path, artifact_json: &str) {
    std::fs::write(root.join("marker.txt"), "MARKER_PRESENT is here.\n").unwrap();
    let specs_dir = root.join("specs");
    std::fs::create_dir_all(&specs_dir).unwrap();
    std::fs::write(specs_dir.join("obligations-v0.1.json"), artifact_json).unwrap();
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

fn obligations_by_id(json: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
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

// ── Always runs, even with enforcement off ──────────────────────────────────

#[test]
fn test_obligations_populated_and_shaped_when_enforcement_off() {
    let (_dir, path) = init_temp_project(false);
    write_fixture(&path, FIXTURE_ARTIFACT);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "enforcement disabled → exit 0 regardless of obligation findings"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("check --json must always emit valid JSON");

    // Backward-compatible keys, byte-shape unchanged.
    assert!(json.get("g8_version").is_some());
    assert!(json.get("errors").and_then(|e| e.as_array()).is_some());
    assert_eq!(json["exit_code"], serde_json::json!(0));
    assert_eq!(json["enforcement_disabled"], serde_json::json!(true));

    let by_id = obligations_by_id(&json);
    assert_eq!(
        by_id.len(),
        4,
        "expected all 4 fixture obligations: {stdout}"
    );

    assert_eq!(by_id["OBL-TEST-PASS-01"]["status"], "passed");
    assert_eq!(by_id["OBL-TEST-PASS-01"]["trust"], "checked");
    assert_eq!(by_id["OBL-TEST-FAIL-01"]["status"], "failed");
    assert_eq!(by_id["OBL-TEST-ERROR-01"]["status"], "error");
    assert_eq!(by_id["OBL-TEST-ADVISORY-FAIL-01"]["status"], "failed");

    // JSON shape (contract §5): every entry carries id/status/trust/evidence,
    // and evidence carries the full sub-check transparency array.
    for (id, o) in &by_id {
        assert!(o.get("status").is_some(), "{id} missing status");
        assert!(o.get("trust").is_some(), "{id} missing trust");
        let evidence = o
            .get("evidence")
            .unwrap_or_else(|| panic!("{id} missing evidence"));
        assert!(
            evidence.get("backend").is_some(),
            "{id} evidence missing backend"
        );
        assert!(
            evidence.get("summary").is_some(),
            "{id} evidence missing summary"
        );
        assert!(
            evidence.get("duration_ms").is_some(),
            "{id} evidence missing duration_ms"
        );
        assert!(
            evidence.get("detail").is_some(),
            "{id} evidence missing detail"
        );
        assert!(
            evidence
                .get("checks_run")
                .and_then(|c| c.as_array())
                .is_some(),
            "{id} evidence missing checks_run array"
        );
    }
}

// ── Enforcement-on exit-code mapping ─────────────────────────────────────────

#[test]
fn test_non_advisory_obligation_failure_trips_exit_code_when_enforcement_on() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    // OBL-TEST-FAIL-01 (non-advisory, failed) and OBL-TEST-ERROR-01
    // (non-advisory, errored) both must trip it; no pairing errors exist
    // (nothing was scanned), so this exit code is attributable to
    // obligations alone.
    assert_eq!(output.status.code(), Some(1), "stdout: {stdout}");
    assert_eq!(json["exit_code"], serde_json::json!(1));
    // Enforcement ON → key absent entirely (unchanged pre-existing shape).
    assert!(json.get("enforcement_disabled").is_none());

    let by_id = obligations_by_id(&json);
    assert_eq!(by_id.len(), 4);
    assert!(classifications(&json).contains(&"unaccounted_drift"));
}

#[test]
fn test_advisory_failure_alone_does_not_trip_exit_code_when_enforcement_on() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT_ADVISORY_ONLY);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-TEST-ADVISORY-FAIL-01"]["status"], "failed");

    // The only failure present is advisory, and there are no pairing
    // errors — contract §5.1: advisory failures never affect exit code.
    assert!(
        output.status.success(),
        "advisory-only failure must not trip exit code; stdout: {stdout}"
    );
    assert_eq!(json["exit_code"], serde_json::json!(0));
    assert!(classifications(&json).is_empty());
}

// ── Rigor, attestations, and ratification ───────────────────────────────────

#[test]
fn test_checked_gate_without_attestation_is_insufficient_rigor() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT_RIGOR);
    ratify(&path);

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .expect("run check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(output.status.code(), Some(1), "stdout: {stdout}");
    assert_eq!(classifications(&json), vec!["insufficient_rigor"]);
    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-TEST-RIGOR-01"]["status"], "passed");
    assert_eq!(by_id["OBL-TEST-RIGOR-01"]["trust"], "checked");
}

#[test]
fn test_attest_writer_creates_fresh_asserted_evidence() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT_RIGOR);
    ratify(&path);

    g8().current_dir(&path)
        .args([
            "attest",
            "OBL-TEST-RIGOR-01",
            "--files",
            "marker.txt",
            "--evidence",
            "proof.md#obl-test-rigor-01",
        ])
        .assert()
        .success();

    let sidecar_text = std::fs::read_to_string(path.join("specs/attestations-v0.1.json"))
        .expect("read attestation sidecar");
    let sidecar: serde_json::Value =
        serde_json::from_str(&sidecar_text).expect("valid sidecar JSON");
    let record = &sidecar["attestations"][0];
    assert_eq!(record["obligation_id"], "OBL-TEST-RIGOR-01");
    assert_eq!(record["evidence_pointer"], "proof.md#obl-test-rigor-01");
    assert_eq!(record["files"], serde_json::json!(["marker.txt"]));
    assert!(record["pinned_content_hash"]
        .as_str()
        .expect("pin string")
        .starts_with("sha256:"));

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .expect("run check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(output.status.success(), "stdout: {stdout}");
    assert!(classifications(&json).is_empty());
    let by_id = obligations_by_id(&json);
    assert_eq!(by_id["OBL-TEST-RIGOR-01"]["status"], "passed");
    assert_eq!(by_id["OBL-TEST-RIGOR-01"]["trust"], "asserted");
}

#[test]
fn test_changed_attested_file_is_stale_attestation_pin() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT_RIGOR);
    ratify(&path);
    g8().current_dir(&path)
        .args(["attest", "OBL-TEST-RIGOR-01", "--files", "marker.txt"])
        .assert()
        .success();
    std::fs::write(path.join("marker.txt"), "evidence changed\n")
        .expect("mutate attested evidence");

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .expect("run check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(output.status.code(), Some(1), "stdout: {stdout}");
    assert_eq!(classifications(&json), vec!["stale_attestation_pin"]);
}

#[test]
fn test_changed_spec_is_unratified_spec_change() {
    let (_dir, path) = init_temp_project(true);
    write_fixture(&path, FIXTURE_ARTIFACT_RIGOR);
    ratify(&path);
    g8().current_dir(&path)
        .args(["attest", "OBL-TEST-RIGOR-01", "--files", "marker.txt"])
        .assert()
        .success();

    let spec_path = path.join("specs/obligations-v0.1.json");
    let mut spec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&spec_path).expect("read spec"))
            .expect("parse spec");
    spec["meta"]["changed_after_ratification"] = serde_json::json!(true);
    std::fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize changed spec"),
    )
    .expect("write changed spec");

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .expect("run check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(output.status.code(), Some(1), "stdout: {stdout}");
    assert_eq!(classifications(&json), vec!["unratified_spec_change"]);
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.join("g8.lock")).expect("read lock"))
            .expect("valid lock");
    assert_eq!(lock["spec_path"], "specs/obligations-v0.1.json");
    assert!(lock["spec_sha256"]
        .as_str()
        .expect("lock pin")
        .starts_with("sha256:"));
}

// ── Missing artifact: graceful, not an error ────────────────────────────────

#[test]
fn test_missing_obligations_artifact_enforcement_off_is_not_an_error() {
    let (_dir, path) = init_temp_project(false);
    // Deliberately do NOT write specs/obligations-v0.1.json.

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(json["obligations"], serde_json::json!([]));
    assert!(
        json.get("obligations_note")
            .and_then(|v| v.as_str())
            .is_some(),
        "missing artifact should still explain itself via obligations_note: {stdout}"
    );
}

#[test]
fn test_missing_obligations_artifact_enforcement_on_still_exits_zero() {
    let (_dir, path) = init_temp_project(true);
    // No artifact, no capabilities scanned → nothing to fail on.

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "a missing obligations artifact must never itself cause exit != 0"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(json["obligations"], serde_json::json!([]));
    assert_eq!(json["exit_code"], serde_json::json!(0));
}

// ── Real artifact, end-to-end (ignored by default) ──────────────────────────

/// Copy the whole repo (minus `target/` and other non-source cruft) into a
/// fresh tempdir. The real obligations checkers need the REAL `crates/`
/// tree alongside `specs/obligations-v0.1.json` — several backends
/// (`CargoMetadataNoDep`, `CargoMetadataDepGraph`, and OBL-D7-01's
/// `BinarySource::CargoRun`) need a genuinely buildable Cargo workspace at
/// `workspace_root`, not just the JSON file in isolation.
///
/// Copies rather than pointing at the real repo directly: this repo has no
/// VCS (`o-g8-defuse-and-wire-20260702.md`'s own header: "NO VCS in this
/// tree"), so a test that ran `g8 init` against the real repo root would
/// leave `.g8/store.db`/`.g8/config.toml` behind with no `git checkout`
/// to revert an interrupted run. Copying into a tempdir can never touch the
/// real repo, at the cost of a few MB of I/O (`crates/` + `specs/` together
/// are well under 2 MB).
fn copy_repo_to(dest: &Path) {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    copy_dir_filtered(&repo_root, dest);
}

fn copy_dir_filtered(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        // `target/` is the huge, regenerable build cache; the others are
        // non-source cruft irrelevant to the obligations checkers.
        if matches!(
            name.to_string_lossy().as_ref(),
            "target" | ".backup-20260702" | ".colab" | ".git"
        ) {
            continue;
        }
        let dst_path = dst.join(&name);
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir_filtered(&entry.path(), &dst_path);
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dst_path).unwrap();
        }
        // No symlinks anywhere in this tree today — nothing else to handle.
    }
}

/// Real end-to-end proof against the actual 27-obligation artifact —
/// T6/T7's explicit re-verification tool, not part of the default `cargo
/// test` run. Slow: OBL-D7-01 (`BinarySource::CargoRun`) triggers real
/// `cargo run -p g8 [--features serve]` builds, and this test runs the
/// full 27-obligation suite twice (once per enforcement mode). Run
/// explicitly:
///
/// ```text
/// cargo test -p g8 --test obligations_check -- --ignored \
///     test_real_obligations_artifact_end_to_end --nocapture
/// ```
#[test]
#[ignore = "slow: copies the repo + triggers real cargo builds via OBL-D7-01; \
            run explicitly with --ignored"]
fn test_real_obligations_artifact_end_to_end() {
    let dir = TempDir::new().unwrap();
    copy_repo_to(dir.path());

    g8().current_dir(dir.path())
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    let output = g8()
        .current_dir(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "enforcement disabled → exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("g8 check --json against the real artifact must emit valid JSON");

    let by_id = obligations_by_id(&json);
    assert_eq!(
        by_id.len(),
        27,
        "expected all 27 real obligations, got {}: {stdout}",
        by_id.len()
    );

    let mut by_status: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, o) in &by_id {
        let status = o["status"].as_str().unwrap().to_string();
        assert_ne!(
            status, "unknown",
            "{id} resolved to unknown: contract §2's own tally is 27/27 typed"
        );
        by_status.entry(status).or_default().push(id.clone());
    }
    eprintln!("real obligations artifact summary: {by_status:#?}");

    // Cross-check the exit-code arithmetic against an independently-enforced
    // run without hardcoding which checks pass. The enforcement_failures
    // array now includes checker drift, rigor-floor gaps, stale evidence, and
    // unratified spec changes.
    g8().current_dir(dir.path())
        .args(["init", "--enforce", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    let enforced_output = g8()
        .current_dir(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();
    let enforced_stdout = String::from_utf8_lossy(&enforced_output.stdout);
    let enforced_json: serde_json::Value =
        serde_json::from_str(&enforced_stdout).expect("valid JSON");

    let expected_exit = i32::from(
        !enforced_json["enforcement_failures"]
            .as_array()
            .expect("enforcement_failures array")
            .is_empty(),
    );

    assert_eq!(
        enforced_output.status.code(),
        Some(expected_exit),
        "exit-code arithmetic (contract §5.1) mismatch; enforced json: {enforced_stdout}"
    );
    assert_eq!(enforced_json["exit_code"], serde_json::json!(expected_exit));
}
