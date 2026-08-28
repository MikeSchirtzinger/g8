//! Write-side action handlers for the dashboard.
//!
//! These mirror the logic in `cmd/plan.rs` and `cmd/substrate.rs`, but operate
//! from a store path (not a CLI `Ctx`) and return JSON `Value`s instead of
//! rendering to stdout. Crucially, every mutation goes through `g8-store`'s
//! typed API — `update_plan_status` enforces the lifecycle state machine, so an
//! illegal transition requested from the UI fails the same way it would on the
//! CLI. WIP-cap enforcement on dispatch mirrors `plan_dispatch`.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use g8_core::{
    now_millis,
    plan::{PlanDraft, Recommendation},
    CapabilityId, OverlapKind, Plan, PlanId, PlanStatus,
};
use g8_store::{RusqliteStore, StoreConnection};

/// New-plan request body (`POST /api/plan`).
#[derive(Debug, Deserialize)]
pub struct NewPlanReq {
    pub title: String,
    #[serde(default)]
    pub substrate: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub touches: Vec<String>,
    /// "idea" (default) | "park" | "dispatch".
    #[serde(default)]
    pub mode: Option<String>,
}

/// Action request body (`POST /api/plan/<id>/<action>`).
#[derive(Debug, Deserialize, Default)]
pub struct ActionReq {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Add/update-substrate request body (`POST /api/substrate`).
#[derive(Debug, Deserialize)]
pub struct SubstrateReq {
    pub name: String,
    #[serde(default)]
    pub wip_cap: Option<u32>,
    #[serde(default)]
    pub stale_days: Option<u32>,
}

fn open(store_path: &Path) -> Result<RusqliteStore> {
    let mut store = RusqliteStore::open(store_path)
        .with_context(|| format!("opening store at {}", store_path.display()))?;
    store.migrate().context("running store migrations")?;
    Ok(store)
}

fn require_default_space(store: &RusqliteStore) -> Result<g8_core::ConvergenceSpace> {
    store
        .get_default_space()
        .context("get_default_space")?
        .ok_or_else(|| anyhow!("no space found: run `g8 init` first"))
}

/// `POST /api/plan` — create a plan after running the planner fit-check.
pub fn create_plan(store_path: &Path, req: NewPlanReq) -> Result<Value> {
    if req.title.trim().is_empty() {
        return Err(anyhow!("title is required"));
    }
    let mode = req.mode.as_deref().unwrap_or("idea");

    let store = open(store_path)?;
    let space = require_default_space(&store)?;

    let draft = PlanDraft {
        title: req.title.clone(),
        description: req.description.clone(),
        substrate: req.substrate.clone(),
        touched_capabilities: req.touches.clone(),
        touched_paths: vec![],
        space_id: space.id.clone(),
        project_id: None,
    };

    let report = g8_planner::Planner::plan_check(&store, &draft).context("running planner")?;

    // Dispatching directly requires a clean planner recommendation, same gate as
    // `g8 plan new --dispatch`.
    if mode == "dispatch" && report.recommendation != Recommendation::Proceed {
        return Ok(json!({
            "ok": false,
            "error": format!(
                "refusing to dispatch: planner recommendation is {:?}",
                report.recommendation
            ),
            "recommendation": format!("{:?}", report.recommendation),
            "report": report,
        }));
    }

    // Resolve touched capability names → IDs across all projects in the space.
    let projects = store.list_projects(&space.id).context("list_projects")?;
    let mut resolved: Vec<CapabilityId> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for raw in &req.touches {
        let canonical = store.resolve_canonical(raw).unwrap_or_else(|_| raw.clone());
        let mut hit = false;
        for project in &projects {
            if let Some(cap) = store
                .find_capability_by_name(&project.id, &canonical)
                .context("find_capability_by_name")?
            {
                resolved.push(cap.id);
                hit = true;
            }
        }
        if !hit {
            unresolved.push(raw.clone());
        }
    }

    let initial_status = match mode {
        "park" => PlanStatus::Parked,
        "dispatch" => PlanStatus::Dispatched,
        _ => PlanStatus::Idea,
    };

    let mut store_w = open(store_path)?;
    let plan = Plan {
        // serve is the sanctioned-randomness zone (Q4 rider): random plan
        // IDs are fine here and nanoid is already a serve-feature dependency.
        id: PlanId::from_string(nanoid::nanoid!()).expect("nanoid ids are non-empty"),
        space_id: space.id.clone(),
        project_id: None,
        title: req.title.clone(),
        description: req.description.clone(),
        status: initial_status,
        substrate: req.substrate.clone(),
        touched_capabilities: resolved.clone(),
        derived_from_intents: vec![],
        governed_by_decisions: vec![],
        wip_weight: 1,
        parked_reason: None,
        blocked_reason: None,
        dispatched_at: (mode == "dispatch").then(now_millis),
        completed_at: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store_w.create_plan(&plan).context("create_plan")?;
    for cap_id in &resolved {
        store_w
            .link_plan_capability(&plan.id, cap_id, OverlapKind::Touches)
            .context("link_plan_capability")?;
    }

    Ok(json!({
        "ok": true,
        "plan_id": plan.id.as_str(),
        "recommendation": format!("{:?}", report.recommendation),
        "unresolved_touches": unresolved,
        "report": report,
    }))
}

/// `POST /api/plan/<id>/<action>` — lifecycle transition.
pub fn plan_action(store_path: &Path, id: &str, action: &str, req: ActionReq) -> Result<Value> {
    let plan_id =
        PlanId::from_string(id.to_string()).map_err(|e| anyhow!("invalid plan id '{id}': {e}"))?;

    // Dispatch re-runs the planner to enforce the substrate WIP cap, exactly as
    // `g8 plan dispatch` does.
    if action == "dispatch" {
        let store = open(store_path)?;
        let plan = store
            .get_plan(&plan_id)
            .context("get_plan")?
            .ok_or_else(|| anyhow!("plan {id} not found"))?;
        if let Some(ref substrate) = plan.substrate {
            let draft = PlanDraft {
                title: plan.title.clone(),
                description: plan.description.clone(),
                substrate: Some(substrate.clone()),
                touched_capabilities: plan
                    .touched_capabilities
                    .iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                touched_paths: vec![],
                space_id: plan.space_id.clone(),
                project_id: plan.project_id.clone(),
            };
            let report =
                g8_planner::Planner::plan_check(&store, &draft).context("running planner")?;
            if let Some(b) = report.budget_status.as_ref() {
                if b.at_cap {
                    return Ok(json!({
                        "ok": false,
                        "error": format!(
                            "refusing to dispatch: substrate '{}' is at WIP cap ({}/{})",
                            b.substrate, b.wip_current, b.wip_cap
                        ),
                    }));
                }
            }
        }
    }

    let (new_status, reason): (PlanStatus, Option<String>) = match action {
        "scope" | "promote" => (PlanStatus::Scoped, None),
        "dispatch" | "unblock" => (PlanStatus::Dispatched, None),
        "block" => (PlanStatus::Blocked, req.reason),
        "complete" => (PlanStatus::Done, None),
        "park" => (PlanStatus::Parked, req.reason),
        other => return Err(anyhow!("unknown action '{other}'")),
    };

    let mut store = open(store_path)?;
    store
        .update_plan_status(&plan_id, new_status, reason)
        .with_context(|| format!("{action} plan {id}"))?;

    Ok(json!({ "ok": true, "id": id, "action": action, "new_status": format!("{new_status:?}") }))
}

/// `POST /api/substrate` — register/update a substrate WIP budget.
pub fn add_substrate(store_path: &Path, req: SubstrateReq) -> Result<Value> {
    if req.name.trim().is_empty() {
        return Err(anyhow!("substrate name is required"));
    }
    let mut store = open(store_path)?;
    let space = require_default_space(&store)?;
    let wip_cap = req.wip_cap.unwrap_or(g8_core::DEFAULT_WIP_CAP);
    let stale_days = req.stale_days.unwrap_or(14);
    store
        .set_substrate_budget(&space.id, &req.name, wip_cap, stale_days)
        .context("set_substrate_budget")?;
    Ok(json!({ "ok": true, "substrate": req.name, "wip_cap": wip_cap, "stale_days": stale_days }))
}
