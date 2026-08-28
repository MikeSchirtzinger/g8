//! Convergence-space graph model + read-only builder.
//!
//! The dashboard renders the whole `ConvergenceSpace` as a node-edge graph:
//! plans, capabilities, intents, decisions, projects, and the substrates that
//! group them. This module reads the store **read-only** (a dedicated
//! `OPEN_READ_ONLY` connection) and projects it into a serializable [`Graph`].
//!
//! Reads go direct-to-SQLite here for full control (the store trait exposes no
//! "list every intent in the space" verb). Writes never happen on this path —
//! mutations go through `g8-store`'s typed API in [`super::actions`], which
//! enforces the lifecycle state machine.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::Path;

/// The full convergence graph for one space.
#[derive(Debug, Serialize)]
pub struct Graph {
    pub generated_at: i64,
    pub space: SpaceInfo,
    pub stats: Stats,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct SpaceInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub plans: usize,
    pub capabilities: usize,
    pub intents: usize,
    pub decisions: usize,
    pub substrates: usize,
    pub projects: usize,
    /// Plan count keyed by status string (Idea/Scoped/Dispatched/Blocked/Done/Parked).
    pub plans_by_status: std::collections::BTreeMap<String, usize>,
    /// Substrate names whose WIP count (Dispatched + Blocked) is at/over cap.
    pub over_cap_substrates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Node {
    pub id: String,
    /// One of: plan | capability | intent | decision | substrate | project.
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substrate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Kind-specific fields for the detail panel (free-form object).
    pub detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    /// Relationship label: on | touches | extends | replaces | conflicts |
    /// satisfies | contradicts | derived_from | governed_by | violates |
    /// supersedes | in.
    pub kind: String,
}

/// Synthetic node id for a substrate (substrates are string labels, not rows).
fn substrate_id(name: &str) -> String {
    format!("substrate:{name}")
}

/// Legal next *actions* (endpoint names) for a plan in `status`. Mirrors
/// `g8_core::validate_status_transition`; used so the UI only offers buttons
/// that will succeed.
pub fn plan_actions(status: &str) -> Vec<&'static str> {
    match status {
        "Idea" => vec!["scope", "park"],
        "Scoped" => vec!["dispatch", "park"],
        "Dispatched" => vec!["block", "complete", "park"],
        "Blocked" => vec!["unblock", "park"],
        "Parked" => vec!["promote"],
        _ => vec![], // Done is terminal
    }
}

impl Graph {
    /// Build the graph for the default (earliest) space in the store at `db_path`.
    pub fn build(db_path: &Path, generated_at: i64) -> Result<Graph> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("opening store read-only at {}", db_path.display()))?;

        let (space_id, space_name) = conn
            .query_row(
                "SELECT id, name FROM convergence_space ORDER BY created_at ASC LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional_ctx("no convergence_space in store: run `g8 init` first")?;

        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut substrates: std::collections::BTreeMap<String, SubAccum> = Default::default();
        let mut plans_by_status: std::collections::BTreeMap<String, usize> = Default::default();

        // ── Projects ────────────────────────────────────────────────────────
        let mut project_count = 0usize;
        {
            let mut stmt = conn
                .prepare("SELECT id, name, language, root_path FROM project WHERE space_id = ?1")?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (id, name, language, root_path) = row?;
                project_count += 1;
                nodes.push(Node {
                    id: id.clone(),
                    kind: "project".into(),
                    label: name.clone(),
                    status: None,
                    substrate: None,
                    project: Some(name),
                    detail: serde_json::json!({ "language": language, "root_path": root_path }),
                });
            }
        }

        // ── Capabilities (project-scoped → join project for space filter) ────
        let mut capability_count = 0usize;
        {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.name, c.description, c.substrate, c.status, c.stub, \
                        c.file_path, c.line_number, c.project_id, p.name \
                   FROM capability c JOIN project p ON p.id = c.project_id \
                  WHERE p.space_id = ?1",
            )?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok(CapRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    substrate: r.get(3)?,
                    status: r.get(4)?,
                    stub: r.get::<_, i64>(5)? != 0,
                    file_path: r.get(6)?,
                    line_number: r.get(7)?,
                    project_id: r.get(8)?,
                    project_name: r.get(9)?,
                })
            })?;
            for row in rows {
                let c = row?;
                capability_count += 1;
                if let Some(ref s) = c.substrate {
                    substrates.entry(s.clone()).or_default();
                    edges.push(Edge {
                        source: c.id.clone(),
                        target: substrate_id(s),
                        kind: "on".into(),
                    });
                }
                edges.push(Edge {
                    source: c.id.clone(),
                    target: c.project_id.clone(),
                    kind: "in".into(),
                });
                let loc = match (c.file_path.as_ref(), c.line_number) {
                    (Some(f), Some(l)) => Some(format!("{f}:{l}")),
                    (Some(f), None) => Some(f.clone()),
                    _ => None,
                };
                nodes.push(Node {
                    id: c.id,
                    kind: "capability".into(),
                    label: c.name,
                    status: Some(c.status.clone()),
                    substrate: c.substrate,
                    project: Some(c.project_name),
                    detail: serde_json::json!({
                        "description": c.description,
                        "stub": c.stub,
                        "location": loc,
                    }),
                });
            }
        }

        // ── Intents ─────────────────────────────────────────────────────────
        let mut intent_count = 0usize;
        {
            let mut stmt = conn.prepare(
                "SELECT id, kind, heading, description, substrate, source_kind, \
                        source_file, source_line FROM intent WHERE space_id = ?1",
            )?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok(IntentRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    heading: r.get(2)?,
                    description: r.get(3)?,
                    substrate: r.get(4)?,
                    source_kind: r.get(5)?,
                    source_file: r.get(6)?,
                    source_line: r.get(7)?,
                })
            })?;
            for row in rows {
                let i = row?;
                intent_count += 1;
                if let Some(ref s) = i.substrate {
                    substrates.entry(s.clone()).or_default();
                    edges.push(Edge {
                        source: i.id.clone(),
                        target: substrate_id(s),
                        kind: "on".into(),
                    });
                }
                nodes.push(Node {
                    id: i.id,
                    kind: "intent".into(),
                    label: i.heading.clone(),
                    status: None,
                    substrate: i.substrate,
                    project: None,
                    detail: serde_json::json!({
                        "intent_kind": i.kind,
                        "heading": i.heading,
                        "description": i.description,
                        "source_kind": i.source_kind,
                        "source_file": i.source_file,
                        "source_line": i.source_line,
                    }),
                });
            }
        }

        // ── Decisions ───────────────────────────────────────────────────────
        let mut decision_count = 0usize;
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, status, context, source_file FROM decision WHERE space_id = ?1",
            )?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            for row in rows {
                let (id, title, status, context, source_file) = row?;
                decision_count += 1;
                nodes.push(Node {
                    id,
                    kind: "decision".into(),
                    label: title,
                    status: Some(status),
                    substrate: None,
                    project: None,
                    detail: serde_json::json!({ "context": context, "source_file": source_file }),
                });
            }
        }

        // ── Plans ───────────────────────────────────────────────────────────
        let mut plan_count = 0usize;
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, description, status, substrate, parked_reason, \
                        blocked_reason, dispatched_at, completed_at, created_at, updated_at \
                   FROM plan WHERE space_id = ?1",
            )?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok(PlanRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    description: r.get(2)?,
                    status: r.get(3)?,
                    substrate: r.get(4)?,
                    parked_reason: r.get(5)?,
                    blocked_reason: r.get(6)?,
                    dispatched_at: r.get(7)?,
                    completed_at: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                })
            })?;
            for row in rows {
                let p = row?;
                plan_count += 1;
                *plans_by_status.entry(p.status.clone()).or_default() += 1;
                if let Some(ref s) = p.substrate {
                    let acc = substrates.entry(s.clone()).or_default();
                    // WIP = Dispatched + Blocked (SPEC §7: Blocked occupies a slot).
                    if p.status == "Dispatched" || p.status == "Blocked" {
                        acc.wip_current += 1;
                    }
                    edges.push(Edge {
                        source: p.id.clone(),
                        target: substrate_id(s),
                        kind: "on".into(),
                    });
                }
                nodes.push(Node {
                    id: p.id,
                    kind: "plan".into(),
                    label: p.title,
                    status: Some(p.status.clone()),
                    substrate: p.substrate,
                    project: None,
                    detail: serde_json::json!({
                        "description": p.description,
                        "parked_reason": p.parked_reason,
                        "blocked_reason": p.blocked_reason,
                        "dispatched_at": p.dispatched_at,
                        "completed_at": p.completed_at,
                        "created_at": p.created_at,
                        "updated_at": p.updated_at,
                        "actions": plan_actions(&p.status),
                    }),
                });
            }
        }

        // ── plan → capability / intent / decision link edges ─────────────────
        link_edges(
            &conn,
            &space_id,
            "plan_capability",
            "capability_id",
            "overlap_kind",
            &mut edges,
        )?;
        link_edges(
            &conn,
            &space_id,
            "plan_intent",
            "intent_id",
            "relation",
            &mut edges,
        )?;
        link_edges(
            &conn,
            &space_id,
            "plan_decision",
            "decision_id",
            "relation",
            &mut edges,
        )?;

        // ── Substrate budgets → enrich accumulators + add substrate nodes ────
        {
            let mut stmt = conn.prepare(
                "SELECT substrate, wip_cap, stale_threshold_days FROM substrate_budget WHERE space_id = ?1",
            )?;
            let rows = stmt.query_map([&space_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (name, cap, stale) = row?;
                let acc = substrates.entry(name).or_default();
                acc.wip_cap = Some(cap);
                acc.stale_days = Some(stale);
            }
        }

        let mut over_cap_substrates = Vec::new();
        let substrate_count = substrates.len();
        const DEFAULT_WIP_CAP: i64 = 3;
        for (name, acc) in &substrates {
            let cap = acc.wip_cap.unwrap_or(DEFAULT_WIP_CAP);
            let at_cap = acc.wip_current as i64 >= cap;
            if at_cap {
                over_cap_substrates.push(name.clone());
            }
            nodes.push(Node {
                id: substrate_id(name),
                kind: "substrate".into(),
                label: name.clone(),
                status: None,
                substrate: Some(name.clone()),
                project: None,
                detail: serde_json::json!({
                    "wip_cap": cap,
                    "wip_current": acc.wip_current,
                    "at_cap": at_cap,
                    "stale_days": acc.stale_days.unwrap_or(14),
                    "explicit_budget": acc.wip_cap.is_some(),
                }),
            });
        }

        Ok(Graph {
            generated_at,
            space: SpaceInfo {
                id: space_id,
                name: space_name,
            },
            stats: Stats {
                plans: plan_count,
                capabilities: capability_count,
                intents: intent_count,
                decisions: decision_count,
                substrates: substrate_count,
                projects: project_count,
                plans_by_status,
                over_cap_substrates,
            },
            nodes,
            edges,
        })
    }
}

#[derive(Default)]
struct SubAccum {
    wip_current: usize,
    wip_cap: Option<i64>,
    stale_days: Option<i64>,
}

struct CapRow {
    id: String,
    name: String,
    description: Option<String>,
    substrate: Option<String>,
    status: String,
    stub: bool,
    file_path: Option<String>,
    line_number: Option<i64>,
    project_id: String,
    project_name: String,
}

struct IntentRow {
    id: String,
    kind: String,
    heading: String,
    description: String,
    substrate: Option<String>,
    source_kind: String,
    source_file: String,
    source_line: Option<i64>,
}

struct PlanRow {
    id: String,
    title: String,
    description: Option<String>,
    status: String,
    substrate: Option<String>,
    parked_reason: Option<String>,
    blocked_reason: Option<String>,
    dispatched_at: Option<i64>,
    completed_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

/// Emit `plan -> target` edges from a `plan_<x>` link table, scoped to the space
/// via a join back to `plan`. `relation_col` carries the edge label.
fn link_edges(
    conn: &Connection,
    space_id: &str,
    table: &str,
    target_col: &str,
    relation_col: &str,
    edges: &mut Vec<Edge>,
) -> Result<()> {
    let sql = format!(
        "SELECT l.plan_id, l.{target_col}, l.{relation_col} \
           FROM {table} l JOIN plan p ON p.id = l.plan_id \
          WHERE p.space_id = ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([space_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (plan_id, target, relation) = row?;
        edges.push(Edge {
            source: plan_id,
            target,
            kind: relation,
        });
    }
    Ok(())
}

/// Small ext-trait: turn an `Option`-yielding query_row into a contextful error.
trait OptionalCtx<T> {
    fn optional_ctx(self, msg: &'static str) -> Result<T>;
}
impl<T> OptionalCtx<T> for std::result::Result<T, rusqlite::Error> {
    fn optional_ctx(self, msg: &'static str) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(anyhow::anyhow!(msg)),
            Err(e) => Err(anyhow::Error::new(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g8_core::{
        now_millis, ConvergenceSpace, Plan, PlanId, PlanStatus, Project, ProjectId, SpaceId,
    };
    use g8_store::{RusqliteStore, StoreConnection};

    fn plan(space: &SpaceId, title: &str, status: PlanStatus, substrate: &str) -> Plan {
        Plan {
            id: PlanId::sequential_for_tests(),
            space_id: space.clone(),
            project_id: None,
            title: title.into(),
            description: None,
            status,
            substrate: Some(substrate.into()),
            touched_capabilities: vec![],
            derived_from_intents: vec![],
            governed_by_decisions: vec![],
            wip_weight: 1,
            parked_reason: None,
            blocked_reason: None,
            dispatched_at: None,
            completed_at: None,
            created_at: now_millis(),
            updated_at: now_millis(),
            meta: None,
        }
    }

    #[test]
    fn graph_substrate_wip_counts_blocked_and_flags_over_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("store.db");
        {
            let mut store = RusqliteStore::open(&db).unwrap();
            store.migrate().unwrap();
            let space = ConvergenceSpace {
                id: SpaceId::sequential_for_tests(),
                name: "test-space".into(),
                root_path: tmp.path().to_path_buf(),
                created_at: now_millis(),
                updated_at: now_millis(),
                members: vec![],
                meta: None,
            };
            store.init_space(&space).unwrap();
            store
                .upsert_project(&Project {
                    id: ProjectId::sequential_for_tests(),
                    space_id: space.id.clone(),
                    name: "proj".into(),
                    description: None,
                    root_path: tmp.path().to_path_buf(),
                    language: None,
                    created_at: now_millis(),
                    updated_at: now_millis(),
                    meta: None,
                })
                .unwrap();
            store
                .set_substrate_budget(&space.id, "auth", 2, 14)
                .unwrap();
            // 1 Dispatched + 1 Blocked on auth = 2 occupied slots = at cap.
            store
                .create_plan(&plan(&space.id, "a", PlanStatus::Dispatched, "auth"))
                .unwrap();
            store
                .create_plan(&plan(&space.id, "b", PlanStatus::Blocked, "auth"))
                .unwrap();
        } // drop writer before opening read-only

        let g = Graph::build(&db, now_millis().as_i64()).unwrap();

        assert_eq!(g.stats.plans, 2);
        let auth = g
            .nodes
            .iter()
            .find(|n| n.kind == "substrate" && n.label == "auth")
            .expect("auth substrate node");
        assert_eq!(
            auth.detail["wip_current"].as_i64(),
            Some(2),
            "Blocked must count toward WIP"
        );
        assert_eq!(auth.detail["at_cap"].as_bool(), Some(true));
        assert!(g.stats.over_cap_substrates.contains(&"auth".to_string()));
    }

    #[test]
    fn plan_actions_match_lifecycle() {
        assert_eq!(plan_actions("Idea"), vec!["scope", "park"]);
        assert_eq!(plan_actions("Blocked"), vec!["unblock", "park"]);
        assert!(plan_actions("Done").is_empty());
    }
}
