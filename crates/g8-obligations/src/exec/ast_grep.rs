//! `AstGrepNoMatch` + `AstGrepMatchCount` — contract §1.1.
//!
//! Shells out to `ast-grep run --pattern <p> --lang <l> --json=stream
//! <files...>` (never `ast-grep scan` with a persisted rule file — that's
//! `g8-extractor`'s job for the fixed annotation grammar; this backend
//! needs ad-hoc, per-obligation patterns). Glob/exclude_glob scoping is
//! resolved ourselves via the `ignore` crate (the same crate
//! `g8-extractor` already walks directory trees with) into a concrete file
//! list, which is then passed as explicit positional args — `ast-grep run`
//! itself has no `--glob`/`--exclude` flags.
//!
//! # Exit-code semantics (verified empirically against the installed
//! `ast-grep` binary — NOT the same convention as `g8-extractor`'s `scan`
//! subcommand, whose own doc comment says "exits 0 whether or not matches
//! were found")
//!
//! `ast-grep run` exits 0 when it finds at least one match, but exits **1**
//! both for "ran fine, found zero matches" AND for "a real error" (e.g. a
//! nonexistent file path) — those two cases are NOT distinguished by exit
//! code alone. Distinguishing signal: a genuine error also writes a
//! non-empty, `ERROR:`-prefixed message to stderr; a legitimate zero-match
//! run's stderr is empty. Since this module only ever passes file paths it
//! already confirmed exist (via its own glob expansion), the "missing file"
//! error case should not occur in practice — the stderr check is
//! defense-in-depth against everything else that could make exit=1 mean
//! something other than "zero matches" (permissions, unreadable files, a
//! future ast-grep behavior change).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::{AstGrepArgs, AstGrepCountArgs, AstGrepLang, CapturePredicate};
use crate::proc::run_tool;
use crate::result::ObligationStatus;

use super::globbing::{expand_glob, relativize};
use super::text_scan::find_function_line_span;

#[derive(Debug, Clone)]
struct AstMatch {
    /// Workspace-relative, forward-slash.
    file: String,
    /// 1-based.
    line: u32,
    text: String,
    /// `metaVariables.single.<NAME>.text`, keyed by capture name.
    captures: BTreeMap<String, String>,
}

// ── AstGrepNoMatch ───────────────────────────────────────────────────────────

pub(crate) fn no_match(args: &AstGrepArgs, workspace_root: &Path) -> (ObligationStatus, Value) {
    no_match_with_binary(Path::new("ast-grep"), args, workspace_root)
}

fn no_match_with_binary(
    ast_grep_bin: &Path,
    args: &AstGrepArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let files = match expand_glob(workspace_root, &args.glob, &args.exclude_glob) {
        Ok(f) => f,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let mut all_matches: Vec<AstMatch> = Vec::new();
    for pattern in &args.patterns {
        match run_ast_grep_pattern(ast_grep_bin, pattern, args.lang, &files, workspace_root) {
            Ok(m) => all_matches.extend(m),
            Err(e) => {
                return (
                    ObligationStatus::Error,
                    json!({ "error": e, "pattern": pattern }),
                )
            }
        }
    }

    let hits: Vec<&AstMatch> = all_matches
        .iter()
        .filter(|m| predicate_satisfied(&args.capture_predicate, m))
        .collect();

    if hits.is_empty() {
        (
            ObligationStatus::Passed,
            json!({ "patterns_checked": args.patterns.len(), "files_scanned": files.len() }),
        )
    } else {
        let sample: Vec<Value> = hits
            .iter()
            .take(20)
            .map(|m| json!({ "file": m.file, "line": m.line, "text": m.text }))
            .collect();
        (
            ObligationStatus::Failed,
            json!({ "match_count": hits.len(), "matches": sample }),
        )
    }
}

fn predicate_satisfied(pred: &Option<CapturePredicate>, m: &AstMatch) -> bool {
    match pred {
        None => true,
        Some(p) => match (&p.forbidden_substring, m.captures.get(&p.capture)) {
            (Some(sub), Some(text)) => text.contains(sub.as_str()),
            (Some(_), None) => false,
            (None, _) => true,
        },
    }
}

// ── AstGrepMatchCount ────────────────────────────────────────────────────────

pub(crate) fn match_count(
    args: &AstGrepCountArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    match_count_with_binary(Path::new("ast-grep"), args, workspace_root)
}

fn match_count_with_binary(
    ast_grep_bin: &Path,
    args: &AstGrepCountArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let files = match expand_glob(workspace_root, &args.glob, &[]) {
        Ok(f) => f,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let matches = match run_ast_grep_pattern(
        ast_grep_bin,
        &args.pattern,
        args.lang,
        &files,
        workspace_root,
    ) {
        Ok(m) => m,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let scoped: Vec<AstMatch> = match &args.scope {
        None => matches,
        Some(scope) => {
            let mut spans: Vec<(String, u32, u32)> = Vec::new();
            for f in &files {
                let Ok(content) = std::fs::read_to_string(f) else {
                    continue;
                };
                if let Some((start, end)) = find_function_line_span(&content, &scope.function) {
                    spans.push((relativize(f, workspace_root), start, end));
                }
            }
            if spans.is_empty() {
                return (
                    ObligationStatus::Error,
                    json!({ "error": format!("function `{}` not found in glob", scope.function) }),
                );
            }
            matches
                .into_iter()
                .filter(|m| {
                    spans
                        .iter()
                        .any(|(f, s, e)| *f == m.file && m.line >= *s && m.line <= *e)
                })
                .collect()
        }
    };

    let actual = scoped.len() as u32;
    let count_ok = args.expected.is_met_by(actual);
    let capture_ok = match &args.capture_equals {
        None => true,
        Some((cap, expected_val)) => scoped
            .iter()
            .all(|m| m.captures.get(cap) == Some(expected_val)),
    };

    let detail_matches: Vec<Value> = scoped
        .iter()
        .take(20)
        .map(|m| json!({ "file": m.file, "line": m.line, "captures": m.captures }))
        .collect();

    if count_ok && capture_ok {
        (
            ObligationStatus::Passed,
            json!({ "actual_count": actual, "expected": format!("{:?}", args.expected) }),
        )
    } else {
        (
            ObligationStatus::Failed,
            json!({
                "actual_count": actual,
                "expected": format!("{:?}", args.expected),
                "count_ok": count_ok,
                "capture_ok": capture_ok,
                "matches": detail_matches,
            }),
        )
    }
}

// ── Shared: subprocess invocation + NDJSON parsing ───────────────────────────

fn run_ast_grep_pattern(
    ast_grep_bin: &Path,
    pattern: &str,
    lang: AstGrepLang,
    files: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<AstMatch>, String> {
    if files.is_empty() {
        // Passing zero path args would make ast-grep default to searching
        // the CURRENT directory (`[PATHS]... [default: .]`) — the opposite
        // of "nothing matched the glob". Short-circuit instead.
        return Ok(vec![]);
    }
    let mut owned_args: Vec<String> = vec![
        "run".to_string(),
        "--pattern".to_string(),
        pattern.to_string(),
        "--lang".to_string(),
        lang.as_ast_grep_arg().to_string(),
        "--json=stream".to_string(),
    ];
    for f in files {
        owned_args.push(f.to_string_lossy().into_owned());
    }
    let arg_refs: Vec<&str> = owned_args.iter().map(String::as_str).collect();

    let captured = run_tool(ast_grep_bin, &arg_refs, None).map_err(|e| e.to_string())?;
    match captured.exit_code {
        0 => parse_ndjson(&captured.stdout, workspace_root),
        1 if captured.stderr.trim().is_empty() => Ok(vec![]),
        1 => Err(format!("ast-grep error: {}", captured.stderr.trim())),
        other => Err(format!(
            "ast-grep exited {other}: {}",
            captured.stderr.trim()
        )),
    }
}

#[derive(Deserialize)]
struct RawMatch {
    text: String,
    range: RawRange,
    file: String,
    #[serde(rename = "metaVariables", default)]
    meta_variables: Option<RawMetaVars>,
}

#[derive(Deserialize)]
struct RawRange {
    start: RawPos,
}

#[derive(Deserialize)]
struct RawPos {
    line: usize,
}

#[derive(Deserialize, Default)]
struct RawMetaVars {
    #[serde(default)]
    single: BTreeMap<String, RawCapture>,
}

#[derive(Deserialize)]
struct RawCapture {
    text: String,
}

fn parse_ndjson(stdout: &str, workspace_root: &Path) -> Result<Vec<AstMatch>, String> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let raw: RawMatch = serde_json::from_str(line)
            .map_err(|e| format!("failed to parse ast-grep JSON line: {e}"))?;
        let captures = raw
            .meta_variables
            .map(|mv| mv.single.into_iter().map(|(k, v)| (k, v.text)).collect())
            .unwrap_or_default();
        out.push(AstMatch {
            file: relativize(Path::new(&raw.file), workspace_root),
            line: raw.range.start.line as u32 + 1, // ast-grep reports 0-based lines
            text: raw.text,
            captures,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{CountExpectation, FnScope};

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    // ── AstGrepNoMatch ──

    #[test]
    fn no_match_passes_when_pattern_absent() {
        let args = AstGrepArgs {
            patterns: vec!["ThisPatternDoesNotExistAnywhere12345()".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-core/src/**".to_string()],
            exclude_glob: vec![],
            capture_predicate: None,
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_match_fails_when_pattern_present() {
        // g8-store/src/store_impl.rs genuinely contains `tx.execute(...)` calls.
        let args = AstGrepArgs {
            patterns: vec!["tx.execute($$$ARGS)".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            exclude_glob: vec![],
            capture_predicate: None,
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert!(detail["match_count"].as_u64().unwrap() > 0);
    }

    #[test]
    fn no_match_exclude_glob_removes_the_hit() {
        let args = AstGrepArgs {
            patterns: vec!["tx.execute($$$ARGS)".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/**".to_string()],
            exclude_glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            capture_predicate: None,
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_match_capture_predicate_filters_hits() {
        // `self.lock().$METHOD(...)` calls exist (execute/query_row/
        // execute_batch) but none is ever named "ZZZ_NEVER_PRESENT_ZZZ" —
        // the predicate filters every raw match away -> Passed. Uses a
        // genuine SINGLE ($METHOD) capture, verified live against this file
        // (`execute`:14, `query_row`:7, `execute_batch`:1) — unlike a
        // variadic ($$$ARGS) capture, which never populates
        // `metaVariables.single` at all.
        let args = AstGrepArgs {
            patterns: vec!["self.lock().$METHOD($$$ARGS)".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            exclude_glob: vec![],
            capture_predicate: Some(CapturePredicate {
                capture: "METHOD".to_string(),
                forbidden_substring: Some("ZZZ_NEVER_PRESENT_ZZZ".to_string()),
            }),
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_match_capture_predicate_lets_a_real_hit_through() {
        // Same pattern, but the forbidden substring genuinely matches one of
        // the real method names -> the predicate lets those hits through.
        let args = AstGrepArgs {
            patterns: vec!["self.lock().$METHOD($$$ARGS)".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            exclude_glob: vec![],
            capture_predicate: Some(CapturePredicate {
                capture: "METHOD".to_string(),
                forbidden_substring: Some("query_row".to_string()),
            }),
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["match_count"], json!(7));
    }

    #[test]
    fn no_match_empty_glob_is_vacuously_passed_without_spawning() {
        let args = AstGrepArgs {
            patterns: vec!["anything".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/this-dir-does-not-exist/**".to_string()],
            exclude_glob: vec![],
            capture_predicate: None,
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["files_scanned"], json!(0));
    }

    #[test]
    fn no_match_error_path_missing_binary() {
        let args = AstGrepArgs {
            patterns: vec!["x".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-core/src/lib.rs".to_string()],
            exclude_glob: vec![],
            capture_predicate: None,
        };
        let (status, detail) = no_match_with_binary(
            Path::new("/nonexistent/ast-grep-binary"),
            &args,
            &workspace_root(),
        );
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn no_match_error_path_invalid_glob_syntax() {
        let args = AstGrepArgs {
            patterns: vec!["x".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["[".to_string()], // malformed glob
            exclude_glob: vec![],
            capture_predicate: None,
        };
        let (status, detail) = no_match(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    // ── AstGrepMatchCount ──

    #[test]
    fn match_count_passes_with_exact_expectation() {
        let args = AstGrepCountArgs {
            pattern: "tx.execute($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            scope: None,
            expected: CountExpectation::AtLeast { value: 1 },
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn match_count_fails_on_wrong_count() {
        let args = AstGrepCountArgs {
            pattern: "tx.execute($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            scope: None,
            expected: CountExpectation::Zero,
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
    }

    #[test]
    fn match_count_scopes_to_named_function() {
        // `apply_pragmas` contains exactly one `conn.execute_batch(...)` call;
        // scoping to a DIFFERENT function ("now", which has none) must find 0.
        let args = AstGrepCountArgs {
            pattern: "conn.execute_batch($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            scope: Some(FnScope {
                function: "apply_pragmas".to_string(),
            }),
            expected: CountExpectation::Exactly { value: 1 },
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");

        let args_wrong_scope = AstGrepCountArgs {
            scope: Some(FnScope {
                function: "now".to_string(),
            }),
            ..args
        };
        let (status2, detail2) = match_count(&args_wrong_scope, &workspace_root());
        assert_eq!(status2, ObligationStatus::Failed, "detail: {detail2}");
    }

    #[test]
    fn match_count_scopes_correctly_into_a_generic_function_past_a_misleading_doc_comment() {
        // The REAL OBL-D6-01 sub-check (a), end to end — not just the
        // isolated text_scan helper. `g8-planner::plan_check<S>`'s own doc
        // comment quotes a stale, non-generic signature as prose well before
        // the real definition; a comment-blind scope finder would anchor
        // there and miss the real call entirely (T2c erratum / T3b task
        // brief, confirmed live before this fix: scoped count was 0, not 1).
        let args = AstGrepCountArgs {
            pattern: "store.$METHOD($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-planner/src/lib.rs".to_string()],
            scope: Some(FnScope {
                function: "plan_check".to_string(),
            }),
            expected: CountExpectation::Exactly { value: 1 },
            capture_equals: Some(("METHOD".to_string(), "planner_intent_check".to_string())),
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn match_count_capture_equals_enforced() {
        // `self.lock().$METHOD(...)` is a genuine SINGLE capture (verified
        // live: execute=14, query_row=7, execute_batch=1) — none of them
        // equal "this-never-matches", so every match violates capture_equals.
        let args = AstGrepCountArgs {
            pattern: "self.lock().$METHOD($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            scope: None,
            expected: CountExpectation::AtLeast { value: 1 },
            capture_equals: Some(("METHOD".to_string(), "this-never-matches".to_string())),
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["capture_ok"], json!(false));
    }

    #[test]
    fn match_count_capture_equals_passes_on_synthetic_single_match_fixture() {
        // A controlled fixture with exactly one call and a known capture
        // value, so both the count AND the capture_equals check can be
        // asserted precisely (real-repo files mix method names, which would
        // make a "every match equals X" assertion incidentally fail).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fixture.rs"),
            "fn plan_check() {\n    store.planner_intent_check(x);\n}\n",
        )
        .unwrap();
        let args = AstGrepCountArgs {
            pattern: "store.$METHOD($$$ARGS)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["fixture.rs".to_string()],
            scope: None,
            expected: CountExpectation::Exactly { value: 1 },
            capture_equals: Some(("METHOD".to_string(), "planner_intent_check".to_string())),
        };
        let (status, detail) = match_count(&args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn match_count_error_path_scope_function_not_found() {
        let args = AstGrepCountArgs {
            pattern: "x".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-store/src/store_impl.rs".to_string()],
            scope: Some(FnScope {
                function: "this_fn_does_not_exist".to_string(),
            }),
            expected: CountExpectation::Zero,
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn match_count_error_path_missing_binary() {
        let args = AstGrepCountArgs {
            pattern: "x".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-core/src/lib.rs".to_string()],
            scope: None,
            expected: CountExpectation::Zero,
            capture_equals: None,
        };
        let (status, _) = match_count_with_binary(
            Path::new("/nonexistent/ast-grep-binary"),
            &args,
            &workspace_root(),
        );
        assert_eq!(status, ObligationStatus::Error);
    }
}
