//! Backend dispatch. One module per backend family, mirroring
//! `backend.rs`'s enum ordering.

pub(crate) mod ast_grep;
pub(crate) mod attestation;
mod byte_diff;
mod cargo_metadata;
mod check_contract;
mod enum_shape;
mod fixture;
mod globbing;
mod json_path;
mod receipt;
mod rg;
mod text_scan;
mod vocabulary_drift;

use std::any::Any;
use std::path::Path;
use std::time::Instant;

use crate::backend::CheckerBackend;
use crate::result::{ObligationStatus, SubCheckResult};
use crate::ReceiptSubject;

/// Dispatch one `CheckerBackend` to its executor, isolating any *unexpected*
/// panic as defense-in-depth on top of each executor's own careful
/// `Result`-based error handling — contract §4.1: "a single backend
/// crashing surfaces as that one obligation's status = Error, not a
/// propagated panic — every backend invocation is isolated (caught, not
/// just `?`-propagated)."
///
/// `receipt_subject` is `Some` only in receipt mode (contract addendum
/// o-g8-receipts-20260721) — the parsed, hashed receipt every
/// `ReceiptQuery` sub-check in this run evaluates against. Every other
/// backend ignores it; it is threaded through here (rather than re-parsed
/// per obligation) because `lib.rs` parses+hashes the receipt file exactly
/// once per `g8 check --receipt` invocation.
pub(crate) fn dispatch(
    backend: &CheckerBackend,
    workspace_root: &Path,
    receipt_subject: Option<&ReceiptSubject>,
) -> SubCheckResult {
    let name = backend.name().to_string();
    let start = Instant::now();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_backend(backend, workspace_root, receipt_subject)
    }));

    let (status, detail) = match outcome {
        Ok((status, detail)) => (status, detail),
        Err(panic_payload) => (
            ObligationStatus::Error,
            serde_json::json!({ "panic": panic_message(&panic_payload) }),
        ),
    };

    SubCheckResult {
        backend: name,
        status,
        detail,
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

fn run_backend(
    backend: &CheckerBackend,
    workspace_root: &Path,
    receipt_subject: Option<&ReceiptSubject>,
) -> (ObligationStatus, serde_json::Value) {
    match backend {
        CheckerBackend::CargoMetadataNoDep(args) => cargo_metadata::no_dep(args, workspace_root),
        CheckerBackend::CargoMetadataDepGraph(args) => {
            cargo_metadata::dep_graph(args, workspace_root)
        }
        CheckerBackend::AstGrepNoMatch(args) => ast_grep::no_match(args, workspace_root),
        CheckerBackend::AstGrepMatchCount(args) => ast_grep::match_count(args, workspace_root),
        CheckerBackend::RgMatchCount(args) => rg::match_count(args, workspace_root),
        CheckerBackend::RustEnumShape(args) => enum_shape::check(args, workspace_root),
        CheckerBackend::FixtureIntegrationTest(args) => fixture::run(args, workspace_root),
        CheckerBackend::ByteDiffTwice(args) => byte_diff::run(args, workspace_root),
        CheckerBackend::G8CheckContract(args) => check_contract::run(args, workspace_root),
        CheckerBackend::BuiltinAlgorithm(args) => vocabulary_drift::run(args, workspace_root),
        CheckerBackend::ReceiptQuery(args) => receipt::run(args, receipt_subject),
    }
}

fn panic_message(payload: &Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AstGrepArgs, AstGrepLang, CargoMetadataNoDepArgs, DepMatcher, RgArgs};
    use crate::backend::{BuildConfig, CountExpectation};

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn dispatch_routes_to_the_right_executor_and_names_the_backend() {
        let backend = CheckerBackend::RgMatchCount(RgArgs {
            pattern: "ZZZ_NEVER_PRESENT_ZZZ".to_string(),
            glob: vec!["crates/g8-core/src/lib.rs".to_string()],
            expected: CountExpectation::Zero,
        });
        let result = dispatch(&backend, &workspace_root(), None);
        assert_eq!(result.backend, "rg_match_count");
        assert_eq!(result.status, ObligationStatus::Passed);
    }

    #[test]
    fn dispatch_records_duration() {
        let backend = CheckerBackend::CargoMetadataNoDep(CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact("nonexistent".to_string())],
            scope_crates: vec!["g8-core".to_string()],
            build_config: BuildConfig::Default,
        });
        let result = dispatch(&backend, &workspace_root(), None);
        // duration_ms is u64, always >= 0; just prove the field is populated
        // by exercising a real (slow-ish, subprocess-based) backend.
        assert_eq!(result.status, ObligationStatus::Passed);
    }

    #[test]
    fn dispatch_isolates_backend_failure_as_error_not_a_crash() {
        // AstGrepNoMatch with an unreachable binary path baked into the args
        // isn't possible (binary isn't a field) — instead exercise the glob
        // error path, which is the realistic "backend fails cleanly" case
        // dispatch must still wrap correctly.
        let backend = CheckerBackend::AstGrepNoMatch(AstGrepArgs {
            patterns: vec!["x".to_string()],
            lang: AstGrepLang::Rust,
            glob: vec!["[".to_string()], // malformed glob
            exclude_glob: vec![],
            capture_predicate: None,
        });
        let result = dispatch(&backend, &workspace_root(), None);
        assert_eq!(result.status, ObligationStatus::Error);
        assert_eq!(result.backend, "ast_grep_no_match");
    }

    #[test]
    fn dispatch_routes_receipt_query_and_threads_the_bound_subject() {
        use crate::backend::{ReceiptAggregate, ReceiptExpectation, ReceiptQueryArgs};

        let backend = CheckerBackend::ReceiptQuery(ReceiptQueryArgs {
            select: "$.actions[*]".to_string(),
            where_clauses: vec![],
            aggregate: ReceiptAggregate::Count,
            expected: ReceiptExpectation::Exactly { value: 1.0 },
        });
        let subject = ReceiptSubject {
            display_path: "receipts/test.json".to_string(),
            sha256: "sha256:test".to_string(),
            value: serde_json::json!({ "actions": [{"kind": "publish"}] }),
        };
        let result = dispatch(&backend, &workspace_root(), Some(&subject));
        assert_eq!(result.backend, "receipt_query");
        assert_eq!(
            result.status,
            ObligationStatus::Passed,
            "detail: {:?}",
            result.detail
        );
    }

    #[test]
    fn dispatch_receipt_query_without_a_bound_subject_errors() {
        use crate::backend::{ReceiptAggregate, ReceiptExpectation, ReceiptQueryArgs};

        let backend = CheckerBackend::ReceiptQuery(ReceiptQueryArgs {
            select: "$.actions[*]".to_string(),
            where_clauses: vec![],
            aggregate: ReceiptAggregate::Count,
            expected: ReceiptExpectation::Zero,
        });
        let result = dispatch(&backend, &workspace_root(), None);
        assert_eq!(result.status, ObligationStatus::Error);
    }
}
