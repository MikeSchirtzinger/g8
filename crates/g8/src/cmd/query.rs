//! `g8 query <QUERY-DSL>` — ad-hoc queries via the minimal v0.1 DSL.
//!
//! Supported forms per ARCH §9.1:
//!   plans:in_flight
//!   plans:parked
//!   capabilities --substrate=<name>
//!   intents --path=<path>
//!   bottlenecks --substrate=<name>
//!   drift --substrate=<name>
//!   parked --like=<pattern>

use anyhow::{bail, Context, Result};

use g8_core::PlanStatus;
use g8_store::{PlanFilter, StoreConnection};

use crate::cli::{OutputMode, QueryArgs};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, args: &QueryArgs) -> Result<i32> {
    require_init(ctx)?;

    let store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    // Parse the DSL expression.
    let expr = args.query.trim();

    if expr.starts_with("plans:") {
        let status_str = expr.trim_start_matches("plans:");
        let status = parse_status(status_str)?;
        let filter = PlanFilter {
            status: Some(vec![status]),
            ..Default::default()
        };
        let plans = store.list_plans(&space.id, filter).context("list_plans")?;
        match ctx.output {
            OutputMode::Json => json::render_plan_list(&plans),
            OutputMode::Pretty => pretty::render_plan_list(&plans, ctx.color),
        }
        return Ok(0);
    }

    if expr.starts_with("capabilities") {
        let substrate = extract_flag(expr, "--substrate");
        let projects = store.list_projects(&space.id).context("list_projects")?;
        let mut caps = Vec::new();
        for project in &projects {
            let project_caps = store
                .list_capabilities(&project.id)
                .context("list_capabilities")?;
            caps.extend(project_caps);
        }
        if let Some(sub) = substrate {
            caps.retain(|c| c.substrate.as_deref() == Some(sub));
        }
        let value = serde_json::to_value(&caps).unwrap_or_default();
        match ctx.output {
            OutputMode::Json => json::render_value(&value),
            OutputMode::Pretty => println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            ),
        }
        return Ok(0);
    }

    if expr.starts_with("intents") {
        let path_str = extract_flag(expr, "--path");
        let path = std::path::Path::new(path_str.unwrap_or(""));
        let intents = store
            .list_intents_at_path(&space.id, path)
            .context("list_intents_at_path")?;
        let value = serde_json::to_value(&intents).unwrap_or_default();
        match ctx.output {
            OutputMode::Json => json::render_value(&value),
            OutputMode::Pretty => println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            ),
        }
        return Ok(0);
    }

    if expr.starts_with("bottlenecks") {
        let substrate = extract_flag(expr, "--substrate");
        let filter = PlanFilter {
            status: Some(vec![PlanStatus::Dispatched, PlanStatus::Blocked]),
            substrate: substrate.map(|s| s.to_string()),
            ..Default::default()
        };
        let plans = store.list_plans(&space.id, filter).context("list_plans")?;
        match ctx.output {
            OutputMode::Json => json::render_plan_list(&plans),
            OutputMode::Pretty => pretty::render_plan_list(&plans, ctx.color),
        }
        return Ok(0);
    }

    if expr.starts_with("drift") {
        let substrate = extract_flag(expr, "--substrate").map(|s| s.to_string());
        let stale = store
            .stale_dispatched_plans(&space.id)
            .context("stale_dispatched_plans")?;
        let filtered: Vec<_> = stale
            .into_iter()
            .filter(|s| substrate.is_none() || s.substrate.as_deref() == substrate.as_deref())
            .collect();
        let value = serde_json::to_value(&filtered).unwrap_or_default();
        match ctx.output {
            OutputMode::Json => json::render_value(&value),
            OutputMode::Pretty => println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            ),
        }
        return Ok(0);
    }

    if expr.starts_with("parked") {
        let like_pat = extract_flag(expr, "--like");
        let filter = PlanFilter {
            status: Some(vec![PlanStatus::Parked]),
            ..Default::default()
        };
        let mut plans = store.list_plans(&space.id, filter).context("list_plans")?;
        if let Some(pat) = like_pat {
            let pat_lower = pat.to_lowercase();
            plans.retain(|p| p.title.to_lowercase().contains(&pat_lower));
        }
        match ctx.output {
            OutputMode::Json => json::render_plan_list(&plans),
            OutputMode::Pretty => pretty::render_plan_list(&plans, ctx.color),
        }
        return Ok(0);
    }

    bail!(
        "Unknown query DSL expression: '{expr}'\n\
         Supported forms: plans:<status>, capabilities [--substrate=X], \
         intents [--path=X], bottlenecks [--substrate=X], \
         drift [--substrate=X], parked [--like=X]"
    )
}

fn parse_status(s: &str) -> Result<PlanStatus> {
    match s.to_lowercase().as_str() {
        "in_flight" | "inflight" | "dispatched" => Ok(PlanStatus::Dispatched),
        "parked" => Ok(PlanStatus::Parked),
        "idea" => Ok(PlanStatus::Idea),
        "scoped" => Ok(PlanStatus::Scoped),
        "blocked" => Ok(PlanStatus::Blocked),
        "done" => Ok(PlanStatus::Done),
        _ => bail!("unknown status '{s}'"),
    }
}

/// Extract a `--key=value` flag from a space-separated expression.
fn extract_flag<'a>(expr: &'a str, flag: &str) -> Option<&'a str> {
    let prefix = format!("{flag}=");
    expr.split_whitespace()
        .find(|t| t.starts_with(&prefix))
        .map(|t| t.trim_start_matches(&prefix))
}
