//! `g8 plan <SUBCOMMAND>` — plan lifecycle management.
//!
//! Subcommands: new, list, show, park, promote, scope, dispatch, block, unblock, complete.

use anyhow::{bail, Context, Result};

use g8_core::{
    now_millis,
    plan::{PlanDraft, Recommendation},
    OverlapKind, Plan, PlanId, PlanStatus, ProjectId,
};
use g8_store::{PlanFilter, StoreConnection};

use crate::cli::{
    OutputMode, PlanBlockArgs, PlanDispatchArgs, PlanListArgs, PlanNewArgs, PlanParkArgs,
    PlanPromoteArgs, PlanScopeArgs, PlanShowArgs, PlanSubcommand, PlanUnblockArgs,
};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, subcmd: &PlanSubcommand) -> Result<i32> {
    require_init(ctx)?;

    match subcmd {
        PlanSubcommand::New(args) => plan_new(ctx, args),
        PlanSubcommand::List(args) => plan_list(ctx, args),
        PlanSubcommand::Show(args) => plan_show(ctx, args),
        PlanSubcommand::Park(args) => plan_park(ctx, args),
        PlanSubcommand::Promote(args) => plan_promote(ctx, args),
        PlanSubcommand::Scope(args) => plan_scope(ctx, args),
        PlanSubcommand::Dispatch(args) => plan_dispatch(ctx, args),
        PlanSubcommand::Block(args) => plan_block(ctx, args),
        PlanSubcommand::Unblock(args) => plan_unblock(ctx, args),
        PlanSubcommand::Complete(args) => plan_complete(ctx, args),
    }
}

// ── plan new ──────────────────────────────────────────────────────────────────

fn plan_new(ctx: &Ctx, args: &PlanNewArgs) -> Result<i32> {
    let store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    let touches: Vec<String> = args.touches.clone().unwrap_or_default();

    let draft = PlanDraft {
        title: args.title.clone(),
        description: args.description.clone(),
        substrate: args.substrate.clone(),
        touched_capabilities: touches.clone(),
        touched_paths: vec![],
        space_id: space.id.clone(),
        project_id: None,
    };

    // Run planner_intent_check via Planner.
    let report = g8_planner::Planner::plan_check(&store, &draft).context("running planner")?;

    // Enforce WIP cap / governance when the user asked to dispatch directly.
    // Anything other than Proceed is the planner saying "don't dispatch this now".
    if args.dispatch && report.recommendation != Recommendation::Proceed {
        let use_json = args.json || ctx.output == OutputMode::Json;
        if use_json {
            json::render_fit_report(&report);
        } else {
            pretty::render_fit_report(&report, ctx.color);
        }
        bail!(
            "refusing to dispatch: planner recommendation is {:?} (use without --dispatch to create as Idea/Park)",
            report.recommendation
        );
    }

    // Resolve touched capability names → IDs across all projects in this space.
    // Names that match multiple projects produce multiple links (intentional —
    // capability_overlap checks already treat name as the cross-project identity).
    let projects = store.list_projects(&space.id).context("list_projects")?;
    let mut resolved_cap_ids: Vec<g8_core::CapabilityId> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for raw_name in &touches {
        let canonical = store
            .resolve_canonical(raw_name)
            .unwrap_or_else(|_| raw_name.clone());
        let mut hit = false;
        for project in &projects {
            if let Some(cap) = store
                .find_capability_by_name(&project.id, &canonical)
                .context("find_capability_by_name")?
            {
                resolved_cap_ids.push(cap.id);
                hit = true;
            }
        }
        if !hit {
            unresolved.push(raw_name.clone());
        }
    }
    if !unresolved.is_empty() {
        eprintln!(
            "warn: --touches names not found as capabilities in this space (link rows skipped): {}",
            unresolved.join(", ")
        );
    }

    // Persist the plan at the appropriate initial status.
    let initial_status = if args.park {
        PlanStatus::Parked
    } else if args.dispatch {
        PlanStatus::Dispatched
    } else {
        PlanStatus::Idea
    };

    // Re-open mutably for write.
    let mut store_w = ctx.open_store()?;
    // Plans have no natural key (two plans may share a title), so the
    // content-derived ID (Q4 ruling) takes the count of existing plans as a
    // sequence disambiguator — deterministic under replay, distinct for
    // repeated same-title creations.
    let plan_seq = store_w
        .list_plans(
            &space.id,
            PlanFilter {
                status: None,
                substrate: None,
                project: None,
                limit: None,
            },
        )
        .context("counting existing plans for id derivation")?
        .len();
    let plan = Plan {
        id: PlanId::derive(&[space.id.as_str(), &args.title, &plan_seq.to_string()]),
        space_id: space.id.clone(),
        project_id: None,
        title: args.title.clone(),
        description: args.description.clone(),
        status: initial_status,
        substrate: args.substrate.clone(),
        touched_capabilities: resolved_cap_ids.clone(),
        derived_from_intents: vec![],
        governed_by_decisions: vec![],
        wip_weight: 1,
        parked_reason: None,
        blocked_reason: None,
        dispatched_at: if args.dispatch {
            Some(now_millis())
        } else {
            None
        },
        completed_at: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store_w.create_plan(&plan).context("create_plan")?;

    for cap_id in &resolved_cap_ids {
        store_w
            .link_plan_capability(&plan.id, cap_id, OverlapKind::Touches)
            .context("link_plan_capability")?;
    }

    // Render.
    let use_json = args.json || ctx.output == OutputMode::Json;
    if use_json {
        json::render_plan_new_result(&report, &plan.id);
    } else {
        pretty::render_fit_report(&report, ctx.color);
        println!("Created plan: {}", plan.id.as_str());
    }

    Ok(0)
}

// ── plan list ─────────────────────────────────────────────────────────────────

fn plan_list(ctx: &Ctx, args: &PlanListArgs) -> Result<i32> {
    let store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    let mut status_filter = args.status.clone().map(|statuses| {
        statuses
            .iter()
            .filter_map(|s| parse_plan_status(s))
            .collect::<Vec<_>>()
    });

    if args.parked {
        status_filter = Some(vec![PlanStatus::Parked]);
    }

    let project_id: Option<ProjectId> = if let Some(name) = args.project.as_ref() {
        Some(
            store
                .find_project_by_name(&space.id, name)
                .context("find_project_by_name")?
                .with_context(|| format!("project '{name}' not found in current space"))?
                .id,
        )
    } else {
        None
    };

    let filter = PlanFilter {
        status: status_filter,
        substrate: args.substrate.clone(),
        project: project_id,
        limit: None,
    };

    let plans = store.list_plans(&space.id, filter).context("list_plans")?;

    match ctx.output {
        OutputMode::Json => json::render_plan_list(&plans),
        OutputMode::Pretty => pretty::render_plan_list(&plans, ctx.color),
    }

    Ok(0)
}

// ── plan show ─────────────────────────────────────────────────────────────────

fn plan_show(ctx: &Ctx, args: &PlanShowArgs) -> Result<i32> {
    let store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    let plan = store
        .get_plan(&plan_id)
        .context("get_plan")?
        .with_context(|| format!("plan {} not found", args.id))?;

    match ctx.output {
        OutputMode::Json => json::render_plan_detail(&plan),
        OutputMode::Pretty => pretty::render_plan_detail(&plan, ctx.color),
    }

    Ok(0)
}

// ── plan park ─────────────────────────────────────────────────────────────────

fn plan_park(ctx: &Ctx, args: &PlanParkArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    store
        .update_plan_status(&plan_id, PlanStatus::Parked, args.reason.clone())
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} parked.", args.id))
}

// ── plan promote ─────────────────────────────────────────────────────────────

fn plan_promote(ctx: &Ctx, args: &PlanPromoteArgs) -> Result<i32> {
    let store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;

    // Re-run planner before promoting.
    let plan = store
        .get_plan(&plan_id)
        .context("get_plan")?
        .with_context(|| format!("plan {} not found", args.id))?;

    let draft = PlanDraft {
        title: plan.title.clone(),
        description: plan.description.clone(),
        substrate: plan.substrate.clone(),
        touched_capabilities: plan
            .touched_capabilities
            .iter()
            .map(|id| id.as_str().to_string())
            .collect(),
        touched_paths: vec![],
        space_id: plan.space_id.clone(),
        project_id: plan.project_id.clone(),
    };

    let report = g8_planner::Planner::plan_check(&store, &draft).context("running planner")?;

    let mut store_w = ctx.open_store()?;
    store_w
        .update_plan_status(&plan_id, PlanStatus::Scoped, None)
        .context("update_plan_status")?;

    match ctx.output {
        OutputMode::Json => json::render_fit_report(&report),
        OutputMode::Pretty => {
            pretty::render_fit_report(&report, ctx.color);
            pretty::ok(&format!("Plan {} promoted to Scoped.", args.id), ctx.color);
        }
    }

    Ok(0)
}

// ── plan scope ────────────────────────────────────────────────────────────────

fn plan_scope(ctx: &Ctx, args: &PlanScopeArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    store
        .update_plan_status(&plan_id, PlanStatus::Scoped, None)
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} scoped.", args.id))
}

// ── plan dispatch ─────────────────────────────────────────────────────────────

fn plan_dispatch(ctx: &Ctx, args: &PlanDispatchArgs) -> Result<i32> {
    let store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    let plan = store
        .get_plan(&plan_id)
        .context("get_plan")?
        .with_context(|| format!("plan {} not found", args.id))?;

    // Enforce the substrate WIP cap before dispatching. We re-run the composite
    // planner check against a draft mirroring the plan so we get the
    // BudgetStatus the same way `plan new --dispatch` does.
    if let Some(ref substrate) = plan.substrate {
        let draft = PlanDraft {
            title: plan.title.clone(),
            description: plan.description.clone(),
            substrate: Some(substrate.clone()),
            touched_capabilities: plan
                .touched_capabilities
                .iter()
                .map(|id| id.as_str().to_string())
                .collect(),
            touched_paths: vec![],
            space_id: plan.space_id.clone(),
            project_id: plan.project_id.clone(),
        };
        let report = g8_planner::Planner::plan_check(&store, &draft).context("running planner")?;
        if let Some(b) = report.budget_status.as_ref() {
            if b.at_cap {
                bail!(
                    "refusing to dispatch: substrate '{}' is at WIP cap ({}/{})",
                    b.substrate,
                    b.wip_current,
                    b.wip_cap
                );
            }
        }
    }

    let mut store_w = ctx.open_store()?;
    store_w
        .update_plan_status(&plan_id, PlanStatus::Dispatched, None)
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} dispatched.", args.id))
}

// ── plan block ────────────────────────────────────────────────────────────────

fn plan_block(ctx: &Ctx, args: &PlanBlockArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    store
        .update_plan_status(&plan_id, PlanStatus::Blocked, args.reason.clone())
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} blocked.", args.id))
}

// ── plan unblock ──────────────────────────────────────────────────────────────

fn plan_unblock(ctx: &Ctx, args: &PlanUnblockArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    store
        .update_plan_status(&plan_id, PlanStatus::Dispatched, None)
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} unblocked.", args.id))
}

// ── plan complete ─────────────────────────────────────────────────────────────

fn plan_complete(ctx: &Ctx, args: &crate::cli::PlanCompleteArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let plan_id = parse_plan_id(&args.id)?;
    store
        .update_plan_status(&plan_id, PlanStatus::Done, None)
        .context("update_plan_status")?;
    emit_ok(ctx, &format!("Plan {} completed.", args.id))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_plan_id(s: &str) -> Result<PlanId> {
    PlanId::from_string(s.to_string())
        .map_err(|e| anyhow::anyhow!("invalid plan ID '{}': {}", s, e))
}

fn parse_plan_status(s: &str) -> Option<PlanStatus> {
    match s.to_lowercase().as_str() {
        "idea" => Some(PlanStatus::Idea),
        "scoped" => Some(PlanStatus::Scoped),
        "dispatched" => Some(PlanStatus::Dispatched),
        "blocked" => Some(PlanStatus::Blocked),
        "done" => Some(PlanStatus::Done),
        "parked" => Some(PlanStatus::Parked),
        _ => None,
    }
}

fn emit_ok(ctx: &Ctx, msg: &str) -> Result<i32> {
    match ctx.output {
        OutputMode::Json => json::render_ok(msg),
        OutputMode::Pretty => {
            if !ctx.quiet {
                pretty::ok(msg, ctx.color);
            }
        }
    }
    Ok(0)
}
