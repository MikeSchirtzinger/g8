//! Annotation types and the magic-comment grammar parser.
//!
//! `Annotation` is the *transient* output of `g8-extractor`.  It is never
//! stored directly; the store converts it to `Capability`, `Intent`, `Plan`,
//! or `Decision` rows.
//!
//! The parser is a hand-rolled scanner (no pest/nom) per ARCHITECTURE.md §6.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::G8Error;
use crate::model::IntentSourceKind;

// ── SourceLocation ───────────────────────────────────────────────────────────

/// Points to the location in source where an annotation was found.
///
/// Column is 0-based; line is 1-based (matching most editors).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    /// Construct a synthetic `SourceLocation` with zero line/column for cases
    /// where no position is available (e.g. Markdown-section annotations).
    pub fn unknown(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            line: 0,
            column: 0,
        }
    }
}

// ── AnnotationKind ───────────────────────────────────────────────────────────

/// Discriminant for the five annotation kinds defined in SPEC Decision 2.
///
/// # Examples
///
/// ```
/// use g8_core::AnnotationKind;
///
/// let kind = AnnotationKind::Capability;
/// assert!(matches!(kind, AnnotationKind::Capability));
/// let json = serde_json::to_string(&kind).unwrap();
/// assert_eq!(json, r#""capability""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Capability,
    Intent,
    ConvergenceTest,
    Decision,
    Plan,
}

impl AnnotationKind {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "capability" => Some(Self::Capability),
            "intent" => Some(Self::Intent),
            "convergence_test" => Some(Self::ConvergenceTest),
            "decision" => Some(Self::Decision),
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }
}

// ── AnnotationValue ──────────────────────────────────────────────────────────

/// A typed value in an annotation field (string, bool, integer, or list).
///
/// # Examples
///
/// ```
/// use g8_core::AnnotationValue;
///
/// let v = AnnotationValue::String("in_flight".into());
/// assert!(matches!(v, AnnotationValue::String(_)));
/// let list = AnnotationValue::List(vec!["A".into(), "B".into()]);
/// assert!(matches!(list, AnnotationValue::List(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnnotationValue {
    String(String),
    Bool(bool),
    Number(i64),
    List(Vec<String>),
}

// ── RawAnnotation ────────────────────────────────────────────────────────────

/// Output of `parse_annotation_grammar`.
///
/// Contains the kind discriminant as a raw string (so callers can pattern-match
/// without importing the enum) plus the parsed field map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAnnotation {
    /// Raw kind string: `"capability"`, `"intent"`, etc.
    pub kind_str: String,
    /// Parsed fields in stable BTreeMap order.
    pub fields: BTreeMap<String, AnnotationValue>,
}

// ── Annotation ───────────────────────────────────────────────────────────────

/// A typed annotation record produced by `g8-extractor`.
///
/// This is a transient extractor output, never stored directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub kind: AnnotationKind,
    /// Present for `Capability` and `Plan` kinds (from the `name` field).
    pub name: Option<String>,
    /// Present for `Intent` kind (section heading).
    pub heading: Option<String>,
    pub fields: BTreeMap<String, AnnotationValue>,
    pub source: SourceLocation,
    pub source_kind: IntentSourceKind,
    /// Set when extractor flagged `dev_only=true`, `cfg_attr`, or `tests/` dir.
    pub conditional: bool,
    /// The raw annotation text (stripped of `//` prefixes, whitespace-normalised).
    pub raw_text: String,
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Parse the magic-comment annotation grammar.
///
/// Accepts the form `@g8.<kind>(key = "value", ...)` with the `@g8.` prefix
/// present. The caller strips `//` or `#` comment prefixes and concatenates
/// multi-line forms before calling this function.
///
/// Grammar (ARCHITECTURE.md §6.1):
/// ```text
/// annotation := "@g8." kind "(" args ")"
/// kind       := "capability" | "intent" | "convergence_test" | "decision" | "plan"
/// args       := arg ("," arg)*  | ε
/// arg        := ident "=" value
/// ident      := [a-zA-Z_][a-zA-Z0-9_]*
/// value      := string | bool | int | list
/// string     := '"' [^"]* '"'
/// bool       := "true" | "false"
/// int        := -?[0-9]+
/// list       := "[" string ("," string)* "]"
/// ```
///
/// # Examples
///
/// Single-line form (the common case in v0.1):
///
/// ```
/// use g8_core::parse_annotation_grammar;
/// use g8_core::AnnotationValue;
///
/// let raw = r#"@g8.capability(name = "http-fetch", status = "in_flight", substrate = "network")"#;
/// let ann = parse_annotation_grammar(raw).unwrap();
/// assert_eq!(ann.kind_str, "capability");
/// assert_eq!(
///     ann.fields.get("name"),
///     Some(&AnnotationValue::String("http-fetch".into()))
/// );
/// assert_eq!(
///     ann.fields.get("status"),
///     Some(&AnnotationValue::String("in_flight".into()))
/// );
/// ```
///
/// # Errors
///
/// Returns [`G8Error::InvalidAnnotation`] if the input does not match the grammar.
pub fn parse_annotation_grammar(raw: &str) -> Result<RawAnnotation, G8Error> {
    let s = raw.trim();

    // Strip the `@g8.` prefix. `@govern.` is the pre-rename spelling and is
    // still accepted so annotated code does not need a sweep.
    let s = s
        .strip_prefix("@g8.")
        .or_else(|| s.strip_prefix("@govern."))
        .ok_or_else(|| G8Error::InvalidAnnotation(format!("expected '@g8.' prefix: {raw}")))?;

    // Parse kind up to the opening paren
    let paren_pos = s
        .find('(')
        .ok_or_else(|| G8Error::InvalidAnnotation(format!("expected '(' after kind in: {raw}")))?;
    let kind_str = s[..paren_pos].trim().to_string();
    if AnnotationKind::from_str(&kind_str).is_none() {
        return Err(G8Error::InvalidAnnotation(format!(
            "unknown annotation kind '{kind_str}'; expected one of: \
             capability, intent, convergence_test, decision, plan"
        )));
    }

    // Strip closing paren
    let args_part = &s[paren_pos + 1..];
    let args_part = args_part.trim();
    let args_part = args_part
        .strip_suffix(')')
        .ok_or_else(|| G8Error::InvalidAnnotation(format!("expected closing ')' in: {raw}")))?
        .trim();

    let fields = parse_args(args_part)
        .map_err(|e| G8Error::InvalidAnnotation(format!("{e} (in: {raw})")))?;

    Ok(RawAnnotation { kind_str, fields })
}

/// Parse the argument list `key = value, key = value, ...`
fn parse_args(s: &str) -> Result<BTreeMap<String, AnnotationValue>, String> {
    let mut fields = BTreeMap::new();
    if s.is_empty() {
        return Ok(fields);
    }

    let mut cursor = s;
    loop {
        cursor = cursor.trim_start();
        if cursor.is_empty() {
            break;
        }

        // Parse ident
        let (ident, rest) = parse_ident(cursor)?;
        let rest = rest.trim_start();

        // Expect '='
        let rest = rest
            .strip_prefix('=')
            .ok_or_else(|| format!("expected '=' after key '{ident}'"))?
            .trim_start();

        // Parse value
        let (value, rest) = parse_value(rest)?;
        fields.insert(ident, value);

        let rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        // Expect optional comma
        let rest = rest
            .strip_prefix(',')
            .ok_or_else(|| format!("expected ',' or end of args, found: {rest}"))?;
        cursor = rest;
    }

    Ok(fields)
}

fn parse_ident(s: &str) -> Result<(String, &str), String> {
    let end = s
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    if end == 0 {
        return Err(format!("expected identifier at: {s}"));
    }
    let ident = s[..end].to_string();
    if !ident
        .chars()
        .next()
        .map(|c| c.is_alphabetic() || c == '_')
        .unwrap_or(false)
    {
        return Err(format!("identifier must start with letter or '_': {ident}"));
    }
    Ok((ident, &s[end..]))
}

fn parse_value(s: &str) -> Result<(AnnotationValue, &str), String> {
    if s.starts_with('"') {
        parse_string(s).map(|(v, r)| (AnnotationValue::String(v), r))
    } else if s.starts_with('[') {
        parse_list(s).map(|(v, r)| (AnnotationValue::List(v), r))
    } else if s.starts_with("true")
        && (s.len() == 4 || s[4..].starts_with(|c: char| !c.is_alphanumeric() && c != '_'))
    {
        Ok((AnnotationValue::Bool(true), &s[4..]))
    } else if s.starts_with("false")
        && (s.len() == 5 || s[5..].starts_with(|c: char| !c.is_alphanumeric() && c != '_'))
    {
        Ok((AnnotationValue::Bool(false), &s[5..]))
    } else {
        parse_int(s).map(|(v, r)| (AnnotationValue::Number(v), r))
    }
}

fn parse_string(s: &str) -> Result<(String, &str), String> {
    // Must start with '"'
    let s = s.strip_prefix('"').ok_or("expected opening '\"'")?;
    // Find closing quote (no escape handling needed per grammar: `[^"]*`)
    let end = s.find('"').ok_or("unterminated string literal")?;
    let value = s[..end].to_string();
    Ok((value, &s[end + 1..]))
}

fn parse_list(s: &str) -> Result<(Vec<String>, &str), String> {
    let s = s.strip_prefix('[').ok_or("expected '['")?;
    let mut items = Vec::new();
    let mut cursor = s.trim_start();

    loop {
        cursor = cursor.trim_start();
        if let Some(rest) = cursor.strip_prefix(']') {
            return Ok((items, rest));
        }
        if cursor.starts_with(',') {
            cursor = &cursor[1..];
            continue;
        }
        if cursor.is_empty() {
            return Err("unterminated list: missing ']'".into());
        }
        let (item, rest) = parse_string(cursor)?;
        items.push(item);
        cursor = rest.trim_start();
    }
}

fn parse_int(s: &str) -> Result<(i64, &str), String> {
    let (neg, s) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s)
    };
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return Err(format!("expected integer value at: {s}"));
    }
    let n: i64 = s[..end]
        .parse()
        .map_err(|e| format!("integer parse error: {e}"))?;
    Ok((if neg { -n } else { n }, &s[end..]))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_capability() {
        let r = parse_annotation_grammar(
            r#"@g8.capability(name = "http-fetch", status = "in_flight")"#,
        )
        .unwrap();
        assert_eq!(r.kind_str, "capability");
        assert!(
            matches!(r.fields.get("name"), Some(AnnotationValue::String(s)) if s == "http-fetch")
        );
        assert!(
            matches!(r.fields.get("status"), Some(AnnotationValue::String(s)) if s == "in_flight")
        );
    }

    #[test]
    fn parse_list_values() {
        let r = parse_annotation_grammar(
            r#"@g8.capability(name = "x", consumes = ["A", "B"], produces = ["C"])"#,
        )
        .unwrap();
        assert!(matches!(
            r.fields.get("consumes"),
            Some(AnnotationValue::List(v)) if v == &["A", "B"]
        ));
        assert!(matches!(
            r.fields.get("produces"),
            Some(AnnotationValue::List(v)) if v == &["C"]
        ));
    }

    #[test]
    fn parse_bool_and_int() {
        let r =
            parse_annotation_grammar(r#"@g8.capability(name = "y", stub = true, wip_weight = 2)"#)
                .unwrap();
        assert!(matches!(
            r.fields.get("stub"),
            Some(AnnotationValue::Bool(true))
        ));
        assert!(matches!(
            r.fields.get("wip_weight"),
            Some(AnnotationValue::Number(2))
        ));
    }

    #[test]
    fn parse_empty_args() {
        let r = parse_annotation_grammar("@g8.intent()").unwrap();
        assert_eq!(r.kind_str, "intent");
        assert!(r.fields.is_empty());
    }

    #[test]
    fn parse_unknown_kind_errors() {
        assert!(parse_annotation_grammar("@g8.unknown(name = \"x\")").is_err());
    }

    #[test]
    fn parse_missing_prefix_errors() {
        assert!(parse_annotation_grammar("capability(name = \"x\")").is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let r =
            parse_annotation_grammar(r#"@g8.plan(title = "streaming-export", status = "Idea")"#)
                .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let back: RawAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind_str, "plan");
        assert_eq!(back.fields.get("title"), r.fields.get("title"));
    }

    #[test]
    fn parse_decision_kind() {
        let r = parse_annotation_grammar(
            r#"@g8.decision(title = "Use rusqlite", status = "accepted")"#,
        )
        .unwrap();
        assert_eq!(r.kind_str, "decision");
    }

    #[test]
    fn parse_convergence_test() {
        let r = parse_annotation_grammar(
            r#"@g8.convergence_test(for_capability = "http-fetch", scenario = "timeout")"#,
        )
        .unwrap();
        assert_eq!(r.kind_str, "convergence_test");
        assert!(r.fields.contains_key("for_capability"));
    }

    #[test]
    fn parse_negative_int() {
        let r = parse_annotation_grammar(r#"@g8.capability(name = "x", wip_weight = -1)"#).unwrap();
        assert!(matches!(
            r.fields.get("wip_weight"),
            Some(AnnotationValue::Number(-1))
        ));
    }

    #[test]
    fn parse_empty_list() {
        let r = parse_annotation_grammar(r#"@g8.capability(name = "x", consumes = [])"#).unwrap();
        assert!(matches!(
            r.fields.get("consumes"),
            Some(AnnotationValue::List(v)) if v.is_empty()
        ));
    }
}
