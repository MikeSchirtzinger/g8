//! `g8 space {add,remove,list}` — federation of multi-project spaces.

use anyhow::{Context, Result};

use g8_core::{now_millis, Project, ProjectId};
use g8_store::StoreConnection;

use crate::cli::{OutputMode, SpaceAddArgs, SpaceRemoveArgs, SpaceSubcommand};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, subcmd: &SpaceSubcommand) -> Result<i32> {
    require_init(ctx)?;

    match subcmd {
        SpaceSubcommand::Add(args) => space_add(ctx, args),
        SpaceSubcommand::Remove(args) => space_remove(ctx, args),
        SpaceSubcommand::List => space_list(ctx),
    }
}

fn space_add(ctx: &Ctx, args: &SpaceAddArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    let path = std::path::PathBuf::from(&args.path);
    let path = path.canonicalize().unwrap_or(path);

    let project_name = args.name.clone().unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string())
    });

    let project = Project {
        id: ProjectId::derive(&[space.id.as_str(), &project_name]),
        space_id: space.id.clone(),
        name: project_name.clone(),
        description: None,
        root_path: path.clone(),
        language: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };

    store.upsert_project(&project).context("upsert_project")?;

    // SPEC Decision 3 (locked): the ConvergenceSpace is DECLARED by
    // `.g8/space.toml` at the space root — the store row alone is not the
    // declaration. Written after the store upsert succeeds so a failed
    // upsert never leaves a member declared but unqueryable.
    crate::space_toml::upsert_member(
        &ctx.g8_dir,
        &crate::space_toml::SpaceIdentity {
            id: space.id.as_str(),
            name: &space.name,
        },
        &project_name,
        &args.path,
    )
    .context("writing .g8/space.toml")?;

    let msg = format!(
        "Added project '{project_name}' at {} to space.",
        path.display()
    );
    match ctx.output {
        OutputMode::Json => json::render_ok(msg),
        OutputMode::Pretty => {
            if !ctx.quiet {
                pretty::ok(&msg, ctx.color);
            }
        }
    }
    Ok(0)
}

fn space_remove(ctx: &Ctx, args: &SpaceRemoveArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    let project = store
        .find_project_by_name(&space.id, &args.project)
        .context("find_project_by_name")?
        .with_context(|| format!("project '{}' not found in space", args.project))?;

    let project_id = project.id.clone();
    store
        .delete_project(&project_id)
        .context("delete_project")?;

    // Keep the space.toml declaration in sync with the store (SPEC
    // Decision 3) — tolerant of stores predating the space.toml write.
    crate::space_toml::remove_member(&ctx.g8_dir, &args.project)
        .context("updating .g8/space.toml")?;

    let msg = format!(
        "Removed project '{}' ({}).",
        args.project,
        project_id.as_str()
    );
    match ctx.output {
        OutputMode::Json => json::render_ok(msg),
        OutputMode::Pretty => {
            if !ctx.quiet {
                pretty::ok(&msg, ctx.color);
            }
        }
    }
    Ok(0)
}

fn space_list(ctx: &Ctx) -> Result<i32> {
    let store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;
    let projects = store.list_projects(&space.id).context("list_projects")?;

    match ctx.output {
        OutputMode::Json => {
            let value = serde_json::json!({
                "g8_version": crate::render::json::G8_VERSION,
                "space": {
                    "id": space.id.as_str(),
                    "name": space.name,
                },
                "projects": projects,
                "count": projects.len(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("serializable")
            );
        }
        OutputMode::Pretty => {
            println!("Space: {} ({})", space.name, space.id.as_str());
            println!("Projects:");
            for p in &projects {
                println!(
                    "  - {}: {} ({})",
                    p.name,
                    p.root_path.display(),
                    p.id.as_str()
                );
            }
        }
    }

    Ok(0)
}
