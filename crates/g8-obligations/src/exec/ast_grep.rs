//! `AstGrepNoMatch` + `AstGrepMatchCount` — contract §1.1.
//!
//! Shells out to `ast-grep run --pattern <p> --lang <l> --json=stream
//! <files...>` for the per-obligation pattern, and, when `scope.function` is
//! set, to one `ast-grep scan --inline-rules <json> --json=stream
//! <files...>` that resolves the function's spans (see
//! `find_function_spans`). Never a persisted rule file: the inline rule is
//! built in memory from typed fields and passed as one argv element (a
//! rule file on disk is `g8-extractor`'s job for the fixed annotation
//! grammar). Glob/exclude_glob scoping is resolved ourselves via the
//! `ignore` crate (the same crate `g8-extractor` already walks directory
//! trees with) into a concrete file list, which is then passed as explicit
//! positional args — `ast-grep run` itself has no `--glob`/`--exclude`
//! flags.
//!
//! # Exit-code semantics (verified empirically against the installed
//! `ast-grep` binary, 0.45.3)
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
//!
//! `ast-grep scan` differs again: it exits 0 whether or not anything
//! matched, and ALSO exits 0 when it fails to read a path (the failure is
//! only an `ERROR:` line on stderr). A rule it cannot parse exits non-zero.
//! So the scope lookup treats any non-zero exit or any non-empty stderr as
//! an error, never as "function not present".
//!
//! # `scope.function` semantics
//!
//! The name is validated against `^[A-Za-z_$][A-Za-z0-9_$]*$` and
//! regex-escaped before it reaches the rule. Spans come from the node kinds
//! in `AstGrepLang::function_kinds` (free functions and methods in Rust,
//! Python, TypeScript and Go). A name defined more than once in the glob
//! scopes to the union of all its spans, every one listed in the detail's
//! `scope_spans`. A pattern match is in scope when its start line falls
//! inside a span. TypeScript arrow functions assigned to a `const` are not
//! resolved (known gap: they are a `variable_declarator`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::{AstGrepArgs, AstGrepCountArgs, AstGrepLang, CapturePredicate};
use crate::proc::run_tool;
use crate::result::ObligationStatus;

use super::globbing::{expand_glob, relativize};

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
    match_count_over_files(ast_grep_bin, args, &files, workspace_root)
}

/// `match_count` after glob expansion. The scope is resolved first, so an
/// invalid name or a failed span lookup is reported before the pattern runs.
fn match_count_over_files(
    ast_grep_bin: &Path,
    args: &AstGrepCountArgs,
    files: &[PathBuf],
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let spans: Option<Vec<FunctionSpan>> = match &args.scope {
        None => None,
        Some(scope) => {
            match find_function_spans(
                ast_grep_bin,
                args.lang,
                &scope.function,
                files,
                workspace_root,
            ) {
                Ok(spans) if spans.is_empty() => {
                    return (
                        ObligationStatus::Error,
                        json!({ "error": format!("function `{}` not found in glob", scope.function) }),
                    )
                }
                Ok(spans) => Some(spans),
                Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
            }
        }
    };

    let matches = match run_ast_grep_pattern(
        ast_grep_bin,
        &args.pattern,
        args.lang,
        files,
        workspace_root,
    ) {
        Ok(m) => m,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let scoped: Vec<AstMatch> = match &spans {
        None => matches,
        Some(spans) => matches
            .into_iter()
            .filter(|m| {
                spans
                    .iter()
                    .any(|(f, s, e)| *f == m.file && m.line >= *s && m.line <= *e)
            })
            .collect(),
    };
    let scope_spans: Option<Vec<Value>> = spans.as_ref().map(|spans| {
        spans
            .iter()
            .map(|(f, s, e)| json!({ "file": f, "start_line": s, "end_line": e }))
            .collect()
    });

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

    let (status, mut detail) = if count_ok && capture_ok {
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
    };
    if let Some(scope_spans) = scope_spans {
        detail["scope_spans"] = Value::Array(scope_spans);
    }
    (status, detail)
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

/// `(workspace-relative file, 1-based start line, 1-based end line)`.
type FunctionSpan = (String, u32, u32);

/// The `scope.function` name rule, `^[A-Za-z_$][A-Za-z0-9_$]*$`, checked
/// before the name reaches the inline rule's regex. Covers Rust, Python and
/// Go identifiers and the ASCII subset of JS/TS identifiers (`$` included).
fn validate_function_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$');
    let tail_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if head_ok && tail_ok {
        Ok(())
    } else {
        Err(format!(
            "scope.function `{name}` is not a valid function name: it must match ^[A-Za-z_$][A-Za-z0-9_$]*$"
        ))
    }
}

/// Every definition of function `name` in `files`, in ast-grep's output
/// order (file, then position). One `ast-grep scan --inline-rules <json>
/// --json=stream <files...>` call; the rule is built with `json!` from typed
/// fields only (`lang`'s [`AstGrepLang::function_kinds`] and the validated,
/// regex-escaped name) and passed as a single argv element, never written
/// to disk. Only the node kind and its `name` field are matched, so a
/// comment or string that quotes the signature is never a definition.
///
/// `scan` exits 0 even when it fails to read a path, reporting the failure
/// only as an `ERROR:` line on stderr, so non-empty stderr is an error
/// whatever the exit code.
fn find_function_spans(
    ast_grep_bin: &Path,
    lang: AstGrepLang,
    name: &str,
    files: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<FunctionSpan>, String> {
    validate_function_name(name)?;
    if files.is_empty() {
        // Same reason as `run_ast_grep_pattern`: no path args means `.`.
        return Ok(vec![]);
    }
    let kinds: Vec<Value> = lang
        .function_kinds()
        .iter()
        .map(|kind| json!({ "kind": kind }))
        .collect();
    let rule = json!({
        "id": "g8-scope-function",
        "language": lang.as_ast_grep_arg(),
        "rule": {
            "any": kinds,
            "has": { "field": "name", "regex": format!("^{}$", regex::escape(name)) },
        },
    });
    let mut owned_args: Vec<String> = vec![
        "scan".to_string(),
        "--inline-rules".to_string(),
        rule.to_string(),
        "--json=stream".to_string(),
    ];
    for f in files {
        owned_args.push(f.to_string_lossy().into_owned());
    }
    let arg_refs: Vec<&str> = owned_args.iter().map(String::as_str).collect();

    let captured = run_tool(ast_grep_bin, &arg_refs, None).map_err(|e| e.to_string())?;
    if captured.exit_code != 0 || !captured.stderr.trim().is_empty() {
        return Err(format!(
            "ast-grep scan (scope.function `{name}`) exited {}: {}",
            captured.exit_code,
            captured.stderr.trim()
        ));
    }
    let mut spans = Vec::new();
    for line in captured.stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let raw: RawMatch = serde_json::from_str(line)
            .map_err(|e| format!("failed to parse ast-grep scan JSON line: {e}"))?;
        spans.push((
            relativize(Path::new(&raw.file), workspace_root),
            raw.range.start.line as u32 + 1,
            raw.range.end.line as u32 + 1,
        ));
    }
    Ok(spans)
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
    end: RawPos,
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

    #[cfg(unix)]
    #[test]
    fn match_count_unreadable_file_is_error_not_zero() {
        // Issue #16: a file the checker cannot open must not read as "no match",
        // which passes a `zero` expectation and undercounts everything else.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sealed = dir.path().join("app.py");
        std::fs::write(&sealed, "def handle(x):\n    return store.save(x)\n").unwrap();
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o0)).unwrap();
        let args = AstGrepCountArgs {
            pattern: "store.save($$$A)".to_string(),
            lang: AstGrepLang::Python,
            glob: vec!["app.py".to_string()],
            scope: None,
            expected: CountExpectation::Zero,
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, dir.path());
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o644)).unwrap();
        if detail["error"]
            .as_str()
            .is_some_and(|e| e.contains("cannot read"))
        {
            assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
        } else {
            // root can read a mode-0 file; then the honest result is the real count.
            assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        }
    }

    // ── scope.function across languages ──
    //
    // Each fixture defines the target function plus a decoy that makes the
    // same call, so an exact count proves both that the span was found and
    // that matches outside it are dropped.

    fn scoped_fixture_count(
        lang: AstGrepLang,
        file_name: &str,
        source: &str,
        pattern: &str,
        function: &str,
        expected: u32,
    ) -> (ObligationStatus, Value) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file_name), source).unwrap();
        let args = AstGrepCountArgs {
            pattern: pattern.to_string(),
            lang,
            glob: vec![file_name.to_string()],
            scope: Some(FnScope {
                function: function.to_string(),
            }),
            expected: CountExpectation::Exactly { value: expected },
            capture_equals: None,
        };
        match_count(&args, dir.path())
    }

    #[test]
    fn scope_python_async_def() {
        let src = concat!(
            "def decoy():\n",
            "    helper(0)\n",
            "\n",
            "async def target(x):\n",
            "    await helper(x)\n",
            "    return helper(x)\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 2);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_python_annotated_def() {
        // The shape a `def $N($$$A): $$$B` pattern misses.
        let src = concat!(
            "def decoy() -> None:\n",
            "    helper(0)\n",
            "\n",
            "async def target(x: int, *, y: str = \"a\") -> dict[str, int]:\n",
            "    return helper(x)\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 1);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_python_decorated_def() {
        let src = concat!(
            "import functools\n",
            "\n",
            "@functools.cache\n",
            "@other(helper(9))\n",
            "def target(x):\n",
            "    return helper(x)\n",
            "\n",
            "def decoy():\n",
            "    helper(0)\n",
        );
        // The decorator's own `helper(9)` sits outside the def's span.
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 1);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_python_method() {
        let src = concat!(
            "class Service:\n",
            "    def decoy(self):\n",
            "        helper(0)\n",
            "\n",
            "    def target(self, x):\n",
            "        helper(x)\n",
            "        helper(x + 1)\n",
            "\n",
            "helper(2)\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 2);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_python_comment_mention_is_not_a_definition() {
        // A comment quoting `def target(` must not count as a span, so a
        // file with only the mention is "not found", not a silent zero.
        let src = "# def target(x): the old signature\ndef decoy():\n    helper(0)\n";
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 0);
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn scope_union_when_python_name_is_defined_twice_in_one_file() {
        // Decision D8c: every definition of the name is in scope.
        let src = concat!(
            "def target(x):\n",
            "    helper(1)\n",
            "\n",
            "def decoy():\n",
            "    helper(0)\n",
            "\n",
            "def target(x, y):\n",
            "    helper(2)\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", "target", 2);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(
            detail["scope_spans"],
            json!([
                { "file": "a.py", "start_line": 1, "end_line": 2 },
                { "file": "a.py", "start_line": 7, "end_line": 8 },
            ]),
            "detail: {detail}"
        );
    }

    #[test]
    fn scope_typescript_function() {
        let src = concat!(
            "function decoy(): void {\n",
            "  helper(0);\n",
            "}\n",
            "\n",
            "export async function target<T>(a: T): Promise<number> {\n",
            "  return helper(a);\n",
            "}\n",
        );
        let (status, detail) = scoped_fixture_count(
            AstGrepLang::TypeScript,
            "a.ts",
            src,
            "helper($A)",
            "target",
            1,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_typescript_class_method() {
        let src = concat!(
            "class Service {\n",
            "  decoy(): void {\n",
            "    helper(0);\n",
            "  }\n",
            "\n",
            "  async target(a: number): Promise<void> {\n",
            "    helper(a);\n",
            "    helper(a + 1);\n",
            "  }\n",
            "}\n",
        );
        let (status, detail) = scoped_fixture_count(
            AstGrepLang::TypeScript,
            "a.ts",
            src,
            "helper($A)",
            "target",
            2,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_typescript_dollar_name_is_escaped() {
        // `$` is legal in a JS identifier and a regex metacharacter: it must
        // be escaped, or `^$init$` would match nothing.
        let src = "function $init() {\n  helper(1);\n}\nfunction init() {\n  helper(0);\n}\n";
        let (status, detail) = scoped_fixture_count(
            AstGrepLang::TypeScript,
            "a.ts",
            src,
            "helper($A)",
            "$init",
            1,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_go_func() {
        let src = concat!(
            "package main\n",
            "\n",
            "func decoy() {\n",
            "\thelper(0)\n",
            "}\n",
            "\n",
            "func target(x int) (int, error) {\n",
            "\treturn helper(x), nil\n",
            "}\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Go, "a.go", src, "helper($A)", "target", 1);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_go_receiver_method() {
        let src = concat!(
            "package main\n",
            "\n",
            "type Recv struct{}\n",
            "\n",
            "func (r *Recv) decoy() {\n",
            "\thelper(0)\n",
            "}\n",
            "\n",
            "func (r *Recv) target(x int) {\n",
            "\thelper(x)\n",
            "\thelper(x + 1)\n",
            "}\n",
        );
        let (status, detail) =
            scoped_fixture_count(AstGrepLang::Go, "a.go", src, "helper($A)", "target", 2);
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_rust_doc_comment_trap() {
        // Moved from text_scan.rs with the old resolver. A doc comment that
        // quotes the function's signature as prose must not anchor the span
        // (the real plan_check bug; `§` and `…` kept from the original).
        let src = concat!(
            "//! ARCHITECTURE.md §3.4 specifies `pub fn plan_check(store: &dyn StoreConnection, …)`.\n",
            "//! The real signature differs.\n",
            "\n",
            "pub fn other() {\n",
            "    store.planner_intent_check(0);\n",
            "}\n",
            "\n",
            "/// Also mentions fn plan_check( in a doc comment.\n",
            "pub fn plan_check<S>(store: &S, draft: &PlanDraft) -> Result<FitReport, PlannerError>\n",
            "where\n",
            "    S: StoreConnection,\n",
            "{\n",
            "    let raw = store.planner_intent_check(draft)?;\n",
            "    Ok(raw)\n",
            "}\n",
        );
        let (status, detail) = scoped_fixture_count(
            AstGrepLang::Rust,
            "lib.rs",
            src,
            "store.planner_intent_check($A)",
            "plan_check",
            1,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(
            detail["scope_spans"],
            json!([{ "file": "lib.rs", "start_line": 9, "end_line": 15 }]),
            "detail: {detail}"
        );
    }

    #[test]
    fn scope_rust_where_clause() {
        // Generics, a lifetime and a where-clause together, as an impl
        // method next to a same-shaped free function decoy.
        let src = concat!(
            "fn decoy() {\n",
            "    store.planner_intent_check(0);\n",
            "}\n",
            "\n",
            "impl Planner {\n",
            "    fn complex<'a, S>(&self, store: &'a S) -> Result<(), E>\n",
            "    where\n",
            "        S: StoreConnection + 'a,\n",
            "    {\n",
            "        store.planner_intent_check(1)?;\n",
            "        Ok(())\n",
            "    }\n",
            "}\n",
        );
        let (status, detail) = scoped_fixture_count(
            AstGrepLang::Rust,
            "lib.rs",
            src,
            "store.planner_intent_check($A)",
            "complex",
            1,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn scope_real_plan_check_span_covers_the_real_call() {
        // Moved from text_scan.rs: the real g8-planner file, end to end.
        // `plan_check`'s one `store.planner_intent_check(draft)` call must
        // land inside the resolved span.
        let args = AstGrepCountArgs {
            pattern: "store.planner_intent_check($A)".to_string(),
            lang: AstGrepLang::Rust,
            glob: vec!["crates/g8-planner/src/lib.rs".to_string()],
            scope: Some(FnScope {
                function: "plan_check".to_string(),
            }),
            expected: CountExpectation::Exactly { value: 1 },
            capture_equals: None,
        };
        let (status, detail) = match_count(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    // ── scope.function negative controls ──

    #[test]
    fn scope_invalid_function_name_is_an_error_naming_the_rule() {
        // `def target` exists, so a zero count here would be the old silent
        // shape; each malformed name must be refused before any regex use.
        let src = "def target():\n    helper(1)\n";
        for bad in ["foo(", "a b", ".hidden", "", "target|decoy", "^target$"] {
            let (status, detail) =
                scoped_fixture_count(AstGrepLang::Python, "a.py", src, "helper($A)", bad, 0);
            assert_eq!(status, ObligationStatus::Error, "name {bad:?}: {detail}");
            let msg = detail["error"].as_str().unwrap();
            assert!(
                msg.contains("^[A-Za-z_$][A-Za-z0-9_$]*$"),
                "name {bad:?}: error must name the rule, got {msg}"
            );
        }
    }

    #[test]
    fn scope_valid_function_names_pass_validation() {
        for good in [
            "target",
            "_private",
            "$init",
            "a1",
            "__init__",
            "camelCase$",
        ] {
            assert!(validate_function_name(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn scope_ast_grep_scan_stderr_with_exit_zero_is_an_error() {
        // A file that vanishes between glob expansion and the scan: ast-grep
        // scan prints `ERROR: ... No such file or directory` and exits 0
        // while still reporting the span it found in the readable file.
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("a.py");
        std::fs::write(&present, "def target():\n    helper(1)\n").unwrap();
        let vanished = dir.path().join("gone.py");
        let args = AstGrepCountArgs {
            pattern: "helper($A)".to_string(),
            lang: AstGrepLang::Python,
            glob: vec!["*.py".to_string()],
            scope: Some(FnScope {
                function: "target".to_string(),
            }),
            expected: CountExpectation::Exactly { value: 1 },
            capture_equals: None,
        };
        let (status, detail) = match_count_over_files(
            Path::new("ast-grep"),
            &args,
            &[present, vanished],
            dir.path(),
        );
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
        let msg = detail["error"].as_str().unwrap();
        assert!(
            msg.contains("ast-grep scan") && msg.contains("exited 0"),
            "got {msg}"
        );
    }

    #[test]
    fn scope_ast_grep_missing_from_path_is_an_error() {
        // A bare program name is looked up on PATH, as `ast-grep` is.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "def target():\n    helper(1)\n").unwrap();
        let args = AstGrepCountArgs {
            pattern: "helper($A)".to_string(),
            lang: AstGrepLang::Python,
            glob: vec!["a.py".to_string()],
            scope: Some(FnScope {
                function: "target".to_string(),
            }),
            expected: CountExpectation::Zero,
            capture_equals: None,
        };
        let (status, detail) =
            match_count_with_binary(Path::new("g8-test-ast-grep-not-on-path"), &args, dir.path());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
        assert!(
            detail["error"]
                .as_str()
                .unwrap()
                .contains("failed to spawn"),
            "detail: {detail}"
        );
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
