//! `g8 merge --from <PATH>` — cross-space conflict detection.
//!
//! Opens a remote store (from `<PATH>/.g8/store.db`), attaches it, and
//! calls `g8_conflict::ConflictDetector::compare_spaces`.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use g8_conflict::{ConflictAdapter, ConflictDetector};
use g8_store::StoreConnection;

use crate::cli::{MergeArgs, OutputMode};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, args: &MergeArgs) -> Result<i32> {
    require_init(ctx)?;

    let remote_path = PathBuf::from(&args.from);
    let remote_g8 = remote_path.join(".g8");
    if !remote_g8.exists() {
        bail!(
            "No .g8/ directory found at {}. Is this an g8 project?",
            remote_path.display()
        );
    }

    let remote_store_path = remote_g8.join("store.db");
    if !remote_store_path.exists() {
        bail!(
            "No store.db found at {}. Run `g8 scan` in the remote project first.",
            remote_store_path.display()
        );
    }

    // Open local store.
    let local_store = ctx.open_store()?;
    let local_space = require_space(&local_store, ctx.space_id())?;

    // Open remote store.
    let mut remote_store = g8_store::RusqliteStore::open(&remote_store_path)
        .with_context(|| format!("opening remote store at {}", remote_store_path.display()))?;
    remote_store.migrate().context("migrating remote store")?;

    let remote_space = remote_store
        .get_default_space()
        .context("get_default_space on remote")?
        .context("remote store has no space: run `g8 init` in the remote project")?;

    // Build ConflictStore adapters.
    let local_adapter = ConflictAdapter::new(&local_store);
    let remote_adapter = ConflictAdapter::new(&remote_store);

    // Run conflict detection.
    let all_conflicts = ConflictDetector::compare_spaces(
        &local_adapter,
        &local_space.id,
        &remote_adapter,
        &remote_space.id,
    )
    .context("compare_spaces")?;

    // Filter by kind if requested.
    let conflicts: Vec<_> = if args.kind.is_empty() {
        all_conflicts
    } else {
        all_conflicts
            .into_iter()
            .filter(|c| {
                args.kind.iter().any(|k| {
                    let kind_str = format!("{:?}", c.kind).to_lowercase();
                    kind_str.contains(&k.to_lowercase())
                })
            })
            .collect()
    };

    // Render.
    let use_json = args.json || ctx.output == OutputMode::Json;
    if use_json {
        json::render_conflicts(&conflicts);
    } else {
        pretty::render_conflicts(&conflicts, ctx.color);
    }

    Ok(0)
}
