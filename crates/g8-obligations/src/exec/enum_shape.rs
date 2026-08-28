//! `RustEnumShape` — contract §1.1.
//!
//! Extracts one Rust enum's variant list (+ optional `#[serde(rename_all =
//! ...)]`) via the brace-balanced text scanner in [`super::text_scan`], and
//! compares against an expected shape (order-independent variant set +
//! exact rename_all match).
//!
//! # Casing-convention comparison (T2c ruling, erratum item 1)
//!
//! ast-grep/text-scan extraction necessarily returns variant identifiers
//! exactly as written in source — always PascalCase, because that is the
//! only casing the Rust compiler accepts for enum variants, *independent*
//! of any `#[serde(rename_all = ...)]` attribute (the attribute only
//! changes the *serialized wire form*, never the source-level identifier).
//! `expected_variants` is wire-format (e.g. `CapabilityStatus`'s locked list
//! is `["proposed","in_flight",...]`, snake_case) — so comparing raw
//! PascalCase extraction directly against it could never match, regardless
//! of whether the real code is correct. Ruled: the checker must (a) extract
//! raw identifiers, (b) compare the enum's actual `#[serde(rename_all)]`
//! attribute against `expected_serde_rename_all` as an independent failure
//! mode (this is what lets the check catch someone silently changing or
//! deleting the attribute — raw-identifier comparison alone could never
//! detect that), (c) apply the transform named by `expected_serde_rename_all`
//! to each raw identifier, (d) order-independent set-compare the
//! *transformed* names against `expected_variants`.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::backend::EnumShapeArgs;
use crate::result::ObligationStatus;

use super::text_scan::{
    find_enum_anchor_byte, find_enum_body, find_preceding_serde_rename_all, split_top_level_commas,
    variant_name_from_segment,
};

pub(crate) fn check(args: &EnumShapeArgs, workspace_root: &Path) -> (ObligationStatus, Value) {
    let path = workspace_root.join(&args.file);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return (
                ObligationStatus::Error,
                json!({ "error": format!("failed to read {}: {e}", args.file) }),
            )
        }
    };

    let Some((body, start_line, end_line)) = find_enum_body(&content, &args.enum_name) else {
        return (
            ObligationStatus::Error,
            json!({ "error": format!("enum `{}` not found in {}", args.enum_name, args.file) }),
        );
    };

    let raw_variants: Vec<String> = split_top_level_commas(&body)
        .iter()
        .filter_map(|seg| variant_name_from_segment(seg))
        .collect();

    // (c) apply the EXPECTED rename_all rule to each raw identifier — using
    // the expected rule (not whatever the source actually has) so the
    // variant-SET comparison stays meaningful even when the attribute
    // itself is what's wrong (that's caught separately by rename_all_ok,
    // just below); an unrecognized rule name is a checker error, never a
    // silent identity fallback.
    let transformed_variants: Vec<String> = match &args.expected_serde_rename_all {
        None => raw_variants.clone(),
        Some(rule) => {
            let mut out = Vec::with_capacity(raw_variants.len());
            for raw in &raw_variants {
                match apply_rename_all(raw, rule) {
                    Some(t) => out.push(t),
                    None => {
                        return (
                            ObligationStatus::Error,
                            json!({ "error": format!("unrecognized expected_serde_rename_all rule `{rule}`") }),
                        )
                    }
                }
            }
            out
        }
    };

    let actual_set: HashSet<&str> = transformed_variants.iter().map(String::as_str).collect();
    let expected_set: HashSet<&str> = args.expected_variants.iter().map(String::as_str).collect();

    let missing: Vec<&str> = expected_set.difference(&actual_set).copied().collect();
    let extra: Vec<&str> = actual_set.difference(&expected_set).copied().collect();
    let variants_ok = missing.is_empty() && extra.is_empty();

    let anchor = find_enum_anchor_byte(&content, &args.enum_name).unwrap_or(0);
    let actual_rename_all = find_preceding_serde_rename_all(&content, anchor);
    // (b) attribute mismatch is a Failed in its own right.
    let rename_all_ok = actual_rename_all == args.expected_serde_rename_all;

    if variants_ok && rename_all_ok {
        (
            ObligationStatus::Passed,
            json!({
                "enum": args.enum_name,
                "variant_count": raw_variants.len(),
                "rename_all": actual_rename_all,
                "line_range": [start_line, end_line],
            }),
        )
    } else {
        let mut missing_sorted: Vec<&str> = missing;
        missing_sorted.sort();
        let mut extra_sorted: Vec<&str> = extra;
        extra_sorted.sort();
        (
            ObligationStatus::Failed,
            json!({
                "enum": args.enum_name,
                "raw_variants": raw_variants,
                "transformed_variants": transformed_variants,
                "missing_variants": missing_sorted,
                "extra_variants": extra_sorted,
                "expected_rename_all": args.expected_serde_rename_all,
                "actual_rename_all": actual_rename_all,
            }),
        )
    }
}

/// Apply a `#[serde(rename_all = "...")]` casing rule to a raw PascalCase
/// Rust variant identifier (the only casing the compiler accepts at the
/// source level). Covers serde's full documented `rename_all` vocabulary,
/// not just the two rules the current 27 obligations exercise (`snake_case`,
/// `PascalCase`) — an obligation added later that locks a different casing
/// should not need a checker change. Returns `None` for an unrecognized
/// rule name; the caller must treat that as a checker error, never a silent
/// identity fallback (this crate never guesses at an unrecognized args
/// value — same discipline as the closed `CheckerBackend` enum itself).
fn apply_rename_all(raw: &str, rule: &str) -> Option<String> {
    match rule {
        "lowercase" => Some(raw.to_lowercase()),
        "UPPERCASE" => Some(raw.to_uppercase()),
        "PascalCase" => Some(raw.to_string()),
        "snake_case" => Some(pascal_to_snake(raw)),
        "SCREAMING_SNAKE_CASE" => Some(pascal_to_snake(raw).to_uppercase()),
        "kebab-case" => Some(pascal_to_snake(raw).replace('_', "-")),
        "SCREAMING-KEBAB-CASE" => Some(pascal_to_snake(raw).to_uppercase().replace('_', "-")),
        "camelCase" => Some(snake_to_camel(&pascal_to_snake(raw))),
        _ => None,
    }
}

/// `InFlight` → `in_flight`, `ConvergenceTest` → `convergence_test`.
fn pascal_to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `in_flight` → `inFlight`.
fn snake_to_camel(s: &str) -> String {
    let mut out = String::new();
    let mut cap_next = false;
    for c in s.chars() {
        if c == '_' {
            cap_next = true;
        } else if cap_next {
            out.extend(c.to_uppercase());
            cap_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_rename_all_snake_case() {
        assert_eq!(
            apply_rename_all("InFlight", "snake_case").as_deref(),
            Some("in_flight")
        );
        assert_eq!(
            apply_rename_all("ConvergenceTest", "snake_case").as_deref(),
            Some("convergence_test")
        );
        assert_eq!(
            apply_rename_all("Plan", "snake_case").as_deref(),
            Some("plan")
        );
    }

    #[test]
    fn apply_rename_all_pascal_case_is_identity() {
        assert_eq!(
            apply_rename_all("Idea", "PascalCase").as_deref(),
            Some("Idea")
        );
    }

    #[test]
    fn apply_rename_all_camel_case() {
        assert_eq!(
            apply_rename_all("InFlight", "camelCase").as_deref(),
            Some("inFlight")
        );
    }

    #[test]
    fn apply_rename_all_unrecognized_rule_is_none() {
        assert_eq!(apply_rename_all("Idea", "not_a_real_rule"), None);
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn passes_on_real_capability_status_enum() {
        // contract §2 row 6 (T2c-corrected): CapabilityStatus@model.rs, 6
        // variants, snake_case — expected_variants is WIRE-FORMAT (matches
        // the artifact's own pre-existing rule.params values), not the raw
        // PascalCase Rust identifiers.
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "CapabilityStatus".to_string(),
            expected_variants: vec![
                "proposed",
                "in_flight",
                "landed",
                "stale",
                "superseded",
                "parked",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            expected_serde_rename_all: Some("snake_case".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn passes_on_real_plan_status_enum_pascal_case() {
        // PascalCase is an identity transform — raw and wire-format
        // coincide here, unlike the two snake_case enums above/below.
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/plan.rs".to_string(),
            enum_name: "PlanStatus".to_string(),
            expected_variants: vec!["Idea", "Scoped", "Dispatched", "Blocked", "Done", "Parked"]
                .into_iter()
                .map(String::from)
                .collect(),
            expected_serde_rename_all: Some("PascalCase".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn passes_on_real_annotation_kind_enum() {
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/annotation.rs".to_string(),
            enum_name: "AnnotationKind".to_string(),
            expected_variants: vec![
                "capability",
                "intent",
                "convergence_test",
                "decision",
                "plan",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            expected_serde_rename_all: Some("snake_case".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn fails_when_expected_list_omits_a_real_variant() {
        // The real enum HAS "parked" (wire-format); expected_variants
        // doesn't list it — that's an EXTRA variant (present in code, not
        // in the expected set), the mirror image of "missing" (expected but
        // absent).
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "CapabilityStatus".to_string(),
            expected_variants: vec!["proposed", "in_flight", "landed", "stale", "superseded"]
                .into_iter()
                .map(String::from)
                .collect(), // omits "parked"
            expected_serde_rename_all: Some("snake_case".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["extra_variants"], json!(["parked"]));
        assert_eq!(detail["missing_variants"], json!([]));
    }

    #[test]
    fn fails_when_expected_list_claims_a_variant_that_does_not_exist() {
        // Mirror case: expected_variants claims a 7th variant the real enum
        // does not have — a genuine MISSING variant.
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "CapabilityStatus".to_string(),
            expected_variants: vec![
                "proposed",
                "in_flight",
                "landed",
                "stale",
                "superseded",
                "parked",
                "retired",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            expected_serde_rename_all: Some("snake_case".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["missing_variants"], json!(["retired"]));
        assert_eq!(detail["extra_variants"], json!([]));
    }

    #[test]
    fn fails_on_wrong_rename_all() {
        // Even though `expected_variants` below happens to equal what
        // camelCase transform would produce for THESE particular single-
        // word-after-first-letter variants is irrelevant — the point is the
        // ATTRIBUTE comparison (rename_all_ok) fails on its own, per the
        // T2c ruling's point (b): "a mismatch here is a Failed in its own
        // right," independent of whether the variant set matches.
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "CapabilityStatus".to_string(),
            expected_variants: vec![
                "proposed",
                "inFlight",
                "landed",
                "stale",
                "superseded",
                "parked",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            expected_serde_rename_all: Some("camelCase".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["actual_rename_all"], json!("snake_case"));
        assert_eq!(detail["expected_rename_all"], json!("camelCase"));
    }

    #[test]
    fn error_on_unrecognized_rename_all_rule() {
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "CapabilityStatus".to_string(),
            expected_variants: vec!["proposed".to_string()],
            expected_serde_rename_all: Some("not_a_real_serde_rule".to_string()),
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn error_on_missing_file() {
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/does_not_exist.rs".to_string(),
            enum_name: "Whatever".to_string(),
            expected_variants: vec![],
            expected_serde_rename_all: None,
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn error_on_enum_not_found_in_file() {
        let args = EnumShapeArgs {
            file: "crates/g8-core/src/model.rs".to_string(),
            enum_name: "ThisEnumDoesNotExist".to_string(),
            expected_variants: vec![],
            expected_serde_rename_all: None,
        };
        let (status, detail) = check(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }
}
