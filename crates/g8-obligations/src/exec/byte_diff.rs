//! `ByteDiffTwice` — contract §1.1/§9, semantics corrected by Erratum 4
//! (§2.6).
//!
//! OBL-D11-04's binding prose: *"run the same `g8 check --json` (or `g8
//! query`) command twice against an identical, >=2-element fixture store;
//! byte-diff the two stdout captures"* — ONE populated store, TWO reads of
//! it. Because both reads see the literally-identical input (same paths,
//! same timestamps, same IDs), the ONLY thing that can make the two stdout
//! captures diverge is genuine read-path nondeterminism — exactly the
//! `HashMap`/`HashSet` iteration-order hazard (DEF-11) the obligation's
//! `rule.params` names. (`HashMap`'s SipHash seed is re-randomized per
//! process, and each `compare` run is its own process — so unordered
//! iteration feeding serialized output WILL eventually flip between runs.)
//!
//! The pre-Erratum-4 shape (two independently-bootstrapped branches) was a
//! misreading of the obligation: two fresh stores are NOT "an identical
//! fixture store" — each legitimately embeds its own absolute scope paths
//! and wall-clock timestamps, so the comparison failed on environment
//! noise (first divergence: the scratch dirs' own paths) regardless of
//! whether any real ordering bug existed. See contract §2.6 for the full
//! adjudication and evidence.
//!
//! Flow: create ONE scratch fixture → write `seed_files` → run `setup`
//! once → run `compare` TWICE (run 1's process exits before run 2 starts,
//! contract §9 point 5) → byte-compare stdout after masking only the
//! declared timing fields (never ordering, never content). Empty
//! `setup`+`seed_files` = no bootstrap; `compare` runs twice against
//! `workspace_root` itself.

use std::path::Path;

use serde_json::Value;

use crate::backend::ByteDiffArgs;
use crate::proc::run_invocation;
use crate::result::ObligationStatus;

pub(crate) fn run(
    args: &ByteDiffArgs,
    workspace_root: &Path,
) -> (ObligationStatus, serde_json::Value) {
    let binary = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": format!("current_exe: {e}") }),
            )
        }
    };
    run_with_binary(&binary, args, workspace_root)
}

fn run_with_binary(
    binary: &Path,
    args: &ByteDiffArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    // ONE fixture; the guard stays alive until both compare runs finish.
    let (_guard, fixture) = match bootstrap_fixture(binary, args, workspace_root) {
        Ok(x) => x,
        Err(e) => return (ObligationStatus::Error, serde_json::json!({ "error": e })),
    };

    // Run 1 completes fully (process exit) before run 2 spawns — §9 point 5.
    let captured_1 = match run_invocation(binary, workspace_root, &args.compare, &fixture) {
        Ok(c) => c,
        Err(e) => {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": e.to_string(), "run": 1, "step": "compare" }),
            )
        }
    };
    let captured_2 = match run_invocation(binary, workspace_root, &args.compare, &fixture) {
        Ok(c) => c,
        Err(e) => {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": e.to_string(), "run": 2, "step": "compare" }),
            )
        }
    };

    match compare_outputs(
        &captured_1.stdout,
        &captured_2.stdout,
        &args.normalize_paths,
    ) {
        Ok((true, _)) => (
            ObligationStatus::Passed,
            serde_json::json!({
                "shape": "same_store_read_twice",
                "seeded_files": args.seed_files.len(),
                "setup_steps": args.setup.len(),
            }),
        ),
        Ok((false, diff)) => (ObligationStatus::Failed, diff),
        Err(e) => (ObligationStatus::Error, serde_json::json!({ "error": e })),
    }
}

/// Empty `setup` AND empty `seed_files` ⇒ no bootstrap; `compare` reads
/// `workspace_root` itself twice. Otherwise: one fresh scratch dir, seed
/// files written first (parent dirs created), then `setup` run once in
/// order.
///
/// Returns the scratch guard (kept alive by the caller so the directory
/// outlives both compare runs) alongside the resolved fixture path.
fn bootstrap_fixture(
    binary: &Path,
    args: &ByteDiffArgs,
    workspace_root: &Path,
) -> Result<(Option<crate::scratch::ScratchDir>, std::path::PathBuf), String> {
    if args.setup.is_empty() && args.seed_files.is_empty() {
        return Ok((None, workspace_root.to_path_buf()));
    }
    let dir = crate::scratch::ScratchDir::new().map_err(|e| format!("scratch dir: {e}"))?;
    for seed in &args.seed_files {
        let target = dir.path().join(&seed.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("seed_files mkdir {}: {e}", parent.display()))?;
        }
        std::fs::write(&target, &seed.content)
            .map_err(|e| format!("seed_files write {}: {e}", target.display()))?;
    }
    for (i, inv) in args.setup.iter().enumerate() {
        run_invocation(binary, workspace_root, inv, dir.path())
            .map_err(|e| format!("setup step {i} failed to spawn: {e}"))?;
    }
    let path = dir.path().to_path_buf();
    Ok((Some(dir), path))
}

// ── Pure comparison logic — this backend's actual "logic" (contract §4's
// testing-strategy note); unit-tested directly with synthetic stdout
// strings, independent of any subprocess ─────────────────────────────────

/// Returns `Ok((true, _))` if, after masking, both outputs are identical;
/// `Ok((false, diff_detail))` if they genuinely diverge; `Err` if either
/// stdout isn't valid JSON or a `normalize_paths` entry uses unsupported
/// syntax (a checker-level problem, not a claim about the code).
fn compare_outputs(
    stdout_a: &str,
    stdout_b: &str,
    normalize_paths: &[String],
) -> Result<(bool, Value), String> {
    let masked_a = mask_and_serialize(stdout_a, normalize_paths)?;
    let masked_b = mask_and_serialize(stdout_b, normalize_paths)?;
    if masked_a == masked_b {
        Ok((true, Value::Null))
    } else {
        Ok((false, diverges_at(&masked_a, &masked_b)))
    }
}

/// Supported `normalize_paths` syntax: `$..<key>` (recursive-descent
/// mask-by-key — the only form contract §9's own example uses). Anything
/// else is a checker error, never silently ignored (masking is a narrow,
/// explicit allowlist per contract §9 point 3 — never used to hide ordering
/// or content).
fn mask_and_serialize(stdout: &str, normalize_paths: &[String]) -> Result<String, String> {
    let mut value: Value =
        serde_json::from_str(stdout).map_err(|e| format!("stdout is not valid JSON: {e}"))?;
    for np in normalize_paths {
        let key = np.strip_prefix("$..").ok_or_else(|| {
            format!("unsupported normalize_paths syntax `{np}`: only `$..<key>` is supported")
        })?;
        mask_key_recursive(&mut value, key);
    }
    Ok(serde_json::to_string_pretty(&value).expect("serde_json::Value always serializes"))
}

fn mask_key_recursive(value: &mut Value, key: &str) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == key {
                    *v = Value::String("<normalized>".to_string());
                } else {
                    mask_key_recursive(v, key);
                }
            }
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                mask_key_recursive(v, key);
            }
        }
        _ => {}
    }
}

/// First byte offset where the two (already-masked) strings diverge, with
/// surrounding context on both sides — "naming the specific byte
/// offset/field where the two runs diverge" per contract §9.
fn diverges_at(a: &str, b: &str) -> Value {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let min_len = ab.len().min(bb.len());
    let mut i = 0usize;
    while i < min_len && ab[i] == bb[i] {
        i += 1;
    }
    let context = |s: &str, at: usize| -> String {
        let start = at.saturating_sub(40);
        let end = (at + 40).min(s.len());
        // `get` is char-boundary-safe; a mid-character offset just yields no
        // context rather than panicking.
        (start..=end)
            .rev()
            .find_map(|e| s.get(start..e))
            .unwrap_or("")
            .to_string()
    };
    serde_json::json!({
        "byte_offset": i,
        "length_a": ab.len(),
        "length_b": bb.len(),
        "context_a": context(a, i),
        "context_b": context(b, i),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{CliInvocation, SeedFile};
    use std::collections::BTreeMap;

    fn cargo_bin() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
    }

    // ── Pure comparison logic ──

    #[test]
    fn identical_json_passes() {
        let a = r#"{"g8_version":"0.1.0","errors":[]}"#;
        let b = r#"{"g8_version":"0.1.0","errors":[]}"#;
        let (equal, _) = compare_outputs(a, b, &[]).unwrap();
        assert!(equal);
    }

    #[test]
    fn masks_declared_timing_field_before_comparing() {
        let a = r#"{"g8_version":"0.1.0","duration_ms":42}"#;
        let b = r#"{"g8_version":"0.1.0","duration_ms":999}"#;
        let (equal, _) = compare_outputs(a, b, &["$..duration_ms".to_string()]).unwrap();
        assert!(equal);
    }

    #[test]
    fn masks_nested_timing_fields_recursively() {
        let a =
            r#"{"obligations":[{"evidence":{"duration_ms":1}},{"evidence":{"duration_ms":2}}]}"#;
        let b =
            r#"{"obligations":[{"evidence":{"duration_ms":99}},{"evidence":{"duration_ms":100}}]}"#;
        let (equal, _) = compare_outputs(a, b, &["$..duration_ms".to_string()]).unwrap();
        assert!(equal);
    }

    #[test]
    fn genuine_array_order_difference_is_never_masked_away() {
        // This is the exact class of bug (OBL-D11-04 / HashMap iteration
        // order) this backend exists to catch — array order must NEVER be
        // silently normalized, even with normalize_paths set.
        let a = r#"{"errors":["x","y"]}"#;
        let b = r#"{"errors":["y","x"]}"#;
        let (equal, diff) = compare_outputs(a, b, &["$..duration_ms".to_string()]).unwrap();
        assert!(!equal, "diff: {diff}");
    }

    #[test]
    fn diverges_at_reports_a_sensible_byte_offset() {
        let a = r#"{"x":"aaa"}"#;
        let b = r#"{"x":"bbb"}"#;
        let (equal, diff) = compare_outputs(a, b, &[]).unwrap();
        assert!(!equal);
        assert!(diff["byte_offset"].as_u64().unwrap() > 0);
    }

    #[test]
    fn malformed_json_stdout_is_a_checker_error_not_a_silent_pass() {
        let result = compare_outputs("not json", "not json", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn unsupported_normalize_paths_syntax_is_a_checker_error() {
        let a = r#"{"x":1}"#;
        let result = compare_outputs(a, a, &["$.x.y".to_string()]); // not `$..` recursive form
        assert!(result.is_err());
    }

    // ── Plumbing: real subprocess spawn + full run_with_binary flow
    // (`cargo` stand-in per contract §4's testing-strategy note) ──

    #[test]
    fn empty_setup_runs_compare_twice_against_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        let args = ByteDiffArgs {
            seed_files: vec![],
            setup: vec![],
            compare: CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            },
            normalize_paths: vec![],
        };
        // `cargo --version` output isn't JSON, so the pure comparator would
        // error — that's expected and fine for a plumbing-only test; assert
        // the run at least reaches the comparison stage (Error from JSON
        // parsing, not from a spawn failure) to prove both runs spawned.
        let (status, detail) = run_with_binary(&cargo_bin(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
        assert!(detail["error"].as_str().unwrap().contains("not valid JSON"));
    }

    #[test]
    fn nonempty_setup_bootstraps_one_fixture_then_compares_twice() {
        let dir = tempfile::tempdir().unwrap();
        let args = ByteDiffArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            compare: CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            },
            normalize_paths: vec![],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &args, dir.path());
        // Same reasoning as above: reaching the JSON-parse stage proves
        // setup ran and both compare runs spawned against the one fixture.
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn seed_files_are_written_into_the_fixture_before_setup() {
        let dir = tempfile::tempdir().unwrap();
        let args = ByteDiffArgs {
            seed_files: vec![SeedFile {
                path: "src/lib.rs".to_string(),
                content: "// seeded\n".to_string(),
            }],
            // No setup steps — seeding alone must still force a scratch
            // fixture (not workspace_root).
            setup: vec![],
            compare: CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            },
            normalize_paths: vec![],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &args, dir.path());
        // Reaching JSON-parse proves the bootstrap (including the seed
        // write, whose failure would surface as a bootstrap Error with a
        // different message) succeeded.
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
        assert!(detail["error"].as_str().unwrap().contains("not valid JSON"));
    }

    #[test]
    fn error_path_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let args = ByteDiffArgs {
            seed_files: vec![],
            setup: vec![],
            compare: CliInvocation {
                args: vec![],
                env: BTreeMap::new(),
                ..Default::default()
            },
            normalize_paths: vec![],
        };
        let (status, _) = run_with_binary(Path::new("/nonexistent/binary"), &args, dir.path());
        assert_eq!(status, ObligationStatus::Error);
    }

    #[test]
    fn error_path_setup_step_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let args = ByteDiffArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec![],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            compare: CliInvocation {
                args: vec![],
                env: BTreeMap::new(),
                ..Default::default()
            },
            normalize_paths: vec![],
        };
        let (status, detail) = run_with_binary(Path::new("/nonexistent/binary"), &args, dir.path());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }
}
