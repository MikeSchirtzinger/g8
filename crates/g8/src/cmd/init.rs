//! `g8 init` — initialize the `.g8/` substrate in a directory.
//!
//! Per ARCHITECTURE.md §9 and SPEC.md Decision 9:
//! 1. Create `.g8/` directory.
//! 2. Apply migrations to `.g8/store.db`.
//! 3. Run extractor in report-only mode (AUDIT scan).
//! 4. Print AUDIT summary.
//! 5. Write `.g8/config.toml` with `enforcement = "off"` by default.
//! 6. Append `@.g8/INTENT_SUMMARY.md` to `CLAUDE.md` (unless `--no-claude-import`).
//! 7. Drop `.claude/agents/g8-planner.md` (unless `--no-subagents`).
//! 8. Print next steps.
//!
//! `g8 init --enforce` flips `.g8/config.toml::enforcement = "on"`.

use anyhow::{Context, Result};

use g8_core::{now_millis, ConvergenceSpace, Project, ProjectId, SpaceId};
use g8_store::{RusqliteStore, StoreConnection};

use crate::cli::InitArgs;
use crate::ctx::{default_project_name, write_config, Ctx, G8Config, G8Section};
use crate::render::{json, pretty};

/// Subagent definition baked into the binary (ARCH §16).
const SUBAGENT_PLANNER: &str = include_str!("../assets/g8-planner.md");

pub fn run(ctx: &Ctx, args: &InitArgs) -> Result<i32> {
    let g8_dir = &ctx.g8_dir;
    let root = g8_dir.parent().unwrap_or(g8_dir).to_path_buf();

    // 1. Create `.g8/` directory.
    std::fs::create_dir_all(g8_dir).with_context(|| format!("creating {}", g8_dir.display()))?;

    // 2. Open + migrate store.
    let mut store = RusqliteStore::open(&ctx.store_path)
        .with_context(|| format!("opening store at {}", ctx.store_path.display()))?;
    store.migrate().context("running migrations")?;

    let space_name = args
        .name
        .clone()
        .unwrap_or_else(|| default_project_name(ctx));
    let project_name = space_name.clone();

    // Create a space + project if none exists yet.
    let (space, _project) =
        ensure_space_and_project_init(&mut store, &space_name, &project_name, &root)?;

    // 3. Run AUDIT scan (report-only — no writes to store).
    let audit = run_audit_scan(&root);

    // 4. Print AUDIT summary (to stderr in JSON mode to keep stdout clean).
    if !ctx.quiet {
        match ctx.output {
            crate::cli::OutputMode::Json => match &audit {
                AuditOutcome::Ok(summary) => {
                    eprintln!(
                        "AUDIT (report-only): capabilities={} intents={} decisions={}",
                        summary.capabilities, summary.intents, summary.decisions
                    );
                }
                AuditOutcome::Skipped(reason) => {
                    eprintln!("AUDIT (report-only): SKIPPED: {reason}");
                }
            },
            crate::cli::OutputMode::Pretty => print_audit_summary(&audit, ctx.color),
        }
    }

    // 5. Write `.g8/config.toml`.
    let enforcement = if args.enforce { "on" } else { "off" };
    let config = G8Config {
        g8: G8Section {
            version: Some("0.1.0".to_string()),
            enforcement: Some(enforcement.to_string()),
            project_name: Some(project_name.clone()),
            project_id: None,
            space_id: Some(space.id.as_str().to_string()),
        },
        ..Default::default()
    };
    write_config(g8_dir, &config)?;

    // 6. Append `@.g8/INTENT_SUMMARY.md` to `CLAUDE.md`.
    if !args.no_claude_import {
        append_claude_md_import(&root).unwrap_or_else(|e| {
            eprintln!("warn: could not update CLAUDE.md: {e}");
        });
    }

    // 7. Drop `.claude/agents/g8-planner.md`.
    if !args.no_subagents {
        drop_subagent_definition(&root).unwrap_or_else(|e| {
            eprintln!("warn: could not write subagent definition: {e}");
        });
    }

    // 7b. Install `.git/hooks/pre-commit` if requested.
    if args.install_pre_commit {
        match install_pre_commit_hook(&root) {
            Ok(path) => {
                if !ctx.quiet {
                    eprintln!("installed pre-commit hook at {}", path.display());
                }
            }
            Err(e) => {
                eprintln!("warn: could not install pre-commit hook: {e:#}");
            }
        }
    }

    // 8. Print next steps.
    if !ctx.quiet {
        let msg = format!(
            "g8 initialized at {}. Enforcement: {enforcement}.",
            g8_dir.display()
        );
        match ctx.output {
            crate::cli::OutputMode::Json => json::render_ok(msg),
            crate::cli::OutputMode::Pretty => {
                pretty::ok(&msg, ctx.color);
                println!();
                println!("Next steps:");
                println!("  g8 scan           : populate the store");
                println!("  g8 status         : view current intent summary");
                if enforcement == "off" {
                    println!("  g8 init --enforce : flip enforcement gate ON");
                }
            }
        }
    }

    Ok(0)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ensure_space_and_project_init(
    store: &mut RusqliteStore,
    space_name: &str,
    project_name: &str,
    root: &std::path::Path,
) -> Result<(ConvergenceSpace, Project)> {
    if let Some(space) = store.get_default_space().context("get_default_space")? {
        let projects = store.list_projects(&space.id).context("list_projects")?;
        if let Some(project) = projects.into_iter().find(|p| p.name == project_name) {
            return Ok((space, project));
        }
        let project = Project {
            id: ProjectId::derive(&[space.id.as_str(), project_name]),
            space_id: space.id.clone(),
            name: project_name.to_string(),
            description: None,
            root_path: root.to_path_buf(),
            language: None,
            created_at: now_millis(),
            updated_at: now_millis(),
            meta: None,
        };
        store.upsert_project(&project).context("upsert_project")?;
        return Ok((space, project));
    }

    let space = ConvergenceSpace {
        id: SpaceId::derive(&[&root.to_string_lossy(), space_name]),
        name: space_name.to_string(),
        root_path: root.to_path_buf(),
        created_at: now_millis(),
        updated_at: now_millis(),
        members: vec![],
        meta: None,
    };
    store.init_space(&space).context("init_space")?;

    let project = Project {
        id: ProjectId::derive(&[space.id.as_str(), project_name]),
        space_id: space.id.clone(),
        name: project_name.to_string(),
        description: None,
        root_path: root.to_path_buf(),
        language: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store.upsert_project(&project).context("upsert_project")?;

    Ok((space, project))
}

// ── AUDIT scan ────────────────────────────────────────────────────────────────

struct AuditSummary {
    capabilities: usize,
    intents: usize,
    decisions: usize,
}

/// Tri-state outcome for the init-time AUDIT scan.
///
/// `Skipped(reason)` is the honest representation of "extractor not available"
/// or "scan errored". Reporting it as `0/0/0` (the previous behaviour) was
/// misleading — users assumed a clean scan when in reality nothing ran.
enum AuditOutcome {
    Ok(AuditSummary),
    Skipped(String),
}

fn run_audit_scan(root: &std::path::Path) -> AuditOutcome {
    match g8_extractor::Extractor::new() {
        Ok(extractor) => match extractor.scan_dir(root) {
            Ok(result) => {
                let caps = result
                    .annotations
                    .iter()
                    .filter(|a| a.kind == g8_core::AnnotationKind::Capability)
                    .count();
                let intents = result
                    .annotations
                    .iter()
                    .filter(|a| a.kind == g8_core::AnnotationKind::Intent)
                    .count();
                let decisions = result
                    .annotations
                    .iter()
                    .filter(|a| a.kind == g8_core::AnnotationKind::Decision)
                    .count();
                AuditOutcome::Ok(AuditSummary {
                    capabilities: caps,
                    intents,
                    decisions,
                })
            }
            Err(e) => AuditOutcome::Skipped(format!("scan failed: {e}")),
        },
        Err(e) => AuditOutcome::Skipped(format!("extractor unavailable: {e}")),
    }
}

fn print_audit_summary(outcome: &AuditOutcome, color: bool) {
    println!("AUDIT scan (report-only):");
    match outcome {
        AuditOutcome::Ok(audit) => println!(
            "  Capabilities: {}  Intents: {}  Decisions: {}",
            audit.capabilities, audit.intents, audit.decisions
        ),
        AuditOutcome::Skipped(reason) => {
            println!("  SKIPPED: {reason}");
            println!("  (re-run `g8 scan` once the prerequisite is available)");
        }
    }
    let _ = color; // May colour counts in future
}

// ── CLAUDE.md import ──────────────────────────────────────────────────────────

const CLAUDE_IMPORT_LINE: &str = "@.g8/INTENT_SUMMARY.md";

fn append_claude_md_import(root: &std::path::Path) -> Result<()> {
    let claude_md = root.join("CLAUDE.md");
    let mut content = if claude_md.exists() {
        std::fs::read_to_string(&claude_md).context("reading CLAUDE.md")?
    } else {
        String::new()
    };

    if !content.contains(CLAUDE_IMPORT_LINE) {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(CLAUDE_IMPORT_LINE);
        content.push('\n');
        std::fs::write(&claude_md, &content).context("writing CLAUDE.md")?;
    }
    Ok(())
}

// ── Pre-commit hook ──────────────────────────────────────────────────────────

const PRE_COMMIT_HOOK: &str = r#"#!/usr/bin/env sh
# Installed by `g8 init --install-pre-commit`.
# Fails the commit if the pairing gate finds errors. Skip with `--no-verify`.
set -e
if ! command -v g8 >/dev/null 2>&1; then
    echo "g8 not on PATH: skipping pairing gate" >&2
    exit 0
fi
exec g8 check --json >/dev/null
"#;

fn install_pre_commit_hook(root: &std::path::Path) -> Result<std::path::PathBuf> {
    use anyhow::bail;

    let git_dir = root.join(".git");
    if !git_dir.is_dir() {
        bail!(
            "no .git/ directory at {} (not a git repository)",
            root.display()
        );
    }
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir).context("creating .git/hooks/")?;
    let path = hooks_dir.join("pre-commit");
    if path.exists() {
        bail!(
            "{} already exists: remove it first if you want g8 to manage it",
            path.display()
        );
    }
    std::fs::write(&path, PRE_COMMIT_HOOK)
        .with_context(|| format!("writing {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)
            .with_context(|| format!("chmod +x {}", path.display()))?;
    }

    Ok(path)
}

// ── Subagent definition ───────────────────────────────────────────────────────

fn drop_subagent_definition(root: &std::path::Path) -> Result<()> {
    let agents_dir = root.join(".claude").join("agents");
    std::fs::create_dir_all(&agents_dir).context("creating .claude/agents/")?;
    let dest = agents_dir.join("g8-planner.md");
    std::fs::write(&dest, SUBAGENT_PLANNER).with_context(|| format!("writing {}", dest.display()))
}
