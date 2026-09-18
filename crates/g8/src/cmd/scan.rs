//! `g8 scan [PATH]` — extract annotations and populate the store.

use std::path::PathBuf;

use anyhow::{Context, Result};

use g8_store::{ScanResult, StoreConnection};

use crate::cli::{OutputMode, ScanArgs};
use crate::ctx::{default_project_name, ensure_space_and_project, require_init, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, args: &ScanArgs) -> Result<i32> {
    require_init(ctx)?;

    let root: PathBuf = args
        .path
        .clone()
        .unwrap_or_else(|| ctx.g8_dir.parent().unwrap_or(&ctx.g8_dir).to_path_buf());
    let root = root.canonicalize().unwrap_or(root);

    // Run extractor.
    let extractor = g8_extractor::Extractor::new()
        .context("initializing extractor (is ast-grep installed?)")?;

    if !ctx.quiet {
        eprintln!("Scanning {}...", root.display());
    }

    let scan_result = extractor
        .scan_dir(&root)
        .with_context(|| format!("scanning {}", root.display()))?;

    // Print warnings.
    for w in &scan_result.warnings {
        eprintln!("warn: {}:{:?}: {}", w.file.display(), w.line, w.message);
    }

    let annotation_count = scan_result.annotations.len();

    if args.report_only {
        // AUDIT mode: report what was found, don't write to store.
        let msg = format!(
            "AUDIT (report-only): found {} annotations in {} files ({} ms)",
            annotation_count, scan_result.stats.files_scanned, scan_result.stats.duration_ms,
        );
        match ctx.output {
            OutputMode::Json => json::render_ok(msg),
            OutputMode::Pretty => {
                if !ctx.quiet {
                    pretty::ok(&msg, ctx.color);
                }
            }
        }
        return Ok(0);
    }

    // Write to store.
    let mut store = ctx.open_store()?;
    let project_name = args
        .project
        .clone()
        .unwrap_or_else(|| default_project_name(ctx));

    let (space, project) =
        ensure_space_and_project(&mut store, &project_name, &project_name, &root)?;
    let _ = space;

    // Convert extractor's ScanResult to store's ScanResult.
    let store_scan = ScanResult {
        annotations: scan_result.annotations.clone(),
        warnings: scan_result
            .warnings
            .iter()
            .map(|w| w.message.clone())
            .collect(),
    };

    let report = store
        .apply_scan(&project.id, store_scan)
        .context("applying scan to store")?;

    // Regenerate INTENT_SUMMARY.md
    let root_path = ctx.g8_dir.parent().unwrap_or(&ctx.g8_dir);
    crate::cmd::status::regenerate_intent_summary(ctx, &store, root_path)
        .unwrap_or_else(|e| eprintln!("warn: could not regenerate INTENT_SUMMARY.md: {e:#}"));

    match ctx.output {
        OutputMode::Json => json::render_scan_result(
            report.inserted_capabilities,
            report.inserted_intents,
            report.inserted_decisions,
            scan_result.stats.files_scanned,
            scan_result.stats.duration_ms,
        ),
        OutputMode::Pretty => {
            if !ctx.quiet {
                let msg = format!(
                    "Scan complete: {} capabilities, {} intents, {} decisions inserted.",
                    report.inserted_capabilities,
                    report.inserted_intents,
                    report.inserted_decisions
                );
                pretty::ok(&msg, ctx.color);
            }
        }
    }

    Ok(0)
}
