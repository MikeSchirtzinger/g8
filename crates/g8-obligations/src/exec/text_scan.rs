//! Lightweight, dependency-free Rust source scanning: locate an enum's body
//! via brace-balance counting, extract struct/enum/type declaration names,
//! split an enum body into top-level variant segments.
//!
//! Deliberately NOT ast-grep for these: empirically verified against this
//! repo that ast-grep's pattern inference for bare `struct $NAME` / `enum
//! $NAME` forms does not match real declaration syntax the way a naive
//! reading suggests (`ast-grep run --pattern 'struct $NAME' ...` matches
//! zero times against a real `pub struct Foo { .. }`). A plain scanner over
//! the declaration grammar is simpler and more predictable for this crate's
//! two declaration-shaped backends (`RustEnumShape`,
//! `BuiltinAlgorithm::VocabularyDrift`).
//!
//! # Known limitation (disclosed, per this project's own culture of naming
//! its approximations rather than hiding them — contract §10)
//!
//! The brace-balance scanner skips over `"string"` and `//`/`/* */` comment
//! content so stray braces there don't unbalance the count, but does NOT
//! disambiguate Rust char literals (`'{'`/`'}'`) from lifetime syntax
//! (`'a`) — a bare brace char-literal inside a scanned body could miscount.
//! None of this crate's own targets (contract-listed enums, obligation
//! fixture functions) exercise that edge case.

use regex::Regex;

/// Boolean mask, one entry per byte of `content`: `true` where that byte is
/// "real code" (not inside a `//` line comment, `/* */` block comment, or
/// `"..."` string literal).
///
/// Exists to fix a real, confirmed bug: a plain `Regex::find` for `fn
/// plan_check` matched a *doc comment* — `g8-planner/src/lib.rs`'s own
/// `# ARCHITECTURE_GAP` note says "ARCHITECTURE.md §3.4 specifies `pub fn
/// plan_check(store: &dyn StoreConnection, ...)`" as prose, 126 lines before
/// the real generic definition (`pub fn plan_check<S>(...)` at line 145) —
/// so the scope-finder anchored on the comment's prose instead, producing a
/// scope span that excluded the function's real body entirely (T2c erratum
/// item 3 / T3b task brief). `scope.function` has since moved to ast-grep
/// (`exec::ast_grep::find_function_spans`); the mask still guards
/// `find_enum_anchor_byte`.
///
/// Building this as a separate mask alongside the string (rather than
/// rewriting the string in place) is what keeps this UTF-8-safe: this
/// codebase's doc comments contain multi-byte characters (`§`, `…`) that a
/// byte-level in-place blank-out could corrupt mid-character. The mask never
/// touches `content` itself, only marks it — `find_enum_anchor_byte` then
/// filters regex matches (found against the original, un-rewritten string)
/// by whether their start byte is real code.
fn real_code_mask(content: &str) -> Vec<bool> {
    let bytes = content.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut i = 0usize;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_line_comment {
            mask[i] = false;
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            mask[i] = false;
            if c == b'*' && bytes.get(i + 1) == Some(&b'/') {
                mask[i + 1] = false;
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_string {
            mask[i] = false;
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                mask[i] = false;
                mask[i + 1] = false;
                in_line_comment = true;
                i += 2;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                mask[i] = false;
                mask[i + 1] = false;
                in_block_comment = true;
                i += 2;
            }
            b'"' => {
                mask[i] = false;
                in_string = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    mask
}

/// First regex match whose start byte is real code per `real_code_mask` —
/// the comment/string-aware replacement for a bare `Regex::find`.
fn find_in_real_code<'a>(content: &'a str, re: &Regex) -> Option<regex::Match<'a>> {
    let mask = real_code_mask(content);
    re.find_iter(content)
        .find(|m| mask.get(m.start()).copied().unwrap_or(false))
}

/// Balanced-brace scan starting at/after `from_byte`, skipping string and
/// comment content. Returns `(byte_index_of_'{', byte_index_just_past_matching_'}')`.
fn find_balanced_braces(content: &str, from_byte: usize) -> Option<(usize, usize)> {
    let bytes = content.as_bytes();
    let mut i = from_byte;
    while i < bytes.len() && bytes[i] != b'{' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let start = i;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_line_comment {
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                in_line_comment = true;
                i += 2;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                in_block_comment = true;
                i += 2;
            }
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some((start, i));
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// Find the `{ ... }` body starting at/after `anchor_byte`, returning its
/// inner text (braces excluded) and the 1-based [start_line, end_line] the
/// whole `anchor { ... }` block spans (start_line = the anchor's own line).
pub(crate) fn find_block_after_byte(
    content: &str,
    anchor_byte: usize,
) -> Option<(String, u32, u32)> {
    let (brace_start, brace_end) = find_balanced_braces(content, anchor_byte)?;
    let inner = content[brace_start + 1..brace_end - 1].to_string();
    let start_line = 1 + content[..anchor_byte].matches('\n').count() as u32;
    let end_line = 1 + content[..brace_end].matches('\n').count() as u32;
    Some((inner, start_line, end_line))
}

/// Byte offset of `enum <name>`'s own keyword — the anchor other lookups
/// (e.g. the preceding `#[serde(rename_all)]` attribute) are relative to.
/// Comment/string-aware (`find_in_real_code`) so a doc comment that merely
/// mentions `enum Foo` as example/reference text is never mistaken for the
/// real declaration — same class of bug confirmed for `fn` (see
/// `real_code_mask`'s doc comment); fixed defensively here too even though
/// no current obligation has hit it for an enum specifically.
pub(crate) fn find_enum_anchor_byte(content: &str, enum_name: &str) -> Option<usize> {
    let re = Regex::new(&format!(r"\benum\s+{}\b", regex::escape(enum_name))).ok()?;
    find_in_real_code(content, &re).map(|m| m.start())
}

/// Find the `enum <name> { ... }` body. Returns `(inner_body, start_line, end_line)`.
pub(crate) fn find_enum_body(content: &str, enum_name: &str) -> Option<(String, u32, u32)> {
    let anchor = find_enum_anchor_byte(content, enum_name)?;
    find_block_after_byte(content, anchor)
}

/// The nearest `#[serde(rename_all = "...")]` attribute strictly before
/// `anchor_byte`, provided nothing but whitespace/attributes/doc-comments —
/// i.e. no `}` — separates it from the anchor (a `}` would mean we crossed
/// into a *different*, earlier item's closing brace).
pub(crate) fn find_preceding_serde_rename_all(content: &str, anchor_byte: usize) -> Option<String> {
    let re = Regex::new(r#"#\[serde\(rename_all\s*=\s*"([^"]+)"\)\]"#).expect("static regex");
    let mut best: Option<(usize, String)> = None;
    for m in re.captures_iter(&content[..anchor_byte]) {
        let whole = m.get(0).unwrap();
        best = Some((whole.end(), m.get(1).unwrap().as_str().to_string()));
    }
    let (end, value) = best?;
    if content[end..anchor_byte].contains('}') {
        return None;
    }
    Some(value)
}

/// Split an enum body into top-level variant segments (comma-separated,
/// respecting `()`/`{}`/`[]` nesting so a data-carrying variant's own commas
/// don't split it).
pub(crate) fn split_top_level_commas(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut in_line_comment = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_line_comment {
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                in_line_comment = true;
                i += 2;
                continue;
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                out.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < s.len() {
        let tail = s[start..].trim();
        if !tail.is_empty() {
            out.push(tail.to_string());
        }
    }
    out
}

/// Extract a variant segment's leading identifier, skipping any `#[...]`
/// attributes and `///` doc-comment lines first.
pub(crate) fn variant_name_from_segment(seg: &str) -> Option<String> {
    let mut s = seg.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("#[") {
            if let Some(end) = rest.find(']') {
                s = rest[end + 1..].trim_start();
                continue;
            }
        }
        // Skip any leading line comment up to and including its newline —
        // `//`, `///` (doc), `//!` (inner doc), `////`. Previously only
        // `///` was skipped, so the first variant after a `// ==== section
        // ==== ` divider comment was silently dropped: confirmed live
        // against ag-ui-core's `EventType`, whose ActivitySnapshot /
        // ReasoningStart / CheckpointCreated (each the first variant under
        // such a divider) went missing. A comment with no trailing newline
        // is the whole remaining segment — no identifier follows, so None.
        if s.starts_with("//") {
            let nl = s.find('\n')?;
            s = s[nl + 1..].trim_start();
            continue;
        }
        // Skip a leading `/* ... */` block comment between variants.
        if let Some(rest) = s.strip_prefix("/*") {
            if let Some(end) = rest.find("*/") {
                s = rest[end + 2..].trim_start();
                continue;
            }
        }
        break;
    }
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some(s[..end].to_string())
    }
}

/// Extract every top-level `struct`/`enum`/`type` declaration name in
/// `content`. Used by `BuiltinAlgorithm::VocabularyDrift`'s candidate-name
/// extraction.
pub(crate) fn find_type_declaration_names(content: &str) -> Vec<String> {
    let re = Regex::new(
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|type)\s+([A-Za-z_][A-Za-z0-9_]*)",
    )
    .expect("static regex");
    re.captures_iter(content)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_simple_enum_body() {
        let content = "enum Foo {\n    A,\n    B,\n}\n";
        let (body, start, end) = find_enum_body(content, "Foo").unwrap();
        assert!(body.contains('A') && body.contains('B'));
        assert_eq!(start, 1);
        assert_eq!(end, 4);
    }

    #[test]
    fn skips_braces_inside_strings_and_comments() {
        let content =
            "enum Foo {\n    // a comment with a brace {\n    A, // \"another {\"\n    B,\n}\n";
        let (body, _, end) = find_enum_body(content, "Foo").unwrap();
        assert!(body.contains('A') && body.contains('B'));
        assert_eq!(end, 5);
    }

    #[test]
    fn extracts_struct_enum_type_names() {
        let content =
            "pub struct Foo {\n    x: i32,\n}\n\nenum Bar { A, B }\n\ntype Baz = Foo;\n\npub(crate) struct Qux;\n";
        let names = find_type_declaration_names(content);
        assert_eq!(names, vec!["Foo", "Bar", "Baz", "Qux"]);
    }

    #[test]
    fn preceding_rename_all_found_immediately_above() {
        let content = "/// doc\n#[derive(Debug)]\n#[serde(rename_all = \"snake_case\")]\npub enum Foo {\n    A,\n}\n";
        let anchor = content.find("pub enum Foo").unwrap();
        assert_eq!(
            find_preceding_serde_rename_all(content, anchor).as_deref(),
            Some("snake_case")
        );
    }

    #[test]
    fn preceding_rename_all_does_not_leak_across_a_prior_item() {
        // rename_all belongs to Bar, not Foo — separated by Bar's closing brace.
        let content =
            "#[serde(rename_all = \"snake_case\")]\nenum Bar { X }\n\nenum Foo {\n    A,\n}\n";
        let anchor = content.find("enum Foo").unwrap();
        assert_eq!(find_preceding_serde_rename_all(content, anchor), None);
    }

    #[test]
    fn split_top_level_commas_respects_nesting() {
        let body = "A,\n    B(String),\n    C { x: i32, y: i32 },\n    D";
        let parts = split_top_level_commas(body);
        assert_eq!(parts.len(), 4);
        assert!(parts[2].contains("x: i32, y: i32"));
    }

    #[test]
    fn variant_name_strips_attributes_and_docs() {
        assert_eq!(variant_name_from_segment("  A"), Some("A".to_string()));
        assert_eq!(
            variant_name_from_segment("  /// doc\n    B(String)"),
            Some("B".to_string())
        );
        assert_eq!(
            variant_name_from_segment("  #[serde(rename = \"x\")]\n    C"),
            Some("C".to_string())
        );
    }

    #[test]
    fn variant_name_skips_a_section_divider_line_comment() {
        // Real bug (found dogfooding against ag-ui-core's EventType): the
        // first variant after a `// ==== ... ====` divider was dropped
        // because only `///` doc comments were skipped, not plain `//`.
        assert_eq!(
            variant_name_from_segment(
                "\n    // ==================== Activity Events ====================\n    /// Event containing a full snapshot\n    ActivitySnapshot"
            ),
            Some("ActivitySnapshot".to_string())
        );
        // Bare `//`, and a `/* */` block comment, each between variants.
        assert_eq!(
            variant_name_from_segment("  // note\n    Foo"),
            Some("Foo".to_string())
        );
        assert_eq!(
            variant_name_from_segment("  /* aside */\n    Bar"),
            Some("Bar".to_string())
        );
    }
}
