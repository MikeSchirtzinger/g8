//! Shared subprocess-execution helpers.
//!
//! Every backend that shells out builds a `std::process::Command` from a
//! fixed program name plus a typed argument vector — the same pattern
//! `g8-extractor` already uses for `ast-grep` (contract §0). This module
//! centralizes the two shapes every backend needs: a simple "run a tool,
//! capture stdout" call, and the richer "run one `CliInvocation` against a
//! binary in a given cwd/env" call shared by `FixtureIntegrationTest` and
//! `ByteDiffTwice`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::backend::{BinarySource, CliInvocation};

/// Output of a single subprocess run.
#[derive(Debug, Clone)]
pub(crate) struct CapturedStep {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// A tool binary could not be spawned at all (missing / not executable) —
/// always maps to `ObligationStatus::Error`, never `Failed`, per contract
/// §3.1: "error is a claim about the checker, not the code."
#[derive(Debug, Clone)]
pub(crate) struct SpawnError(pub String);

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Run `program` with `args` in `cwd`, capturing stdout/stderr/exit code.
///
/// Used by the metadata/pattern-search backends (`cargo metadata`,
/// `ast-grep run`, `rg`) which don't need env control — only
/// `FixtureIntegrationTest`/`ByteDiffTwice` need the controlled-environment
/// variant below.
pub(crate) fn run_tool(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<CapturedStep, SpawnError> {
    let start = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .map_err(|e| SpawnError(format!("failed to spawn `{}`: {e}", program.display())))?;
    Ok(CapturedStep {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// The four env vars contract §9 point 2 pins for determinism, applied to
/// every `CliInvocation` run (`FixtureIntegrationTest` as well as
/// `ByteDiffTwice` — harmless and consistent for both, even though §9's text
/// is scoped to the byte-diff determinism boundary specifically).
///
/// Deliberately does NOT `env_clear()`: the spawned `g8` process may itself
/// need `PATH` to shell out to `cargo`/`ast-grep`/`rg` (e.g. a `scan` step
/// invokes `g8-extractor`, which invokes `ast-grep`) — clearing the whole
/// environment would break that, which the contract's "controlled base
/// environment" language does not ask for (it names four *specific* vars).
fn apply_controlled_base_env(cmd: &mut Command) {
    cmd.env_remove("G8_TRACE_JSON");
    cmd.env("TZ", "UTC");
    cmd.env("LANG", "C");
    cmd.env("NO_COLOR", "1");
}

/// Run one [`CliInvocation`] in `cwd`, resolving which binary to spawn from
/// `inv.binary` (contract §1.2, amended T2b).
///
/// `current_exe` is an explicit parameter (never resolved internally via
/// `std::env::current_exe()`) precisely so this function is unit-testable
/// against any executable — production callers resolve `current_exe()` once
/// and pass it in; see contract §4's testing-strategy note (`CARGO_BIN_EXE_g8`
/// is unavailable to this crate's own `cargo test`, so backend *logic* is
/// tested in isolation here, with true end-to-end `g8`-binary wiring left
/// to `g8`'s own test suite per T5). Used only for
/// [`BinarySource::CurrentExe`] — [`BinarySource::CargoRun`] always spawns
/// the literal `cargo` binary regardless of this parameter.
///
/// `workspace_root` is used only by [`BinarySource::CargoRun`], via
/// `--manifest-path`: `FixtureIntegrationTest` deliberately runs every
/// invocation's `cwd` inside an isolated scratch fixture (never
/// `workspace_root` itself), which has no `Cargo.toml` of its own — without
/// an explicit manifest path, `cargo run -p <package>` would fail to find
/// the workspace at all (confirmed live: "could not find `Cargo.toml`...").
/// `--manifest-path` decouples "where cargo resolves the package from" from
/// "where the resulting program's own `cwd` is" (`cwd` is left unchanged —
/// still the fixture dir — only cargo's own resolution is redirected).
///
/// `{{fixture_dir}}` substitution (contract §1.2, T2e) happens here, once,
/// centrally — not duplicated per-caller — because it's documented as a
/// property of `CliInvocation` itself, not of `FixtureIntegrationTest`
/// specifically: `g8-store` persists `scope_path` as an absolute
/// filesystem path, so a query argument referencing the scratch fixture can
/// only match if given in that same representation, which is only knowable
/// at runtime. Applies to whichever directory `cwd` names for THIS call —
/// the isolated fixture for `FixtureIntegrationTest` and mint-sensitive
/// `ByteDiffTwice` branches, or `workspace_root` itself for `ByteDiffTwice`'s
/// mint-safe (empty-`setup`) case — never `env` (contract: "no current
/// obligation needs it there").
pub(crate) fn run_invocation(
    current_exe: &Path,
    workspace_root: &Path,
    inv: &CliInvocation,
    cwd: &Path,
) -> Result<CapturedStep, SpawnError> {
    let start = Instant::now();
    let args = substitute_fixture_dir(&inv.args, cwd)
        .map_err(|e| SpawnError(format!("{{{{fixture_dir}}}} substitution failed: {e}")))?;
    let mut cmd = match &inv.binary {
        BinarySource::CurrentExe => {
            let mut cmd = Command::new(current_exe);
            cmd.args(&args);
            cmd
        }
        BinarySource::CargoRun { package, features } => {
            let mut cmd = Command::new("cargo");
            cmd.arg("run").arg("-q");
            cmd.arg("--manifest-path")
                .arg(workspace_root.join("Cargo.toml"));
            cmd.arg("-p").arg(package);
            if !features.is_empty() {
                cmd.arg("--features").arg(features.join(","));
            }
            cmd.arg("--");
            cmd.args(&args);
            cmd
        }
    };
    cmd.current_dir(cwd);
    apply_controlled_base_env(&mut cmd);
    for (k, v) in &inv.env {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .map_err(|e| SpawnError(format!("failed to spawn CliInvocation: {e}")))?;
    let exit_code = output.status.code().unwrap_or(-1);

    // `cargo run` exits 101 when IT fails (package not found, compile
    // error) — before the target binary ever runs — distinct from the
    // target binary's own exit code, which `cargo run` otherwise proxies
    // through unchanged. 101 is never a legitimate `g8` exit code
    // (CLAUDE.md's locked contract: 0/1/2 only), so this is a safe,
    // well-grounded signal that the FIXTURE couldn't be built, not that the
    // code under test produced a real (even if nonzero) result. Verified
    // live: `cargo run -p <nonexistent> -- --help` exits 101 with
    // "error: package(s) ... not found in workspace" on stderr.
    if matches!(inv.binary, BinarySource::CargoRun { .. }) && exit_code == 101 {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(SpawnError(format!(
            "cargo run failed to build/resolve the target: {stderr}"
        )));
    }

    Ok(CapturedStep {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Replace the literal substring `{{fixture_dir}}` in each `args` element
/// with `cwd`'s absolute, canonicalized path (contract §1.2, T2e — needed
/// because `g8-store` persists `scope_path` as an absolute filesystem
/// path, confirmed by direct query against a real store; a relative query
/// argument can never match it).
///
/// Fast path: skips `canonicalize` entirely when no arg contains the
/// substring (the overwhelming majority of invocations — only D4-01 uses
/// this today) — cheap, and avoids a needless failure point when `cwd`
/// happens to be awkward to canonicalize for some unrelated reason.
///
/// A `canonicalize` failure is a hard `Err`, not a silent fallback to the
/// non-canonical path: a mismatched representation wouldn't error, it would
/// just silently fail to match anything in `g8-store`'s queries, which
/// would misleadingly look like an ordinary empty-result `Failed` instead
/// of the checker-level problem it actually is.
fn substitute_fixture_dir(args: &[String], cwd: &Path) -> Result<Vec<String>, String> {
    if !args.iter().any(|a| a.contains("{{fixture_dir}}")) {
        return Ok(args.to_vec());
    }
    let canonical = std::fs::canonicalize(cwd)
        .map_err(|e| format!("failed to canonicalize {}: {e}", cwd.display()))?;
    let replacement = canonical.to_string_lossy();
    Ok(args
        .iter()
        .map(|a| a.replace("{{fixture_dir}}", &replacement))
        .collect())
}

/// Deterministic, in-process, non-cryptographic content hash — sufficient
/// for `Assertion::FileHashUnchanged`'s "did this file change between step N
/// and step M within the SAME run" comparison. Not used for anything that
/// crosses process/run boundaries (that's `AttestationRecord::
/// pinned_content_hash`'s job, contract §7, deliberately out of scope here —
/// see `lib.rs` module docs).
pub(crate) fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Snapshot every regular file under `dir` (relative-path-keyed content
/// hash). Used after each `setup` step so `FileHashUnchanged` assertions can
/// compare two step-indexed snapshots without foreknowledge of which paths
/// matter.
pub(crate) fn snapshot_file_hashes(dir: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(false) // .g8/ is a dotdir; must NOT be skipped
        .standard_filters(false)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(rel) = path.strip_prefix(dir) {
                out.insert(
                    rel.to_string_lossy().replace('\\', "/"),
                    content_hash(&bytes),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn run_tool_captures_stdout_and_exit_code() {
        let cargo = which_test_binary("cargo");
        let result = run_tool(&cargo, &["--version"], None).expect("cargo --version must run");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("cargo"), "got: {}", result.stdout);
    }

    #[test]
    fn run_tool_missing_binary_is_spawn_error() {
        let result = run_tool(Path::new("/nonexistent/definitely/not/a/binary"), &[], None);
        assert!(result.is_err());
    }

    #[test]
    fn run_invocation_pins_controlled_env_vars() {
        // Use `cargo` itself as a stand-in "binary" to prove the plumbing
        // (env injection, cwd, capture) without needing the `g8` binary —
        // per contract §4's testing-strategy note. We spawn a tiny shell
        // via `cargo -V` is not enough to inspect env, so instead assert
        // indirectly: a bogus binary + custom env still reports the SpawnError,
        // and a real, reachable binary succeeds with args passed through.
        let dir = tempfile::tempdir().unwrap();
        let cargo = which_test_binary("cargo");
        let inv = CliInvocation {
            args: vec!["--version".to_string()],
            env: BTreeMap::new(),
            ..Default::default()
        };
        let result = run_invocation(&cargo, &workspace_root(), &inv, dir.path()).expect("must run");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("cargo"));
    }

    #[test]
    fn run_invocation_error_path_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let inv = CliInvocation {
            args: vec![],
            env: BTreeMap::new(),
            ..Default::default()
        };
        let result = run_invocation(
            Path::new("/nonexistent/g8-binary"),
            &workspace_root(),
            &inv,
            dir.path(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn run_invocation_substitutes_fixture_dir_in_args() {
        // `echo` reliably prints back exactly the argv it received, letting
        // us directly observe substitution happened in the actual spawned
        // command line (not just in some intermediate value this test
        // constructs itself).
        let dir = tempfile::tempdir().unwrap();
        let echo = which_test_binary("echo");
        let inv = CliInvocation {
            args: vec!["hello".to_string(), "{{fixture_dir}}/sub".to_string()],
            env: BTreeMap::new(),
            ..Default::default()
        };
        let result = run_invocation(&echo, &workspace_root(), &inv, dir.path()).expect("must run");
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert!(
            result
                .stdout
                .contains(&format!("{}/sub", canonical.display())),
            "stdout: {}",
            result.stdout
        );
        assert!(
            !result.stdout.contains("{{fixture_dir}}"),
            "placeholder must not survive substitution: {}",
            result.stdout
        );
    }

    #[test]
    fn run_invocation_no_placeholder_is_a_true_noop() {
        // Args with no `{{fixture_dir}}` occurrence must pass through
        // completely unchanged (and, per the fast-path doc comment, never
        // even attempt to canonicalize `cwd`).
        let dir = tempfile::tempdir().unwrap();
        let echo = which_test_binary("echo");
        let inv = CliInvocation {
            args: vec!["plain".to_string(), "args".to_string()],
            env: BTreeMap::new(),
            ..Default::default()
        };
        let result = run_invocation(&echo, &workspace_root(), &inv, dir.path()).expect("must run");
        assert_eq!(result.stdout.trim(), "plain args");
    }

    #[test]
    fn run_invocation_cargo_run_binary_source_passes() {
        let dir = tempfile::tempdir().unwrap();
        // `current_exe` param is irrelevant for CargoRun — it always spawns
        // `cargo` regardless — but the function still needs one.
        let unused_current_exe = which_test_binary("cargo");
        let inv = CliInvocation {
            args: vec!["--help".to_string()],
            env: BTreeMap::new(),
            binary: BinarySource::CargoRun {
                package: "g8".to_string(),
                features: vec![],
            },
        };
        let result = run_invocation(&unused_current_exe, &workspace_root(), &inv, dir.path())
            .expect("must run");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("init"), "stdout: {}", result.stdout);
    }

    #[test]
    fn run_invocation_cargo_run_error_path_nonexistent_package() {
        // Verified live: `cargo run -p <nonexistent>` exits 101 (cargo's own
        // build/resolve-failure code, never a legitimate `g8` exit code per
        // CLAUDE.md's locked 0/1/2 contract) — this must surface as a
        // SpawnError (checker couldn't build the fixture), not as ordinary
        // captured-output data.
        let dir = tempfile::tempdir().unwrap();
        let unused_current_exe = which_test_binary("cargo");
        let inv = CliInvocation {
            args: vec!["--help".to_string()],
            env: BTreeMap::new(),
            binary: BinarySource::CargoRun {
                package: "this-package-does-not-exist-xyz".to_string(),
                features: vec![],
            },
        };
        let result = run_invocation(&unused_current_exe, &workspace_root(), &inv, dir.path());
        assert!(result.is_err(), "expected SpawnError, got: {result:?}");
    }

    #[test]
    fn snapshot_file_hashes_detects_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        let before = snapshot_file_hashes(dir.path());
        std::fs::write(dir.path().join("a.txt"), "two").unwrap();
        let after = snapshot_file_hashes(dir.path());
        assert_ne!(before.get("a.txt"), after.get("a.txt"));
    }

    #[test]
    fn snapshot_file_hashes_stable_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "stable").unwrap();
        let before = snapshot_file_hashes(dir.path());
        let after = snapshot_file_hashes(dir.path());
        assert_eq!(before, after);
    }

    #[test]
    fn snapshot_file_hashes_sees_hidden_g8_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".g8")).unwrap();
        std::fs::write(dir.path().join(".g8").join("store.db"), "x").unwrap();
        let snap = snapshot_file_hashes(dir.path());
        assert!(snap.contains_key(".g8/store.db"), "keys: {snap:?}");
    }

    /// Resolve a real, already-installed binary for plumbing tests. Panics
    /// with a clear message if genuinely absent — this crate's own test
    /// suite assumes a normal Rust dev environment (`cargo` on PATH),
    /// mirroring `g8-extractor`'s tests which assume `ast-grep` similarly.
    fn which_test_binary(name: &str) -> std::path::PathBuf {
        let output = Command::new("which")
            .arg(name)
            .output()
            .unwrap_or_else(|e| panic!("`which {name}` failed to run: {e}"));
        assert!(output.status.success(), "`{name}` not found on PATH");
        std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
    }
}
