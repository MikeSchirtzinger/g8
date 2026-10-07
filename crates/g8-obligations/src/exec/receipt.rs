//! `ReceiptQuery` — contract addendum (o-g8-receipts-20260721), backend
//! #11. A structural evaluator over a *parsed action receipt* — never the
//! repository. The receipt is parsed and SHA-256-hashed exactly once per
//! `g8 check --receipt` invocation (see [`crate::load_receipt_subject`],
//! [`crate::ReceiptSubject`]); this module only evaluates against the
//! already-parsed `serde_json::Value`, never touches the filesystem itself.
//!
//! Path grammar is [`super::json_path`]'s dialect (the one
//! `FixtureIntegrationTest`'s `JsonField`/`JsonPathNonEmpty` assertions use).
//! `select` and `where` paths go through the multi-valued
//! [`resolve_json_path_all`], which adds recursive descent `..key` and `[*]`
//! at any position: `select` yields every node reached (a `[*]` explodes each
//! array among them), and a `where` clause whose path reaches several values
//! holds when ANY value satisfies the op on its own. `sum` stays
//! single-valued: a `sum` path that fans out is an Error.
//!
//! No subprocess, no scratch directory — this backend is pure, in-memory
//! evaluation over a `serde_json::Value`, which is also why its trust tier
//! is `Checked`, not `Verified` (contract §3.2's own table: `Checked` is
//! "static/structural inspection... no real runtime behavior exercised" —
//! exactly what this backend does, honestly).

use serde_json::Value;

use crate::backend::{ReceiptAggregate, ReceiptQueryArgs, WhereClause, WhereOp};
use crate::result::ObligationStatus;
use crate::ReceiptSubject;

use super::json_path::{
    path_fans_out, resolve_json_path, resolve_json_path_all, validate_supported_path,
};

pub(crate) fn run(
    args: &ReceiptQueryArgs,
    receipt: Option<&ReceiptSubject>,
) -> (ObligationStatus, Value) {
    let Some(subject) = receipt else {
        // Should be unreachable in production: `lib.rs`'s scope-filtering
        // pass only ever dispatches `ReceiptQuery` when a receipt subject is
        // bound (receipt mode + an `action_receipt`-wired obligation).
        // Defensive, not decorative — never silently "pass" a check that
        // never actually ran against anything.
        return (
            ObligationStatus::Error,
            serde_json::json!({
                "error": "receipt_query executed with no receipt subject bound (internal wiring error)"
            }),
        );
    };
    evaluate(args, &subject.value)
}

fn evaluate(args: &ReceiptQueryArgs, root: &Value) -> (ObligationStatus, Value) {
    // Unsupported path syntax anywhere in the query is a checker Error, never a
    // silent "matched nothing": a `$..` select with `expected: zero` read as a
    // pass on 0.1.2 (the fail-open fixed in 0.1.3). Clause and sum paths are
    // validated up front for the same reason, before any candidate is looked at.
    let candidates = match select_candidates(root, &args.select) {
        Ok(candidates) => candidates,
        Err(error) => {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": error }),
            )
        }
    };
    for clause in &args.where_clauses {
        if let Err(error) = resolve_json_path_all(&Value::Null, &clause.path) {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": error, "where": clause.path }),
            );
        }
    }
    if let ReceiptAggregate::Sum { path } = &args.aggregate {
        if let Err(error) = validate_sum_path(path) {
            return (
                ObligationStatus::Error,
                serde_json::json!({ "error": error, "sum": path }),
            );
        }
    }

    let mut matched: Vec<&Value> = Vec::new();
    for candidate in &candidates {
        match clauses_match(&args.where_clauses, candidate) {
            Ok(true) => matched.push(candidate),
            Ok(false) => {}
            Err(error) => {
                return (
                    ObligationStatus::Error,
                    serde_json::json!({ "error": error }),
                )
            }
        }
    }

    let actual = match &args.aggregate {
        ReceiptAggregate::Count => matched.len() as f64,
        ReceiptAggregate::Sum { path } => {
            let mut total = 0.0_f64;
            for candidate in &matched {
                match resolve_json_path(candidate, path).and_then(Value::as_f64) {
                    Some(n) => total += n,
                    None => {
                        return (
                            ObligationStatus::Error,
                            serde_json::json!({
                                "error": format!(
                                    "sum over '{path}' found a missing or non-numeric value on a \
                                     selected candidate: a spend cap that silently skipped an \
                                     unparseable amount would be a hole"
                                ),
                                "candidate": candidate,
                            }),
                        );
                    }
                }
            }
            total
        }
    };

    let detail = serde_json::json!({
        "select": args.select,
        "selected_count": candidates.len(),
        "matched_count": matched.len(),
        "aggregate": aggregate_name(&args.aggregate),
        "actual": actual,
        "expected": format!("{:?}", args.expected),
    });

    if args.expected.is_met_by(actual) {
        (ObligationStatus::Passed, detail)
    } else {
        (ObligationStatus::Failed, detail)
    }
}

fn aggregate_name(aggregate: &ReceiptAggregate) -> &'static str {
    match aggregate {
        ReceiptAggregate::Count => "count",
        ReceiptAggregate::Sum { .. } => "sum",
    }
}

/// `sum` names exactly one amount per candidate. A path that can fan out
/// (`..key` or `[*]`) is refused in this version: a spend cap over several
/// values per candidate must be written explicitly, not inferred. Anything
/// else goes through 0.1.3's single-valued validator.
fn validate_sum_path(path: &str) -> Result<(), String> {
    if path_fans_out(path)? {
        return Err(format!(
            "sum over a multi-valued path is not supported in this version: `{path}` uses `..` or \
             `[*]`; `sum` is a spend cap and must name exactly one amount per selected candidate"
        ));
    }
    validate_supported_path(path)
}

/// `select` yields every node `path` reaches ([`resolve_json_path_all`]).
/// A `[*]` explodes each array among the nodes it applies to into its
/// elements; a non-array node there is kept as itself rather than erroring
/// (a receipt shaped differently than expected). Without `[*]`, each reached
/// node (array or not) is one candidate, e.g. `$.meta.run_id` (a single
/// string) never needs `[*]`, and `$..claims` yields each `claims` array
/// whole. A path that reaches nothing yields zero candidates. Unparseable
/// syntax is an `Err`, never zero candidates.
fn select_candidates<'a>(root: &'a Value, select: &str) -> Result<Vec<&'a Value>, String> {
    resolve_json_path_all(root, select)
}

fn clauses_match(clauses: &[WhereClause], candidate: &Value) -> Result<bool, String> {
    for clause in clauses {
        if !clause_matches(clause, candidate)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluates one clause against one candidate. The clause path resolves to
/// zero or more values ([`resolve_json_path_all`]).
///
/// One value: the op as written (`Eq`, `In`, `Matches`, the comparisons,
/// and their negations `Ne`/`NotIn`), unchanged from the single-valued
/// dialect.
///
/// Several values: the clause holds when ANY value satisfies the op on its
/// own. So `Ne` holds when at least one value differs and `NotIn` when at
/// least one value is outside the list: the canonical allowlist gate counts
/// `targets: ["allowed", "evil"]` as a violation. These are not set
/// negations of `Eq`/`In`; both `Eq` and `Ne` can hold for one candidate.
///
/// No value (missing path, or a descent or `[*]` that reaches nothing): the
/// fail-closed table (backend.rs's `WhereClause` doc), positive ops `false`
/// and negative ops `true`. `Exists` holds when at least one value is
/// reached and `Absent` when none is. The configured `value` is validated
/// before any of this, so a malformed clause errors on every candidate.
fn clause_matches(clause: &WhereClause, candidate: &Value) -> Result<bool, String> {
    let resolved = resolve_json_path_all(candidate, &clause.path)?;
    let predicate: Box<dyn Fn(&Value) -> bool> = match clause.op {
        WhereOp::Exists => return Ok(!resolved.is_empty()),
        WhereOp::Absent => return Ok(resolved.is_empty()),
        WhereOp::Eq => {
            let configured = require_value(clause)?;
            Box::new(move |v| v == configured)
        }
        WhereOp::Ne => {
            let configured = require_value(clause)?;
            Box::new(move |v| v != configured)
        }
        WhereOp::In => {
            let configured = require_array_value(clause)?;
            Box::new(move |v| configured.contains(v))
        }
        WhereOp::NotIn => {
            let configured = require_array_value(clause)?;
            Box::new(move |v| !configured.contains(v))
        }
        WhereOp::Matches => {
            let pattern = require_str_value(clause)?;
            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("invalid regex '{pattern}': {e}"))?;
            Box::new(move |v| v.as_str().is_some_and(|s| re.is_match(s)))
        }
        WhereOp::Gt => numeric(require_num_value(clause)?, |a, b| a > b),
        WhereOp::Gte => numeric(require_num_value(clause)?, |a, b| a >= b),
        WhereOp::Lt => numeric(require_num_value(clause)?, |a, b| a < b),
        WhereOp::Lte => numeric(require_num_value(clause)?, |a, b| a <= b),
    };
    if resolved.is_empty() {
        return Ok(is_negative_op(clause.op));
    }
    Ok(resolved.into_iter().any(predicate))
}

/// The ops that hold on a missing path (the fail-closed table).
fn is_negative_op(op: WhereOp) -> bool {
    matches!(op, WhereOp::Ne | WhereOp::NotIn | WhereOp::Absent)
}

/// A numeric comparison against `configured`; a non-numeric value fails
/// closed (does not satisfy it) rather than erroring.
fn numeric(configured: f64, cmp: fn(f64, f64) -> bool) -> Box<dyn Fn(&Value) -> bool> {
    Box::new(move |v| v.as_f64().is_some_and(|actual| cmp(actual, configured)))
}

fn require_value(clause: &WhereClause) -> Result<&Value, String> {
    clause.value.as_ref().ok_or_else(|| {
        format!(
            "op '{:?}' on path '{}' requires a 'value'",
            clause.op, clause.path
        )
    })
}

fn require_array_value(clause: &WhereClause) -> Result<&[Value], String> {
    let value = require_value(clause)?;
    value.as_array().map(Vec::as_slice).ok_or_else(|| {
        format!(
            "op '{:?}' on path '{}' requires 'value' to be a JSON array",
            clause.op, clause.path
        )
    })
}

fn require_str_value(clause: &WhereClause) -> Result<&str, String> {
    let value = require_value(clause)?;
    value.as_str().ok_or_else(|| {
        format!(
            "op '{:?}' on path '{}' requires 'value' to be a string",
            clause.op, clause.path
        )
    })
}

fn require_num_value(clause: &WhereClause) -> Result<f64, String> {
    let value = require_value(clause)?;
    value.as_f64().ok_or_else(|| {
        format!(
            "op '{:?}' on path '{}' requires 'value' to be numeric",
            clause.op, clause.path
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ReceiptAggregate, ReceiptExpectation};
    use serde_json::json;

    fn args(
        select: &str,
        where_clauses: Vec<WhereClause>,
        aggregate: ReceiptAggregate,
        expected: ReceiptExpectation,
    ) -> ReceiptQueryArgs {
        ReceiptQueryArgs {
            select: select.to_string(),
            where_clauses,
            aggregate,
            expected,
        }
    }

    fn clause(path: &str, op: WhereOp, value: Option<Value>) -> WhereClause {
        WhereClause {
            path: path.to_string(),
            op,
            value,
        }
    }

    fn receipt() -> Value {
        json!({
            "meta": { "run_id": "run-1" },
            "actions": [
                { "kind": "publish", "target": { "domain": "example.com" }, "source_ref": "docs/x.md" },
                { "kind": "publish", "target": { "domain": "evil.example" } },
                { "kind": "spend", "amount": 12.5 },
                { "kind": "spend", "amount": 7.5 },
                { "kind": "post", "escalate": true }
            ]
        })
    }

    // ── select ──────────────────────────────────────────────────────────

    #[test]
    fn select_with_star_explodes_array_into_candidates() {
        let root = receipt();
        let candidates = select_candidates(&root, "$.actions[*]").unwrap();
        assert_eq!(candidates.len(), 5);
    }

    #[test]
    fn select_without_star_treats_whole_array_as_one_candidate() {
        let root = receipt();
        let candidates = select_candidates(&root, "$.actions").unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].is_array());
    }

    #[test]
    fn select_single_node_is_the_sole_candidate() {
        let root = receipt();
        let candidates = select_candidates(&root, "$.meta.run_id").unwrap();
        assert_eq!(candidates, vec![&json!("run-1")]);
    }

    #[test]
    fn select_missing_path_yields_zero_candidates() {
        let root = receipt();
        assert!(select_candidates(&root, "$.nonexistent[*]")
            .unwrap()
            .is_empty());
        assert!(select_candidates(&root, "$.nonexistent")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn select_star_on_a_non_array_falls_back_to_single_candidate() {
        let root = receipt();
        // `[*]` on a path that resolves to a non-array (defensive fallback,
        // not an error): the resolved node itself is the sole candidate.
        let candidates = select_candidates(&root, "$.meta.run_id[*]").unwrap();
        assert_eq!(candidates, vec![&json!("run-1")]);
    }

    #[test]
    fn empty_select_result_makes_aggregate_count_zero() {
        let root = json!({ "actions": [] });
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![],
                ReceiptAggregate::Count,
                ReceiptExpectation::Zero,
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["matched_count"], 0);
    }

    // ── ops: positive fails closed / negative satisfies on missing path ──

    #[test]
    fn eq_fails_closed_on_missing_path() {
        let candidate = json!({});
        assert!(!clause_matches(
            &clause("$.kind", WhereOp::Eq, Some(json!("publish"))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn ne_satisfies_on_missing_path() {
        let candidate = json!({});
        assert!(clause_matches(
            &clause("$.kind", WhereOp::Ne, Some(json!("publish"))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn in_fails_closed_on_missing_path() {
        let candidate = json!({});
        assert!(!clause_matches(
            &clause("$.target.domain", WhereOp::In, Some(json!(["example.com"]))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn not_in_satisfies_on_missing_path_the_allowlist_canonical_case() {
        // "count of actions whose target is not_in the allowlist must be
        // zero" — an action with NO domain field must count as violating,
        // i.e. `not_in` must be TRUE (satisfied) when the path is missing.
        let candidate = json!({ "kind": "publish" }); // no target.domain at all
        assert!(clause_matches(
            &clause(
                "$.target.domain",
                WhereOp::NotIn,
                Some(json!(["example.com"]))
            ),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn exists_false_and_absent_true_on_missing_path() {
        let candidate = json!({});
        assert!(
            !clause_matches(&clause("$.source_ref", WhereOp::Exists, None), &candidate).unwrap()
        );
        assert!(
            clause_matches(&clause("$.source_ref", WhereOp::Absent, None), &candidate).unwrap()
        );
    }

    #[test]
    fn exists_true_and_absent_false_when_present_including_null() {
        let candidate = json!({ "source_ref": Value::Null });
        assert!(
            clause_matches(&clause("$.source_ref", WhereOp::Exists, None), &candidate).unwrap()
        );
        assert!(
            !clause_matches(&clause("$.source_ref", WhereOp::Absent, None), &candidate).unwrap()
        );
    }

    #[test]
    fn matches_fails_closed_on_missing_path() {
        let candidate = json!({});
        assert!(!clause_matches(
            &clause("$.note", WhereOp::Matches, Some(json!("^ok$"))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn matches_runs_regex_against_string_value() {
        let candidate = json!({ "note": "escalation: high risk" });
        assert!(clause_matches(
            &clause("$.note", WhereOp::Matches, Some(json!("^escalation"))),
            &candidate
        )
        .unwrap());
        assert!(!clause_matches(
            &clause("$.note", WhereOp::Matches, Some(json!("^nope"))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn matches_on_non_string_value_fails_closed_not_error() {
        let candidate = json!({ "note": 123 });
        assert!(!clause_matches(
            &clause("$.note", WhereOp::Matches, Some(json!("123"))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn matches_invalid_regex_is_a_config_error() {
        let candidate = json!({ "note": "x" });
        let error = clause_matches(
            &clause("$.note", WhereOp::Matches, Some(json!("("))),
            &candidate,
        )
        .unwrap_err();
        assert!(error.contains("invalid regex"), "error: {error}");
    }

    #[test]
    fn comparisons_fail_closed_on_missing_path() {
        let candidate = json!({});
        for op in [WhereOp::Gt, WhereOp::Gte, WhereOp::Lt, WhereOp::Lte] {
            assert!(
                !clause_matches(&clause("$.amount", op, Some(json!(10))), &candidate).unwrap(),
                "{op:?} should fail closed on a missing path"
            );
        }
    }

    #[test]
    fn comparisons_evaluate_numerically() {
        let candidate = json!({ "amount": 10 });
        assert!(
            clause_matches(&clause("$.amount", WhereOp::Gt, Some(json!(5))), &candidate).unwrap()
        );
        assert!(!clause_matches(
            &clause("$.amount", WhereOp::Gt, Some(json!(10))),
            &candidate
        )
        .unwrap());
        assert!(clause_matches(
            &clause("$.amount", WhereOp::Gte, Some(json!(10))),
            &candidate
        )
        .unwrap());
        assert!(clause_matches(
            &clause("$.amount", WhereOp::Lt, Some(json!(11))),
            &candidate
        )
        .unwrap());
        assert!(!clause_matches(
            &clause("$.amount", WhereOp::Lt, Some(json!(10))),
            &candidate
        )
        .unwrap());
        assert!(clause_matches(
            &clause("$.amount", WhereOp::Lte, Some(json!(10))),
            &candidate
        )
        .unwrap());
    }

    #[test]
    fn comparison_on_non_numeric_present_value_fails_closed_not_error() {
        let candidate = json!({ "amount": "a lot" });
        assert!(
            !clause_matches(&clause("$.amount", WhereOp::Gt, Some(json!(5))), &candidate).unwrap()
        );
    }

    #[test]
    fn missing_configured_value_is_a_config_error_for_ops_that_need_one() {
        let candidate = json!({ "kind": "publish" });
        let error = clause_matches(&clause("$.kind", WhereOp::Eq, None), &candidate).unwrap_err();
        assert!(error.contains("requires a 'value'"), "error: {error}");
    }

    #[test]
    fn in_with_non_array_configured_value_is_a_config_error() {
        let candidate = json!({ "kind": "publish" });
        let error = clause_matches(
            &clause("$.kind", WhereOp::In, Some(json!("not-an-array"))),
            &candidate,
        )
        .unwrap_err();
        assert!(error.contains("JSON array"), "error: {error}");
    }

    // ── aggregate: count / sum, and sum's loud non-numeric rule ──────────

    #[test]
    fn count_aggregate_counts_matching_candidates() {
        let root = receipt();
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![clause("$.kind", WhereOp::Eq, Some(json!("publish")))],
                ReceiptAggregate::Count,
                ReceiptExpectation::Exactly { value: 2.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["matched_count"], 2);
    }

    #[test]
    fn sum_aggregate_sums_matching_candidates() {
        let root = receipt();
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![clause("$.kind", WhereOp::Eq, Some(json!("spend")))],
                ReceiptAggregate::Sum {
                    path: "$.amount".to_string(),
                },
                ReceiptExpectation::AtMost { value: 25.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["actual"], 20.0);
    }

    #[test]
    fn sum_over_missing_value_on_a_matching_candidate_errors_loudly() {
        let root = json!({
            "actions": [
                { "kind": "spend", "amount": 5 },
                { "kind": "spend" } // no amount at all
            ]
        });
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![clause("$.kind", WhereOp::Eq, Some(json!("spend")))],
                ReceiptAggregate::Sum {
                    path: "$.amount".to_string(),
                },
                ReceiptExpectation::AtMost { value: 100.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn sum_over_non_numeric_value_on_a_matching_candidate_errors_loudly() {
        let root = json!({
            "actions": [ { "kind": "spend", "amount": "a lot" } ]
        });
        let (status, _) = evaluate(
            &args(
                "$.actions[*]",
                vec![clause("$.kind", WhereOp::Eq, Some(json!("spend")))],
                ReceiptAggregate::Sum {
                    path: "$.amount".to_string(),
                },
                ReceiptExpectation::AtMost { value: 100.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Error);
    }

    #[test]
    fn sum_ignores_missing_value_on_a_non_matching_candidate() {
        // A candidate that does NOT pass `where` is irrelevant to the sum —
        // its own missing/non-numeric `amount` must not error the check.
        let root = json!({
            "actions": [
                { "kind": "spend", "amount": 5 },
                { "kind": "publish" } // not a spend action; no amount at all
            ]
        });
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![clause("$.kind", WhereOp::Eq, Some(json!("spend")))],
                ReceiptAggregate::Sum {
                    path: "$.amount".to_string(),
                },
                ReceiptExpectation::AtMost { value: 100.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["actual"], 5.0);
    }

    // ── expected: all four kinds ──────────────────────────────────────────

    #[test]
    fn expectation_zero() {
        assert!(ReceiptExpectation::Zero.is_met_by(0.0));
        assert!(!ReceiptExpectation::Zero.is_met_by(1.0));
    }

    #[test]
    fn expectation_exactly() {
        let e = ReceiptExpectation::Exactly { value: 3.0 };
        assert!(e.is_met_by(3.0));
        assert!(!e.is_met_by(2.0));
        assert!(!e.is_met_by(4.0));
    }

    #[test]
    fn expectation_at_least() {
        let e = ReceiptExpectation::AtLeast { value: 3.0 };
        assert!(e.is_met_by(3.0));
        assert!(e.is_met_by(4.0));
        assert!(!e.is_met_by(2.0));
    }

    #[test]
    fn expectation_at_most() {
        let e = ReceiptExpectation::AtMost { value: 3.0 };
        assert!(e.is_met_by(3.0));
        assert!(e.is_met_by(2.0));
        assert!(!e.is_met_by(4.0));
    }

    // ── run(): the no-subject defensive path ──────────────────────────────

    #[test]
    fn run_without_a_bound_receipt_subject_errors_rather_than_silently_passing() {
        let (status, _) = run(
            &args(
                "$.actions[*]",
                vec![],
                ReceiptAggregate::Count,
                ReceiptExpectation::Zero,
            ),
            None,
        );
        assert_eq!(status, ObligationStatus::Error);
    }

    // ── end-to-end evaluate(): the addendum's own canonical patterns ─────

    #[test]
    fn canonical_provenance_pattern() {
        // "publish-actions with source_ref absent: expected zero."
        let root = receipt();
        let (status, detail) = evaluate(
            &args(
                "$.actions[*]",
                vec![
                    clause("$.kind", WhereOp::Eq, Some(json!("publish"))),
                    clause("$.source_ref", WhereOp::Absent, None),
                ],
                ReceiptAggregate::Count,
                ReceiptExpectation::Zero,
            ),
            &root,
        );
        // The fixture has exactly one publish action missing source_ref
        // (the "evil.example" one) — this obligation should FAIL on it.
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["matched_count"], 1);
    }

    #[test]
    fn canonical_allowlist_pattern() {
        // "actions whose target is not_in [...]: expected zero."
        let root = receipt();
        let (status, _) = evaluate(
            &args(
                "$.actions[*]",
                vec![
                    clause("$.kind", WhereOp::Eq, Some(json!("publish"))),
                    clause(
                        "$.target.domain",
                        WhereOp::NotIn,
                        Some(json!(["example.com"])),
                    ),
                ],
                ReceiptAggregate::Count,
                ReceiptExpectation::Zero,
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Failed); // evil.example violates
    }

    #[test]
    fn canonical_envelope_sanity_pattern() {
        // "select $.meta.run_id, count: expected exactly 1."
        let root = receipt();
        let (status, detail) = evaluate(
            &args(
                "$.meta.run_id",
                vec![],
                ReceiptAggregate::Count,
                ReceiptExpectation::Exactly { value: 1.0 },
            ),
            &root,
        );
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn canonical_escalation_marking_pattern() {
        // "risky actions without escalate: true: expected zero." (advisory
        // in practice, but the evaluator itself doesn't know about
        // advisory — that's `Signal.advisory`, layered on top.)
        let root = receipt();
        let (status, _) = evaluate(
            &args(
                "$.actions[*]",
                vec![
                    clause("$.kind", WhereOp::Eq, Some(json!("post"))),
                    clause("$.escalate", WhereOp::Ne, Some(json!(true))),
                ],
                ReceiptAggregate::Count,
                ReceiptExpectation::Zero,
            ),
            &root,
        );
        // The one "post" action DOES have escalate: true, so zero posts
        // violate the rule.
        assert_eq!(status, ObligationStatus::Passed);
    }

    // ── unsupported path syntax is an Error, never a silent pass (0.1.3) ──

    #[test]
    fn select_with_malformed_descent_is_refused() {
        let root = receipt();
        for select in [
            "$...actions[*]",
            "$..",
            "$.actions..",
            "$..[*]",
            "$.actions[x]",
        ] {
            let err = select_candidates(&root, select).unwrap_err();
            assert!(err.contains("unsupported path syntax"), "{select}: {err}");
        }
    }

    #[test]
    fn recursive_select_with_expected_zero_counts_the_violations_instead_of_passing() {
        // The 0.1.2 fail-open: three violations present, `$..` matched nothing,
        // gate passed. 0.1.3 made it an Error; with descent implemented the
        // three violations are counted and the gate fails.
        let root =
            serde_json::json!({"claims": [{"kind": "bad"}, {"kind": "bad"}, {"kind": "bad"}]});
        let args = ReceiptQueryArgs {
            select: "$..claims[*]".into(),
            where_clauses: vec![WhereClause {
                path: "$.kind".into(),
                op: WhereOp::Eq,
                value: Some(serde_json::json!("bad")),
            }],
            aggregate: ReceiptAggregate::Count,
            expected: ReceiptExpectation::Zero,
        };
        let (status, detail) = evaluate(&args, &root);
        assert_eq!(status, ObligationStatus::Failed, "{detail}");
        assert_eq!(detail["matched_count"], 3, "{detail}");
    }

    #[test]
    fn where_path_with_wildcard_counts_the_violation_instead_of_passing() {
        let root = serde_json::json!({"claims": [{"kind": "bad"}]});
        let args = ReceiptQueryArgs {
            select: "$".into(),
            where_clauses: vec![WhereClause {
                path: "$.claims[*].kind".into(),
                op: WhereOp::Eq,
                value: Some(serde_json::json!("bad")),
            }],
            aggregate: ReceiptAggregate::Count,
            expected: ReceiptExpectation::Zero,
        };
        let (status, detail) = evaluate(&args, &root);
        assert_eq!(status, ObligationStatus::Failed, "{detail}");
        assert_eq!(detail["matched_count"], 1, "{detail}");
    }

    #[test]
    fn where_path_with_malformed_syntax_errors_instead_of_passing() {
        let root = serde_json::json!({"claims": [{"kind": "bad"}]});
        for path in ["$.claims[*]..", "$...kind", "$.claims[x].kind", "$.*"] {
            let args = ReceiptQueryArgs {
                select: "$".into(),
                where_clauses: vec![WhereClause {
                    path: path.into(),
                    op: WhereOp::NotIn,
                    value: Some(serde_json::json!(["good"])),
                }],
                aggregate: ReceiptAggregate::Count,
                expected: ReceiptExpectation::Zero,
            };
            let (status, detail) = evaluate(&args, &root);
            assert_eq!(status, ObligationStatus::Error, "{path}: {detail}");
            assert!(
                detail["error"]
                    .as_str()
                    .unwrap()
                    .contains("unsupported path syntax"),
                "{detail}"
            );
        }
    }

    #[test]
    fn sum_path_with_recursive_descent_errors() {
        let root = serde_json::json!({"actions": [{"amount": 1}]});
        let args = ReceiptQueryArgs {
            select: "$.actions[*]".into(),
            where_clauses: vec![],
            aggregate: ReceiptAggregate::Sum {
                path: "$..amount".into(),
            },
            expected: ReceiptExpectation::Zero,
        };
        let (status, detail) = evaluate(&args, &root);
        assert_eq!(status, ObligationStatus::Error);
        assert!(
            detail["error"]
                .as_str()
                .unwrap()
                .contains("multi-valued path is not supported"),
            "{detail}"
        );
    }

    #[test]
    fn sum_path_with_wildcard_errors() {
        let root = serde_json::json!({"actions": [{"amounts": [1, 2]}]});
        let args = ReceiptQueryArgs {
            select: "$.actions[*]".into(),
            where_clauses: vec![],
            aggregate: ReceiptAggregate::Sum {
                path: "$.amounts[*]".into(),
            },
            expected: ReceiptExpectation::AtMost { value: 100.0 },
        };
        let (status, detail) = evaluate(&args, &root);
        assert_eq!(status, ObligationStatus::Error, "{detail}");
        assert!(
            detail["error"]
                .as_str()
                .unwrap()
                .contains("multi-valued path is not supported"),
            "{detail}"
        );
    }

    // ── recursive select (D2) ────────────────────────────────────────────

    fn nested_claims() -> Value {
        json!({
            "claims": [
                {"id": "C1", "gate": true, "claims": [
                    {"id": "C1a", "check": {"kind": "command"}},
                    {"id": "C1b", "gate": true, "claims": [
                        {"id": "C1b-i", "check": {"kind": "human"}}
                    ]}
                ]},
                {"id": "C2", "check": {"kind": "rg"}}
            ]
        })
    }

    fn ids(candidates: &[&Value]) -> Vec<String> {
        candidates
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn recursive_select_returns_every_claim_at_every_depth() {
        let root = nested_claims();
        let candidates = select_candidates(&root, "$..claims[*]").unwrap();
        assert_eq!(ids(&candidates), ["C1", "C2", "C1a", "C1b", "C1b-i"]);
    }

    #[test]
    fn recursive_select_reaches_claims_under_children_only_by_key() {
        // JSONPath `..claims` follows the key name: claims nested under a
        // `children` key are not under `claims`, and `$..children[*]`
        // selects them.
        let root = json!({
            "claims": [{"id": "C1", "children": [{"id": "C1a"}, {"id": "C1b", "children": [{"id": "C1b-i"}]}]}]
        });
        assert_eq!(
            ids(&select_candidates(&root, "$..claims[*]").unwrap()),
            ["C1"]
        );
        assert_eq!(
            ids(&select_candidates(&root, "$..children[*]").unwrap()),
            ["C1a", "C1b", "C1b-i"]
        );
    }

    #[test]
    fn recursive_select_without_star_yields_each_matched_array() {
        let root = nested_claims();
        let candidates = select_candidates(&root, "$..claims").unwrap();
        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().all(|c| c.is_array()));
    }

    #[test]
    fn recursive_select_star_falls_back_to_each_non_array_node() {
        let root = json!({"a": {"k": "x"}, "b": [{"k": ["y", "z"]}]});
        let candidates = select_candidates(&root, "$..k[*]").unwrap();
        assert_eq!(candidates, vec![&json!("x"), &json!("y"), &json!("z")]);
    }

    #[test]
    fn recursive_select_counts_the_nested_human_gate_claim() {
        let root = nested_claims();
        let query = args(
            "$..claims[*]",
            vec![
                clause("$.gate", WhereOp::Ne, Some(json!(false))),
                clause("$.check.kind", WhereOp::Eq, Some(json!("human"))),
            ],
            ReceiptAggregate::Count,
            ReceiptExpectation::Zero,
        );
        let (status, detail) = evaluate(&query, &root);
        assert_eq!(status, ObligationStatus::Failed, "{detail}");
        assert_eq!(detail["selected_count"], 5, "{detail}");
        assert_eq!(detail["matched_count"], 1, "{detail}");
    }

    // ── multi-valued where: ANY value satisfies the per-value predicate (D3) ──

    fn multi(op: WhereOp, value: Option<Value>, targets: Value) -> bool {
        clause_matches(
            &clause("$.targets[*]", op, value),
            &json!({ "targets": targets }),
        )
        .unwrap()
    }

    #[test]
    fn multi_eq_is_any_value_equal() {
        assert!(multi(WhereOp::Eq, Some(json!("b")), json!(["a", "b"])));
        assert!(!multi(WhereOp::Eq, Some(json!("c")), json!(["a", "b"])));
    }

    #[test]
    fn multi_ne_is_any_value_different_not_a_set_negation() {
        assert!(multi(WhereOp::Ne, Some(json!("a")), json!(["a", "b"])));
        assert!(!multi(WhereOp::Ne, Some(json!("a")), json!(["a", "a"])));
        // Not the negation of `eq`: both hold for ["a", "b"].
        assert!(multi(WhereOp::Eq, Some(json!("a")), json!(["a", "b"])));
    }

    #[test]
    fn multi_in_is_any_value_member() {
        assert!(multi(
            WhereOp::In,
            Some(json!(["b", "z"])),
            json!(["a", "b"])
        ));
        assert!(!multi(WhereOp::In, Some(json!(["z"])), json!(["a", "b"])));
    }

    #[test]
    fn multi_not_in_is_any_value_outside() {
        assert!(multi(
            WhereOp::NotIn,
            Some(json!(["allowed"])),
            json!(["allowed", "evil"])
        ));
        assert!(!multi(
            WhereOp::NotIn,
            Some(json!(["allowed"])),
            json!(["allowed", "allowed"])
        ));
    }

    #[test]
    fn multi_matches_is_any_string_value_matching() {
        assert!(multi(
            WhereOp::Matches,
            Some(json!("^ev")),
            json!([1, "ok", "evil"])
        ));
        assert!(!multi(
            WhereOp::Matches,
            Some(json!("^ev")),
            json!([1, "ok"])
        ));
    }

    #[test]
    fn multi_comparisons_are_any_value() {
        let values = json!([1, "x", 10]);
        assert!(multi(WhereOp::Gt, Some(json!(5)), values.clone()));
        assert!(!multi(WhereOp::Gt, Some(json!(10)), values.clone()));
        assert!(multi(WhereOp::Gte, Some(json!(10)), values.clone()));
        assert!(multi(WhereOp::Lt, Some(json!(2)), values.clone()));
        assert!(!multi(WhereOp::Lt, Some(json!(1)), values.clone()));
        assert!(multi(WhereOp::Lte, Some(json!(1)), values));
    }

    #[test]
    fn multi_exists_and_absent() {
        assert!(multi(WhereOp::Exists, None, json!(["a", "b"])));
        assert!(!multi(WhereOp::Absent, None, json!(["a", "b"])));
        assert!(!multi(WhereOp::Exists, None, json!([])));
        assert!(multi(WhereOp::Absent, None, json!([])));
    }

    #[test]
    fn multi_empty_result_keeps_the_missing_path_table() {
        // Positive ops false, negative ops true, exactly as for a missing path.
        for (op, value, expected) in [
            (WhereOp::Eq, json!("a"), false),
            (WhereOp::In, json!(["a"]), false),
            (WhereOp::Matches, json!("a"), false),
            (WhereOp::Gt, json!(0), false),
            (WhereOp::Gte, json!(0), false),
            (WhereOp::Lt, json!(0), false),
            (WhereOp::Lte, json!(0), false),
            (WhereOp::Ne, json!("a"), true),
            (WhereOp::NotIn, json!(["a"]), true),
        ] {
            assert_eq!(
                multi(op, Some(value.clone()), json!([])),
                expected,
                "{op:?} over []"
            );
            assert_eq!(
                clause_matches(&clause("$..nowhere", op, Some(value)), &json!({"a": 1})).unwrap(),
                expected,
                "{op:?} over a descent that finds nothing"
            );
        }
    }

    #[test]
    fn multi_empty_result_still_validates_the_configured_value() {
        let error = clause_matches(
            &clause("$.targets[*]", WhereOp::NotIn, None),
            &json!({"targets": []}),
        )
        .unwrap_err();
        assert!(error.contains("requires a 'value'"), "{error}");
        let error = clause_matches(
            &clause("$.targets[*]", WhereOp::Matches, Some(json!("("))),
            &json!({"targets": []}),
        )
        .unwrap_err();
        assert!(error.contains("invalid regex"), "{error}");
    }

    #[test]
    fn canonical_allowlist_gate_over_a_multi_valued_path_fails_closed() {
        let query = args(
            "$.actions[*]",
            vec![clause(
                "$.targets[*]",
                WhereOp::NotIn,
                Some(json!(["allowed"])),
            )],
            ReceiptAggregate::Count,
            ReceiptExpectation::Zero,
        );
        let (status, detail) = evaluate(
            &query,
            &json!({"actions": [{"targets": ["allowed", "evil"]}]}),
        );
        assert_eq!(status, ObligationStatus::Failed, "{detail}");
        assert_eq!(detail["matched_count"], 1, "{detail}");

        let (status, detail) = evaluate(&query, &json!({"actions": [{"targets": ["allowed"]}]}));
        assert_eq!(status, ObligationStatus::Passed, "{detail}");
        assert_eq!(detail["matched_count"], 0, "{detail}");
    }
}
