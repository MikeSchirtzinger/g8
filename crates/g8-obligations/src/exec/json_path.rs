//! Shared `$.`-prefixed JSON path resolution dialect.
//!
//! Originally lived in `exec::fixture` (built for `FixtureIntegrationTest`'s
//! `JsonField`/`JsonPathNonEmpty` assertions); factored out here, verbatim,
//! so `exec::receipt`'s `receipt_query` backend (contract addendum
//! o-g8-receipts-20260721) can reuse the exact same dialect instead of a
//! second, drifting implementation. Behavior is unchanged from the original
//! `fixture.rs` version — this is a pure relocation.

use serde_json::Value;

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
}
