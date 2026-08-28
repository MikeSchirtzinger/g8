//! `g8 substrate {add,list}` — substrate vocabulary management.
//!
//! Implements the Vocabulary Council pattern: substrates are registered
//! deliberately rather than accumulating ad-hoc.

use anyhow::Context;
use anyhow::Result;

use g8_core::DEFAULT_WIP_CAP;
use g8_store::StoreConnection;

use crate::cli::{OutputMode, SubstrateAddArgs, SubstrateSubcommand};
use crate::ctx::{require_init, require_space, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, subcmd: &SubstrateSubcommand) -> Result<i32> {
    require_init(ctx)?;

    match subcmd {
        SubstrateSubcommand::Add(args) => substrate_add(ctx, args),
        SubstrateSubcommand::List => substrate_list(ctx),
    }
}

fn substrate_add(ctx: &Ctx, args: &SubstrateAddArgs) -> Result<i32> {
    let mut store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    let wip_cap = args.wip_cap.unwrap_or(DEFAULT_WIP_CAP);
    let stale_days = args.stale_days.unwrap_or(14);

    store
        .set_substrate_budget(&space.id, &args.name, wip_cap, stale_days)
        .context("set_substrate_budget")?;

    let msg = format!(
        "Substrate '{}' registered (wip_cap={}, stale_days={}).",
        args.name, wip_cap, stale_days
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

fn substrate_list(ctx: &Ctx) -> Result<i32> {
    let store = ctx.open_store()?;
    let space = require_space(&store, ctx.space_id())?;

    use g8_core::PlanStatus;
    use g8_store::PlanFilter;

    // Registered substrates come from substrate_budget. We also pull in any
    // substrates that show up on Dispatched plans but were never registered,
    // so listing surfaces drift between vocab and reality.
    let mut registered = store
        .list_substrate_budgets(&space.id)
        .context("list_substrate_budgets")?;

    let dispatched_plans = store
        .list_plans(
            &space.id,
            PlanFilter {
                status: Some(vec![PlanStatus::Dispatched]),
                ..Default::default()
            },
        )
        .unwrap_or_default();

    let registered_names: std::collections::HashSet<String> =
        registered.iter().map(|b| b.substrate.clone()).collect();

    for p in &dispatched_plans {
        if let Some(name) = p.substrate.as_ref() {
            if !registered_names.contains(name) {
                registered.push(g8_core::SubstrateBudget::default_for(name.clone()));
            }
        }
    }
    registered.sort_by(|a, b| a.substrate.cmp(&b.substrate));

    let active_count = |substrate: &str| -> usize {
        dispatched_plans
            .iter()
            .filter(|p| p.substrate.as_deref() == Some(substrate))
            .count()
    };

    match ctx.output {
        OutputMode::Json => {
            let rows: Vec<_> = registered
                .iter()
                .map(|b| {
                    let active = active_count(&b.substrate);
                    serde_json::json!({
                        "substrate": b.substrate,
                        "wip_cap": b.wip_cap,
                        "stale_threshold_days": b.stale_threshold_days,
                        "active": active,
                        "available": (b.wip_cap as i64) - (active as i64),
                    })
                })
                .collect();
            let value = serde_json::json!({
                "g8_version": crate::render::json::G8_VERSION,
                "substrates": rows,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("serializable")
            );
        }
        OutputMode::Pretty => {
            if registered.is_empty() {
                println!("No substrates registered. Use `g8 substrate add <NAME>`.");
            } else {
                println!(
                    "{:<20} | {:>7} | {:>6} | {:>9}",
                    "Substrate", "WIP Cap", "Active", "Available"
                );
                println!("{}", "-".repeat(50));
                for b in &registered {
                    let active = active_count(&b.substrate);
                    let available = (b.wip_cap as i64) - (active as i64);
                    println!(
                        "{:<20} | {:>7} | {:>6} | {:>9}",
                        b.substrate, b.wip_cap, active, available
                    );
                }
            }
        }
    }

    Ok(0)
}
