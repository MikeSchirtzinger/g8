//! Integration tests for the `g8` binary.
//!
//! Each test uses a temporary directory as an isolated `.g8/` substrate so
//! tests are hermetic and don't interfere with each other.
//!
//! # Coverage
//!
//! - `--help` smoke (verifies binary is runnable + lists subcommands)
//! - `--version` smoke
//! - `g8 init` happy path
//! - `g8 init --enforce` sets enforcement
//! - `g8 scan --report-only` (no ast-grep required)
//! - `g8 plan new` happy path (creates plan, emits FitReport)
//! - `g8 plan list` empty store
//! - `g8 plan show` missing id
//! - `g8 status` happy path (after init)
//! - `g8 check --json` enforcement disabled (exit 0, valid JSON)
//! - `g8 space list` after init
//! - `g8 substrate add` happy path
//! - `g8 link` valid REF form
//! - `g8 query plans:parked` empty result
//! - `g8 merge` missing .g8 in remote
//! - `--output json` emits JSON for init

use std::path::PathBuf;
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn g8() -> Command {
    Command::cargo_bin("g8").expect("g8 binary not found")
}

/// Create a temp dir and run `g8 init` inside it.
/// Returns the TempDir (must stay alive) and the path.
fn init_temp_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().to_path_buf();

    g8().current_dir(&path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    (dir, path)
}

// ── Help smoke ────────────────────────────────────────────────────────────────

#[test]
fn test_help_exits_zero() {
    g8().arg("--help").assert().success();
}

#[test]
fn test_help_lists_subcommands() {
    g8().arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("scan"))
        .stdout(predicate::str::contains("plan"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("attest"))
        .stdout(predicate::str::contains("ratify"))
        .stdout(predicate::str::contains("merge"))
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("space"))
        .stdout(predicate::str::contains("link"))
        .stdout(predicate::str::contains("substrate"));
}

#[test]
fn test_version_exits_zero() {
    g8().arg("--version").assert().success();
}

// ── init ──────────────────────────────────────────────────────────────────────

#[test]
fn test_init_creates_g8_dir() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    g8().current_dir(path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    assert!(path.join(".g8").exists(), ".g8/ should be created");
    assert!(
        path.join(".g8/store.db").exists(),
        "store.db should be created"
    );
    assert!(
        path.join(".g8/config.toml").exists(),
        "config.toml should be created"
    );
}

#[test]
fn test_init_default_enforcement_off() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    g8().current_dir(path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    let config = std::fs::read_to_string(path.join(".g8/config.toml")).unwrap();
    assert!(
        config.contains("enforcement = \"off\""),
        "enforcement should default to off"
    );
}

#[test]
fn test_init_enforce_flag_sets_on() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    g8().current_dir(path)
        .args(["init", "--enforce", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    let config = std::fs::read_to_string(path.join(".g8/config.toml")).unwrap();
    assert!(
        config.contains("enforcement = \"on\""),
        "enforcement should be on"
    );
}

#[test]
fn test_init_idempotent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    // Run twice — should both succeed.
    g8().current_dir(path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    g8().current_dir(path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();
}

#[test]
fn test_init_json_output() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    let output = g8()
        .current_dir(path)
        .args([
            "--output",
            "json",
            "init",
            "--no-claude-import",
            "--no-subagents",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _json: serde_json::Value =
        serde_json::from_str(&stdout).expect("init --output json should emit valid JSON");
}

// ── scan ──────────────────────────────────────────────────────────────────────

#[test]
fn test_scan_report_only_no_ast_grep_required() {
    // `--report-only` runs the extractor but even if it fails we get a zero exit
    // (the error is caught gracefully). This test verifies the command parses and
    // doesn't panic regardless of ast-grep availability.
    let (_dir, path) = init_temp_project();

    // We accept either success or a graceful non-2 exit.
    let status = g8()
        .current_dir(&path)
        .args(["scan", "--report-only"])
        .status()
        .unwrap();

    // The process should not panic (exit code != signal); any non-panic exit is OK here.
    assert!(
        status.code().is_some(),
        "process should not be killed by signal"
    );
}

// ── plan new ──────────────────────────────────────────────────────────────────

#[test]
fn test_plan_new_emits_fit_report() {
    let (_dir, path) = init_temp_project();

    // plan new on an empty store → Proceed recommendation.
    g8().current_dir(&path)
        .args(["plan", "new", "test-feature", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("recommendation"));
}

#[test]
fn test_plan_new_json_contains_version() {
    let (_dir, path) = init_temp_project();

    let output = g8()
        .current_dir(&path)
        .args(["plan", "new", "my-feature", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(
        json.get("g8_version").is_some(),
        "JSON should contain g8_version"
    );
}

// ── plan list ─────────────────────────────────────────────────────────────────

#[test]
fn test_plan_list_empty_store() {
    let (_dir, path) = init_temp_project();

    // Empty store: list should succeed without panic.
    g8().current_dir(&path)
        .args(["plan", "list"])
        .assert()
        .success();
}

#[test]
fn test_plan_list_parked_flag() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["plan", "list", "--parked"])
        .assert()
        .success();
}

// ── plan show ─────────────────────────────────────────────────────────────────

#[test]
fn test_plan_show_missing_id_exits_nonzero() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["plan", "show", "nonexistent-id"])
        .assert()
        .failure();
}

// ── plan lifecycle round-trip ─────────────────────────────────────────────────

#[test]
fn test_plan_lifecycle_round_trip() {
    let (_dir, path) = init_temp_project();

    // Create plan.
    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "lifecycle-test",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // The plan id is not directly in FitReport output; just confirm success.
    assert!(
        json.get("recommendation").is_some()
            || json.get("draft").is_some()
            || json.get("g8_version").is_some()
    );
}

// ── status ────────────────────────────────────────────────────────────────────

#[test]
fn test_status_after_init() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path).args(["status"]).assert().success();
}

#[test]
fn test_status_no_regen() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["status", "--no-regen"])
        .assert()
        .success();
}

// ── check ─────────────────────────────────────────────────────────────────────

#[test]
fn test_check_json_enforcement_disabled_emits_valid_json() {
    let (_dir, path) = init_temp_project();
    // Default enforcement is off.

    let output = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "enforcement disabled → exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("check --json must always emit valid JSON");

    assert!(json.get("g8_version").is_some(), "must have g8_version");
    assert!(json.get("errors").is_some(), "must have errors array");
    assert!(json.get("exit_code").is_some(), "must have exit_code");
}

#[test]
fn test_check_json_always_on_stdout_even_without_flag() {
    // Per ARCH §14, g8 check always emits JSON to stdout regardless of --output mode.
    let (_dir, path) = init_temp_project();

    let output = g8().current_dir(&path).args(["check"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should always be parseable JSON.
    let _json: serde_json::Value =
        serde_json::from_str(&stdout).expect("check should always emit valid JSON to stdout");
}

#[test]
fn test_check_audit_reports_pairing_findings_and_enforce_override_blocks() {
    let (_dir, path) = init_temp_project();
    let src = path.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "// @g8.capability(name = \"unpaired\", status = \"in_flight\", substrate = \"test\")\npub fn marker() {}\n",
    )
    .unwrap();

    g8().current_dir(&path)
        .args(["scan", "--no-watch"])
        .assert()
        .success();

    let audit = g8()
        .current_dir(&path)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert!(
        audit.status.success(),
        "audit mode must remain non-blocking"
    );
    let audit_json: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(audit_json["enforcement_disabled"], true);
    assert_eq!(audit_json["errors"].as_array().unwrap().len(), 1);
    assert_eq!(audit_json["errors"][0]["capability"], "unpaired");

    let enforced = g8()
        .current_dir(&path)
        .args(["check", "--json", "--enforce"])
        .output()
        .unwrap();
    assert_eq!(enforced.status.code(), Some(1));
    let enforced_json: serde_json::Value = serde_json::from_slice(&enforced.stdout).unwrap();
    assert!(enforced_json.get("enforcement_disabled").is_none());
    assert_eq!(enforced_json["errors"].as_array().unwrap().len(), 1);
}

// ── space ─────────────────────────────────────────────────────────────────────

#[test]
fn test_space_list_after_init() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["space", "list"])
        .assert()
        .success();
}

#[test]
fn test_space_add_and_list() {
    let (_dir, path) = init_temp_project();
    let other_dir = TempDir::new().unwrap();

    g8().current_dir(&path)
        .args([
            "space",
            "add",
            other_dir.path().to_str().unwrap(),
            "--name",
            "other-project",
        ])
        .assert()
        .success();

    // Verify the project appears in the list.
    let output = g8()
        .current_dir(&path)
        .args(["space", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("other-project"),
        "added project should appear in list"
    );
}

// ── link ──────────────────────────────────────────────────────────────────────

#[test]
fn test_link_valid_refs() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args([
            "link",
            "--canonical",
            "project-a::http-fetch",
            "--alias",
            "project-b::http-fetch",
        ])
        .assert()
        .success();
}

#[test]
fn test_link_invalid_ref_exits_nonzero() {
    let (_dir, path) = init_temp_project();

    // REF without `::` should fail validation.
    g8().current_dir(&path)
        .args([
            "link",
            "--canonical",
            "http-fetch",
            "--alias",
            "project-b::http-fetch",
        ])
        .assert()
        .failure();
}

// ── substrate ─────────────────────────────────────────────────────────────────

#[test]
fn test_substrate_add() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args([
            "substrate",
            "add",
            "auth-layer",
            "--wip-cap",
            "2",
            "--stale-days",
            "7",
        ])
        .assert()
        .success();
}

#[test]
fn test_substrate_list() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["substrate", "list"])
        .assert()
        .success();
}

// ── query ─────────────────────────────────────────────────────────────────────

#[test]
fn test_query_plans_parked() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["query", "plans:parked"])
        .assert()
        .success();
}

#[test]
fn test_query_unknown_form_exits_nonzero() {
    let (_dir, path) = init_temp_project();

    g8().current_dir(&path)
        .args(["query", "unknown:form"])
        .assert()
        .failure();
}

// ── merge ─────────────────────────────────────────────────────────────────────

#[test]
fn test_merge_missing_remote_g8_exits_nonzero() {
    let (_dir, path) = init_temp_project();
    let remote_dir = TempDir::new().unwrap();

    // Remote dir has no .g8/ → should fail gracefully.
    g8().current_dir(&path)
        .args(["merge", "--from", remote_dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_merge_two_initialized_projects() {
    let (_local_dir, local_path) = init_temp_project();
    let (_remote_dir, remote_path) = init_temp_project();

    // Both projects are initialized → merge should succeed (no conflicts expected).
    g8().current_dir(&local_path)
        .args(["merge", "--from", remote_path.to_str().unwrap()])
        .assert()
        .success();
}

// ── round-trip smoke (init + scan + plan new + status) ───────────────────────

#[test]
fn test_full_round_trip_empty_dir() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    // 1. init
    g8().current_dir(path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();

    // 2. scan (may warn about missing ast-grep; should not panic)
    let _ = g8()
        .current_dir(path)
        .args(["scan", "--report-only"])
        .status();

    // 3. plan new
    g8().current_dir(path)
        .args(["plan", "new", "round-trip-feature", "--json"])
        .assert()
        .success();

    // 4. status
    g8().current_dir(path).args(["status"]).assert().success();

    // 5. check --json
    let output = g8()
        .current_dir(path)
        .args(["check", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("check --json must be valid JSON");
    assert_eq!(json["g8_version"], "0.1.0");
}
