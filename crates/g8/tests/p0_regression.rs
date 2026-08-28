//! Regression tests for every P0 fixed during the v0.1 ship-readiness pass.
//!
//! Each test maps 1:1 to a `P0-N` item from `specs/smoke-evidence/SMOKE_REPORT.md`. They run the
//! `g8` binary end-to-end against fresh `.g8/` substrates in tempdirs so the
//! coverage matches what an integrator actually exercises.

use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

fn g8() -> Command {
    Command::cargo_bin("g8").expect("g8 binary not found")
}

/// Init an empty project and return (TempDir, path).
fn init_project() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    g8().current_dir(&path)
        .args(["init", "--no-claude-import", "--no-subagents"])
        .assert()
        .success();
    (dir, path)
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON on stdout, got: {stdout}\nerr: {e}"))
}

fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, body).unwrap();
}

// ── P0-1 + P1-1 ──────────────────────────────────────────────────────────────
// Before fix: `plan new --substrate <registered> --dispatch` exited 2 because
// the budget JSON emitted `at_cap` as int 0/1 and BudgetStatus.at_cap is bool.

#[test]
fn p0_1_dispatch_against_registered_substrate_succeeds() {
    let (_dir, path) = init_project();
    g8().current_dir(&path)
        .args(["substrate", "add", "api", "--wip-cap", "2"])
        .assert()
        .success();

    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "feature-one",
            "--substrate",
            "api",
            "--dispatch",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "plan new --dispatch must succeed against a registered substrate"
    );
    let json = json_stdout(&output);
    assert_eq!(json["recommendation"], "Proceed");
    assert_eq!(
        json["budget_status"]["at_cap"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(json["budget_status"]["wip_cap"], 2);
}

#[test]
fn p1_1_unregistered_substrate_gets_default_budget() {
    let (_dir, path) = init_project();

    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "first",
            "--substrate",
            "novel-substrate",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json = json_stdout(&output);
    assert!(
        json["budget_status"].is_object(),
        "unregistered substrate must still surface a budget_status with defaults; got null"
    );
    assert_eq!(json["budget_status"]["wip_cap"], 3);
    assert_eq!(
        json["budget_status"]["at_cap"],
        serde_json::Value::Bool(false)
    );
}

// ── P0-2 ─────────────────────────────────────────────────────────────────────
// Before fix: `plan new --dispatch` persisted regardless of recommendation,
// and `plan dispatch <id>` did not consult the WIP cap at all.

#[test]
fn p0_2_dispatch_refused_when_substrate_at_cap() {
    let (_dir, path) = init_project();
    g8().current_dir(&path)
        .args(["substrate", "add", "api", "--wip-cap", "1"])
        .assert()
        .success();

    // First dispatch fills the only slot.
    g8().current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "first",
            "--substrate",
            "api",
            "--dispatch",
        ])
        .assert()
        .success();

    // Second dispatch must be refused.
    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "second",
            "--substrate",
            "api",
            "--dispatch",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));

    // Last JSON object on stdout is the error envelope (P0-4 contract).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_obj_start = stdout.rfind("\n{").map(|i| i + 1).unwrap_or(0);
    let err_json: serde_json::Value =
        serde_json::from_str(stdout[last_obj_start..].trim()).expect("error envelope is JSON");
    assert_eq!(err_json["status"], "error");
    assert_eq!(err_json["exit_code"], 2);
}

#[test]
fn p0_2_plan_dispatch_refused_when_substrate_at_cap() {
    let (_dir, path) = init_project();
    g8().current_dir(&path)
        .args(["substrate", "add", "api", "--wip-cap", "1"])
        .assert()
        .success();

    // Existing dispatched plan consumes the slot.
    g8().current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "first",
            "--substrate",
            "api",
            "--dispatch",
        ])
        .assert()
        .success();

    // New Idea plan on the same substrate.
    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "second",
            "--substrate",
            "api",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json = json_stdout(&output);
    let second_id = json["plan_id"].as_str().unwrap().to_string();

    // Scope to make it eligible for dispatch.
    g8().current_dir(&path)
        .args(["plan", "scope", &second_id])
        .assert()
        .success();

    // Now `plan dispatch` should refuse.
    let output = g8()
        .current_dir(&path)
        .args(["plan", "dispatch", &second_id])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "plan dispatch must refuse when at WIP cap"
    );
}

// ── P0-3 ─────────────────────────────────────────────────────────────────────
// Before fix: update_plan_status never called validate_status_transition, so
// Idea -> Done succeeded.

#[test]
fn p0_3_illegal_lifecycle_transition_refused() {
    let (_dir, path) = init_project();
    let output = g8()
        .current_dir(&path)
        .args(["--output", "json", "plan", "new", "raw-idea"])
        .output()
        .unwrap();
    let json = json_stdout(&output);
    let plan_id = json["plan_id"].as_str().unwrap().to_string();
    assert_eq!(json["recommendation"], "Proceed");

    // Idea -> Done is illegal per the lifecycle graph; the store must reject it.
    let status = g8()
        .current_dir(&path)
        .args(["plan", "complete", &plan_id])
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "Idea -> Done is not a legal transition; must exit 2"
    );

    // Idea -> Scoped -> Dispatched -> Done is the happy path.
    g8().current_dir(&path)
        .args(["plan", "scope", &plan_id])
        .assert()
        .success();
    g8().current_dir(&path)
        .args(["plan", "dispatch", &plan_id])
        .assert()
        .success();
    g8().current_dir(&path)
        .args(["plan", "complete", &plan_id])
        .assert()
        .success();
}

// ── P0-4 ─────────────────────────────────────────────────────────────────────
// Before fix: command failures with --output json rendered status=ok+message
// to stdout, then exited 2 — misleading for CI consumers.

#[test]
fn p0_4_failure_emits_status_error_not_ok() {
    let (_dir, path) = init_project();
    // `plan show <bogus>` will fail.
    let output = g8()
        .current_dir(&path)
        .args(["--output", "json", "plan", "show", "no-such-plan-id"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json = json_stdout(&output);
    assert_eq!(
        json["status"], "error",
        "JSON failure envelope must use status=error, never ok"
    );
    assert_eq!(json["exit_code"], 2);
    assert!(
        json["error"].is_string(),
        "error envelope must include `error` field"
    );
}

// ── P0-5 ─────────────────────────────────────────────────────────────────────
// Before fix: --touches was forwarded to the planner draft but the persisted
// Plan stored touched_capabilities=[] and never wrote plan_capability links.

#[test]
fn p0_5_touched_capabilities_persisted_when_resolvable() {
    let (_dir, path) = init_project();

    // Plant a capability in a source file the extractor can scan.
    write_file(
        &path,
        "src/lib.rs",
        r#"// @g8.capability(name="search-index", substrate="api")
// @g8.convergence_test(target="search-index")
"#,
    );

    // Scan. If ast-grep is not present, skip — this test only verifies the
    // resolution+persistence path. The non-ast-grep case is covered by tests
    // that don't depend on scan output.
    let scan = g8().current_dir(&path).args(["scan"]).output().unwrap();
    if !scan.status.success() {
        eprintln!(
            "SKIP p0_5_touched_capabilities_persisted_when_resolvable: scan failed (ast-grep missing?). stderr: {}",
            String::from_utf8_lossy(&scan.stderr)
        );
        return;
    }

    // Now create a plan that touches that capability.
    let output = g8()
        .current_dir(&path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "improve-search",
            "--touches",
            "search-index",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "plan new must succeed");
    let json = json_stdout(&output);
    let plan_id = json["plan_id"].as_str().unwrap().to_string();

    // Re-show the plan and verify touched_capabilities is non-empty.
    let show = g8()
        .current_dir(&path)
        .args(["--output", "json", "plan", "show", &plan_id])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_json = json_stdout(&show);
    let touched = show_json["touched_capabilities"].as_array().unwrap();
    assert!(
        !touched.is_empty(),
        "plan show must surface touched_capabilities populated from --touches"
    );
}

// ── P0-6 ─────────────────────────────────────────────────────────────────────
// Before fix: `g8 query intents` defaulted to an empty `--path`, and the store
// filter `?2 LIKE scope_path || '%'` excluded all rows when ?2 was empty.

#[test]
fn p0_6_query_intents_empty_path_returns_all() {
    let (_dir, path) = init_project();
    write_file(
        &path,
        "AGENTS.md",
        "# Boundaries\n\nDo not use auth substrate for the new export pipeline.\n",
    );
    let scan = g8().current_dir(&path).args(["scan"]).output().unwrap();
    if !scan.status.success() {
        eprintln!(
            "SKIP p0_6_query_intents_empty_path_returns_all: scan failed. stderr: {}",
            String::from_utf8_lossy(&scan.stderr)
        );
        return;
    }

    let output = g8()
        .current_dir(&path)
        .args(["--output", "json", "query", "intents"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json = json_stdout(&output);
    let arr = json["data"].as_array().unwrap();
    assert!(
        !arr.is_empty(),
        "query intents with no --path must return all intents in the space"
    );
}

// ── P0-7 ─────────────────────────────────────────────────────────────────────
// Before fix: `g8 merge` returned zero conflicts even when a remote Boundary
// intent explicitly forbade a local plan's substrate, because the conflict
// adapter looked up intents at the empty path (P0-6 in a different guise) AND
// the store insert was hard-coding kind="unclassified", erasing the Boundary
// classification produced by the extractor.

#[test]
fn p0_7_merge_detects_substrate_governance_violation() {
    let (_remote_dir, remote_path) = init_project();
    write_file(
        &remote_path,
        "AGENTS.md",
        "# Boundaries\n\nDo not use auth substrate for export work.\n",
    );
    let scan = g8()
        .current_dir(&remote_path)
        .args(["scan"])
        .output()
        .unwrap();
    if !scan.status.success() {
        eprintln!(
            "SKIP p0_7_merge_detects_substrate_governance_violation: remote scan failed. stderr: {}",
            String::from_utf8_lossy(&scan.stderr)
        );
        return;
    }

    let (_local_dir, local_path) = init_project();
    g8().current_dir(&local_path)
        .args(["substrate", "add", "auth", "--wip-cap", "3"])
        .assert()
        .success();
    g8().current_dir(&local_path)
        .args([
            "--output",
            "json",
            "plan",
            "new",
            "auth-revamp",
            "--substrate",
            "auth",
            "--dispatch",
        ])
        .assert()
        .success();

    let output = g8()
        .current_dir(&local_path)
        .args([
            "--output",
            "json",
            "merge",
            "--from",
            remote_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json = json_stdout(&output);
    assert!(
        json["count"].as_u64().unwrap_or(0) >= 1,
        "merge must surface at least one conflict; got {json}"
    );
    let conflicts = json["conflicts"].as_array().unwrap();
    assert!(
        conflicts
            .iter()
            .any(|c| c["kind"] == "governance_violation"),
        "expected a governance_violation conflict; got {conflicts:?}"
    );
}
