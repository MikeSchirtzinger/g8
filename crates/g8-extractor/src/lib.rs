//! `g8-extractor` — annotation extraction for the G8.
//!
//! Detects `// @g8.<kind>(...)` magic-comment annotations (and `# @g8.<kind>(...)`
//! for Python) across Rust, TypeScript, Python, and Go source files via the
//! `ast-grep` CLI shell-out. Also parses `AGENTS.md`, `CLAUDE.md`, and `*.g8.md`
//! sidecar files into `Intent` / `Plan` / `Decision` annotations via a native
//! Markdown-section scanner.
//!
//! # Python comment syntax
//!
//! Python uses `#` as its comment prefix, not `//`. Both forms are accepted:
//! - `# @g8.capability(name = "x")` — Python-native, recommended for Python files.
//! - `// @g8.capability(name = "x")` — uniform cross-language form, also accepted.
//!
//! The ast-grep `comment` node kind rule for Python matches both because
//! Python's tree-sitter grammar represents all comment text under `comment` nodes
//! regardless of the leading character. The annotation parser strips either prefix.
//!
//! # Architecture
//!
//! ```text
//! Extractor::scan_dir(root)
//!   ├── walk via `ignore` crate (respects .gitignore)
//!   ├── source files (.rs .ts .tsx .py .go)
//!   │     └── ast-grep CLI shell-out (--json=stream, per R2 §6)
//!   │           └── parse NDJSON → Vec<RawAstMatch>
//!   │                 └── strip comment prefix → call g8_core::parse_annotation_grammar
//!   └── sidecar files (AGENTS.md, CLAUDE.md, *.g8.md)
//!         └── Markdown-section scanner (per ARCH §3.2 / Decision 4)
//!               └── heading regex → IntentKind mapping
//! ```
//!
//! Returns [`ScanResult`] with a deduplicated `Vec<Annotation>`, warnings, and stats.
//!
//! # Runtime dependency
//!
//! The `ast-grep` binary (`sg`) must be on `PATH` (or configured via
//! [`Extractor::with_binary`]). Install with `cargo install ast-grep` or via
//! Homebrew (`brew install ast-grep`). The extractor checks at construction time
//! and returns [`ExtractorError::BinaryNotFound`] if absent.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use ignore::WalkBuilder;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, instrument, warn};

use g8_core::{
    Annotation, AnnotationKind, AnnotationValue, IntentKind, IntentSourceKind, SourceLocation,
};

// ── Public error type ────────────────────────────────────────────────────────

/// Errors produced by `g8-extractor`.
#[derive(Debug, Error)]
pub enum ExtractorError {
    /// The `ast-grep` binary is not on `PATH` (or not at the configured path).
    ///
    /// Install with:
    /// ```text
    /// cargo install ast-grep
    /// # or
    /// brew install ast-grep
    /// ```
    #[error(
        "ast-grep binary not found; install with `cargo install ast-grep` or `brew install ast-grep`"
    )]
    BinaryNotFound,

    /// The `ast-grep` subprocess exited with a non-zero status.
    #[error("ast-grep subprocess failed: {0}")]
    SubprocessFailed(String),

    /// A line of NDJSON from `ast-grep` could not be deserialized.
    #[error("failed to parse ast-grep JSON output: {0}")]
    JsonParse(String),

    /// An annotation string was syntactically invalid.
    #[error("invalid magic-comment annotation at {file}:{line}: {message}")]
    InvalidAnnotation {
        file: PathBuf,
        line: u32,
        message: String,
    },

    /// An I/O error occurred while spawning the subprocess or reading output.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ── Public output types ──────────────────────────────────────────────────────

/// The result of a full directory-tree scan.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// All extracted annotations, deduplicated by (file, line, kind).
    pub annotations: Vec<Annotation>,
    /// Non-fatal warnings encountered during extraction.
    pub warnings: Vec<ScanWarning>,
    /// Scan statistics.
    pub stats: ScanStats,
}

/// A non-fatal warning encountered during extraction.
#[derive(Debug)]
pub struct ScanWarning {
    pub file: PathBuf,
    pub line: Option<u32>,
    pub message: String,
}

/// Aggregate statistics for a scan run.
#[derive(Debug, Default)]
pub struct ScanStats {
    pub files_scanned: u32,
    pub annotations_found: u32,
    pub duration_ms: u64,
}

// ── Extractor ────────────────────────────────────────────────────────────────

/// The main entry point for annotation extraction.
///
/// Construct with [`Extractor::new`] (auto-discovers `ast-grep` on `PATH`) or
/// [`Extractor::with_binary`] (explicit binary path).
pub struct Extractor {
    rules_path: PathBuf,
    ast_grep_bin: PathBuf,
}

/// The bundled ast-grep rules, embedded at compile time.
///
/// Embedded (rather than looked up via `env!("CARGO_MANIFEST_DIR")`) so the
/// compiled binary is relocatable: a build-time path bakes the build machine's
/// source checkout location into the binary, and every `scan` breaks the
/// moment that checkout moves or the binary runs on another machine.
const EMBEDDED_RULES: &str = include_str!("../rules/g8-all-languages.yaml");

/// Write the embedded rules to a content-addressed file in the OS temp dir.
///
/// `ast-grep` runs as a subprocess, so it needs a real file on disk. The
/// filename carries a digest of the content: concurrent g8 processes
/// converge on the same file, a binary carrying newer rules never picks up a
/// stale copy, and the write is skipped once the file exists. Write-then-rename
/// keeps a concurrent reader from ever seeing a half-written file.
fn materialize_embedded_rules() -> Result<PathBuf, ExtractorError> {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(EMBEDDED_RULES.as_bytes());
        let full = hasher.finalize();
        // 64 bits of the digest is plenty for a cache key.
        full[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let path = std::env::temp_dir().join(format!("g8-rules-{digest}.yaml"));
    if !path.exists() {
        let staging = path.with_extension(format!("yaml.{}.tmp", std::process::id()));
        std::fs::write(&staging, EMBEDDED_RULES)?;
        std::fs::rename(&staging, &path)?;
    }
    Ok(path)
}

impl Extractor {
    /// Construct an `Extractor` using the bundled (compile-time embedded) rules
    /// and `ast-grep` from `PATH`.
    ///
    /// Returns [`ExtractorError::BinaryNotFound`] if `ast-grep` cannot be located.
    pub fn new() -> Result<Self, ExtractorError> {
        Self::with_binary(materialize_embedded_rules()?, find_ast_grep()?)
    }

    /// Construct using a custom rules YAML path and `ast-grep` from `PATH`.
    pub fn with_rules(rules_path: PathBuf) -> Result<Self, ExtractorError> {
        Self::with_binary(rules_path, find_ast_grep()?)
    }

    /// Construct using explicit paths for both the rules file and the binary.
    pub fn with_binary(rules_path: PathBuf, ast_grep_bin: PathBuf) -> Result<Self, ExtractorError> {
        // Verify binary is executable.
        Command::new(&ast_grep_bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| ExtractorError::BinaryNotFound)?;
        Ok(Self {
            rules_path,
            ast_grep_bin,
        })
    }

    /// Scan an entire directory tree for G8 annotations.
    ///
    /// Uses the `ignore` crate to walk the tree (respects `.gitignore`).
    /// Source files are processed via ast-grep; Markdown sidecars are processed
    /// by the native section scanner.
    ///
    /// The file `.g8/INTENT_SUMMARY.md` is excluded from extraction to avoid
    /// a feedback loop (per ARCHITECTURE.md §3.2).
    #[instrument(skip(self), fields(root = %root.display()))]
    pub fn scan_dir(&self, root: &Path) -> Result<ScanResult, ExtractorError> {
        let start = Instant::now();
        let mut result = ScanResult::default();

        // Collect source files and sidecar files by walking the tree.
        let mut source_files: Vec<PathBuf> = Vec::new();
        let mut sidecar_files: Vec<PathBuf> = Vec::new();

        let walker = WalkBuilder::new(root)
            .standard_filters(true) // respects .gitignore, .ignore, etc.
            .build();

        for entry in walker.flatten() {
            let path = entry.into_path();
            if !path.is_file() {
                continue;
            }

            // Exclude INTENT_SUMMARY.md (auto-generated; would create a loop).
            if path
                .file_name()
                .and_then(OsStr::to_str)
                .map(|n| n == "INTENT_SUMMARY.md")
                .unwrap_or(false)
            {
                continue;
            }

            match classify_file(&path) {
                FileKind::Source => source_files.push(path),
                FileKind::Sidecar(_sk) => sidecar_files.push(path.clone()),
                FileKind::Other => {}
            }
        }

        result.stats.files_scanned = (source_files.len() + sidecar_files.len()) as u32;

        // ── ast-grep pass for source files ──
        if !source_files.is_empty() {
            let ast_annotations = self.run_ast_grep(root, &mut result.warnings)?;
            result.annotations.extend(ast_annotations);
        }

        // ── Markdown sidecar pass ──
        for sidecar in &sidecar_files {
            let sk = sidecar_source_kind(sidecar);
            match parse_sidecar(sidecar, sk) {
                Ok(mut anns) => result.annotations.append(&mut anns),
                Err(e) => result.warnings.push(ScanWarning {
                    file: sidecar.clone(),
                    line: None,
                    message: e.to_string(),
                }),
            }
        }

        // Deduplicate: same (file, line, kind) triple.
        dedup_annotations(&mut result.annotations);

        result.stats.annotations_found = result.annotations.len() as u32;
        result.stats.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }

    /// Scan a single source file for magic-comment annotations.
    ///
    /// For Markdown sidecars, delegates to the native section parser.
    /// For other file types, invokes `ast-grep` restricted to that one file.
    #[instrument(skip(self), fields(file = %file.display()))]
    pub fn scan_file(&self, file: &Path) -> Result<Vec<Annotation>, ExtractorError> {
        match classify_file(file) {
            FileKind::Source => {
                let mut warnings = Vec::new();
                self.run_ast_grep(file, &mut warnings)
            }
            FileKind::Sidecar(sk) => parse_sidecar(file, sk).map_err(ExtractorError::Io),
            FileKind::Other => Ok(Vec::new()),
        }
    }

    /// Invoke `ast-grep scan --rule <rules> --json=stream <target>` and parse output.
    #[instrument(skip(self, warnings), fields(target = %target.display()))]
    fn run_ast_grep(
        &self,
        target: &Path,
        warnings: &mut Vec<ScanWarning>,
    ) -> Result<Vec<Annotation>, ExtractorError> {
        let mut child = Command::new(&self.ast_grep_bin)
            .args(["scan", "--rule"])
            .arg(&self.rules_path)
            .arg("--json=stream")
            .arg(target)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // suppress progress/colour noise
            .spawn()
            .map_err(|_| ExtractorError::BinaryNotFound)?;

        let stdout = child.stdout.take().expect("piped stdout");
        let reader = BufReader::new(stdout);

        let name_re = Regex::new(r#"name\s*=\s*"([^"]+)""#).expect("static regex");
        let mut annotations = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let raw: RawAstMatch = match serde_json::from_str(line) {
                Ok(m) => m,
                Err(e) => {
                    warn!("failed to parse ast-grep NDJSON line: {e}");
                    continue;
                }
            };
            debug!(rule = ?raw.rule_id, file = %raw.file, "ast-grep match");

            match convert_ast_match(&raw, &name_re) {
                Ok(Some(ann)) => annotations.push(ann),
                Ok(None) => {} // match did not produce a usable annotation
                Err(e) => warnings.push(ScanWarning {
                    file: PathBuf::from(&raw.file),
                    line: Some(raw.range.start.line as u32 + 1),
                    message: e,
                }),
            }
        }

        let status = child.wait()?;
        // ast-grep exits 0 whether or not matches were found; non-zero means a real error.
        if !status.success() {
            return Err(ExtractorError::SubprocessFailed(format!(
                "exit code: {}",
                status.code().unwrap_or(-1)
            )));
        }

        Ok(annotations)
    }
}

// ── Also expose a top-level free function matching ARCH §3.2 `scan_project` ─

/// Scan a directory tree for all G8 annotations.
///
/// This is a convenience wrapper around [`Extractor::new`] + [`Extractor::scan_dir`].
/// Returns `Err` if the `ast-grep` binary is not found.
pub fn scan_project(root: &Path) -> Result<Vec<Annotation>, ExtractorError> {
    let extractor = Extractor::new()?;
    extractor.scan_dir(root).map(|r| r.annotations)
}

// ── Internal: ast-grep JSON types ────────────────────────────────────────────

/// Raw JSON object emitted by `ast-grep scan --json=stream`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAstMatch {
    text: String,
    file: String,
    range: RawRange,
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRange {
    start: RawPos,
    #[allow(dead_code)]
    end: RawPos,
}

#[derive(Debug, Deserialize)]
struct RawPos {
    line: usize,
    column: usize,
}

// ── Internal: file classification ────────────────────────────────────────────

#[derive(Debug)]
enum FileKind {
    Source,
    Sidecar(IntentSourceKind),
    Other,
}

fn classify_file(path: &Path) -> FileKind {
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();

    // Exact filename matches first.
    if file_name == "AGENTS.md" {
        return FileKind::Sidecar(IntentSourceKind::AgentsMd);
    }
    if file_name == "CLAUDE.md" {
        return FileKind::Sidecar(IntentSourceKind::ClaudeMd);
    }

    let ext = path.extension().and_then(OsStr::to_str).unwrap_or_default();

    match ext {
        "rs" | "ts" | "tsx" | "py" | "go" => FileKind::Source,
        "md" | "markdown" => {
            // *.g8.md sidecar: any .md file that isn't the auto-generated summary.
            // We treat all other .md files (other than AGENTS.md/CLAUDE.md already
            // handled above) as G8 sidecars if they match *.g8.md, otherwise Other.
            let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or_default();
            if stem.ends_with(".g8")
                || file_name.ends_with(".g8.md")
                || file_name.ends_with(".govern.md")
            {
                FileKind::Sidecar(IntentSourceKind::G8Sidecar)
            } else {
                FileKind::Other
            }
        }
        _ => FileKind::Other,
    }
}

fn sidecar_source_kind(path: &Path) -> IntentSourceKind {
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if file_name == "AGENTS.md" {
        IntentSourceKind::AgentsMd
    } else if file_name == "CLAUDE.md" {
        IntentSourceKind::ClaudeMd
    } else {
        IntentSourceKind::G8Sidecar
    }
}

// ── Internal: convert ast-grep match → Annotation ────────────────────────────

/// Convert one raw ast-grep match into an `Annotation`, or `None` if the match
/// does not contain a parseable G8 annotation.
fn convert_ast_match(
    raw: &RawAstMatch,
    _name_re: &Regex, // kept for potential future use
) -> Result<Option<Annotation>, String> {
    // Strip comment prefixes to isolate the `@g8.<kind>(...)` text.
    let text = strip_comment_prefixes(&raw.text);

    // Find the @g8. annotation within the (potentially multiline) text.
    let g8_start = match text.find("@g8.").or_else(|| text.find("@govern.")) {
        Some(pos) => pos,
        None => return Ok(None),
    };
    let annotation_text = &text[g8_start..];

    // Parse the annotation grammar.
    let raw_ann = match g8_core::parse_annotation_grammar(annotation_text) {
        Ok(r) => r,
        Err(e) => return Err(e.to_string()),
    };

    let kind = annotation_kind_from_str(&raw_ann.kind_str)?;

    // Extract name (for Capability / Plan) and heading (for Intent).
    let name = match kind {
        AnnotationKind::Capability | AnnotationKind::Plan => raw_ann
            .fields
            .get("name")
            .or_else(|| raw_ann.fields.get("title"))
            .and_then(|v| {
                if let AnnotationValue::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            }),
        _ => None,
    };

    let heading = if kind == AnnotationKind::Intent {
        raw_ann
            .fields
            .get("description")
            .or_else(|| raw_ann.fields.get("title"))
            .and_then(|v| {
                if let AnnotationValue::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
    } else {
        None
    };

    // Detect conditional flag.
    let conditional = is_conditional(&raw.text, &raw_ann.fields);

    // Detect tests-dir flag.
    let path = PathBuf::from(&raw.file);
    let in_tests_dir = is_tests_dir(&path);
    let conditional = conditional || in_tests_dir;

    // Determine source_kind: inline comment.
    let source_kind = IntentSourceKind::InlineComment;

    // ast-grep reports 0-based lines; SourceLocation.line is 1-based.
    let source = SourceLocation {
        file: path,
        line: raw.range.start.line as u32 + 1,
        column: raw.range.start.column as u32,
    };

    Ok(Some(Annotation {
        kind,
        name,
        heading,
        fields: raw_ann.fields,
        source,
        source_kind,
        conditional,
        raw_text: annotation_text.to_string(),
    }))
}

/// Strip `//` and `#` comment prefixes from each line of a multiline match,
/// then join all lines with a single space to produce a flat annotation string
/// ready for [`g8_core::parse_annotation_grammar`].
///
/// This handles the uniform `// @g8.` prefix (all languages) and the Python-native
/// `# @g8.` prefix. Lines that do not start with a recognised comment prefix are
/// preserved as-is (trimmed).
///
/// ## v0.1 behaviour (space-join)
///
/// Consecutive `//`-prefixed continuation lines are joined with a single space.
/// This means the multi-line form:
///
/// ```text
/// // @g8.capability(name = "http-fetch",
/// //                 status = "in_flight")
/// ```
///
/// becomes the flat string:
///
/// ```text
/// @g8.capability(name = "http-fetch", status = "in_flight")
/// ```
///
/// which the grammar parser handles correctly because whitespace between tokens is
/// ignored (ARCHITECTURE.md §6.1).
///
/// ## v0.2 roadmap note
///
/// The space-join approach works for all well-formed multi-line annotations but
/// may produce confusing error messages when a continuation line is malformed
/// (the error position refers to the flat string, not the original line number).
/// A token-aware multi-line splitter that preserves per-line source positions is
/// tracked for v0.2 (SPEC §10 roadmap, "native language idioms").
fn strip_comment_prefixes(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("// ") {
                rest
            } else if let Some(rest) = trimmed.strip_prefix("//") {
                rest
            } else if let Some(rest) = trimmed.strip_prefix("# ") {
                rest
            } else if let Some(rest) = trimmed.strip_prefix('#') {
                rest
            } else {
                trimmed
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a kind string from `RawAnnotation` to `AnnotationKind`.
fn annotation_kind_from_str(s: &str) -> Result<AnnotationKind, String> {
    match s {
        "capability" => Ok(AnnotationKind::Capability),
        "intent" => Ok(AnnotationKind::Intent),
        "convergence_test" => Ok(AnnotationKind::ConvergenceTest),
        "decision" => Ok(AnnotationKind::Decision),
        "plan" => Ok(AnnotationKind::Plan),
        other => Err(format!("unknown annotation kind: {other}")),
    }
}

/// Returns `true` if the annotation should be flagged as conditional.
///
/// Three triggers (ARCHITECTURE.md §6.3):
/// 1. `dev_only = true` in the annotation fields.
/// 2. (Rust only) the raw text contains `#[cfg(` or `#[cfg_attr(` on the preceding line.
/// 3. File path contains a `tests`, `tests-*`, or `*-tests` directory component
///    (handled by `is_tests_dir` separately).
fn is_conditional(raw_text: &str, fields: &BTreeMap<String, AnnotationValue>) -> bool {
    // Check dev_only field.
    if let Some(AnnotationValue::Bool(true)) = fields.get("dev_only") {
        return true;
    }
    // Check raw text for cfg_attr / cfg preceding the annotation.
    if raw_text.contains("#[cfg(") || raw_text.contains("#[cfg_attr(") {
        return true;
    }
    false
}

/// Returns `true` if the file path lives under a `tests`, `tests-*`, or `*-tests` directory.
fn is_tests_dir(path: &Path) -> bool {
    path.components().any(|c| {
        if let std::path::Component::Normal(s) = c {
            let s = s.to_string_lossy();
            s == "tests" || s.starts_with("tests-") || s.ends_with("-tests")
        } else {
            false
        }
    })
}

// ── Internal: Markdown sidecar parser ────────────────────────────────────────

/// Parse an `AGENTS.md`, `CLAUDE.md`, or `*.g8.md` file into `Annotation` records.
///
/// Each heading (`#` or `##`) starts a new section. The heading text is mapped
/// to an `IntentKind` (or `Plan` for parked-ideas sections) via the locked table
/// in ARCHITECTURE.md §3.2 / SPEC Decision 4.
///
/// # Precedence
/// The `scope_depth` (directory depth from the file's location) is encoded in
/// `Annotation::source.line = 0` + caller uses file path depth for ordering.
/// Deeper files win when the planner queries intents (ORDER BY scope_depth DESC).
///
/// # Excluded file
/// `.g8/INTENT_SUMMARY.md` is excluded by the caller (`scan_dir`) before this
/// function is called.
fn parse_sidecar(
    path: &Path,
    source_kind: IntentSourceKind,
) -> Result<Vec<Annotation>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    let mut annotations = Vec::new();

    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    let mut current_line_number: u32 = 0;
    let mut section_start_line: u32 = 1;

    let flush =
        |heading: &str, lines: &[String], line_num: u32, annotations: &mut Vec<Annotation>| {
            let description = lines.join("\n").trim().to_string();
            let (kind, is_owner, is_parked) = classify_heading(heading);

            if is_owner {
                // Owner sections are parsed and merged into the previous intent.
                // For simplicity in v0.1, emit as Unclassified Intent; the store
                // layer picks up owner_agent / owner_team / owner_contact columns.
                let mut fields = BTreeMap::new();
                parse_owner_block(&description, &mut fields);
                fields.insert(
                    "heading".to_string(),
                    AnnotationValue::String(heading.to_string()),
                );
                annotations.push(Annotation {
                    kind: AnnotationKind::Intent,
                    name: None,
                    heading: Some(heading.to_string()),
                    fields,
                    source: SourceLocation {
                        file: path.to_path_buf(),
                        line: line_num,
                        column: 0,
                    },
                    source_kind,
                    conditional: false,
                    raw_text: description,
                });
                return;
            }

            if is_parked {
                // Parked Ideas: parse line-by-line `- <name>: <reason>` entries.
                for (i, line) in lines.iter().enumerate() {
                    let line = line.trim();
                    if let Some(rest) = line.strip_prefix("- ") {
                        let (title, reason) = parse_parked_line(rest);
                        let mut fields = BTreeMap::new();
                        fields.insert("title".to_string(), AnnotationValue::String(title.clone()));
                        fields.insert(
                            "status".to_string(),
                            AnnotationValue::String("Parked".to_string()),
                        );
                        if let Some(r) = reason {
                            fields.insert("parked_reason".to_string(), AnnotationValue::String(r));
                        }
                        annotations.push(Annotation {
                            kind: AnnotationKind::Plan,
                            name: Some(title),
                            heading: None,
                            fields,
                            source: SourceLocation {
                                file: path.to_path_buf(),
                                line: line_num + i as u32,
                                column: 0,
                            },
                            source_kind,
                            conditional: false,
                            raw_text: line.to_string(),
                        });
                    } else if !line.is_empty() {
                        // Non-matching line — emit a trace-level warning via tracing.
                        // We don't fail; warn!() per ARCH spec.
                        tracing::warn!(
                            file = %path.display(),
                            "parked-ideas line does not match `- <name>: <reason>` form: {line}"
                        );
                    }
                }
                return;
            }

            // Normal intent section.
            let mut fields = BTreeMap::new();
            fields.insert(
                "description".to_string(),
                AnnotationValue::String(description.clone()),
            );
            fields.insert(
                "heading".to_string(),
                AnnotationValue::String(heading.to_string()),
            );
            if let AnnotationKind::Intent = AnnotationKind::Intent {
                let kind_str = intent_kind_str(kind);
                fields.insert(
                    "intent_kind".to_string(),
                    AnnotationValue::String(kind_str.to_string()),
                );
            }

            annotations.push(Annotation {
                kind: AnnotationKind::Intent,
                name: None,
                heading: Some(heading.to_string()),
                fields,
                source: SourceLocation {
                    file: path.to_path_buf(),
                    line: line_num,
                    column: 0,
                },
                source_kind,
                conditional: false,
                raw_text: description,
            });
        };

    for line in content.lines() {
        current_line_number += 1;

        // Detect headings: `## ` or `# `.
        let heading_text = if let Some(h) = line.strip_prefix("## ") {
            Some(h.trim())
        } else {
            line.strip_prefix("# ").map(|h| h.trim())
        };

        if let Some(h) = heading_text {
            // Flush the previous section.
            if let Some(prev_heading) = current_heading.take() {
                flush(
                    &prev_heading,
                    &current_lines,
                    section_start_line,
                    &mut annotations,
                );
                current_lines.clear();
            }
            current_heading = Some(h.to_string());
            section_start_line = current_line_number;
        } else if current_heading.is_some() {
            current_lines.push(line.to_string());
        }
    }

    // Flush the last section.
    if let Some(prev_heading) = current_heading.take() {
        flush(
            &prev_heading,
            &current_lines,
            section_start_line,
            &mut annotations,
        );
    }

    Ok(annotations)
}

/// Map a heading string to (IntentKind, is_owner_section, is_parked_section).
///
/// Case-insensitive. Per SPEC Decision 4 / ARCHITECTURE.md §3.2 locked table.
fn classify_heading(heading: &str) -> (IntentKind, bool, bool) {
    let h = heading.to_lowercase();
    let h = h.trim();

    // Owner section — parsed separately.
    if matches!(h, "owner" | "ownership") {
        return (IntentKind::Unclassified, true, false);
    }

    // Parked ideas section.
    if matches!(h, "parked ideas" | "deferred") {
        return (IntentKind::Unclassified, false, true);
    }

    // Active plans (ignored by extractor — those rows live in `plan` table).
    if matches!(h, "active plans" | "in flight" | "in-flight") {
        return (IntentKind::Unclassified, false, false);
    }

    let kind = if matches!(h, "architecture" | "design" | "overview") {
        IntentKind::ArchitecturalScope
    } else if matches!(h, "boundaries" | "never do" | "constraints") {
        IntentKind::Boundary
    } else if matches!(h, "commands" | "build" | "test" | "testing") {
        IntentKind::Operational
    } else if matches!(h, "stack" | "dependencies") {
        IntentKind::TechStack
    } else if h == "intent" {
        IntentKind::ExplicitIntent
    } else if matches!(h, "decisions" | "architectural decisions") {
        // Decisions heading → emit as Intent::Unclassified for now;
        // the store layer can promote to Decision rows.
        IntentKind::Unclassified
    } else {
        IntentKind::Unclassified
    };

    (kind, false, false)
}

/// Convert `IntentKind` to its snake_case string representation.
fn intent_kind_str(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::ArchitecturalScope => "architectural_scope",
        IntentKind::Boundary => "boundary",
        IntentKind::Operational => "operational",
        IntentKind::TechStack => "tech_stack",
        IntentKind::ExplicitIntent => "explicit_intent",
        IntentKind::Unclassified => "unclassified",
    }
}

/// Parse `# Owner` block content into a fields map.
///
/// Accepted lines:
/// ```text
/// agent: scorer-specialist
/// team: core-ranker
/// contact: #checkout-api-core
/// ```
fn parse_owner_block(content: &str, fields: &mut BTreeMap<String, AnnotationValue>) {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("agent:") {
            fields.insert(
                "owner_agent".to_string(),
                AnnotationValue::String(rest.trim().to_string()),
            );
        } else if let Some(rest) = line.strip_prefix("team:") {
            fields.insert(
                "owner_team".to_string(),
                AnnotationValue::String(rest.trim().to_string()),
            );
        } else if let Some(rest) = line.strip_prefix("contact:") {
            fields.insert(
                "owner_contact".to_string(),
                AnnotationValue::String(rest.trim().to_string()),
            );
        }
    }
}

/// Parse a `- <name>: <reason>` parked-ideas line.
///
/// Returns `(name, Some(reason))` or `(name, None)` if no colon separator.
fn parse_parked_line(rest: &str) -> (String, Option<String>) {
    if let Some(colon_pos) = rest.find(": ") {
        let name = rest[..colon_pos].trim().to_string();
        let reason = rest[colon_pos + 2..].trim().to_string();
        (name, Some(reason))
    } else {
        (rest.trim().to_string(), None)
    }
}

// ── Internal: deduplication ───────────────────────────────────────────────────

fn dedup_annotations(annotations: &mut Vec<Annotation>) {
    use std::collections::HashSet;
    let mut seen: HashSet<(PathBuf, u32, AnnotationKind)> = HashSet::new();
    annotations.retain(|a| {
        let key = (a.source.file.clone(), a.source.line, a.kind);
        seen.insert(key)
    });
}

// ── Internal: find ast-grep binary ───────────────────────────────────────────

/// Locate `ast-grep` on `PATH`, trying common install locations.
fn find_ast_grep() -> Result<PathBuf, ExtractorError> {
    // Try common known paths first, then fall back to PATH search.
    let candidates = [
        "/opt/homebrew/bin/ast-grep",
        "/usr/local/bin/ast-grep",
        "/usr/bin/ast-grep",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    // Fall back to PATH lookup via `which`-style check.
    which_ast_grep()
}

fn which_ast_grep() -> Result<PathBuf, ExtractorError> {
    // Use `which` on Unix.
    let output = Command::new("which")
        .arg("ast-grep")
        .output()
        .map_err(|_| ExtractorError::BinaryNotFound)?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            return Ok(PathBuf::from(path_str));
        }
    }
    Err(ExtractorError::BinaryNotFound)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Unit tests: strip_comment_prefixes ──

    #[test]
    fn strip_double_slash_prefix() {
        let result = strip_comment_prefixes("// @g8.capability(name = \"x\")");
        assert!(result.contains("@g8.capability"), "got: {result}");
    }

    #[test]
    fn strip_hash_prefix() {
        let result = strip_comment_prefixes("# @g8.capability(name = \"x\")");
        assert!(result.contains("@g8.capability"), "got: {result}");
    }

    #[test]
    fn strip_multiline_comment() {
        let text =
            "// @g8.capability(name = \"http-fetch\",\n//                 status = \"in_flight\")";
        let result = strip_comment_prefixes(text);
        assert!(result.contains("@g8.capability"), "got: {result}");
        assert!(result.contains("http-fetch"), "got: {result}");
        assert!(result.contains("in_flight"), "got: {result}");
    }

    // ── Unit tests: classify_heading ──

    #[test]
    fn classify_architecture_heading() {
        let (kind, is_owner, is_parked) = classify_heading("Architecture");
        assert_eq!(kind, IntentKind::ArchitecturalScope);
        assert!(!is_owner);
        assert!(!is_parked);
    }

    #[test]
    fn classify_boundaries_heading() {
        let (kind, is_owner, is_parked) = classify_heading("Boundaries");
        assert_eq!(kind, IntentKind::Boundary);
        assert!(!is_owner);
        assert!(!is_parked);
    }

    #[test]
    fn classify_parked_ideas_heading() {
        let (_, _, is_parked) = classify_heading("Parked Ideas");
        assert!(is_parked);
    }

    #[test]
    fn classify_owner_heading() {
        let (_, is_owner, _) = classify_heading("Owner");
        assert!(is_owner);
    }

    #[test]
    fn classify_unknown_heading_is_unclassified() {
        let (kind, is_owner, is_parked) = classify_heading("Random Section");
        assert_eq!(kind, IntentKind::Unclassified);
        assert!(!is_owner);
        assert!(!is_parked);
    }

    // ── Unit tests: is_tests_dir ──

    #[test]
    fn tests_dir_detection() {
        assert!(is_tests_dir(Path::new("src/tests/foo.rs")));
        assert!(is_tests_dir(Path::new(
            "crates/core/tests-integration/bar.rs"
        )));
        assert!(is_tests_dir(Path::new("unit-tests/baz.rs")));
        assert!(!is_tests_dir(Path::new("src/lib.rs")));
        assert!(!is_tests_dir(Path::new("src/test_helper.rs")));
    }

    // ── Unit tests: parse_parked_line ──

    #[test]
    fn parse_parked_line_with_reason() {
        let (name, reason) = parse_parked_line("config-hot-reload: requires Arc<RwLock<Config>>");
        assert_eq!(name, "config-hot-reload");
        assert_eq!(reason.unwrap(), "requires Arc<RwLock<Config>>");
    }

    #[test]
    fn parse_parked_line_without_reason() {
        let (name, reason) = parse_parked_line("ml-scoring");
        assert_eq!(name, "ml-scoring");
        assert!(reason.is_none());
    }

    // ── Unit test: RawAstMatch JSON deserialization ──

    #[test]
    fn raw_ast_match_deserializes_from_real_ast_grep_output() {
        // Exact format emitted by ast-grep scan --json=stream
        let line = r#"{"text":"// @g8.capability(name = \"test-only-cap\", status = \"in_flight\", dev_only = true)","range":{"byteOffset":{"start":163,"end":244},"start":{"line":4,"column":0},"end":{"line":4,"column":81}},"file":"/tmp/test.rs","lines":"// @g8.capability(name = \"test-only-cap\", status = \"in_flight\", dev_only = true)","charCount":{"leading":0,"trailing":0},"language":"Rust","ruleId":"g8-rust-magic-comment","severity":"hint","note":null,"message":"G8 magic-comment annotation (Rust)","labels":[]}"#;
        let result: Result<RawAstMatch, _> = serde_json::from_str(line);
        match &result {
            Err(e) => panic!("JSON parse failed: {e}\nLine: {line}"),
            Ok(m) => {
                assert_eq!(m.text, "// @g8.capability(name = \"test-only-cap\", status = \"in_flight\", dev_only = true)");
                assert_eq!(m.range.start.line, 4);
                assert_eq!(m.range.start.column, 0);
                assert_eq!(m.file, "/tmp/test.rs");
                assert_eq!(m.rule_id.as_deref(), Some("g8-rust-magic-comment"));
            }
        }
    }

    #[test]
    fn convert_ast_match_roundtrip() {
        // Verifies that a real ast-grep JSON line round-trips to a valid Annotation
        // including conditional=true when dev_only=true is present.
        let line = r#"{"text":"// @g8.capability(name = \"test-only-cap\", status = \"in_flight\", dev_only = true)","range":{"byteOffset":{"start":163,"end":244},"start":{"line":4,"column":0},"end":{"line":4,"column":81}},"file":"/tmp/test.rs","lines":"// @g8.capability(name = \"test-only-cap\", status = \"in_flight\", dev_only = true)","charCount":{"leading":0,"trailing":0},"language":"Rust","ruleId":"g8-rust-magic-comment","severity":"hint","note":null,"message":"G8 magic-comment annotation (Rust)","labels":[]}"#;
        let raw: RawAstMatch = serde_json::from_str(line).expect("parse");
        let name_re = Regex::new(r#"name\s*=\s*"([^"]+)""#).unwrap();
        let result = convert_ast_match(&raw, &name_re);
        match result {
            Err(e) => panic!("convert_ast_match failed: {e}"),
            Ok(None) => panic!("convert_ast_match returned None unexpectedly"),
            Ok(Some(ann)) => {
                assert_eq!(ann.kind, AnnotationKind::Capability);
                assert_eq!(ann.name.as_deref(), Some("test-only-cap"));
                assert!(ann.conditional, "dev_only=true should set conditional");
            }
        }
    }

    // ── Unit tests: is_conditional ──

    #[test]
    fn dev_only_sets_conditional() {
        let mut fields = BTreeMap::new();
        fields.insert("dev_only".to_string(), AnnotationValue::Bool(true));
        assert!(is_conditional("", &fields));
    }

    #[test]
    fn cfg_attr_sets_conditional() {
        let fields = BTreeMap::new();
        assert!(is_conditional("#[cfg_attr(test, g8(...))]", &fields));
    }

    // ── Embedded rules materialization ──

    #[test]
    fn embedded_rules_materialize_deterministically() {
        let first = materialize_embedded_rules().unwrap();
        let second = materialize_embedded_rules().unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), EMBEDDED_RULES);
    }

    // ── Integration tests: parse_sidecar ──

    #[test]
    fn parse_agents_md_fixture() {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/markdown/AGENTS.md"
        ));
        if !fixture.exists() {
            return; // fixture not created yet (but it should be)
        }
        let annotations = parse_sidecar(fixture, IntentSourceKind::AgentsMd).unwrap();
        assert!(
            !annotations.is_empty(),
            "expected annotations from AGENTS.md fixture"
        );
        // At least one ArchitecturalScope intent.
        let has_arch = annotations.iter().any(|a| {
            a.fields
                .get("intent_kind")
                .and_then(|v| {
                    if let AnnotationValue::String(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .map(|s| s == "architectural_scope")
                .unwrap_or(false)
        });
        assert!(has_arch, "expected at least one architectural_scope intent");
    }
}
