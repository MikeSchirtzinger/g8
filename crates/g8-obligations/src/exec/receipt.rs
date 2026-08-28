//! `ReceiptQuery` — contract addendum (o-g8-receipts-20260721), backend
//! #11. A structural evaluator over a *parsed action receipt* — never the
//! repository. The receipt is parsed and SHA-256-hashed exactly once per
//! `g8 check --receipt` invocation (see [`crate::load_receipt_subject`],
//! [`crate::ReceiptSubject`]); this module only evaluates against the
//! already-parsed `serde_json::Value`, never touches the filesystem itself.
//!
//! Path grammar reuses [`super::json_path`] verbatim (the same dialect
//! `FixtureIntegrationTest`'s `JsonField`/`JsonPathNonEmpty` assertions use),
//! plus one addition: a trailing `[*]` on `select` explodes an array into
//! per-element candidates.
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

use super::json_path::resolve_json_path;

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
    let candidates = select_candidates(root, &args.select);

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

/// `select` resolves `path` (after stripping a trailing `[*]`, if present).
/// A trailing `[*]` requests explosion: if the resolved node is an array,
/// candidates are its elements; if it isn't (a receipt shaped differently
/// than expected), the resolved node itself is the sole candidate rather
/// than erroring. Without `[*]`, the resolved node (array or not) is always
/// the sole candidate — e.g. `$.meta.run_id` (a single string) never needs
/// `[*]`. A path that does not resolve at all yields zero candidates,
/// regardless of `[*]`.
fn select_candidates<'a>(root: &'a Value, select: &str) -> Vec<&'a Value> {
    let (base_path, explode) = match select.strip_suffix("[*]") {
        Some(base) => (base, true),
        None => (select, false),
    };
    match resolve_json_path(root, base_path) {
        None => vec![],
        Some(node) => {
            if explode {
                match node.as_array() {
                    Some(array) => array.iter().collect(),
                    None => vec![node],
                }
            } else {
                vec![node]
            }
        }
    }
}

fn clauses_match(clauses: &[WhereClause], candidate: &Value) -> Result<bool, String> {
    for clause in clauses {
        if !clause_matches(clause, candidate)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluates one clause. Each op's "positive" form (`Eq`/`In`/`Matches`/
/// `Exists`/the four comparisons) resolves to `false` when `clause.path`
/// does not resolve; the "negative" forms (`Ne`/`NotIn`/`Absent`) are exact
/// logical negations of their positive counterparts, which is exactly what
/// makes them resolve to `true` on a missing path — the fail-closed table
/// (backend.rs's `WhereClause` doc) is a restatement of this, not a
/// separate rule to keep in sync.
fn clause_matches(clause: &WhereClause, candidate: &Value) -> Result<bool, String> {
    let resolved = resolve_json_path(candidate, &clause.path);
    match clause.op {
        WhereOp::Exists => Ok(resolved.is_some()),
        WhereOp::Absent => Ok(resolved.is_none()),
        WhereOp::Eq => Ok(op_eq(resolved, require_value(clause)?)),
        WhereOp::Ne => Ok(!op_eq(resolved, require_value(clause)?)),
        WhereOp::In => Ok(op_in(resolved, require_array_value(clause)?)),
        WhereOp::NotIn => Ok(!op_in(resolved, require_array_value(clause)?)),
        WhereOp::Matches => op_matches(resolved, require_str_value(clause)?),
        WhereOp::Gt => Ok(op_cmp(resolved, require_num_value(clause)?, |a, b| a > b)),
        WhereOp::Gte => Ok(op_cmp(resolved, require_num_value(clause)?, |a, b| a >= b)),
        WhereOp::Lt => Ok(op_cmp(resolved, require_num_value(clause)?, |a, b| a < b)),
        WhereOp::Lte => Ok(op_cmp(resolved, require_num_value(clause)?, |a, b| a <= b)),
    }
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

/// Deep (structural) equality — `serde_json::Value`'s own `PartialEq`
/// already compares arrays element-wise and objects key-for-key, so this is
/// "deep eq" for free. `None` (missing path) is never equal to anything.
fn op_eq(resolved: Option<&Value>, configured: &Value) -> bool {
    resolved.map(|r| r == configured).unwrap_or(false)
}

/// Membership by the same deep equality. `None` (missing path) is never a
/// member of anything.
fn op_in(resolved: Option<&Value>, configured: &[Value]) -> bool {
    resolved
        .map(|r| configured.iter().any(|item| item == r))
        .unwrap_or(false)
}

fn op_matches(resolved: Option<&Value>, pattern: &str) -> Result<bool, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("invalid regex '{pattern}': {e}"))?;
    Ok(resolved
        .and_then(Value::as_str)
        .map(|s| re.is_match(s))
        .unwrap_or(false))
}

fn op_cmp(resolved: Option<&Value>, configured: f64, cmp: impl Fn(f64, f64) -> bool) -> bool {
    resolved
        .and_then(Value::as_f64)
        .map(|actual| cmp(actual, configured))
        .unwrap_or(false)
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
        let candidates = select_candidates(&root, "$.actions[*]");
        assert_eq!(candidates.len(), 5);
    }

    #[test]
    fn select_without_star_treats_whole_array_as_one_candidate() {
        let root = receipt();
        let candidates = select_candidates(&root, "$.actions");
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].is_array());
    }

    #[test]
    fn select_single_node_is_the_sole_candidate() {
        let root = receipt();
        let candidates = select_candidates(&root, "$.meta.run_id");
        assert_eq!(candidates, vec![&json!("run-1")]);
    }

    #[test]
    fn select_missing_path_yields_zero_candidates() {
        let root = receipt();
        assert!(select_candidates(&root, "$.nonexistent[*]").is_empty());
        assert!(select_candidates(&root, "$.nonexistent").is_empty());
    }

    #[test]
    fn select_star_on_a_non_array_falls_back_to_single_candidate() {
        let root = receipt();
        // `[*]` on a path that resolves to a non-array (defensive fallback,
        // not an error): the resolved node itself is the sole candidate.
        let candidates = select_candidates(&root, "$.meta.run_id[*]");
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
}
