//! `BuiltinAlgorithm::VocabularyDrift` — contract §1.1/§10, OBL-G1-01.
//!
//! Ports, verbatim, the predicate `obligations-v0.1.json`'s own OBL-G1-01
//! entry specifies (`rule.params.similarity_predicate`):
//!
//! ```text
//! levenshtein(candidate, canonical) <= 2
//!   OR starts_with(candidate, canonical)
//!   OR starts_with(canonical, candidate)
//! ```
//!
//! reused from an existing `vocabulary_drift` predicate per that
//! entry's `rule.params.predicate_source` — reproduced deterministically
//! here (no live SurrealDB, per P9 "no Codegraph hard dep"). A candidate
//! that exactly equals a canonical term is correct usage, not drift, and is
//! never flagged even though it trivially satisfies both `starts_with`
//! arms.
//!
//! Candidate extraction is a plain regex over the declaration grammar (see
//! [`super::text_scan::find_type_declaration_names`]), not ast-grep —
//! empirically, ast-grep's pattern inference for bare `struct $NAME` / `enum
//! $NAME` forms does not match real declaration syntax.

use std::path::Path;

use serde_json::{json, Value};

use crate::backend::BuiltinAlgorithmArgs;
use crate::result::ObligationStatus;

use super::globbing::expand_glob;
use super::text_scan::find_type_declaration_names;

pub(crate) fn run(args: &BuiltinAlgorithmArgs, workspace_root: &Path) -> (ObligationStatus, Value) {
    let BuiltinAlgorithmArgs::VocabularyDrift {
        canonical_terms,
        allowlist,
        scope_glob,
    } = args;
    vocabulary_drift(canonical_terms, allowlist, scope_glob, workspace_root)
}

fn vocabulary_drift(
    canonical_terms: &[String],
    allowlist: &[String],
    scope_glob: &[String],
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let files = match expand_glob(workspace_root, scope_glob, &[]) {
        Ok(f) => f,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let mut candidates: Vec<String> = Vec::new();
    for f in &files {
        let Ok(content) = std::fs::read_to_string(f) else {
            continue;
        };
        candidates.extend(find_type_declaration_names(&content));
    }
    candidates.sort();
    candidates.dedup();

    let mut hits: Vec<Value> = Vec::new();
    for candidate in &candidates {
        if allowlist.iter().any(|a| a == candidate) {
            continue;
        }
        for canonical in canonical_terms {
            if candidate == canonical {
                continue; // exact match to its own canonical term is correct usage
            }
            if drift_predicate(candidate, canonical) {
                hits.push(json!({
                    "candidate": candidate,
                    "canonical": canonical,
                    "levenshtein": levenshtein(candidate, canonical),
                }));
            }
        }
    }

    if hits.is_empty() {
        (
            ObligationStatus::Passed,
            json!({ "candidates_checked": candidates.len() }),
        )
    } else {
        (ObligationStatus::Failed, json!({ "drift_hits": hits }))
    }
}

fn drift_predicate(candidate: &str, canonical: &str) -> bool {
    levenshtein(candidate, canonical) <= 2
        || candidate.starts_with(canonical)
        || canonical.starts_with(candidate)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (la, lb) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; lb + 1]; la + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=la {
        for j in 1..=lb {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[la][lb]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basic_cases() {
        assert_eq!(levenshtein("Plan", "Plan"), 0);
        assert_eq!(levenshtein("Capabilty", "Capability"), 1); // missing 'i'
                                                               // Cross-checked independently (Python reference implementation),
                                                               // not hand-counted — Plan/Project share only the leading 'P'.
        assert_eq!(levenshtein("Plan", "Project"), 6);
    }

    #[test]
    fn drift_predicate_catches_suffix_append_and_prefix_match() {
        assert!(drift_predicate("CapabilityDraft", "Capability")); // starts_with
        assert!(drift_predicate("Capabilty", "Capability")); // levenshtein=1
        assert!(!drift_predicate("Project", "Plan")); // neither condition
    }

    #[test]
    fn drift_predicate_known_accepted_gap_prefix_qualifier_not_caught() {
        // Documented in obligations-v0.1.json's own G1-01 entry
        // (`rule.params.known_accepted_gap`): prefix-qualifier drift like
        // "RawCapability" is NOT caught by this predicate — verified here so
        // a future "improvement" to the predicate doesn't silently diverge
        // from the verbatim-reused algorithm.
        assert!(!drift_predicate("RawCapability", "Capability"));
    }

    #[test]
    fn passes_on_clean_synthetic_fixture() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("clean.rs"),
            "pub struct Capability {}\npub enum Plan { A }\n",
        )
        .unwrap();
        let (status, detail) = vocabulary_drift(
            &["Capability".to_string(), "Plan".to_string()],
            &[],
            &["*.rs".to_string()],
            dir.path(),
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn fails_on_drifting_synthetic_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("drift.rs"), "pub struct Capabilty {}\n").unwrap(); // typo'd, real drift
        let (status, detail) = vocabulary_drift(
            &["Capability".to_string()],
            &[],
            &["*.rs".to_string()],
            dir.path(),
        );
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
    }

    #[test]
    fn allowlist_suppresses_a_known_derived_suffix_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.rs"), "pub enum PlanStatus { A }\n").unwrap();
        let (status, detail) = vocabulary_drift(
            &["Plan".to_string()],
            &["PlanStatus".to_string()],
            &["*.rs".to_string()],
            dir.path(),
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn error_path_invalid_glob() {
        let dir = tempfile::tempdir().unwrap();
        let (status, detail) = vocabulary_drift(&[], &[], &["[".to_string()], dir.path());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    /// Real-repo check using G1-01's OWN canonical_terms/allowlist verbatim
    /// from `specs/obligations-v0.1.json`. This is a GENUINE finding, not a
    /// test bug: run against the real `g8-core/src/**`, the literal
    /// predicate flags `IntentSourceKind` (starts_with "Intent", not
    /// allowlisted) — almost certainly an allowlist gap (same
    /// Canonical+Suffix derived-name pattern as the 6 terms already
    /// allowlisted: PlanStatus/CapabilityStatus/AnnotationKind/IntentKind/
    /// DecisionStatus/PlanDraft), not a real naming problem. Documented here
    /// rather than silently expanding the allowlist myself — that call
    /// belongs to whoever owns `obligations-v0.1.json` (T4/T6), not this
    /// executor. See my final report's ESCALATIONS section.
    #[test]
    fn real_g8_core_scope_finds_the_known_intent_source_kind_allowlist_gap() {
        let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let canonical_terms: Vec<String> = [
            "Annotation",
            "Capability",
            "Intent",
            "Plan",
            "ConvergenceSpace",
            "Project",
            "Substrate",
            "Owner",
            "FitReport",
            "Conflict",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let allowlist: Vec<String> = [
            "PlanStatus",
            "CapabilityStatus",
            "AnnotationKind",
            "IntentKind",
            "DecisionStatus",
            "PlanDraft",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let (status, detail) = vocabulary_drift(
            &canonical_terms,
            &allowlist,
            &["crates/g8-core/src/**".to_string()],
            &workspace_root,
        );
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        let hits = detail["drift_hits"].as_array().unwrap();
        assert!(
            hits.iter()
                .any(|h| h["candidate"] == "IntentSourceKind" && h["canonical"] == "Intent"),
            "expected the known IntentSourceKind/Intent hit, got: {hits:?}"
        );
    }
}
