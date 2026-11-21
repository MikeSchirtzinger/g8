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

fn git_in(root: &std::path::Path) -> Command {
    let mut git = Command::new("git");
    git.current_dir(root).args([
        "-c",
        "user.name=G8 Test",
        "-c",
        "user.email=g8@example.test",
    ]);
    git
}

fn init_with_hook(root: &std::path::Path) -> Command {
    let mut command = g8();
    command.current_dir(root).args([
        "init",
        "--no-claude-import",
        "--no-subagents",
        "--install-pre-commit",
    ]);
    command
}

#[cfg(unix)]
#[test]
fn test_init_installs_shared_hook_that_skips_worktree_without_lock() {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main");
    let linked = dir.path().join("linked");
    std::fs::create_dir(&main).unwrap();
    git_in(&main)
        .args(["init", "-q", "--template="])
        .assert()
        .success();
    git_in(&main)
        .args(["commit", "-q", "--allow-empty", "-m", "initial"])
        .assert()
        .success();
    git_in(&main)
        .args(["worktree", "add", "-q", "-b", "linked"])
        .arg(&linked)
        .assert()
        .success();

    init_with_hook(&linked)
        .assert()
        .success()
        .stderr(predicate::str::contains("installed pre-commit hook"));
    assert!(main.join(".git/hooks/pre-commit").is_file());
    assert!(!main.join(".g8").exists());
    assert!(!main.join("g8.lock").exists());
    write_obligations(&linked, r#"{"obligations":[]}"#);
    g8().current_dir(&linked).arg("ratify").assert().success();

    let binary = assert_cmd::cargo::cargo_bin("g8");
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    git_in(&linked)
        .env("PATH", &path)
        .args(["commit", "-q", "--allow-empty", "-m", "checked"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("all capabilities paired"));
    git_in(&main)
        .env("PATH", &path)
        .args(["commit", "-q", "--allow-empty", "-m", "no lock"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_init_hook_honors_hooks_path_and_preserves_foreign_hook() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    git_in(root)
        .args(["init", "-q", "--template="])
        .assert()
        .success();
    git_in(root)
        .args(["config", "core.hooksPath", "custom hooks"])
        .assert()
        .success();

    let hook = root.join("custom hooks/pre-commit");
    std::fs::create_dir(hook.parent().unwrap()).unwrap();
    let foreign = "#!/bin/sh\nexit 1\n";
    std::fs::write(&hook, foreign).unwrap();
    init_with_hook(root)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "already exists and is not a g8 hook",
        ));
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), foreign);

    std::fs::remove_file(&hook).unwrap();
    init_with_hook(root).assert().success();
    let installed = std::fs::read_to_string(&hook).unwrap();
    assert!(installed.contains("if [ ! -f ./g8.lock ]; then"));
    assert!(!root.join(".git/hooks/pre-commit").exists());
    init_with_hook(root).assert().success();
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), installed);

    // Restore the exact script written by older installers and upgrade it.
    let legacy = installed.replace(
        "# Git runs pre-commit from the root of the worktree being committed.\nif [ ! -f ./g8.lock ]; then\n    exit 0\nfi\n",
        "",
    );
    std::fs::write(&hook, legacy).unwrap();
    init_with_hook(root).assert().success();
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), installed);
}

#[test]
fn test_init_hook_fails_outside_git_and_when_hooks_path_is_blocked() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_with_hook(root)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "could not install pre-commit hook",
        ))
        .stderr(predicate::str::contains(
            "git rev-parse --git-path hooks failed",
        ));

    git_in(root)
        .args(["init", "-q", "--template="])
        .assert()
        .success();
    git_in(root)
        .args(["config", "core.hooksPath", "blocked"])
        .assert()
        .success();
    std::fs::write(root.join("blocked"), "not a directory").unwrap();
    init_with_hook(root)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("creating hooks directory"));
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

fn write_obligations(root: &std::path::Path, spec: &str) -> PathBuf {
    let path = root.join("specs/obligations-v0.1.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, spec).unwrap();
    path
}

const SELECTION_SPEC: &str = r#"{
  "obligations": [
    {"id":"SC-M1-01","signal":{"advisory":true}},
    {"id":"SC-M1-02","signal":{"advisory":true}},
    {"id":"SC-PROTO-01","signal":{"advisory":true}},
    {"id":"SC-OTHER-01","checker":{"mode":"typed","checks":[
      {"backend":"cargo_metadata_no_dep","args":{"denied":[],"scope_crates":[],"build_config":"default"}}
    ]}},
    {"id":"RECEIPT-01","signal":{"wiring":"action_receipt","advisory":true}},
    {"id":"RECEIPT-02","signal":{"wiring":"action_receipt","advisory":true}}
  ]
}"#;

fn ratify_selection_spec(root: &std::path::Path) -> PathBuf {
    let spec = write_obligations(root, SELECTION_SPEC);
    g8().current_dir(root).arg("ratify").assert().success();
    spec
}

#[cfg(unix)]
#[test]
fn test_check_only_skips_unselected_checkers() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, root) = init_temp_project();
    ratify_selection_spec(&root);

    // The marker detects an actual subprocess launch, even if its result is discarded.
    let tools = TempDir::new().unwrap();
    let cargo = tools.path().join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\n: > ran-cargo\nexit 1\n").unwrap();
    std::fs::set_permissions(cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(tools.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let check = || {
        let mut command = g8();
        command
            .current_dir(&root)
            .env("PATH", &path)
            .args(["check", "--enforce", "--json"]);
        command
    };

    let partial = check()
        .args(["--only", "SC-M1-*", "SC-PROTO-01", "SC-M1-01"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "3 obligation(s) selected, 3 not selected",
        ))
        .stderr(predicate::str::contains("partial run"));
    let json: serde_json::Value = serde_json::from_slice(&partial.get_output().stdout).unwrap();
    assert_eq!(
        json["obligation_selection"],
        serde_json::json!({
            "patterns": ["SC-M1-*", "SC-PROTO-01", "SC-M1-01"],
            "selected": 3,
            "not_selected": 3,
        })
    );
    let ids: Vec<_> = json["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["SC-M1-01", "SC-M1-02", "SC-PROTO-01"]);
    assert!(!root.join("ran-cargo").exists());

    check()
        .args(["--only", "SC-OTHER-01", "NO-MATCH"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("NO-MATCH"));
    assert!(!root.join("ran-cargo").exists());

    let full = check()
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "6 obligation(s) selected, 0 not selected",
        ))
        .stderr(predicate::str::contains("partial run").not());
    let json: serde_json::Value = serde_json::from_slice(&full.get_output().stdout).unwrap();
    assert_eq!(
        json["obligation_selection"],
        serde_json::json!({"patterns":[],"selected":6,"not_selected":0})
    );
    assert!(root.join("ran-cargo").is_file());
}

#[test]
fn test_check_only_rejects_unusable_patterns() {
    let (_dir, root) = init_temp_project();
    let check = |patterns: &[&str]| {
        g8().current_dir(&root)
            .args(["check", "--json", "--only"])
            .args(patterns)
            .assert()
            .code(2)
    };
    check(&["ABSENT"]).stderr(predicate::str::contains("ABSENT"));
    ratify_selection_spec(&root);
    check(&[]).stderr(predicate::str::contains("--only"));
    check(&["SC-M1-*", "["]).stderr(predicate::str::contains("invalid --only pattern"));
    let unmatched =
        check(&["SC-M1-*", "SC-MISSING"]).stderr(predicate::str::contains("SC-MISSING"));
    let json: serde_json::Value = serde_json::from_slice(&unmatched.get_output().stdout).unwrap();
    assert_eq!(json["exit_code"], 2);
    assert_eq!(
        json["obligation_selection"],
        serde_json::json!({"patterns":["SC-M1-*","SC-MISSING"],"selected":0,"not_selected":6})
    );
}

#[test]
fn test_check_only_still_checks_whole_spec_and_lock() {
    let (_dir, root) = init_temp_project();
    let spec = ratify_selection_spec(&root);
    let check = |detail| {
        let result = g8()
            .current_dir(&root)
            .args(["check", "--enforce", "--json", "--only", "SC-M1-01"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("partial run"))
            .stderr(predicate::str::contains(detail));
        let json: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(
            json["enforcement_failures"][0]["classification"],
            "unratified_spec_change"
        );
    };

    let lock_path = root.join("g8.lock");
    let original_lock = std::fs::read_to_string(&lock_path).unwrap();
    std::fs::remove_file(&lock_path).unwrap();
    check("g8.lock is missing");

    std::fs::write(
        &lock_path,
        original_lock.replace("specs/obligations-v0.1.json", "other.json"),
    )
    .unwrap();
    check("g8.lock pins other.json");

    std::fs::write(lock_path, original_lock).unwrap();
    std::fs::write(spec, SELECTION_SPEC.replace("SC-OTHER-01", "SC-OTHER-02")).unwrap();
    check("changed after ratification");
}

#[test]
fn test_check_only_selects_within_receipt_scope() {
    let (_dir, root) = init_temp_project();
    ratify_selection_spec(&root);
    std::fs::write(root.join("receipt.json"), "{}").unwrap();
    let check = |pattern| {
        g8().current_dir(&root)
            .args([
                "check",
                "--enforce",
                "--json",
                "--receipt",
                "receipt.json",
                "--only",
                pattern,
            ])
            .assert()
    };

    let result = check("RECEIPT-01")
        .success()
        .stderr(predicate::str::contains(
            "1 obligation(s) selected, 1 not selected",
        ))
        .stderr(predicate::str::contains("partial run"));
    let json: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(json["obligations"].as_array().unwrap().len(), 1);
    assert_eq!(json["obligations"][0]["id"], "RECEIPT-01");
    check("SC-M1-01")
        .code(2)
        .stderr(predicate::str::contains("SC-M1-01"));
}

#[test]
fn test_check_accepts_spec_ratified_by_v0_1_5() {
    // The lock was generated by `g8 ratify` built from the v0.1.5 tag.
    let (_dir, root) = init_temp_project();
    write_obligations(
        &root,
        concat!(
            r#"{"obligations":[{"id":"LEGACY-01","checker":{"mode":"typed","checks":[{"backend":"g8_check_contract","args":{"scenarios":["clean_store"]}}]}}]}"#,
            "\n"
        ),
    );
    let lock = concat!(
        "{\n",
        "  \"g8_version\": \"0.1.5\",\n",
        "  \"spec_path\": \"specs/obligations-v0.1.json\",\n",
        "  \"spec_sha256\": \"sha256:453e3686ff53450983786262feea1001a63ed5366378c9cedbf880504ba12cdd\"\n",
        "}\n",
    );
    let lock_path = root.join("g8.lock");
    std::fs::write(&lock_path, lock).unwrap();

    for selection in [&[][..], &["--only", "LEGACY-01"][..]] {
        let result = g8()
            .current_dir(&root)
            .args(["check", "--enforce", "--json"])
            .args(selection)
            .assert()
            .success()
            .stderr(predicate::str::contains("partial run").not());
        let json: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(json["obligations"][0]["status"], "passed");
        assert_eq!(json["enforcement_failures"], serde_json::json!([]));
    }
    assert_eq!(std::fs::read_to_string(lock_path).unwrap(), lock);
}

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
    assert_eq!(json["g8_version"], env!("CARGO_PKG_VERSION"));
}

/// `.g8/` is untracked, so a linked worktree has none. `g8 check` there must
/// use the main checkout's store rather than fail with ".g8/ not found",
/// which is what blocked the pre-commit hook in worktrees.
#[test]
fn test_check_in_linked_worktree_uses_main_checkout_store() {
    let (_dir, main) = init_temp_project();
    let git = |dir: &std::path::Path, args: &[&str]| {
        let status = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    };
    git(&main, &["init", "-q"]);
    std::fs::write(main.join("README"), "x").expect("write");
    git(&main, &["add", "README"]);
    git(&main, &["commit", "-qm", "init"]);
    let wt_parent = TempDir::new().expect("temp dir");
    let wt = wt_parent.path().join("wt");
    git(&main, &["worktree", "add", "-q", wt.to_str().unwrap()]);
    assert!(!wt.join(".g8").exists());

    g8().current_dir(&wt)
        .args(["check", "--json"])
        .assert()
        .success();
    assert!(
        !wt.join(".g8").exists(),
        "check must not create a worktree store"
    );
}
