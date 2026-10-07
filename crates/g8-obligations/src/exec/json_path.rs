//! Shared `$.`-prefixed JSON path resolution dialect.
//!
//! Originally lived in `exec::fixture` (built for `FixtureIntegrationTest`'s
//! `JsonField`/`JsonPathNonEmpty` assertions); factored out here, verbatim,
//! so `exec::receipt`'s `receipt_query` backend (contract addendum
//! o-g8-receipts-20260721) can reuse the exact same dialect instead of a
//! second, drifting implementation. Behavior is unchanged from the original
//! `fixture.rs` version — this is a pure relocation.
//!
//! Two resolvers share the dialect. [`resolve_json_path`] is single-valued
//! (`$.key.key[0]` forms) and serves `fixture.rs` and `receipt_query`'s
//! `sum`. [`resolve_json_path_all`] is multi-valued and serves
//! `receipt_query`'s `select` and `where`: on top of the single-valued forms
//! it implements recursive descent `..key` and the wildcard `[*]` at any
//! position, and it refuses any path it cannot tokenize with an `Err`, so
//! unsupported syntax never reads as "matches nothing".

use serde_json::Value;
use std::collections::HashSet;

/// Minimal path resolver covering the forms the contract's own examples use:
/// a bare key (`"enforcement"`), a `$.`-prefixed dot-path (`"$.drift_hints"`),
/// a bare numeric segment indexing into an array (`"$.data.0"`), and —
/// load-bearing for OBL-D4-01, the contract's own canonical mapping-table
/// example (§2 row 9) — bracket-index array access (`"$.data[0].description"`).
///
/// The original implementation only understood the bare-numeric-segment
/// form. Bracket-index paths silently failed to resolve at all (a segment
/// like `"data[0]"` was looked up as a literal object key, which never
/// exists), making `JsonField`/`JsonPathNonEmpty` assertions written in the
/// contract's own documented syntax unconditionally return `false`/`None`
/// regardless of the underlying JSON — this is what made OBL-D4-01's
/// assertion index 2 fail even once the `{{fixture_dir}}` substitution and
/// query results were confirmed correct (T3d root cause).
pub(crate) fn resolve_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let trimmed = path.strip_prefix('$').unwrap_or(path);
    let trimmed = trimmed.strip_prefix('.').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for segment in trimmed.split('.') {
        current = resolve_segment(current, segment)?;
    }
    Some(current)
}

/// Resolves one dot-separated path segment, which may carry a leading key
/// and/or one or more trailing bracket-index groups: `"data"` (bare key),
/// `"0"` (bare numeric index), `"data[0]"` (key then index), or
/// `"data[0][1]"` (chained indices, for completeness — no obligation today
/// needs more than one level, but the grammar shouldn't silently stop
/// working the moment one does).
fn resolve_segment<'a>(current: &'a Value, segment: &str) -> Option<&'a Value> {
    let Some(bracket_start) = segment.find('[') else {
        return index_into(current, segment);
    };
    let key = &segment[..bracket_start];
    let mut result = if key.is_empty() {
        current
    } else {
        index_into(current, key)?
    };
    let mut rest = &segment[bracket_start..];
    while let Some(after_open) = rest.strip_prefix('[') {
        let close = after_open.find(']')?;
        let idx: usize = after_open[..close].parse().ok()?;
        result = result.get(idx)?;
        rest = &after_open[close + 1..];
    }
    Some(result)
}

/// Rejects path syntax this resolver does not implement, so a gate written
/// in a richer dialect errors instead of silently matching nothing.
///
/// `resolve_json_path` answers `None` for any path it cannot follow, which
/// is right for a key that is absent and wrong for syntax it never parsed:
/// `$..claims[*]` split into an empty segment and resolved to nothing, so a
/// `receipt_query` with `expected: zero` passed against a receipt full of
/// violations (found 2026-10-07 on 0.1.2). Recursive descent (`..`) and a
/// wildcard anywhere but as `select`'s trailing explosion (`[*]`) are the
/// two forms callers reach for; both are refused here until the resolver
/// implements them. Empty segments (`$.a..b`, `$.`), unclosed brackets and
/// non-numeric indices are refused for the same reason.
pub(crate) fn validate_supported_path(path: &str) -> Result<(), String> {
    let refuse = |why: &str| {
        Err(format!(
            "unsupported path syntax `{path}`: {why}; the resolver supports `$.key.key[0]` forms only, \
             and a path it cannot parse must not read as \"matches nothing\""
        ))
    };
    if path.contains("..") {
        return refuse("recursive descent `..` is not implemented");
    }
    if path.contains("[*]") {
        return refuse("wildcard `[*]` is only accepted as the trailing explosion of `select`");
    }
    let trimmed = path.strip_prefix('$').unwrap_or(path);
    if trimmed.is_empty() {
        return Ok(());
    }
    let Some(trimmed) = trimmed.strip_prefix('.') else {
        if path.starts_with('$') {
            return refuse("`$` must be followed by `.`");
        }
        return validate_segments(trimmed, refuse);
    };
    validate_segments(trimmed, refuse)
}

fn validate_segments(
    trimmed: &str,
    refuse: impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if trimmed.is_empty() {
        return refuse("trailing `.` leaves an empty segment");
    }
    for segment in trimmed.split('.') {
        if segment.is_empty() {
            return refuse("empty segment");
        }
        let Some(bracket_start) = segment.find('[') else {
            continue;
        };
        let mut rest = &segment[bracket_start..];
        while let Some(after_open) = rest.strip_prefix('[') {
            let Some(close) = after_open.find(']') else {
                return refuse("unclosed `[`");
            };
            if after_open[..close].parse::<usize>().is_err() {
                return refuse("a bracket index must be a non-negative integer");
            }
            rest = &after_open[close + 1..];
        }
        if !rest.is_empty() {
            return refuse("text after a closing `]`");
        }
    }
    Ok(())
}

/// One step of a multi-valued path, produced by [`tokenize_path`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathToken {
    /// `.key` (or a leading bare `key`): the member `key`, or the array
    /// element at that index if `key` is a plain non-negative integer
    /// (the same rule as [`index_into`]).
    Child(String),
    /// `[n]`: the array element at index `n`.
    Index(usize),
    /// `[*]`: every element of an array; a non-array node yields itself.
    Wildcard,
    /// `..key`: the member `key` of the node and of every node below it,
    /// including a `key` nested inside a matched `key`.
    Descendant(String),
}

/// Splits a path into [`PathToken`]s, or refuses it.
///
/// Grammar: an optional `$`, then any sequence of `.key`, `..key`, `[n]` and
/// `[*]`. A path without `$` may start with a bare key (`enforcement`), as
/// [`resolve_json_path`] allows. A key is a non-empty run of characters up to
/// the next `.` or `[`; a key of `*` is refused (member wildcard is not
/// implemented), as are `..` not followed by a key (`$..`, `$...`, `$..[0]`),
/// empty segments (`$.`, `$.a..`), unclosed or non-numeric brackets and text
/// after a closing `]`.
fn tokenize_path(path: &str) -> Result<Vec<PathToken>, String> {
    let refuse = |why: &str| {
        Err(format!(
            "unsupported path syntax `{path}`: {why}; the resolver supports `$`, `.key`, `..key`, \
             `[n]` and `[*]`, and a path it cannot parse must not read as \"matches nothing\""
        ))
    };
    let rest = match path.strip_prefix('$') {
        Some(rest) => {
            if !(rest.is_empty() || rest.starts_with('.') || rest.starts_with('[')) {
                return refuse("`$` must be followed by `.` or `[`");
            }
            rest.to_string()
        }
        None if path.starts_with('.') || path.starts_with('[') => path.to_string(),
        None => format!(".{path}"),
    };

    let mut tokens = Vec::new();
    let mut rest = rest.as_str();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("..") {
            let (key, tail) = split_key(after);
            if key.is_empty() {
                return refuse("recursive descent `..` must be followed by a key");
            }
            if key == "*" {
                return refuse("wildcard `..*` is not implemented");
            }
            tokens.push(PathToken::Descendant(key.to_string()));
            rest = tail;
        } else if let Some(after) = rest.strip_prefix('.') {
            let (key, tail) = split_key(after);
            if key.is_empty() {
                return refuse("empty segment");
            }
            if key == "*" {
                return refuse("wildcard `.*` is not implemented; use `[*]` on an array");
            }
            tokens.push(PathToken::Child(key.to_string()));
            rest = tail;
        } else if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return refuse("unclosed `[`");
            };
            let inner = &after[..close];
            if inner == "*" {
                tokens.push(PathToken::Wildcard);
            } else {
                match inner.parse::<usize>() {
                    Ok(idx) => tokens.push(PathToken::Index(idx)),
                    Err(_) => {
                        return refuse("a bracket must hold `*` or a non-negative integer index")
                    }
                }
            }
            rest = &after[close + 1..];
            if !(rest.is_empty() || rest.starts_with('.') || rest.starts_with('[')) {
                return refuse("text after a closing `]`");
            }
        } else {
            return refuse("expected `.`, `..` or `[`");
        }
    }
    Ok(tokens)
}

/// Splits `s` at the first `.` or `[`: the key before it and the rest.
fn split_key(s: &str) -> (&str, &str) {
    let end = s.find(['.', '[']).unwrap_or(s.len());
    s.split_at(end)
}

/// Multi-valued resolver: every node `path` reaches from `value`, in
/// document order, each node at most once.
///
/// `..key` follows JSONPath: it visits the node and every node below it and
/// collects each one's `key` member, so a `key` nested inside a matched `key`
/// is collected too (`byte_diff.rs`'s masking walk stops at the first match;
/// this does not). `[*]` yields every element of an array and the node
/// itself for a non-array, matching `receipt_query`'s long-standing `select`
/// fallback. A node reached twice (`$..a..b` with nested `a`s) is yielded
/// once, so a count or a sum never sees one node as two; distinct nodes with
/// equal values stay distinct. A missing key or index yields nothing. A path
/// that does not tokenize is an `Err`.
pub(crate) fn resolve_json_path_all<'a>(
    value: &'a Value,
    path: &str,
) -> Result<Vec<&'a Value>, String> {
    let tokens = tokenize_path(path)?;
    let mut current = vec![value];
    for token in &tokens {
        let mut next = Vec::new();
        for node in current {
            match token {
                PathToken::Child(key) => next.extend(index_into(node, key)),
                PathToken::Index(idx) => next.extend(node.get(*idx)),
                PathToken::Wildcard => match node.as_array() {
                    Some(array) => next.extend(array.iter()),
                    None => next.push(node),
                },
                PathToken::Descendant(key) => collect_key_recursive(node, key, &mut next),
            }
        }
        current = dedup_by_identity(next);
    }
    Ok(current)
}

/// Read-only descendant walk for `..key`: `node`'s own `key` member first,
/// then each child's subtree in order. Unlike `byte_diff.rs`'s
/// `mask_key_recursive`, it keeps descending into a matched value.
fn collect_key_recursive<'a>(node: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                out.push(found);
            }
            for child in map.values() {
                collect_key_recursive(child, key, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_key_recursive(child, key, out);
            }
        }
        _ => {}
    }
}

fn dedup_by_identity(nodes: Vec<&Value>) -> Vec<&Value> {
    let mut seen = HashSet::new();
    nodes
        .into_iter()
        .filter(|node| seen.insert(std::ptr::from_ref::<Value>(node)))
        .collect()
}

/// `true` when `path` can reach more than one node (`..key` or `[*]`).
/// `Err` when it does not tokenize.
pub(crate) fn path_fans_out(path: &str) -> Result<bool, String> {
    Ok(tokenize_path(path)?
        .iter()
        .any(|token| matches!(token, PathToken::Wildcard | PathToken::Descendant(_))))
}

/// Looks up `key` in `current` — as an array index if `key` parses as a
/// plain non-negative integer, otherwise as an object key.
fn index_into<'a>(current: &'a Value, key: &str) -> Option<&'a Value> {
    if let Ok(idx) = key.parse::<usize>() {
        current.get(idx)
    } else {
        current.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bare_key_resolves() {
        let v = json!({"enforcement": true});
        assert_eq!(resolve_json_path(&v, "enforcement"), Some(&json!(true)));
    }

    #[test]
    fn dollar_prefixed_dot_path_resolves() {
        let v = json!({"drift_hints": []});
        assert_eq!(resolve_json_path(&v, "$.drift_hints"), Some(&json!([])));
    }

    #[test]
    fn bare_numeric_segment_indexes_array() {
        let v = json!({"data": ["a", "b"]});
        assert_eq!(resolve_json_path(&v, "$.data.0"), Some(&json!("a")));
    }

    #[test]
    fn bracket_index_resolves() {
        let v = json!({"data": [{"description": "auth"}]});
        assert_eq!(
            resolve_json_path(&v, "$.data[0].description"),
            Some(&json!("auth"))
        );
    }

    #[test]
    fn chained_bracket_indices_resolve() {
        let v = json!({"data": [[1, 2], [3, 4]]});
        assert_eq!(resolve_json_path(&v, "$.data[1][0]"), Some(&json!(3)));
    }

    #[test]
    fn missing_path_resolves_to_none() {
        let v = json!({"data": []});
        assert_eq!(resolve_json_path(&v, "$.nonexistent"), None);
    }

    #[test]
    fn bare_dollar_resolves_to_root() {
        let v = json!({"x": 1});
        assert_eq!(resolve_json_path(&v, "$"), Some(&v));
    }

    // ── resolve_json_path_all: recursive descent and `[*]` ───────────────

    fn all<'a>(v: &'a Value, path: &str) -> Vec<&'a Value> {
        resolve_json_path_all(v, path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn all_descent_collects_key_at_several_depths() {
        let v = json!({
            "k": 1,
            "a": {"k": 2, "b": [{"k": 3}, {"c": {"k": 4}}]},
            "z": [[{"k": 5}]]
        });
        assert_eq!(
            all(&v, "$..k"),
            vec![&json!(1), &json!(2), &json!(3), &json!(4), &json!(5)]
        );
    }

    #[test]
    fn all_descent_includes_key_nested_inside_a_matched_key() {
        let v = json!({"k": {"id": "outer", "k": {"id": "inner", "k": {"id": "deepest"}}}});
        let ids: Vec<&Value> = all(&v, "$..k").iter().map(|n| &n["id"]).collect();
        assert_eq!(
            ids,
            vec![&json!("outer"), &json!("inner"), &json!("deepest")]
        );
    }

    #[test]
    fn all_descent_then_star_explodes_every_matched_array() {
        let v = json!({
            "claims": [{"id": "C1", "claims": [{"id": "C1a"}, {"id": "C1b"}]}, {"id": "C2"}]
        });
        let ids: Vec<&Value> = all(&v, "$..claims[*]").iter().map(|n| &n["id"]).collect();
        assert_eq!(
            ids,
            vec![&json!("C1"), &json!("C2"), &json!("C1a"), &json!("C1b")]
        );
    }

    #[test]
    fn all_descent_star_then_child() {
        let v = json!({
            "claims": [{"x": 1, "claims": [{"x": 2}, {"y": 0}]}, {"x": 3}]
        });
        assert_eq!(
            all(&v, "$..claims[*].x"),
            vec![&json!(1), &json!(3), &json!(2)]
        );
    }

    #[test]
    fn all_mid_path_star_then_child() {
        let v = json!({"a": [{"b": 1}, {"c": 2}, {"b": [3]}]});
        assert_eq!(all(&v, "$.a[*].b"), vec![&json!(1), &json!([3])]);
    }

    #[test]
    fn all_star_on_a_non_array_yields_the_node_itself() {
        let v = json!({"a": {"b": 1}, "s": "x"});
        assert_eq!(all(&v, "$.s[*]"), vec![&json!("x")]);
        assert_eq!(all(&v, "$.a[*].b"), vec![&json!(1)]);
    }

    #[test]
    fn all_star_on_an_empty_array_yields_nothing() {
        let v = json!({"a": []});
        assert!(all(&v, "$.a[*]").is_empty());
    }

    #[test]
    fn all_reaching_one_node_twice_yields_it_once() {
        // `$..a` matches the outer and the inner `a`; `..b` under each reaches
        // the same `b` node. It is one node, so it is one value.
        let v = json!({"a": {"a": {"b": 1}}});
        assert_eq!(all(&v, "$..a..b"), vec![&json!(1)]);
        // Equal values at distinct nodes are not merged.
        let w = json!({"a": [{"b": 1}, {"b": 1}]});
        assert_eq!(all(&w, "$..b"), vec![&json!(1), &json!(1)]);
    }

    #[test]
    fn all_single_valued_paths_match_resolve_json_path() {
        let v = json!({"enforcement": true, "data": [{"description": "auth"}, [1, 2]]});
        for path in [
            "enforcement",
            "$",
            "$.enforcement",
            "$.data.0",
            "$.data[0].description",
            "$.data[1][1]",
            "$.missing",
            "$.data[9]",
            "$.enforcement.deeper",
        ] {
            assert_eq!(
                all(&v, path),
                resolve_json_path(&v, path).into_iter().collect::<Vec<_>>(),
                "{path}"
            );
        }
    }

    #[test]
    fn all_root_index_and_root_star() {
        let v = json!([{"k": 1}, {"k": 2}]);
        assert_eq!(all(&v, "$[1].k"), vec![&json!(2)]);
        assert_eq!(all(&v, "$[*].k"), vec![&json!(1), &json!(2)]);
    }

    #[test]
    fn all_malformed_paths_are_errors() {
        let v = json!({"a": {"b": 1}});
        for path in [
            "$..", "$...", "$...a", "[x]", "$.a[x]", "$.a..", "$.", "$.a.", "$.a..[0]", "$..[*]",
            "$.*", "$..*", "$a", "$.a[", "$.a[0]b", "$.a[-1]", "$.a[]",
        ] {
            let result = resolve_json_path_all(&v, path);
            assert!(result.is_err(), "{path} must be an Err, got {result:?}");
        }
    }

    #[test]
    fn all_malformed_path_error_names_the_path() {
        let v = json!({});
        let err = resolve_json_path_all(&v, "$...a").unwrap_err();
        assert!(err.contains("`$...a`"), "{err}");
    }
}
