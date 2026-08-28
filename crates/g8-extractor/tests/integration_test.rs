//! Integration tests for `g8-extractor`.
//!
//! These tests verify expected annotation counts per fixture directory / file.
//! They require `ast-grep` to be installed on `PATH`.

use std::path::Path;

use g8_core::{AnnotationKind, IntentSourceKind};
use g8_extractor::{Extractor, ExtractorError};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn extractor() -> Extractor {
    Extractor::new().expect("ast-grep must be installed; run `brew install ast-grep`")
}

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

// ── Rust fixture tests ────────────────────────────────────────────────────────

#[test]
fn rust_has_annotations_detects_expected_count() {
    let e = extractor();
    let file = fixture("rust/has_annotations.rs");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    // Expected: capability (http-fetch), capability (json-parse), convergence_test,
    //           intent, plan, decision = 6 annotations total.
    assert_eq!(
        anns.len(),
        6,
        "expected 6 annotations from has_annotations.rs, got {}: {:?}",
        anns.len(),
        anns.iter()
            .map(|a| format!("{:?}:{}", a.kind, a.name.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
    );
}

#[test]
fn rust_no_annotations_returns_empty() {
    let e = extractor();
    let file = fixture("rust/no_annotations.rs");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    assert!(
        anns.is_empty(),
        "expected no annotations from no_annotations.rs, got {}",
        anns.len()
    );
}

#[test]
fn rust_capabilities_have_inline_comment_source_kind() {
    let e = extractor();
    let file = fixture("rust/has_annotations.rs");
    let anns = e.scan_file(&file).unwrap();
    let caps: Vec<_> = anns
        .iter()
        .filter(|a| a.kind == AnnotationKind::Capability)
        .collect();
    assert!(!caps.is_empty(), "expected at least one capability");
    for cap in &caps {
        assert_eq!(
            cap.source_kind,
            IntentSourceKind::InlineComment,
            "capability source_kind should be InlineComment"
        );
    }
}

#[test]
fn rust_conditional_annotations_are_flagged() {
    let e = extractor();
    let file = fixture("rust/conditional_annotations.rs");
    let anns = e.scan_file(&file).unwrap();
    let conditional_count = anns.iter().filter(|a| a.conditional).count();
    // Both annotations have dev_only = true; they should be flagged.
    assert!(
        conditional_count >= 1,
        "expected at least 1 conditional annotation, got {conditional_count}"
    );
}

// ── TypeScript fixture tests ──────────────────────────────────────────────────

#[test]
fn ts_has_annotations_detects_expected_count() {
    let e = extractor();
    let file = fixture("ts/has_annotations.ts");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    // Expected: capability (batch-processor), convergence_test, intent, plan = 4.
    assert_eq!(
        anns.len(),
        4,
        "expected 4 annotations from ts/has_annotations.ts, got {}: {:?}",
        anns.len(),
        anns.iter()
            .map(|a| format!("{:?}", a.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ts_no_annotations_returns_empty() {
    let e = extractor();
    let file = fixture("ts/no_annotations.ts");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    assert!(
        anns.is_empty(),
        "expected no annotations from ts/no_annotations.ts, got {}",
        anns.len()
    );
}

// ── Python fixture tests ──────────────────────────────────────────────────────

#[test]
fn python_has_annotations_detects_expected_count() {
    let e = extractor();
    let file = fixture("python/has_annotations.py");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    // Expected: capability (http-handler), capability (auth-validator),
    //           convergence_test, intent, plan = 5.
    assert_eq!(
        anns.len(),
        5,
        "expected 5 annotations from python/has_annotations.py, got {}: {:?}",
        anns.len(),
        anns.iter()
            .map(|a| format!("{:?}:{}", a.kind, a.name.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
    );
}

#[test]
fn python_no_annotations_returns_empty() {
    let e = extractor();
    let file = fixture("python/no_annotations.py");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    assert!(
        anns.is_empty(),
        "expected no annotations from python/no_annotations.py, got {}",
        anns.len()
    );
}

// ── Go fixture tests ──────────────────────────────────────────────────────────

#[test]
fn go_has_annotations_detects_expected_count() {
    let e = extractor();
    let file = fixture("go/has_annotations.go");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    // Expected: capability (streaming-export), capability (batch-export),
    //           convergence_test, intent, plan = 5.
    assert_eq!(
        anns.len(),
        5,
        "expected 5 annotations from go/has_annotations.go, got {}: {:?}",
        anns.len(),
        anns.iter()
            .map(|a| format!("{:?}:{}", a.kind, a.name.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
    );
}

#[test]
fn go_no_annotations_returns_empty() {
    let e = extractor();
    let file = fixture("go/no_annotations.go");
    let anns = e.scan_file(&file).expect("scan_file should succeed");
    assert!(
        anns.is_empty(),
        "expected no annotations from go/no_annotations.go, got {}",
        anns.len()
    );
}

// ── Markdown sidecar fixture tests ────────────────────────────────────────────

#[test]
fn agents_md_produces_intent_annotations() {
    let e = extractor();
    let file = fixture("markdown/AGENTS.md");
    let anns = e.scan_file(&file).expect("scan_file for AGENTS.md");
    // Should produce at least: Architecture(ArchitecturalScope), Boundaries(Boundary),
    //   Stack(TechStack), Commands(Operational), Owner, Parked Ideas (2 Plan rows),
    //   Decisions(Unclassified), Project Overview(Unclassified) = multiple annotations.
    assert!(!anns.is_empty(), "AGENTS.md should produce annotations");
    // All should have AgentsMd source_kind.
    for ann in &anns {
        assert_eq!(
            ann.source_kind,
            IntentSourceKind::AgentsMd,
            "AGENTS.md annotations should have AgentsMd source_kind"
        );
    }
}

#[test]
fn agents_md_has_architectural_scope_intent() {
    let e = extractor();
    let file = fixture("markdown/AGENTS.md");
    let anns = e.scan_file(&file).unwrap();
    let has_arch = anns.iter().any(|a| {
        a.kind == AnnotationKind::Intent
            && a.fields
                .get("intent_kind")
                .and_then(|v| {
                    if let g8_core::AnnotationValue::String(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .map(|s| s == "architectural_scope")
                .unwrap_or(false)
    });
    assert!(
        has_arch,
        "expected at least one architectural_scope intent from AGENTS.md"
    );
}

#[test]
fn agents_md_has_parked_plan_annotations() {
    let e = extractor();
    let file = fixture("markdown/AGENTS.md");
    let anns = e.scan_file(&file).unwrap();
    let parked: Vec<_> = anns
        .iter()
        .filter(|a| a.kind == AnnotationKind::Plan)
        .collect();
    // AGENTS.md has 2 parked ideas.
    assert_eq!(
        parked.len(),
        2,
        "expected 2 Plan annotations from AGENTS.md Parked Ideas, got {}",
        parked.len()
    );
}

#[test]
fn g8_sidecar_produces_annotations() {
    let e = extractor();
    let file = fixture("markdown/scorer.g8.md");
    let anns = e.scan_file(&file).expect("scan_file for scorer.g8.md");
    assert!(!anns.is_empty(), "scorer.g8.md should produce annotations");
    for ann in &anns {
        assert_eq!(
            ann.source_kind,
            IntentSourceKind::G8Sidecar,
            "*.g8.md annotations should have G8Sidecar source_kind"
        );
    }
}

#[test]
fn claude_md_produces_intent_annotations() {
    let e = extractor();
    let file = fixture("markdown/CLAUDE.md");
    let anns = e.scan_file(&file).expect("scan_file for CLAUDE.md");
    assert!(!anns.is_empty(), "CLAUDE.md should produce annotations");
    for ann in &anns {
        assert_eq!(
            ann.source_kind,
            IntentSourceKind::ClaudeMd,
            "CLAUDE.md annotations should have ClaudeMd source_kind"
        );
    }
}

// ── scan_dir tests ────────────────────────────────────────────────────────────

#[test]
fn scan_dir_returns_annotations_from_all_languages() {
    let e = extractor();
    let root = fixture("");
    let result = e.scan_dir(&root).expect("scan_dir should succeed");
    // Should have annotations from Rust, TS, Python, Go and Markdown fixtures.
    assert!(
        result.annotations.len() >= 10,
        "expected at least 10 total annotations across all fixtures, got {}",
        result.annotations.len()
    );
    assert!(result.stats.files_scanned > 0, "expected files_scanned > 0");
    assert!(
        result.stats.annotations_found > 0,
        "expected annotations_found > 0"
    );
}

#[test]
fn scan_dir_excludes_intent_summary() {
    // Create a temp dir with an INTENT_SUMMARY.md to verify it's excluded.
    let tmp = tempfile::tempdir().unwrap();
    let summary_path = tmp.path().join("INTENT_SUMMARY.md");
    std::fs::write(
        &summary_path,
        "<!-- AUTO-GENERATED -->\n# Architecture\nThis should NOT be ingested.",
    )
    .unwrap();

    let e = extractor();
    let result = e.scan_dir(tmp.path()).expect("scan_dir should succeed");
    // INTENT_SUMMARY.md must not produce any annotations.
    for ann in &result.annotations {
        assert_ne!(
            ann.source.file, summary_path,
            "INTENT_SUMMARY.md must not be ingested"
        );
    }
}

// ── Error case tests ──────────────────────────────────────────────────────────

#[test]
fn extractor_with_bad_binary_returns_error() {
    let result = Extractor::with_binary(
        std::path::PathBuf::from("/nonexistent/rules.yaml"),
        std::path::PathBuf::from("/nonexistent/ast-grep"),
    );
    assert!(
        matches!(result, Err(ExtractorError::BinaryNotFound)),
        "expected BinaryNotFound error"
    );
}

#[test]
fn binary_not_found_message_contains_install_instructions() {
    let err = ExtractorError::BinaryNotFound;
    let msg = err.to_string();
    assert!(
        msg.contains("install"),
        "error message should contain install instructions, got: {msg}"
    );
}

// ── scan_project free function test ──────────────────────────────────────────

#[test]
fn scan_project_free_fn_works() {
    let root = fixture("rust");
    let annotations = g8_extractor::scan_project(&root).expect("scan_project should succeed");
    // has_annotations.rs has 6, conditional_annotations.rs has 2, no_annotations.rs has 0.
    assert!(
        annotations.len() >= 6,
        "expected at least 6 annotations from rust fixtures, got {}",
        annotations.len()
    );
}
