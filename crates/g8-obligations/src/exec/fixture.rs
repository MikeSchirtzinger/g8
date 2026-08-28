//! `FixtureIntegrationTest` — contract §1.1/§4.
//!
//! Runs `setup: Vec<CliInvocation>` against an isolated scratch tempdir
//! (never the real `workspace_root` — this backend's whole point is
//! exercising real CLI behavior without touching the audited repository),
//! then evaluates `assertions` against the captured per-step outcomes and
//! the fixture directory's final state.
//!
//! `binary` is resolved via `std::env::current_exe()` in production (this
//! code runs *as part of* the already-running `g8` process — the same
//! subprocess-boundary trick `g8-extractor` uses for `ast-grep`, spawning
//! itself instead of a third-party tool). Unit tests inject an explicit
//! `binary` instead — see contract §4's testing-strategy note.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::backend::{Assertion, FixtureTestArgs, SeedFile};
use crate::proc::{run_invocation, snapshot_file_hashes, CapturedStep};
use crate::result::ObligationStatus;

use super::json_path::resolve_json_path;

struct StepOutcome {
    captured: CapturedStep,
    /// Content-hash snapshot of every file under the fixture dir, taken
    /// immediately after this step completed.
    hashes: BTreeMap<String, u64>,
}

pub(crate) fn run(args: &FixtureTestArgs, workspace_root: &Path) -> (ObligationStatus, Value) {
    let binary = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return (
                ObligationStatus::Error,
                json!({ "error": format!("current_exe: {e}") }),
            )
        }
    };
    let dir = match crate::scratch::ScratchDir::new() {
        Ok(d) => d,
        Err(e) => {
            return (
                ObligationStatus::Error,
                json!({ "error": format!("scratch dir: {e}") }),
            )
        }
    };
    run_with_binary(&binary, workspace_root, args, dir.path())
}

fn run_with_binary(
    binary: &Path,
    workspace_root: &Path,
    args: &FixtureTestArgs,
    fixture_dir: &Path,
) -> (ObligationStatus, Value) {
    if let Err(e) = write_seed_files(&args.seed_files, fixture_dir) {
        return (
            ObligationStatus::Error,
            json!({ "error": format!("failed to write seed_files: {e}") }),
        );
    }

    let mut steps: Vec<StepOutcome> = Vec::new();
    for (i, inv) in args.setup.iter().enumerate() {
        match run_invocation(binary, workspace_root, inv, fixture_dir) {
            Ok(captured) => {
                let hashes = snapshot_file_hashes(fixture_dir);
                steps.push(StepOutcome { captured, hashes });
            }
            Err(e) => {
                return (
                    ObligationStatus::Error,
                    json!({ "error": format!("setup step {i} failed to spawn: {e}") }),
                )
            }
        }
    }

    let mut failed: Vec<Value> = Vec::new();
    for (i, assertion) in args.assertions.iter().enumerate() {
        match eval_assertion(assertion, &steps, fixture_dir) {
            Ok(true) => {}
            Ok(false) => failed.push(json!({ "index": i, "assertion": format!("{assertion:?}") })),
            Err(e) => {
                return (
                    ObligationStatus::Error,
                    json!({ "error": format!("assertion {i} could not be evaluated: {e}") }),
                )
            }
        }
    }

    let step_summary: Vec<Value> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "index": i,
                "exit_code": s.captured.exit_code,
                "duration_ms": s.captured.duration_ms,
            })
        })
        .collect();

    if failed.is_empty() {
        (
            ObligationStatus::Passed,
            json!({ "steps": step_summary, "assertions_checked": args.assertions.len() }),
        )
    } else {
        (
            ObligationStatus::Failed,
            json!({ "steps": step_summary, "failed_assertions": failed }),
        )
    }
}

/// Write every `seed_files` entry into `fixture_dir` before `setup[0]` runs
/// (contract §1.2, T2e). Parent directories created as needed — D4-01's own
/// seed set writes `src/auth/AGENTS.md`, requiring `src/auth/` to exist
/// first.
fn write_seed_files(seed_files: &[SeedFile], fixture_dir: &Path) -> std::io::Result<()> {
    for seed in seed_files {
        let target = fixture_dir.join(&seed.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &seed.content)?;
    }
    Ok(())
}

// ── Assertion evaluation — this backend's actual "logic" (contract §4's
// testing-strategy note: this is what T3's unit tests should exercise
// directly, independent of spawning any real `g8` binary) ────────────────

fn eval_assertion(
    a: &Assertion,
    steps: &[StepOutcome],
    fixture_dir: &Path,
) -> Result<bool, String> {
    match a {
        Assertion::FileExists { path } => Ok(fixture_dir.join(path).exists()),

        Assertion::FileHashUnchanged {
            path,
            before_step,
            after_step,
        } => {
            let before = step(steps, *before_step)?.hashes.get(path).copied();
            let after = step(steps, *after_step)?.hashes.get(path).copied();
            // Only a provable "existed at both points with the same content"
            // counts as unchanged; a file missing at either snapshot cannot
            // honestly be called unchanged.
            Ok(matches!((before, after), (Some(b), Some(a)) if b == a))
        }

        Assertion::FileHeaderEquals {
            path,
            expected_prefix,
        } => {
            match std::fs::read_to_string(fixture_dir.join(path)) {
                Ok(content) => Ok(content.starts_with(expected_prefix.as_str())),
                Err(_) => Ok(false), // missing file IS the (negative) finding, not a checker error
            }
        }

        Assertion::FileContains { path, needle } => {
            match std::fs::read_to_string(fixture_dir.join(path)) {
                Ok(content) => Ok(content.contains(needle.as_str())),
                Err(_) => Ok(false),
            }
        }

        Assertion::JsonField {
            from_step,
            path,
            equals,
        } => {
            let outcome = step(steps, *from_step)?;
            let Ok(value) = serde_json::from_str::<Value>(&outcome.captured.stdout) else {
                return Ok(false); // stdout wasn't valid JSON — a code defect, not a checker error
            };
            Ok(resolve_json_path(&value, path) == Some(equals))
        }

        Assertion::JsonPathNonEmpty { from_step, path } => {
            let outcome = step(steps, *from_step)?;
            let Ok(value) = serde_json::from_str::<Value>(&outcome.captured.stdout) else {
                return Ok(false);
            };
            Ok(match resolve_json_path(&value, path) {
                None | Some(Value::Null) => false,
                Some(Value::Array(a)) => !a.is_empty(),
                Some(Value::Object(o)) => !o.is_empty(),
                Some(Value::String(s)) => !s.is_empty(),
                Some(_) => true,
            })
        }

        Assertion::ExitCode { from_step, equals } => {
            Ok(step(steps, *from_step)?.captured.exit_code == *equals)
        }

        Assertion::StderrFormat {
            from_step,
            json_lines,
        } => {
            let outcome = step(steps, *from_step)?;
            let lines: Vec<&str> = outcome
                .captured
                .stderr
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect();
            let all_json = lines
                .iter()
                .all(|l| serde_json::from_str::<Value>(l).is_ok());
            Ok(if *json_lines { all_json } else { !all_json })
        }

        Assertion::StdoutContainsAll { from_step, needles } => {
            let outcome = step(steps, *from_step)?;
            Ok(needles
                .iter()
                .all(|n| contains_word(&outcome.captured.stdout, n)))
        }

        Assertion::StdoutNotContains { from_step, needle } => {
            let outcome = step(steps, *from_step)?;
            Ok(!contains_word(&outcome.captured.stdout, needle))
        }

        Assertion::NoAnnotationSourceFile { basename } => {
            // Generic, implementation-defined interpretation (the contract
            // does not pin exact semantics for this assertion): every
            // step's captured stdout, if JSON, must not reference a path
            // whose basename is `basename` anywhere in its string leaves.
            for outcome in steps {
                let Ok(value) = serde_json::from_str::<Value>(&outcome.captured.stdout) else {
                    continue;
                };
                let mut strings = Vec::new();
                collect_json_strings(&value, &mut strings);
                for s in &strings {
                    if Path::new(s).file_name().and_then(|n| n.to_str()) == Some(basename.as_str())
                    {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }

        Assertion::SecondConnectionErrsAfterTimeout => second_connection_errs(fixture_dir),
    }
}

fn step(steps: &[StepOutcome], index: usize) -> Result<&StepOutcome, String> {
    steps.get(index).ok_or_else(|| {
        format!(
            "assertion references step {index} but only {} step(s) ran",
            steps.len()
        )
    })
}

fn collect_json_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect_json_strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| collect_json_strings(x, out)),
        _ => {}
    }
}

/// Whole-word substring match — contract §1.2 (T2b): `StdoutContainsAll`/
/// `StdoutNotContains` match each needle "as a word-boundary substring
/// match," not a bare `.contains()`. Matters for D7-01 specifically:
/// `StdoutNotContains{needle:"serve"}` must not spuriously match if some
/// unrelated word in `--help`'s output happened to contain "serve" as a
/// substring (e.g. a hypothetical "reserved" or "conserve"); a bare
/// substring check would be a silent false-positive-Failed risk.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let pattern = format!(r"\b{}\b", regex::escape(needle));
    regex::Regex::new(&pattern)
        .map(|re| re.is_match(haystack))
        .unwrap_or(false)
}

/// N8-01's genuine concurrency behavioral test: hold a write lock on one
/// connection to the fixture's `.g8/store.db` and assert a second
/// connection's concurrent write is rejected (SQLITE_BUSY) rather than
/// silently corrupting/blocking forever — real SQLite locking, not a
/// simulation, using a throwaway scratch table so this assertion is
/// self-contained (does not depend on the real schema having migrated).
fn second_connection_errs(fixture_dir: &Path) -> Result<bool, String> {
    let store_path = fixture_dir.join(".g8").join("store.db");
    if !store_path.exists() {
        return Err(format!(
            "no store at {}: a setup step must run `init` first",
            store_path.display()
        ));
    }

    let conn1 = rusqlite::Connection::open(&store_path).map_err(|e| e.to_string())?;
    conn1
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS g8_obligations_probe (x INTEGER);",
        )
        .map_err(|e| e.to_string())?;
    conn1
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(|e| e.to_string())?;
    conn1
        .execute("INSERT INTO g8_obligations_probe (x) VALUES (1)", [])
        .map_err(|e| e.to_string())?;
    // conn1 now holds the write lock and does NOT commit.

    let conn2 = rusqlite::Connection::open(&store_path).map_err(|e| e.to_string())?;
    conn2
        .execute_batch("PRAGMA busy_timeout=300;")
        .map_err(|e| e.to_string())?;
    let second_write = conn2.execute("INSERT INTO g8_obligations_probe (x) VALUES (2)", []);

    let _ = conn1.execute_batch("ROLLBACK;");

    Ok(second_write.is_err())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BinarySource, CliInvocation};

    fn cargo_bin() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
    }

    /// End-to-end regression test for T3d, run through the actual public
    /// entry point (`run_obligations`) against a **filtered artifact**
    /// containing only OBL-D4-01 — the same method T4f's targeted re-run
    /// used to isolate this obligation's failure from the full 27-obligation
    /// artifact. Loads OBL-D4-01's *actual* checker straight out of the
    /// shipped `specs/obligations-v0.1.json` (not a hand-copied
    /// approximation that could silently drift from the real file) and runs
    /// it against a real, freshly-(re)built `g8`.
    ///
    /// Every setup step's `binary` is rewritten from the artifact's real
    /// default (`BinarySource::CurrentExe`) to `CargoRun` before running —
    /// `current_exe()` (which `fixture::run` calls internally, with no
    /// override, since production `g8 check` genuinely IS the `g8`
    /// process) resolves to this crate's own test binary inside `cargo
    /// test`, not `g8` (contract §4's testing-strategy note). `CargoRun`
    /// is the same portable, no-prebuilt-binary-path substitution the
    /// existing `cargo_run_binary_source_*` tests above already use, and —
    /// because every setup step uses it — `fixture::run`'s own
    /// `current_exe()` result is never actually consulted, so routing
    /// through the real `run_obligations` public API here is exactly as
    /// faithful as calling `run_with_binary` directly. This is the only
    /// field touched — `seed_files`, `setup[].args`/`env`, and `assertions`
    /// are the literal artifact content, unmodified.
    ///
    /// Before the T3d fix (`resolve_json_path` didn't understand bracket-index
    /// paths like `"$.data[0].description"`), this failed assertion index 2
    /// unconditionally, regardless of query correctness — confirmed via the
    /// same method live against a freshly built `g8` during the
    /// investigation. This test pins that root cause permanently.
    #[test]
    fn obl_d4_01_real_artifact_passes_end_to_end() {
        use crate::artifact::{load_artifact, ObligationArtifact, ObligationChecker};
        use crate::backend::CheckerBackend;

        let artifact_path = workspace_root().join("specs/obligations-v0.1.json");
        let artifact = load_artifact(&artifact_path).expect("load real artifact");
        let mut d401 = artifact
            .obligations
            .iter()
            .find(|o| o.id == "OBL-D4-01")
            .expect("D4-01 present in the real artifact")
            .clone();
        let Some(ObligationChecker::Typed { checks }) = &mut d401.checker else {
            panic!("expected OBL-D4-01 to carry a typed checker");
        };
        let Some(CheckerBackend::FixtureIntegrationTest(args)) = checks.first_mut() else {
            panic!("expected FixtureIntegrationTest, got {checks:?}");
        };
        for inv in &mut args.setup {
            inv.binary = BinarySource::CargoRun {
                package: "g8".to_string(),
                features: vec![],
            };
        }

        // A genuinely filtered artifact — same shape as the full one, just
        // one obligation — run through the real public entry point.
        let filtered = ObligationArtifact {
            meta: artifact.meta.clone(),
            obligations: vec![d401],
            conflicts: artifact.conflicts.clone(),
            open_questions: artifact.open_questions.clone(),
        };
        let results = crate::run_obligations(&filtered, &workspace_root(), None);
        assert_eq!(results.len(), 1);
        let d401_result = &results[0];
        assert_eq!(d401_result.id, "OBL-D4-01");
        assert_eq!(
            d401_result.status,
            ObligationStatus::Passed,
            "OBL-D4-01 must pass end-to-end against the real binary: evidence: {:?}",
            d401_result.evidence
        );
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn cap(stdout: &str, stderr: &str, exit_code: i32) -> CapturedStep {
        CapturedStep {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
            duration_ms: 1,
        }
    }

    fn steps_from(captured: Vec<CapturedStep>) -> Vec<StepOutcome> {
        captured
            .into_iter()
            .map(|captured| StepOutcome {
                captured,
                hashes: BTreeMap::new(),
            })
            .collect()
    }

    // ── seed_files (T2e/T3c) ──

    #[test]
    fn write_seed_files_creates_file_with_exact_content() {
        let dir = tempfile::tempdir().unwrap();
        let seeds = vec![SeedFile {
            path: "src/lib.rs".to_string(),
            content: "// @g8.capability(name = \"test-cap\", status = \"in_flight\", substrate = \"test\")\npub fn foo() {}\n".to_string(),
        }];
        write_seed_files(&seeds, dir.path()).expect("must write");
        let written = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        assert_eq!(written, seeds[0].content);
    }

    #[test]
    fn write_seed_files_creates_nested_parent_dirs() {
        // Exact real-world shape (OBL-D4-01's own seed set): a file two
        // directories deep, neither of which exists yet.
        let dir = tempfile::tempdir().unwrap();
        let seeds = vec![SeedFile {
            path: "src/auth/AGENTS.md".to_string(),
            content: "# Boundaries\n\nAuth-specific boundary text.\n".to_string(),
        }];
        write_seed_files(&seeds, dir.path()).expect("must write");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/auth/AGENTS.md")).unwrap(),
            seeds[0].content
        );
    }

    #[test]
    fn write_seed_files_writes_multiple_files_independently() {
        let dir = tempfile::tempdir().unwrap();
        let seeds = vec![
            SeedFile {
                path: "AGENTS.md".to_string(),
                content: "# Boundaries\n\nRoot-level boundary text.\n".to_string(),
            },
            SeedFile {
                path: "src/auth/AGENTS.md".to_string(),
                content: "# Boundaries\n\nAuth-specific boundary text.\n".to_string(),
            },
        ];
        write_seed_files(&seeds, dir.path()).expect("must write");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
            seeds[0].content
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/auth/AGENTS.md")).unwrap(),
            seeds[1].content
        );
    }

    #[test]
    fn write_seed_files_empty_is_a_true_noop() {
        let dir = tempfile::tempdir().unwrap();
        write_seed_files(&[], dir.path()).expect("must succeed");
        // Nothing created — the dir is still empty.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn run_with_binary_writes_seed_files_before_setup_step_0() {
        // End-to-end through the real entry point: seed_files must exist on
        // disk by the time setup[0] runs. Uses `cargo` as the stand-in
        // binary (contract §4's testing-strategy note) — the assertion
        // checks the fixture directory's own state, not anything the stand-in
        // binary itself needed to observe.
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![SeedFile {
                path: "marker.txt".to_string(),
                content: "seeded-before-setup".to_string(),
            }],
            setup: vec![CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            assertions: vec![Assertion::FileExists {
                path: "marker.txt".to_string(),
            }],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("marker.txt")).unwrap(),
            "seeded-before-setup"
        );
    }

    #[test]
    fn run_with_binary_seed_files_absent_is_a_noop_matching_pre_t2e_behavior() {
        // Backward-compat proof at the executor level: an obligation that
        // never mentions seed_files (all 26 pre-T2e ones) must behave
        // identically to before this feature existed.
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            assertions: vec![Assertion::ExitCode {
                from_step: 0,
                equals: 0,
            }],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    // ── Assertion logic, unit-tested directly (no subprocess) ──

    #[test]
    fn file_exists_true_and_false() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert!(eval_assertion(
            &Assertion::FileExists {
                path: "a.txt".into()
            },
            &[],
            dir.path()
        )
        .unwrap());
        assert!(!eval_assertion(
            &Assertion::FileExists {
                path: "b.txt".into()
            },
            &[],
            dir.path()
        )
        .unwrap());
    }

    #[test]
    fn file_header_equals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "<!-- AUTO-GENERATED -->\nbody").unwrap();
        let a = Assertion::FileHeaderEquals {
            path: "a.md".into(),
            expected_prefix: "<!-- AUTO-GENERATED -->".into(),
        };
        assert!(eval_assertion(&a, &[], dir.path()).unwrap());
        let missing = Assertion::FileHeaderEquals {
            path: "missing.md".into(),
            expected_prefix: "x".into(),
        };
        assert!(!eval_assertion(&missing, &[], dir.path()).unwrap());
    }

    #[test]
    fn file_contains() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        let a = Assertion::FileContains {
            path: "a.txt".into(),
            needle: "world".into(),
        };
        assert!(eval_assertion(&a, &[], dir.path()).unwrap());
        let b = Assertion::FileContains {
            path: "a.txt".into(),
            needle: "zzz".into(),
        };
        assert!(!eval_assertion(&b, &[], dir.path()).unwrap());
    }

    #[test]
    fn json_field_equals_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(r#"{"enforcement":"off","n":3}"#, "", 0)]);
        let ok = Assertion::JsonField {
            from_step: 0,
            path: "enforcement".into(),
            equals: json!("off"),
        };
        assert!(eval_assertion(&ok, &steps, dir.path()).unwrap());
        let bad = Assertion::JsonField {
            from_step: 0,
            path: "enforcement".into(),
            equals: json!("on"),
        };
        assert!(!eval_assertion(&bad, &steps, dir.path()).unwrap());
        let nested = Assertion::JsonField {
            from_step: 0,
            path: "n".into(),
            equals: json!(3),
        };
        assert!(eval_assertion(&nested, &steps, dir.path()).unwrap());
    }

    #[test]
    fn json_field_malformed_stdout_is_failed_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap("not json", "", 0)]);
        let a = Assertion::JsonField {
            from_step: 0,
            path: "x".into(),
            equals: json!(1),
        };
        assert!(!eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    #[test]
    fn json_path_non_empty() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(r#"{"drift_hints":["a"],"empty_one":[]}"#, "", 0)]);
        let ok = Assertion::JsonPathNonEmpty {
            from_step: 0,
            path: "$.drift_hints".into(),
        };
        assert!(eval_assertion(&ok, &steps, dir.path()).unwrap());
        let empty = Assertion::JsonPathNonEmpty {
            from_step: 0,
            path: "$.empty_one".into(),
        };
        assert!(!eval_assertion(&empty, &steps, dir.path()).unwrap());
        let missing = Assertion::JsonPathNonEmpty {
            from_step: 0,
            path: "$.nope".into(),
        };
        assert!(!eval_assertion(&missing, &steps, dir.path()).unwrap());
    }

    // ── bracket-index path resolution (T3d root cause) ──
    //
    // OBL-D4-01's real assertion, byte-for-byte from `specs/obligations-v0.1.json`,
    // is `JsonField{from_step:2, path:"$.data[0].description", equals:"Auth-specific
    // boundary text."}` — the contract's own canonical mapping-table example (§2 row
    // 9). The original `resolve_json_path` only understood bare-numeric dot segments
    // (`"$.data.0.description"`) and treated `"data[0]"` as a literal (nonexistent)
    // object key, so this assertion could never resolve — `Some(equals)` was being
    // compared against an unconditional `None`, regardless of what the query
    // returned. This is the actual root cause behind D4-01's assertion-index-2
    // failure; the `{{fixture_dir}}`/`seed_files` mechanism (T3c) was already
    // correct, confirmed by direct SQLite inspection and by re-running the query
    // step's own output through `jq`/`python` by hand.

    #[test]
    fn json_field_bracket_index_resolves_array_element() {
        // Exact shape of the real D4-01 query-step stdout: an `g8 query --output
        // json` envelope whose `data` array holds intent records, most-specific
        // scope first (`ORDER BY scope_depth DESC`).
        let dir = tempfile::tempdir().unwrap();
        let stdout = r#"{"data":[{"description":"Auth-specific boundary text.","scope_depth":11},{"description":"Root-level boundary text.","scope_depth":9}]}"#;
        let steps = steps_from(vec![cap(stdout, "", 0)]);
        let a = Assertion::JsonField {
            from_step: 0,
            path: "$.data[0].description".into(),
            equals: json!("Auth-specific boundary text."),
        };
        assert!(
            eval_assertion(&a, &steps, dir.path()).unwrap(),
            "bracket-index path must resolve to the first array element's field"
        );
        // The second element, addressed the same way, must resolve to the
        // OTHER record — proves this indexes the array rather than always
        // returning the same (e.g. first-found) value.
        let b = Assertion::JsonField {
            from_step: 0,
            path: "$.data[1].description".into(),
            equals: json!("Root-level boundary text."),
        };
        assert!(eval_assertion(&b, &steps, dir.path()).unwrap());
        // A mismatched expectation at a valid index must still fail — bracket
        // support shouldn't accidentally make the assertion vacuously true.
        let c = Assertion::JsonField {
            from_step: 0,
            path: "$.data[0].description".into(),
            equals: json!("Root-level boundary text."),
        };
        assert!(!eval_assertion(&c, &steps, dir.path()).unwrap());
    }

    #[test]
    fn json_field_bracket_index_out_of_range_is_failed_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(r#"{"data":[{"x":1}]}"#, "", 0)]);
        let a = Assertion::JsonField {
            from_step: 0,
            path: "$.data[5].x".into(),
            equals: json!(1),
        };
        assert!(!eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    #[test]
    fn json_field_chained_bracket_indices_resolve() {
        // Not needed by any obligation today, but the segment grammar
        // shouldn't silently cap out at one bracket level.
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(r#"{"matrix":[[1,2],[3,4]]}"#, "", 0)]);
        let a = Assertion::JsonField {
            from_step: 0,
            path: "$.matrix[1][0]".into(),
            equals: json!(3),
        };
        assert!(eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    #[test]
    fn json_path_non_empty_bracket_index_prefix() {
        // JsonPathNonEmpty is only ever used on a bracket-free prefix
        // (`"$.data"`) in the real artifact, but the shared segment resolver
        // must handle a bracketed prefix too if one were ever written.
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(r#"{"data":[{"tags":["a","b"]}]}"#, "", 0)]);
        let ok = Assertion::JsonPathNonEmpty {
            from_step: 0,
            path: "$.data[0].tags".into(),
        };
        assert!(eval_assertion(&ok, &steps, dir.path()).unwrap());
    }

    #[test]
    fn exit_code_assertion() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap("", "", 0)]);
        assert!(eval_assertion(
            &Assertion::ExitCode {
                from_step: 0,
                equals: 0
            },
            &steps,
            dir.path()
        )
        .unwrap());
        assert!(!eval_assertion(
            &Assertion::ExitCode {
                from_step: 0,
                equals: 1
            },
            &steps,
            dir.path()
        )
        .unwrap());
    }

    #[test]
    fn stderr_format_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let json_steps = steps_from(vec![cap("", "{\"a\":1}\n{\"b\":2}\n", 0)]);
        let human_steps = steps_from(vec![cap("", "INFO g8 check running\n", 0)]);

        assert!(eval_assertion(
            &Assertion::StderrFormat {
                from_step: 0,
                json_lines: true
            },
            &json_steps,
            dir.path()
        )
        .unwrap());
        assert!(!eval_assertion(
            &Assertion::StderrFormat {
                from_step: 0,
                json_lines: true
            },
            &human_steps,
            dir.path()
        )
        .unwrap());
        assert!(eval_assertion(
            &Assertion::StderrFormat {
                from_step: 0,
                json_lines: false
            },
            &human_steps,
            dir.path()
        )
        .unwrap());
    }

    #[test]
    fn file_hash_unchanged_true_and_false() {
        let dir = tempfile::tempdir().unwrap();
        let mut h1 = BTreeMap::new();
        h1.insert("store.db".to_string(), 111u64);
        let mut h2_same = BTreeMap::new();
        h2_same.insert("store.db".to_string(), 111u64);
        let mut h2_diff = BTreeMap::new();
        h2_diff.insert("store.db".to_string(), 222u64);

        let steps_same = vec![
            StepOutcome {
                captured: cap("", "", 0),
                hashes: h1.clone(),
            },
            StepOutcome {
                captured: cap("", "", 0),
                hashes: h2_same,
            },
        ];
        let a = Assertion::FileHashUnchanged {
            path: "store.db".into(),
            before_step: 0,
            after_step: 1,
        };
        assert!(eval_assertion(&a, &steps_same, dir.path()).unwrap());

        let steps_diff = vec![
            StepOutcome {
                captured: cap("", "", 0),
                hashes: h1,
            },
            StepOutcome {
                captured: cap("", "", 0),
                hashes: h2_diff,
            },
        ];
        assert!(!eval_assertion(&a, &steps_diff, dir.path()).unwrap());
    }

    #[test]
    fn assertion_referencing_out_of_range_step_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let a = Assertion::ExitCode {
            from_step: 5,
            equals: 0,
        };
        assert!(eval_assertion(&a, &[], dir.path()).is_err());
    }

    #[test]
    fn no_annotation_source_file_true_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(
            r#"{"files":["src/lib.rs","src/main.rs"]}"#,
            "",
            0,
        )]);
        let a = Assertion::NoAnnotationSourceFile {
            basename: "INTENT_SUMMARY.md".into(),
        };
        assert!(eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    #[test]
    fn no_annotation_source_file_false_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(
            r#"{"annotations":[{"source":{"file":".g8/INTENT_SUMMARY.md"}}]}"#,
            "",
            0,
        )]);
        let a = Assertion::NoAnnotationSourceFile {
            basename: "INTENT_SUMMARY.md".into(),
        };
        assert!(!eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    // ── SecondConnectionErrsAfterTimeout: genuine SQLite locking ──

    #[test]
    fn second_connection_errs_after_timeout_real_lock_contention() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".g8")).unwrap();
        // A minimal, real SQLite file at the expected path — no full `g8 init`
        // needed since the assertion is self-contained (scratch table).
        rusqlite::Connection::open(dir.path().join(".g8").join("store.db"))
            .unwrap()
            .execute_batch("PRAGMA journal_mode=WAL;")
            .unwrap();
        let result = second_connection_errs(dir.path());
        assert_eq!(result, Ok(true));
    }

    #[test]
    fn second_connection_errs_error_path_no_store() {
        let dir = tempfile::tempdir().unwrap();
        let result = second_connection_errs(dir.path());
        assert!(result.is_err());
    }

    // ── Plumbing: real subprocess spawn + full run_with_binary flow (uses
    // `cargo` as the stand-in binary, per contract §4's testing-strategy
    // note — no `g8` binary available to this crate's own `cargo test`) ──

    #[test]
    fn run_with_binary_end_to_end_against_cargo_version() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            assertions: vec![Assertion::ExitCode {
                from_step: 0,
                equals: 0,
            }],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn run_with_binary_reports_failed_assertion() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--version".to_string()],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            assertions: vec![Assertion::ExitCode {
                from_step: 0,
                equals: 99,
            }],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
    }

    #[test]
    fn run_with_binary_error_path_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec![],
                env: BTreeMap::new(),
                ..Default::default()
            }],
            assertions: vec![],
        };
        let (status, detail) = run_with_binary(
            Path::new("/nonexistent/g8-binary"),
            &workspace_root(),
            &args,
            dir.path(),
        );
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    // ── StdoutContainsAll / StdoutNotContains (T2b) ──

    #[test]
    fn stdout_contains_all_passes_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap(
            "Usage: g8 [OPTIONS] <COMMAND>\n\nCommands:\n  init\n  scan\n",
            "",
            0,
        )]);
        let ok = Assertion::StdoutContainsAll {
            from_step: 0,
            needles: vec!["init".to_string(), "scan".to_string()],
        };
        assert!(eval_assertion(&ok, &steps, dir.path()).unwrap());

        let missing = Assertion::StdoutContainsAll {
            from_step: 0,
            needles: vec!["init".to_string(), "serve".to_string()],
        };
        assert!(!eval_assertion(&missing, &steps, dir.path()).unwrap());
    }

    #[test]
    fn stdout_contains_all_matches_whole_words_only() {
        // "reserve" contains "serve" as a bare substring but must NOT count
        // as a match — this is exactly the false-positive risk word-boundary
        // matching exists to avoid (contract §1.2, T2b).
        let dir = tempfile::tempdir().unwrap();
        let steps = steps_from(vec![cap("please reserve a table", "", 0)]);
        let a = Assertion::StdoutContainsAll {
            from_step: 0,
            needles: vec!["serve".to_string()],
        };
        assert!(!eval_assertion(&a, &steps, dir.path()).unwrap());
    }

    #[test]
    fn stdout_not_contains_passes_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let without = steps_from(vec![cap("Commands:\n  init\n  scan\n", "", 0)]);
        let with = steps_from(vec![cap("Commands:\n  init\n  serve\n", "", 0)]);
        let a = Assertion::StdoutNotContains {
            from_step: 0,
            needle: "serve".to_string(),
        };
        assert!(eval_assertion(&a, &without, dir.path()).unwrap());
        assert!(!eval_assertion(&a, &with, dir.path()).unwrap());
    }

    // ── BinarySource::CargoRun seam (T2b) — real `cargo run`, the only
    // consumer initially being D7-01's cross-feature-build --help check ──

    #[test]
    fn cargo_run_binary_source_passes_against_real_g8_cli_default_build() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--help".to_string()],
                env: BTreeMap::new(),
                binary: BinarySource::CargoRun {
                    package: "g8".to_string(),
                    features: vec![],
                },
            }],
            assertions: vec![
                Assertion::StdoutContainsAll {
                    from_step: 0,
                    needles: vec!["init".to_string(), "scan".to_string()],
                },
                Assertion::StdoutNotContains {
                    from_step: 0,
                    needle: "serve".to_string(),
                },
            ],
        };
        // current_exe param is irrelevant here — CargoRun always spawns
        // `cargo` regardless — but `run_with_binary` still needs one.
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn cargo_run_binary_source_sees_serve_with_the_feature_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--help".to_string()],
                env: BTreeMap::new(),
                binary: BinarySource::CargoRun {
                    package: "g8".to_string(),
                    features: vec!["serve".to_string()],
                },
            }],
            assertions: vec![Assertion::StdoutContainsAll {
                from_step: 0,
                needles: vec!["serve".to_string()],
            }],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn cargo_run_binary_source_error_path_nonexistent_package() {
        let dir = tempfile::tempdir().unwrap();
        let args = FixtureTestArgs {
            seed_files: vec![],
            setup: vec![CliInvocation {
                args: vec!["--help".to_string()],
                env: BTreeMap::new(),
                binary: BinarySource::CargoRun {
                    package: "this-package-does-not-exist-xyz".to_string(),
                    features: vec![],
                },
            }],
            assertions: vec![],
        };
        let (status, detail) = run_with_binary(&cargo_bin(), &workspace_root(), &args, dir.path());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }
}
